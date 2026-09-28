# Lead Developer Alignment Response (Iteration 5)

## Executive Summary & Engineering Disposition

As Lead Developer Sub-Agent, I have reviewed the Iteration 5 architecture specification (`architecture.md`), governing vision (`vision.md`), strategic planning backlog (`strategic-planning-backlog.md`), and technical implementation backlog (`technical-backlog.md`).

The architecture has matured substantially through prior iterations: consolidating staging into draft graph topology (D-23), eliminating destructive edge deletions with surrogate keys (D-32), introducing mechanical AST parsing (D-15), standardizing on 0-based byte offsets (D-22), and establishing native full-text search for micro-reflex queries (D-35) have created a coherent, buildable technical foundation.

However, moving into tactical execution exposes critical implementation frictions and operational gaps that must be resolved prior to Phase 1 coding. Most urgently:

1. Document ingestion and Git commit-tree reachability lack document path and slug identifiers, preventing Git tree generation, breaking `git log`/`git diff`, and making span re-anchoring across document revisions impossible (**LD-1**, Blocker).
2. Blocking synchronous libgit2 C calls wrapped in an in-process async mutex risk starving the Tokio reactor and introduce severe Rust concurrency impedance mismatches (**LD-2**, Major).
3. Candidate draft nodes currently enqueue embedding generation tasks immediately upon ingestion, burning external LLM tokens and API calls on false-positive drafts that are never queried (**LD-3**, Major).
4. The append-only `audit_ledger` lacks a concrete DDL schema and a monotonically increasing sequence primary key, jeopardizing total event ordering and point-in-time reconstruction (**LD-4**, Major).
5. Granular staging approvals leak orphaned draft nodes and leave ingestion jobs in an inconsistent operational state (**LD-5**, Major).
6. Draft edge promotion lacks active endpoint validation and active edge supersession mechanics, causing fatal partial unique index violations (**LD-6**, Major).
7. The `embedding_queue` lacks a retry scheduling column, causing worker tight-loop API hammering on HTTP 429 rate limits (**LD-7**, Major).
8. Identity CLI commands operate directly on PostgreSQL, causing desynchronization with the daemon's in-process `moka` auth cache (**LD-8**, Major).

Below are the 10 prioritized technical findings (ranked by severity) followed by 3 concrete component simplifications.

---

## Detailed Findings

### LD-1

* **Severity:** Blocker
* **Target:** `architecture.md` §5.1 State Ownership, §6 Interfaces & Contracts; `strategic-planning-backlog.md` §7 Operational Workflow Specifications
* **Critique:** Missing Document Path/Slug in Ingestion API and Data Model Breaks Git Tree Structure, Commit History (`git log`/`git diff`), and Deterministic Span Re-Anchoring.
  In `POST /api/v1/documents/ingest` and the `ingestion_jobs` schema, documents are identified exclusively by cryptographic hash (`job_id`, `document_hash`, `status`, `error_message`, `retry_count`, `created_at`, `updated_at`). No document path, slug, or filename is required or stored.
  This introduces three fatal operational blockers:
  1. *Git Tree Object Generation Failure:* In TB-1 and Decision D-33, the Git adapter must "update the tree structure in the ODB to include the new or updated document blob alongside existing document entries". A Git tree object cannot store bare, nameless blobs; every Git tree entry requires a path filename (e.g. `specs/vision.md`). Without a document path, libgit2 cannot construct tree objects or commit trees.
  2. *Destruction of Document Revision History:* Decision D-29 explicitly justifies standard Git branch commits on `refs/heads/specs` so that "developers can inspect document revision history using standard Git CLI commands (`git log`, `git diff`)". Without a stable file path across commits, standard Git history tracking is completely broken because each revision appears as an unrelated anonymous blob rather than an evolving document.
  3. *Impossibility of Span Re-Anchoring on Revision:* In §5.1, `source_spans` are stated to be "re-anchored on document revision" (and de-risked under R-6 / CAL-H4). When an edited document is ingested, its content hash changes completely. Without a stable `doc_path` or document entity identifier linking the new ingestion job to the previous document version, the decomposition pipeline cannot locate prior requirement nodes to diff AST blocks or re-anchor byte offsets. Every ingestion upload is treated as a set of brand-new, unrelated candidate nodes, causing runaway duplicate requirement graphs on every edit.
