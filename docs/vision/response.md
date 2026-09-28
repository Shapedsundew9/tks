# Lead Developer Response: Technical Alignment (Iteration 6)

## Executive Summary & Stance

As Lead Developer Sub-Agent, I approach Iteration 6 with deep investment in the Knowledge Substrate's success and full alignment with the Project Initiator's guidance:
> *"Lets not go too much into specification that is the purpose of the next phase. If we understand the technologies, workflows and boundaries and scopes then the implementation team is capable of elaborating the final detail and will flag up if they need too. This is not a direction to stop iterating it is grounding in how much detail an architecture document should have."*

The architecture in `architecture.md` has matured significantly. Decisions D-42 through D-53 from Iteration 5 successfully addressed critical core risks (isolated Git actor thread, total event ordering sequence, draft edge supersession, and thin CLI proxies).

This review targets eight concrete architectural, lifecycle, and operational issues—anchored in system boundaries, state models, and deployment realities—along with three whole-component simplifications to reduce engineering overhead before coding begins.

---

## Architectural Findings

### LD-1

* **Severity:** Blocker
* **Target:** `architecture.md` §3 Invariant INV-6 (and §1 Context & Drivers DR-6)
* **Critique:** Invariant INV-6 specifies: *"Once foundational ingestion and context retrieval are operational (post-Phase 1), all new feature requirements, architectural adjustments, and development tasks must be authored, reviewed, and tracked within the substrate itself."*
  This directly conflicts with governing reference `vision.md` (§3, §5 Invariant I-6) and `strategic-planning-backlog.md` (§2 Phased Execution Roadmap, §3 Bootstrap Phasing Plan). Phase 1 is explicitly scoped as a **read-only** context retrieval gateway and document ingestion pipeline; mutation tools (`propose_node_mutation`), draft lifecycle handling, and per-node governance are Phase 2 deliverables.
  Requiring all development tasks and architectural adjustments to be authored and tracked within the substrate immediately post-Phase 1 imposes an impossible exit gate: the engineering team cannot dogfood mutation tools that do not yet exist, stalling progress at Gate 1 unless Phase 2 write capabilities are prematurely pulled into Phase 1.
* **Proposed Alternative:** Align Invariant INV-6 strictly with `vision.md` Invariant I-6 and the Strategic Backlog:
  1. **Phase 1 Dogfooding Gate (Read-Only Self-Hosting):** Upon completing Phase 1, the project's own documentation (`specs/vision.md`, `specs/strategic-planning-backlog.md`) is ingested into the Git-backed store and decomposed into the graph; external agents and human engineers retrieve bounded context envelopes via read-only MCP (`get_context_envelope`, `query_requirements`) to implement Phase 2 tasks.
  2. **Phase 2 Dogfooding Gate (Autonomous Self-Evolution):** Upon completing Phase 2 mutation tooling, draft lifecycle handling, and governance filters, all subsequent feature requirements, architectural adjustments, and development tasks must be authored, reviewed, and tracked directly within the substrate itself.

---

### LD-2

* **Severity:** Major
* **Target:** `architecture.md` §5.1 Draft Lifecycle and Event Compaction (Squash on Approval), §9 Decision D-42
* **Critique:** Decision D-42 states: *"On document revision, use `doc_path` to correlate prior active requirement nodes, diff CommonMark AST structural blocks, and re-anchor source byte offsets without generating duplicate requirement trees."*
  However, the state model in §5.1 provides no mechanism to achieve this. When an updated specification is ingested at `doc_path`:
  1. Candidate chunks are inserted into `graph_nodes` as new rows with `lifecycle_state = 'DRAFT'`.
  2. During staging approval (`POST /api/v1/staging/approve`), the engine promotes the candidate draft nodes to `ACTIVE` and deletes unapproved draft nodes belonging to the current `job_id`.
  3. The previously active nodes originating from earlier revisions of that `doc_path` are never modified or superseded; they remain in `lifecycle_state = 'ACTIVE'`.
  Consequently, every re-ingestion of a revised document doubles the active requirements in the graph. `query_requirements` and `get_context_envelope` return duplicate requirements from old and new commits, while existing execution tasks remained anchored to stale active nodes.
