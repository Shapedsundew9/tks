# Lead Developer Architectural Review: TKS Technical Alignment

## Executive Summary

As Lead Developer, I have evaluated `/workspaces/tks/docs/vision/architecture.md` against `/workspaces/tks/docs/vision/vision.md` and `/workspaces/tks/docs/vision/strategic-planning-backlog.md`. The architectural thesis—anchoring external AI agents to a unified PostgreSQL property graph and Git document store—is sound, compelling, and well-aligned with our engineering goals.

However, the architecture contains critical implementation friction points, concurrency risks, and transport-level incompatibilities that will block construction and Day-2 operations if not resolved prior to implementation. Specifically, the process model creates immediate port collisions when agents spawn stdio sessions, database transaction isolation choices will induce severe serialization abort cascades under multi-agent workloads, relational constraints as specified will deadlock task insertion, and unmanaged in-process async tasks risk silent ingestion data loss.

Below are 9 prioritized findings (1 Blocker, 8 Major) followed by 3 whole-component architectural simplifications (LD-10 through LD-12).

---

## Detailed Findings

### LD-1

* Severity: Blocker
* Target: ## 4. Component Topology
* Critique: The architecture specifies a single Rust binary compiled from the `tks` crate that concurrently exposes both an MCP server over stdio transport and a REST API HTTP listener within one process. In real-world agent environments, external agent harnesses (Claude Code, Cursor, Windsurf) launch stdio MCP servers as dedicated child processes per session. When an agent launches `tks`, the process attempts to bind its HTTP listener to the configured REST port. Any concurrent agent session, secondary tooling process, or running background server will immediately fail with `EADDRINUSE` (port conflict), crashing the agent session. Furthermore, if `tks` is deployed as a daemon or container, external agents running in independent process spaces cannot attach to its stdio streams across process boundaries. Additionally, diagnostic logs emitted to `stdout` will corrupt JSON-RPC message framing, causing protocol parsing crashes in MCP clients.
* Proposed Alternative: Decouple the runtime transports:
  1. Implement MCP over HTTP/SSE (or stream transport) directly inside `axum` on the same listener port as the REST API (`/mcp/sse`), enabling multiple external agents to connect concurrently to a single running server daemon.
  2. For local stdio agent integration, provide an explicit CLI subcommand (e.g., `tks mcp-stdio`) that either proxies JSON-RPC over HTTP to the running daemon or runs in a dedicated headless stdio mode that does NOT bind the HTTP network port.
  3. Enforce via `tracing-subscriber` configuration that all logs and diagnostics write strictly to `stderr`, leaving `stdout` dedicated to JSON-RPC framing.

### LD-2

* Severity: Major
* Target: ## 3. Architectural Invariants
* Critique: Invariant INV-1 mandates that every functional specification, implementation task, and artifact reference must maintain a valid directed edge path terminating at an authorized requirement node, and specifies that orphan execution tasks must be rejected "at the database constraint level" via foreign keys and triggers. However, in a relational schema where nodes and edges reside in separate tables (`requirement_nodes` and `graph_edges`), an inserted node must exist before an edge referencing its ID can be inserted into `graph_edges` due to foreign key constraints (`from_node_id REFERENCES requirement_nodes(id)`). If an immediate `BEFORE INSERT` or `AFTER INSERT` trigger or constraint checks for an existing ancestor path in `graph_edges`, inserting any valid new task node will unconditionally fail because its linking edge cannot yet exist.
* Proposed Alternative: Explicitly specify that database-level enforcement of INV-1 utilizes PostgreSQL `CONSTRAINT TRIGGER ... DEFERRABLE INITIALLY DEFERRED` evaluated at transaction `COMMIT` time, or enforce relational integrity through a transactional mutation function/CTE that inserts both the node and its linking edge within the same atomic statement before validation triggers execute.

### LD-3

* Severity: Major
* Target: ## 5.2 Concurrency Model
* Critique: Section 5.2 mandates that all graph mutations execute within `SERIALIZABLE` or `REPEATABLE READ` transactions to prevent TOCTOU races on DAG cycle checks and governance policies. In PostgreSQL, Serializable Snapshot Isolation (SSI) tracks predicate locks (`SIREAD`). Recursive CTE traversals over `graph_edges` (used for cycle checks and context envelopes) cause predicate locks to escalate across entire table pages or relation locks. Under concurrent multi-agent mutation workloads, concurrent inserts and recursive checks on `graph_edges` will routinely produce `40001 serialization_failure` errors, triggering cascading transaction retries and severely violating the sub-50ms latency objective (SLA-1).
* Proposed Alternative: Replace blanket `SERIALIZABLE` isolation with PostgreSQL default `READ COMMITTED` paired with fine-grained application-level transaction locking: use transaction-scoped advisory locks (e.g. `pg_advisory_xact_lock(hashtext('graph_mutation'))` or hashed by root node ID) to serialize structural topological mutations and DAG cycle checks. Read-only context queries can then execute concurrently under `READ COMMITTED` without lock escalation or serialization aborts.

### LD-4