* **Proposed Alternative:**
  1. Update `POST /api/v1/documents/ingest` payload to mandate a document path/slug: `{"doc_path": "specs/vision.md", "content": "# Technical Vision..."}`.
  2. Add `doc_path VARCHAR(255) NOT NULL` to `ingestion_jobs` and `source_spans`.
  3. In `src/storage/git/`, use `doc_path` when creating the Git tree object so that successive uploads update the file at `doc_path` on `refs/heads/specs`. Standard `git log specs/vision.md` and `git diff HEAD~1 specs/vision.md` will function out-of-the-box.
  4. When an ingestion job executes for an existing `doc_path`, the decomposition pipeline retrieves prior active nodes anchored to that `doc_path`, correlates AST structural blocks and RFC 2119 keywords, re-anchors source byte spans, and marks modified nodes as revisions rather than generating duplicate trees.

### LD-2

* **Severity:** Major
* **Target:** `architecture.md` §4 Component Topology, §5.2 Concurrency Model; `technical-backlog.md` TB-1
* **Critique:** Premature In-Place Execution of Synchronous Blocking libgit2 C Calls on Tokio Async Worker Threads.
  Decision D-33 and TB-1 encapsulate bare Git operations in an `Arc<tokio::sync::Mutex<git2::Repository>>` inside the `tks serve` daemon.
  In implementation, this presents severe concurrency and runtime hazards:
  1. *Async Reactor Starvation:* `git2::Repository` wraps synchronous, blocking C library calls (`git_blob_create_from_buffer`, `git_treebuilder_write`, `git_commit_create`) that perform synchronous filesystem disk I/O. Executing blocking C code directly inside Tokio worker threads starves Tokio's async reactor.
  2. *Tokio Mutex / Spawn-Blocking Impedance Mismatch:* In Rust, `tokio::task::spawn_blocking` requires a synchronous closure (`FnOnce() -> R + Send + 'static`). You cannot `.await` an async `tokio::sync::Mutex` inside `spawn_blocking`. Conversely, acquiring the mutex outside `spawn_blocking` and attempting to pass the guard into `spawn_blocking` violates Rust's `Send` bounds.
  3. *Unsafe C-Pointer Sharing:* `git2::Repository` does not implement `Sync`, making shared mutex access across Tokio worker threads brittle.
* **Proposed Alternative:**
  Decouple Git operations using a dedicated background Git actor task communicating over a bounded mpsc channel (`tokio::sync::mpsc::channel`). The dedicated actor thread exclusively owns the `git2::Repository` instance, executes blocking libgit2 C calls sequentially off the async runtime, and returns results via `tokio::sync::oneshot` channels. This guarantees strict linear serialization on `refs/heads/specs` without `.lock` collisions, eliminates mutex contention across Axum handlers, prevents Tokio worker starvation, and cleanly isolates unsafe C-pointer handling from async request pipelines.

### LD-3

* **Severity:** Major
* **Target:** `architecture.md` §4 Component Topology, §5.1 State Ownership, §5.2 Concurrency Model
* **Critique:** Premature Enqueueing of Unapproved Draft Nodes in `embedding_queue` Violates Token Minimization and Wastes External API Compute.
  In §5.2 Concurrency Model (line 366), the specification states: "When a node is created or its text modified, an entry is upserted into `embedding_queue` within the same transaction."
  When a document is ingested, 30–60 candidate nodes are created in `graph_nodes` with `lifecycle_state = 'DRAFT'`. If `embedding_queue` is populated on draft creation, the background worker immediately calls external embedding APIs for unverified candidate chunks. Many of these chunks are false positives (revision tables, document headers, non-normative prose) that human supervisors reject during staging review (`tks staging reject`).
  Furthermore, production context queries (`get_context_envelope` and `query_requirements`) filter strictly on `lifecycle_state = 'ACTIVE'`, meaning draft node embeddings are never queried. Generating embeddings for draft nodes wastes LLM tokens, incurs unnecessary cloud API costs, creates rate-limit contention, and directly violates Project Initiator Constraint §2.1 ("minimal LLM reliance"). It also directly contradicts §4 line 164 and §5.1 line 215, which state embeddings are created on node approval.