* **Proposed Alternative:** Formally specify document revision reconciliation and supersession in §5.1:
  1. **Span-Only Re-anchoring:** If an ingested chunk's content matches an existing active node at `doc_path` and only byte offsets shifted, update the existing node's `source_spans` record with the new `doc_hash` and byte offsets without creating a replacement node.
  2. **Node Replacement and Supersession:** When candidate draft nodes representing modified requirements are approved in `POST /api/v1/staging/approve`, the transaction must query existing active nodes anchored to that `doc_path` (via `source_spans`), transition replaced nodes to `lifecycle_state = 'SUPERSEDED'`, supersede their conflicting active edges, and trigger an automated dependency sweep setting active children to `NEEDS_REVERIFICATION`.
  3. **Removed Sections:** Any active nodes previously anchored to `doc_path` that are missing from the approved revision must be transitioned to `SUPERSEDED` (or flagged for supervisory confirmation).

---

### LD-3

* **Severity:** Major
* **Target:** `architecture.md` §7 Technology Stack, §8 Operational Model, §9 Decision D-53
* **Critique:** Decision D-53 and §7 specify that all operational CLI commands, explicitly including `tks migrate`, must operate strictly as thin HTTP clients targeting the local `tks serve` REST API (`http://localhost:8080/api/v1/...`) to avoid database client dependencies in the CLI binary.
  This creates a fatal circular dependency during deployment and automated testing:
  1. `tks serve` cannot boot its connection pool, initialize queries against `agent_identities`, or start successfully against an empty, unmigrated PostgreSQL database.
  2. However, under D-53, `tks migrate` cannot run without `tks serve` already listening on HTTP, because `tks migrate` is a thin HTTP proxy to `tks serve`.
  In Docker Compose manifests, Kubernetes init containers, and CI pipelines, database schema migrations must execute *before* application daemons start, not as an HTTP request to a running server.
* **Proposed Alternative:** Amend Decision D-53 and the operational model:
  1. Restrict thin HTTP client proxying to runtime operational subcommands (`tks mcp-stdio`, `tks staging`, `tks identity`).
  2. Treat database migration as a bootstrap lifecycle responsibility: configure `tks serve` to automatically execute pending embedded migrations on startup (via `refinery::embed_migrations!`) before binding network listeners and starting background workers.
  3. Retain a standalone direct-database migration command (`tks db-migrate`) for offline CI/CD environments that connects directly to `$DATABASE_URL` and terminates immediately upon completion.

---

### LD-4

* **Severity:** Major
* **Target:** `architecture.md` §5.1 State Ownership, §5.2 Concurrency Model, and `strategic-planning-backlog.md` §7 Sequence Diagram (Step 428)
* **Critique:** The architecture exhibits an operational contradiction regarding leaf attribute mutations on active versus draft entities:
  * Sequence diagram step 428 in `strategic-planning-backlog.md` indicates that for `AUTONOMOUS_ELABORATION`, leaf attribute mutations execute: `Update node & append ephemeral patch to draft_revisions`.
  * However, §5.1 defines `draft_revisions` as: *"Ephemeral table tracking intermediate edits and patches to nodes in `DRAFT` state; purged upon approval squashing."*
  If an `ACTIVE` node's attributes are updated and appended to `draft_revisions`, that update will **never** be squashed into `audit_ledger` because `POST /api/v1/staging/approve` is never executed on an entity that is already active! The patch sits in `draft_revisions` indefinitely, violating Invariant INV-2 (requiring all approved mutations to be recorded in `audit_ledger`).
  Conversely, permitting agents to directly mutate normative requirement text in `graph_nodes` without draft staging violates the Project Initiator's constraint for strict status distinction between "Approved / Locked" and "Draft / Editable".