* Severity: Major
* Target: ## 5.1 State Ownership
* Critique: Section 6 specifies administrative rollback utilities (`revert_mutation_batch`, `revert_agent_session`) via "inverse events", but fails to specify rollback cascade mechanics. In a directed graph with strict orphan rejection (INV-1), reverting a batch that created a parent requirement or specification node will cause immediate foreign key constraint failures or orphan invariant violations if subsequent mutations by other agents have attached child tasks or constraints to those nodes. The architecture provides no mechanism for blast-radius containment, leaving it undefined whether child nodes are cascade-reverted, orphaned, or blocked from rolling back.
* Proposed Alternative: Specify the rollback cascade protocol:
  1. Reversion must be non-destructive (INV-2), appending compensating `REVERT` events to `audit_ledger` and transitioning live node states to `SUPERSEDED` or `REVERTED`.
  2. Implement an automated dependency sweep: when a node with active dependent children is reverted, the engine recursively transitions dependent child nodes to `NEEDS_REVERIFICATION` rather than attempting physical deletion or triggering foreign key failures.
  3. If cross-agent dependent tasks exist on a node targeted for rollback, require explicit administrative flags (`--cascade-invalidation` or `--force`) to confirm the operational blast radius.

### LD-5

* Severity: Major
* Target: ## 5.1 State Ownership
* Critique: Section 5.1 and Constraint C-5 mandate storing `governance_policy` and `lifecycle_state` inside untyped JSONB metadata on `requirement_nodes`. This introduces severe operational and performance liabilities on hot paths:
  1. `lifecycle_state` must be evaluated on every step of recursive CTE graph traversals (SLA-1 <50ms) to filter out `ARCHIVED` and `SUPERSEDED` nodes. Extracting JSONB fields (`attributes->>'lifecycle_state'`) inside recursive joins incurs JSON parsing overhead per row and defeats standard B-tree index scans.
  2. `governance_policy` governs every proposed mutation (INV-5). Storing it in JSONB bypasses PostgreSQL static type checking, inviting silent failure modes from subtle string typos (e.g. lowercase vs uppercase) without database-level `CHECK` constraints.
* Proposed Alternative: Promote `lifecycle_state` and `governance_policy` to strongly typed SQL columns (e.g. PostgreSQL `VARCHAR` with `CHECK` constraints or native `ENUM`s) directly on the node table, indexed with standard B-tree indices. Reserve JSONB exclusively for open-ended, domain-specific extensions.

### LD-6

* Severity: Major
* Target: ## 4. Component Topology
* Critique: Section 6 defines `POST /api/v1/documents/ingest` as executing "Synchronous Git commit; async LLM decomposition". Decomposing large technical documents via LLMs and performing deterministic character span re-anchoring is a long-running operation (15–90 seconds). Executing this as an unmanaged in-process async task (e.g., `tokio::spawn`) means that any service restart, process crash, or container redeployment will drop the task silently. The raw text remains committed in Git, but the staged requirement nodes are never created, leaving no persistent failure record or resumption path.
* Proposed Alternative: Implement a persistent job queue table in PostgreSQL (`ingestion_jobs` with states `QUEUED`, `PROCESSING`, `COMPLETED`, `FAILED`, `error_message`, `retry_count`). The ingest endpoint commits the Git artifact, inserts a queued job within the transaction, and returns `202 Accepted` with a `job_id`. A durable worker loop processes the queue with retry tracking, enabling clients and UI to inspect ingestion status via `GET /api/v1/documents/ingest/{job_id}`.

### LD-7

* Severity: Major
* Target: ## 3. Architectural Invariants
* Critique: The architecture creates an unworkable gap between its identity invariant and communication protocols:
  1. Invariant INV-7 and Section 4 require validating OAuth 2.1 / DPoP tokens on every request, but MCP stdio transport has no transport-level HTTP authorization header mechanism.
  2. The MCP tool schemas in Section 6 (`propose_node_mutation`, etc.) omit authentication arguments, leaving no mechanism for an agent to transmit its token over stdio.
  3. Open Question Q-6 is marked as a blocking open question for INV-7, stalling development.
  4. Mandating OAuth 2.1 + DPoP workload federation against an external IdP for Phase 1/Phase 2 severely over-engineers the auth stack for a solo developer / small team on commodity hardware, violating C-9.
* Proposed Alternative:
  1. Resolve Q-6 with a pragmatic two-tier authentication model: For Phase 1 and Phase 2, agents authenticate via pre-shared cryptographic API keys or HMAC bearer tokens mapped to an `agent_identities` table in PostgreSQL.
  2. For stdio MCP, pass the identity token via process environment variables (`TKS_AGENT_ID`, `TKS_AUTH_TOKEN`) at spawn time or via an optional `auth` parameter in tool payloads; for HTTP/SSE MCP and REST, pass `Authorization: Bearer <token>`.
  3. Record the resolved `agent_instance_id` and token fingerprint in `audit_ledger`. Defer external OAuth 2.1 / DPoP federation to Phase 4.

### LD-8