* **Proposed Alternative:**
  Explicitly mandate that `embedding_queue` records are enqueued ONLY when a node transitions to `lifecycle_state = 'ACTIVE'` (during staging approval in `POST /api/v1/staging/approve` or draft promotion) or when an already `ACTIVE` node's content is modified. Nodes created in `DRAFT` state must never insert rows into `embedding_queue`.

### LD-4

* **Severity:** Major
* **Target:** `architecture.md` §3 Architectural Invariants (INV-2), §5.1 State Ownership, §9 Decision D-4
* **Critique:** Unspecified Relational Schema and Missing Monotonically Increasing Sequence in `audit_ledger`.
  The append-only audit ledger is the cornerstone of Invariant INV-2, Invariant INV-7, and Decision D-4. However, `architecture.md` provides no DDL or column definitions for `audit_ledger`.
  Crucially:
  1. *Total Ordering & Snapshot Replay:* Invariant INV-2 specifies a "monotonically increasing audit ledger for approved states" enabling point-in-time reconstruction. Relying on `TIMESTAMPTZ` alone is vulnerable to clock skew and sub-millisecond timestamp collisions during atomic batch approvals, making deterministic event replay impossible. Total ordering requires a monotonic 64-bit integer sequence (`event_seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY`).
  2. *Referential Integrity Isolation:* The schema must not use cascading foreign keys (`REFERENCES graph_nodes(id) ON DELETE CASCADE`) to live graph tables, as physical deletion of draft or reverted nodes would destroy historical audit logs.
  3. *Missing Field Specifications:* Fields for `event_seq`, `event_type` (`APPROVED`, `MUTATED`, `SUPERSEDED`, `REVERTED`), `entity_id UUID NOT NULL`, `entity_type VARCHAR(32) NOT NULL`, `actor_id VARCHAR(64) NOT NULL`, `actor_type VARCHAR(16) NOT NULL`, `token_fingerprint VARCHAR(64) NOT NULL`, `delta JSONB NOT NULL`, `snapshot JSONB NOT NULL`, `draft_evolution_summary JSONB`, and `created_at TIMESTAMPTZ NOT NULL` must be explicitly specified before Phase 1 implementation begins.
* **Proposed Alternative:**
  Add the complete SQL schema definition for `audit_ledger` to §5.1:

  ```sql
  CREATE TABLE audit_ledger (
      event_seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
      event_id UUID NOT NULL DEFAULT gen_random_uuid(),
      event_type VARCHAR(32) NOT NULL,
      entity_id UUID NOT NULL,
      entity_type VARCHAR(32) NOT NULL,
      actor_id VARCHAR(64) NOT NULL,
      actor_type VARCHAR(16) NOT NULL,
      token_fingerprint VARCHAR(64) NOT NULL,
      delta JSONB NOT NULL,
      snapshot JSONB NOT NULL,
      draft_evolution_summary JSONB,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
  );
  CREATE INDEX idx_audit_ledger_entity ON audit_ledger(entity_id, event_seq);
  CREATE INDEX idx_audit_ledger_created_at ON audit_ledger(created_at);
  ```

### LD-5

* **Severity:** Major
* **Target:** `architecture.md` §5.1 State Ownership, §6 Interfaces & Contracts, §9 Decisions D-27, D-36, D-41
* **Critique:** Granular Staging Approval Leaks Orphan Draft Nodes and Leaves `ingestion_jobs` in an Inconsistent State.
  Decision D-27 and D-36 allow supervisors to execute granular approval: `POST /api/v1/staging/approve` accepts an optional `approved_node_ids: Vec<Uuid>` list (CLI `tks staging approve <job_id> --only <id1,id2>`).
  However, §6 and D-36 specify that upon approval, `ingestion_jobs.status` transitions from `STAGED` to `APPROVED`.
  When a supervisor approves only a subset, the candidate draft nodes that were NOT included in `approved_node_ids` remain in `graph_nodes` with `lifecycle_state = 'DRAFT'`. But because the job status is now `APPROVED`, these leftover draft nodes:
  1. Disappear from `tks staging list <job_id>` (which filters on `STAGED`).
  2. Cannot be rejected or purged via `tks staging reject --job-id <job_id>` because the job is marked `APPROVED`.
  3. Remain permanently trapped as orphaned drafts in `graph_nodes`.