* **Proposed Alternative:** Disambiguate the mutation pathways in §5.1 and §5.2:
  1. **Draft Entities:** Ephemeral `draft_revisions` applies strictly to entities with `lifecycle_state = 'DRAFT'`. Intermediate edits mutate `graph_nodes` current state and append patches to `draft_revisions`, squashing into `audit_ledger` upon approval.
  2. **Active Execution Nodes:** Authorized operational updates to active execution entities (e.g. updating an assigned `TASK` node's status to `COMPLETED`) lock the row via `SELECT ... FOR UPDATE`, apply the update, and write a discrete, reversible state transition event directly to `audit_ledger` with monotonic `event_seq`. They never touch `draft_revisions`.
  3. **Active Normative Nodes:** An external agent cannot rewrite the normative content or title of an `ACTIVE` requirement or specification in-place. Such alterations must be submitted as candidate `DRAFT` nodes referencing the parent/target node, requiring supervisory approval.

---

### LD-5

* **Severity:** Major
* **Target:** `architecture.md` §5.1 Entity Typing and Relational Constraints, §6 Interfaces & Contracts
* **Critique:** Throughout `architecture.md` §10 (R-1), `strategic-planning-backlog.md` §4, §5 (Spikes 0, 5), and `vision.md`, requirements, specifications, and tasks are referenced using human-readable canonical identifiers (e.g., `REQ-002`, `REQ-005`, `Task-101`, `INV-1`). Furthermore, TB-2 specifies a deterministic lexical scanner extracting these exact tags (`REQ-*`, `INV-*`, `DR-*`).
  However, in §5.1, `graph_nodes.id` is defined strictly as `UUID PRIMARY KEY DEFAULT gen_random_uuid()`, with no string tag or key column on `graph_nodes`.
  If an agent or human passes `"REQ-002"` to `get_context_envelope(target_node_id)` or `propose_node_mutation`, PostgreSQL rejects the query with: `invalid input syntax for type uuid: "REQ-002"`. Furthermore, without a canonical key, re-ingesting a document cannot match revised AST chunks to existing graph nodes.
* **Proposed Alternative:** Add an optional `node_key VARCHAR(64)` column to `graph_nodes` with a partial unique index on active entities:

  ```sql
  ALTER TABLE graph_nodes ADD COLUMN node_key VARCHAR(64);
  CREATE UNIQUE INDEX idx_graph_nodes_node_key_active
      ON graph_nodes (node_key)
      WHERE lifecycle_state = 'ACTIVE' AND node_key IS NOT NULL;
  ```

  Update `get_context_envelope` and `query_requirements` to accept either UUID or string `node_key` (`WHERE id = $1::uuid OR node_key = $1`). Store extracted tags (e.g., `REQ-001`, `INV-4`) in `node_key` during CommonMark decomposition.

---

### LD-6

* **Severity:** Minor
* **Target:** `architecture.md` §5.1 Entity Typing and Relational Constraints
* **Critique:** In §5.1 line 218, `node_embeddings` is defined as:
  `node_id UUID PRIMARY KEY`, `embedding vector`, `content_hash VARCHAR(64) NOT NULL`, `updated_at TIMESTAMPTZ NOT NULL`.
  `node_id` lacks a foreign key constraint referencing `graph_nodes(id) ON DELETE CASCADE`. When candidate draft nodes or discarded graph nodes are purged, their corresponding rows in `node_embeddings` remain orphaned. Furthermore, when an active node is superseded or reverted, retaining its vector embedding causes approximate nearest-neighbor vector search in `get_context_envelope` to match obsolete requirements.
* **Proposed Alternative:**
  1. Add `REFERENCES graph_nodes(id) ON DELETE CASCADE` to `node_embeddings.node_id`.
  2. Specify that when an active node transitions to `SUPERSEDED` or `REVERTED`, its vector embedding is deleted from `node_embeddings`:

     ```sql
     DELETE FROM node_embeddings WHERE node_id = ANY($superseded_node_ids);
     ```

  This guarantees `node_embeddings` contains strictly active requirements, eliminates orphan vector bloat, and prevents outdated requirements from leaking into agent prompt contexts.

---

### LD-7

* **Severity:** Minor
* **Target:** `architecture.md` §5.1 Entity Typing and Relational Constraints, §5.2 Concurrency Model
* **Critique:** The full-text search index on `graph_nodes` is defined as:
  `CREATE INDEX idx_graph_nodes_search_tsv ON graph_nodes USING gin(search_tsv);`
  This indexes all nodes across all lifecycle states (`DRAFT`, `ACTIVE`, `SUPERSEDED`, `ARCHIVED`, `NEEDS_REVERIFICATION`).
  Decision D-35 and C-14 mandate that `query_requirements` queries strictly active requirements. Indexing drafts and superseded entities in a single global GIN index causes unnecessary index write amplification during draft editing and AST decomposition, and forces PostgreSQL to scan and discard non-active index entries on every requirement query.
* **Proposed Alternative:** Restrict the GIN search index to active entities via a partial index:

  ```sql
  CREATE INDEX idx_graph_nodes_search_tsv
      ON graph_nodes USING gin(search_tsv)
      WHERE lifecycle_state = 'ACTIVE';
  ```

  This eliminates index maintenance overhead during candidate draft generation and ensures sub-5ms search execution.

---

### LD-8

* **Severity:** Minor
* **Target:** `architecture.md` §4 Component Topology, §8 Operational Model (Failure & Recovery), `technical-backlog.md` TB-1
* **Critique:** Section 8 states: *"Dedicated Git actor task is supervised inside `tks serve`; upon thread panic, the supervisor respawns the actor thread with a fresh `git2::Repository` handle on the bare repository volume."*
  In Tokio/Rust, if a background thread exclusively owning a `tokio::sync::mpsc::Receiver<GitCommand>` panics, the channel is closed. If a supervisor spawns a new thread with a fresh channel, all existing `mpsc::Sender` clones held by Axum request handlers and WorkerManager become disconnected, causing subsequent Git commits and reads to fail permanently with `SendError`.
* **Proposed Alternative:** Clarify in §8 and TB-1 that the Git actor thread must encapsulate its sequential execution loop in an internal panic recovery handler (e.g. `std::panic::catch_unwind`), or the supervisor must retain the `Receiver` across thread respawns, ensuring that caller `Sender` channels remain intact across transient Git C-binding failures.

---

## Architectural Simplifications

The following three consolidations remove unnecessary components and indirection without sacrificing any vision invariants:

### LD-9: Consolidate Ephemeral `draft_revisions` Table into `graph_nodes.attributes` JSONB

* **Severity:** Simplification
* **Target:** `architecture.md` §4 Component Topology, §5.1 State Ownership, §9 Decision D-25
* **Critique:** Decision D-25 created a standalone relational table `draft_revisions` (`revision_id`, `node_id`, `actor_id`, `patch`, `created_at`) solely to log intermediate edits while a requirement is in draft, aggregating them into a `draft_evolution_summary` JSONB upon approval and then deleting the rows. Maintaining a dedicated database table with foreign keys, indexes, and write transactions for ephemeral micro-edits adds schema complexity and creates architectural confusion regarding whether active nodes log to it.
* **Proposed Alternative:** Eliminate the `draft_revisions` table entirely. Store intermediate draft revision history directly inside `graph_nodes.attributes->'draft_revisions'` as an array of compact JSON objects while the entity is in `DRAFT` state. Upon staging approval (`POST /api/v1/staging/approve`), the engine reads `attributes->'draft_revisions'`, writes the canonical `APPROVED` event to `audit_ledger`, and updates the node to `ACTIVE`. If a draft is rejected or purged, no secondary table cleanup is required. This removes a whole database table and simplifies transactional logic.

---

### LD-10: Auto-Apply Migrations on Daemon Startup and Eliminate HTTP Migration Proxy

* **Severity:** Simplification
* **Target:** `architecture.md` §4 Component Topology, §7 Technology Stack, §9 Decision D-53
* **Critique:** Decision D-53 forced `tks migrate` to operate as a thin HTTP client calling `tks serve`, requiring a custom REST migration endpoint and producing the deployment deadlock identified in LD-3.
* **Proposed Alternative:** Eliminate the HTTP migration endpoint and CLI proxy. Embed database migrations directly in `tks serve` using `refinery::embed_migrations!`. When `tks serve` starts, it automatically applies pending migrations against PostgreSQL before initializing Axum routes and launching the background workers. For automated CI/CD checks, provide a simple standalone CLI flag `tks serve --migrate-only` that executes pending migrations and exits immediately with code 0. This removes an unnecessary network protocol abstraction and guarantees zero deployment desynchronization.

---

### LD-11: Ancestor Embedding Fallback for Newly Elaborated Tasks in `get_context_envelope`

* **Severity:** Simplification
* **Target:** `technical-backlog.md` TB-6, `architecture.md` §5.2 Concurrency Model
* **Critique:** Decision D-44 restricts `embedding_queue` insertion strictly to approved active nodes. Consequently, when an agent claims a newly created execution task (`Task-101`) and immediately requests a context envelope (`get_context_envelope`), `Task-101` will never have an embedding in `node_embeddings`. Under TB-6, vector search is completely skipped and falls back to pure graph traversal, meaning newly elaborated sub-tasks can never retrieve relevant cross-cutting non-functional governance constraints.
* **Proposed Alternative:** Refine query vector resolution in `src/storage/envelope.rs`: if `target_node_id` lacks an embedding record in `node_embeddings`, resolve the reference query vector from its immediate parent requirement node (`SELECT embedding FROM node_embeddings WHERE node_id = (SELECT to_node_id FROM graph_edges WHERE from_node_id = $1 AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY') LIMIT 1)`). If the parent also has no embedding, fallback cleanly to pure graph topology. This allows newly created tasks to immediately benefit from vector-enriched cross-cutting constraints without requiring synchronous embedding calculation.

---

## Conclusion & Next Steps

The architecture is in excellent health. Addressing the blocker on INV-6 (aligning post-Phase 1 dogfooding with read-only retrieval), resolving document re-ingestion supersession (LD-2), fixing the migration bootstrap deadlock (LD-3), and incorporating the proposed simplifications (LD-9 through LD-11) will ground the architecture firmly for immediate Phase 0/Phase 1 implementation.