* Severity: Major
* Target: ## 4. Component Topology
* Critique: Section 4, 5.1, and 5.2 specify using `libgit2` (via `git2`) against a local filesystem repository. Using `libgit2` with a standard working directory and index causes severe concurrency issues: writing commits touches `.git/index`, causing lock collisions (`.git/index.lock`) under concurrent ingestions. Furthermore, hosting the Git repository on the local container filesystem contradicts the "stateless gateway" premise (C-3): if the container restarts or redeploys without persistent volume bindings, the Git document repository is destroyed while PostgreSQL retains references to missing blob hashes.
* Proposed Alternative:
  1. Specify that `git2` operates against a bare Git repository using direct low-level object-database (ODB) writes (`git_blob_create_from_buffer` and direct tree/commit writes) bypassing the Git index and working tree completely, eliminating lock contention.
  2. Define an explicit storage binding: the bare Git repository must reside on a dedicated persistent volume co-located with PostgreSQL, and document a backup/restore synchronization protocol between PostgreSQL and Git blob storage.

### LD-9

* Severity: Major
* Target: ## 5.1 State Ownership
* Critique: Section 5.1 defines the primary entity table as `requirement_nodes`, yet the system domain models requirements, specifications, implementation tasks, code references, and verification results. Storing all entity types in `requirement_nodes` without a typed discriminator column obscures domain boundaries and prevents relational constraint validation. For example, Invariant INV-1 requires verifying that a task node links to an authorized requirement node; if all entities are polymorphically dumped into `requirement_nodes` with JSONB attributes, enforcing edge-type endpoint validity requires expensive trigger logic rather than simple relational foreign keys and check constraints.
* Proposed Alternative: Rename `requirement_nodes` to `graph_nodes`. Add a first-class, indexed column `node_type VARCHAR NOT NULL` (with check constraints for `REQUIREMENT`, `SPECIFICATION`, `TASK`, `VERIFICATION`, `DECISION`). Structure table constraints and edge rules such that structural edge types (`FULFILLS`, `CONSTRAINED_BY`) enforce valid source-target node type pairings.

---

## Architectural Simplifications

### LD-10

* Severity: Major
* Target: ## 4. Component Topology
* Critique: Section 4 depicts `DAGValidator` as an independent domain engine component positioned between `GovEngine` and `AuditLedger`/`GraphTopo`. In reality, acyclicity validation is not an independent domain service—it is a graph integrity check that must execute within the exact same database transaction as the edge write. Having it as a separate component introduces an artificial layer of indirection and creates a false impression that acyclicity can be validated outside the database transaction context. If executed in the application layer outside a transaction lock, concurrent transactions can easily create cycles.
* Proposed Alternative: Consolidate `DAGValidator` into the PostgreSQL storage transaction layer via a transactional recursive CTE check or PostgreSQL trigger. Eliminate `DAGValidator` as a standalone component in the component topology diagram and table.

### LD-11

* Severity: Major
* Target: ## 4. Component Topology
* Critique: Section 4 and Section 5.1 define `StagingTable` (`staging_queue`) as a distinct PostgreSQL table separate from `GraphTopo`. When requirements are decomposed from documents, or when an agent mutation requires human review (`HUMAN_REVIEW_REQUIRED`), the candidate data is parked in `staging_queue` as serialized JSON payloads. This requires maintaining separate schemas, dual serialization/deserialization logic, and prevents visual graph tooling or topological queries from inspecting how staged nodes relate to the existing graph. Furthermore, approving a staged mutation requires an orchestrator to read from `staging_queue`, deserialize, and insert into `requirement_nodes`, `graph_edges`, and `audit_ledger`, creating unnecessary translation logic.
* Proposed Alternative: Consolidate the staging queue directly into `GraphTopo` by utilizing the node lifecycle status (`status: PENDING_STAGING`, `PENDING_REVIEW`, `ACTIVE`, `SUPERSEDED`, `ARCHIVED`). Draft nodes and edges are written directly to `graph_nodes` and `graph_edges` with status `PENDING_STAGING` or `PENDING_REVIEW`. They are excluded from operational context envelopes by default (`WHERE status = 'ACTIVE'`). Approval becomes a single, atomic status transition (`UPDATE ... SET status = 'ACTIVE'`) and audit event emission. This eliminates the `staging_queue` table, its dual schemas, and staging-to-graph translation code entirely.

### LD-12

* Severity: Major
* Target: ## 4. Component Topology
* Critique: The architecture posits the MCP Server (stdio) and REST API (HTTP) as two independent concurrent server components within the same process. This causes port binding collisions when agents spawn the binary for stdio, makes remote MCP access impossible, and doubles the protocol interface surface.
* Proposed Alternative: Consolidate the gateway into a single unified `Axum` service. Axum natively handles REST endpoints (`/api/v1/...`) and the MCP protocol over HTTP/SSE (`/mcp/sse` and `/mcp/messages`) on a single port. For local agent integration using stdio, implement a lightweight CLI subcommand (`tks mcp-stdio` or `tks client`) in the same binary that translates stdio JSON-RPC to the running Axum server (or runs an isolated local stdio loop without starting an HTTP listener). This consolidates the gateway into one unified HTTP runtime and eliminates process lifecycle conflicts.