* **Proposed Alternative:**
  Specify explicit atomic purging for granular approval:
  When `approved_node_ids` is supplied in `POST /api/v1/staging/approve` (or `--only` / `--exclude` in CLI), any candidate draft nodes linked to `job_id` that are NOT in `approved_node_ids` must be atomically deleted in the same transaction:
  `DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT' AND id != ALL($approved_node_ids);`
  This ensures that staging approval is a clean, definitive promotion gate that eliminates all rejected false positives and transitions the job to `APPROVED` without orphan draft residue.

### LD-6

* **Severity:** Major
* **Target:** `architecture.md` §5.1 State Ownership, §9 Decisions D-21, D-32, D-34
* **Critique:** Draft Edge Promotion Lacks Dual-Endpoint Active Validation and Active Edge Supersession Mechanics.
  In §5.1 line 306, the approval transaction transitions approved nodes to `ACTIVE` "and associated edges to `ACTIVE`".
  However:
  1. *Dual-Endpoint Validation:* If `Node A` is approved, but connected candidate `Node B` is excluded or rejected, promoting edges connected to `Node A` would transition an edge to `ACTIVE` while one endpoint remains `DRAFT` or is deleted, violating graph integrity. An edge must transition from `DRAFT` to `ACTIVE` if and only if BOTH `from_node_id` AND `to_node_id` are in `ACTIVE` state (either already active or in `approved_node_ids`).
  2. *Active Edge Unique Constraint Violation:* `graph_edges` enforces `CREATE UNIQUE INDEX idx_graph_edges_active_unique ON graph_edges (from_node_id, to_node_id, edge_type) WHERE lifecycle_state = 'ACTIVE'`. If an agent proposes a new draft edge between two nodes that already have an existing active edge of that type (e.g. updating or replacing an edge), promoting the draft edge without first superseding the existing active edge triggers a fatal PostgreSQL unique constraint violation (`23505`).
* **Proposed Alternative:**
  Specify the exact edge transition logic during batch approval:
  1. Supersede existing active edges:
     `UPDATE graph_edges SET lifecycle_state = 'SUPERSEDED' WHERE lifecycle_state = 'ACTIVE' AND (from_node_id, to_node_id, edge_type) IN (SELECT from_node_id, to_node_id, edge_type FROM graph_edges WHERE lifecycle_state = 'DRAFT' AND from_node_id = ANY($all_active_ids) AND to_node_id = ANY($all_active_ids));`
  2. Promote candidate draft edges where both endpoints are active:
     `UPDATE graph_edges SET lifecycle_state = 'ACTIVE' WHERE lifecycle_state = 'DRAFT' AND from_node_id = ANY($all_active_ids) AND to_node_id = ANY($all_active_ids);`
  3. Purge orphaned draft edges where either endpoint was discarded:
     `DELETE FROM graph_edges WHERE lifecycle_state = 'DRAFT' AND (from_node_id = ANY($discarded_ids) OR to_node_id = ANY($discarded_ids));`

### LD-7

* **Severity:** Major
* **Target:** `architecture.md` §5.1 State Ownership, §6 Interfaces & Contracts, §9 Decisions D-18, D-26
* **Critique:** Missing Retry Scheduling Column (`scheduled_at`) in `embedding_queue` Causes Tight-Loop API Throttling on Rate Limits.
  Section 5.1 defines `embedding_queue` columns as `(node_id UUID PRIMARY KEY, content_hash VARCHAR(64) NOT NULL, status, retry_count, updated_at)`.
  Section 6 states: "Embedding queue jobs retry up to 5 times with backoff before flagging `FAILED`."
  However, without a `scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW()` column, worker queries using `FOR UPDATE SKIP LOCKED` cannot filter by backoff readiness. When an external embedding provider returns a rate limit (HTTP 429) or temporary outage (HTTP 503), the worker has nowhere to record the backoff delay. If the worker updates `retry_count` and leaves or resets `status = 'PENDING'`, the very next polling query tick (e.g. 500ms later) will re-select the exact same record, immediately re-hammering the throttled provider, exhausting the retry limit within seconds, and permanently failing embeddings.
* **Proposed Alternative:**
  Add `scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW()` to `embedding_queue` and create a partial index:
  `CREATE INDEX idx_embedding_queue_pending ON embedding_queue(scheduled_at) WHERE status = 'PENDING';`
  The worker claim query must filter `WHERE status = 'PENDING' AND scheduled_at <= NOW() ORDER BY scheduled_at LIMIT $1 FOR UPDATE SKIP LOCKED`. On transient failure, the worker updates `scheduled_at = NOW() + (INTERVAL '1 second' * POWER(2, retry_count))` and increments `retry_count`.

### LD-8

* **Severity:** Major
* **Target:** `architecture.md` §4 Component Topology, §8 Operational Model; `technical-backlog.md` TB-5
* **Critique:** In-Process `moka` LRU Cache Desynchronization When Identity CLI Operates Directly Against PostgreSQL.
  Section 4 line 176 and Decision D-39 specify that Axum Tower auth middleware validates credentials against `agent_identities` backed by an in-process `moka` LRU cache inside `tks serve`.
  However, TB-5 specifies `tks identity revoke <agent_id>` and `tks identity create` as CLI commands that connect directly to PostgreSQL.
  If an operator runs `tks identity revoke <agent_id>` from a terminal, the command updates the database row in PostgreSQL, but the running `tks serve` daemon is completely unaware of the change. The compromised agent's credentials remain cached in `tks serve`'s in-process `moka` cache and will continue to be authorized for mutations until the TTL expires (which could be minutes or hours). This breaks blast-radius security containment.
* **Proposed Alternative:**
  Align CLI identity commands with the client-daemon architecture:
  `tks identity create` and `tks identity revoke` must execute via REST endpoints on the running daemon (`POST /api/v1/identities` and `POST /api/v1/identities/{id}/revoke`), allowing `tks serve` to immediately invalidate its local `moka` cache entry upon revocation. If `tks serve` is offline, direct database mutation is permitted as a fallback, but the daemon must invalidate or check cache validity on startup.

### LD-9

* **Severity:** Minor
* **Target:** `architecture.md` §5.2 Concurrency Model, §9 Decision D-28
* **Critique:** Unspecified Query Vector Resolution in `get_context_envelope` Vector Neighbor Ranking.
  Decision D-28 specifies reserving up to 10 slots for vector neighbor retrieval in `get_context_envelope(target_node_id, depth)`. However, the API arguments include only `target_node_id`, and the architecture never specifies where the query vector comes from.
  In implementation, the query vector must be resolved from `node_embeddings` for `target_node_id`. If `target_node_id` does not yet have an embedding (because embeddings are generated asynchronously out-of-band), the vector neighbor query cannot execute.
* **Proposed Alternative:**
  Formally specify in `src/storage/envelope.rs` that the target node's embedding (`SELECT embedding FROM node_embeddings WHERE node_id = $1`) serves as the query vector for neighbor retrieval. If no embedding is found for `target_node_id`, the repository layer must gracefully skip vector search and allocate the full 40-node budget to the topological recursive CTE traversal.

### LD-10

* **Severity:** Minor
* **Target:** `architecture.md` §5.1 State Ownership (Edge relationship rules), §3 Invariant INV-1
* **Critique:** Ambiguous Directed Orientation of `CONSTRAINED_BY` Edges in Recursive Ancestor Traversal.
  Section 5.1 specifies:
  * `FULFILLS` edges originate at `TASK` or `SPECIFICATION` and terminate at `SPECIFICATION` or `REQUIREMENT` (pointing upward toward requirements).
  * `CONSTRAINED_BY` edges "connect nodes of compatible governance hierarchy."
  If `CONSTRAINED_BY` is oriented from Requirement to Specification (`Req -> Spec`), its direction opposes `FULFILLS` (`Task -> Spec`). A standard recursive CTE traversing directed edges upward toward root requirements (`from_node_id -> to_node_id`) will fail to navigate past `CONSTRAINED_BY` edges unless the query explicitly introduces bidirectional joins or case-switched traversal logic, complicating recursive CTEs and threatening SLA-1 (<50ms).
* **Proposed Alternative:**
  Formally standardize the orientation of all governance edges in §5.1: `CONSTRAINED_BY` must originate at the constrained entity and terminate at the governing constraint/requirement (`Spec -CONSTRAINED_BY-> Requirement`). This ensures that all traceability edges uniformly point upward toward governing requirements, allowing simple and fast unidirectional CTE recursive traversal: `JOIN graph_edges e ON e.from_node_id = curr.id AND e.lifecycle_state = 'ACTIVE'`.

---

## Architectural Simplifications

### LD-11

* **Target:** `architecture.md` §4 Component Topology, §5.1 State Ownership
* **Critique / Opportunity:** Redundant Architectural Boundary Between Governance Policy Engine and Storage Repository Layer.
  Section 4 depicts `GovEngine` ("Governance Policy Engine") as an independent middle-tier architectural component in "Core", separate from `StorageRepo` ("Storage Repository Layer") in "Storage". In reality, both components execute purely within the `tks serve` process and interact exclusively via PostgreSQL SQL statements (advisory locking, row-level locking, and transactional CTEs). Having a separate "Governance Engine" introduces conceptual indirection without runtime separation.
* **Proposed Alternative:**
  Consolidate `GovEngine` directly into the Storage Repository layer as `src/storage/mutation.rs` / `src/storage/governance.rs`. The unified storage layer exposes clean Rust methods (e.g. `TksStorage::propose_mutation`, `TksStorage::approve_staging`, `TksStorage::get_context_envelope`) managing both query assembly and transactional lock acquisition, eliminating redundant component boundaries.

### LD-12

* **Target:** `architecture.md` §4 Component Topology, §7 Technology Stack, §9 Decision D-40
* **Critique / Opportunity:** Elimination of Dual `LISTEN/NOTIFY` and Polling Abstractions in Cooperative Worker Manager in Favor of Pure `FOR UPDATE SKIP LOCKED`.
  Decision D-40 and §7 suggest that the `WorkerManager` can use either cooperative polling or PostgreSQL `LISTEN/NOTIFY` triggers. Adding `LISTEN/NOTIFY` requires maintaining dedicated, non-pooled connection state, handling reconnects, managing connection state across dropped notifications, and adding trigger DDL to PostgreSQL tables. Given that Phase 1 and 2 operate on commodity infrastructure with low-frequency job submissions (document uploads and approval batches), a simple, cooperative polling loop using standard pooled connections with `FOR UPDATE SKIP LOCKED` and backoff sleep is vastly simpler, completely reliable, uses standard `deadpool-postgres` connections, and eliminates bespoke database notification plumbing.
* **Proposed Alternative:**
  Standardize `WorkerManager` strictly on cooperative timer polling with `FOR UPDATE SKIP LOCKED` and exponential backoff on empty queue ticks (e.g. 500ms polling interval when active, sleeping up to 2s when idle). Explicitly eliminate `LISTEN/NOTIFY` from the architecture and technology stack.

### LD-13

* **Target:** `architecture.md` §4 Component Topology, §8 Operational Model; `technical-backlog.md` TB-5
* **Critique / Opportunity:** Unification of All CLI Operational Commands Through the Local Axum REST Gateway.
  Currently, CLI operations have fractured execution paths: `tks mcp-stdio` proxies JSON-RPC over HTTP/SSE, `tks staging` calls REST endpoints, but `tks identity` instantiates its own direct PostgreSQL connection. This creates code duplication (multiple CLI commands embedding database drivers vs. HTTP clients) and causes cache desynchronization.
* **Proposed Alternative:**
  Unify all CLI administrative and supervisory subcommands (`tks staging`, `tks identity`, `tks migrate`) to operate as thin HTTP clients targeting the local `tks serve` REST API (`http://localhost:8080/api/v1/...`). The CLI binary embeds only a lightweight HTTP client (`reqwest`), completely eliminating database client dependencies from the CLI subcommands and centralizing all caching, logging, and connection pooling in `tks serve`.
