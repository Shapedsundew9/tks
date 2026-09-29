# Technical Alignment & Architectural Review: Lead Developer Findings

## Executive Summary

As Lead Developer Sub-Agent, I have conducted an implementation-grounded review of `architecture.md`, `vision.md`, `strategic-planning-backlog.md`, and `technical-backlog.md`. The architectural consolidation achieved through iterations 1–6 (embedded migrations via `refinery`, unified Axum server with streaming stdio proxy, dedicated Git write actor, and draft event compaction) provides a robust foundation for Phase 1 construction.

However, several critical implementation frictions, schema contradictions, and concurrency hazards remain that would cause runtime crashes, deadlocks, or agent execution failure if coding commenced without remediation. Most notably:

1. **Graceful Degradation Crash (`LD-1`):** A hard check constraint on `graph_nodes.node_type` rejects the `'UNCLASSIFIED'` type emitted during offline/uncredentialed decomposition, crashing the pipeline on non-normative prose.
2. **Autonomous Elaboration Deadlock (`LD-2`):** Non-normative execution tasks elaborated by agents under `AUTONOMOUS_ELABORATION` nodes are quarantined in `DRAFT` state and invisible to `get_context_envelope`, requiring human CLI intervention before an agent can execute its own subtasks.
3. **Decomposition AST Hierarchy Edge Omission (`LD-3`):** Mechanical CommonMark AST decomposition specifies chunk extraction but omits hierarchical edge generation, causing Invariant INV-1 ancestor validation to abort all batch staging approvals containing specifications or tasks.
4. **Missing Batch Identifier in Audit Ledger (`LD-4`):** `audit_ledger` lacks a transaction/batch correlation key, making the specified `revert_mutation_batch` tool technically impossible to execute.
5. **Advisory/Row Lock Inversion Deadlock (`LD-5`):** `propose_node_mutation` locks rows before advisory locks, while staging approval locks advisory locks before rows, guaranteeing PostgreSQL deadlocks under concurrent multi-agent write traffic.

Below are the 10 prioritized findings (ranked by severity) followed by 3 architectural simplifications.

---

## Prioritized Findings (LD-1 to LD-10)

### LD-1

* **Severity:** Blocker
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints), §5.2 (Two-Stage Mechanical Ingestion), and Decision D-37; `strategic-planning-backlog.md` Spike 4
* **Critique:** Decision D-37, §5.2, and Spike 4 mandate graceful degradation for the Decomposition Worker: when external LLM credentials are not configured or external provider requests fail/timeout, Stage 1 mechanical extraction commits candidate chunks to `graph_nodes` as `DRAFT` with default typing (`node_type = 'REQUIREMENT'` for RFC 2119 matches, `'UNCLASSIFIED'` otherwise). However, the concrete DDL constraint defined in §5.1 restricts `node_type`:

  ```sql
  ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type
      CHECK (node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION'));
  ```

  Because `'UNCLASSIFIED'` is absent from `chk_node_type`, any document ingestion running in offline, uncredentialed, or CI mode (as required by C-9, C-11, and DR-11) that encounters narrative prose lacking RFC 2119 keywords will trigger a fatal PostgreSQL check constraint violation (`23514 check_violation`). This crashes the decomposition worker, rolls back the staging transaction, and transitions the job to `FAILED`, completely breaking graceful degradation and local developer workflows.
* **Proposed Alternative:** Adjust `chk_node_type` to permit `'UNCLASSIFIED'` exclusively while a node is in `DRAFT` lifecycle state, preventing unclassified nodes from ever transitioning to `ACTIVE` without prior classification:

  ```sql
  ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type
      CHECK (
          (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'UNCLASSIFIED'))
          OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION'))
      );
  ```

  This preserves domain integrity on active production requirements while enabling reliable offline mechanical decomposition.

### LD-2

* **Severity:** Blocker
* **Target:** `architecture.md` §3 (INV-1, INV-5), §5.1 (State Ownership & Disambiguated Mutation Pathways), §6 (Interfaces & Contracts: `get_context_envelope`), and Decision D-57
* **Critique:** Invariant INV-5 and `vision.md` Key Capability #5 grant external agents authority for `AUTONOMOUS_ELABORATION` on designated nodes. Decision D-57 disambiguated status updates on existing active tasks, but left task *creation* governed by default candidate draft staging (`lifecycle_state = 'DRAFT'`). Because production context queries (`get_context_envelope`) filter strictly on `lifecycle_state = 'ACTIVE'` (C-13, D-16, §6), an autonomous agent elaborating execution subtasks under an authorized node cannot retrieve context envelopes for its newly created subtasks (`target_node_id` is draft and excluded from queries). Furthermore, promoting draft subtasks to `ACTIVE` currently requires supervisor approval via `POST /api/v1/staging/approve`. This creates an operational deadlock: autonomous agents cannot proceed with executing their own elaborated subtasks without manual human CLI intervention at every step, reducing `AUTONOMOUS_ELABORATION` to `HUMAN_REVIEW_REQUIRED` and breaking the core agent execution loop.
* **Proposed Alternative:** Formally disambiguate task creation under `AUTONOMOUS_ELABORATION`:
  1. For non-normative execution tasks (`node_type = 'TASK'`) created under parent nodes with `governance_policy = 'AUTONOMOUS_ELABORATION'`, permit verified agents to create tasks directly in `ACTIVE` state with immediate commit to `audit_ledger` with monotonic `event_seq`, provided a transactional CTE validates an upward path to an active requirement (satisfying INV-1).
  2. Extend `get_context_envelope` in §6 with an optional `include_drafts: Option<bool>` parameter (default `false`). When `include_drafts = true`, the query traverses both active and draft entities authored by the caller, allowing agents to query context for speculative draft subtrees without leaking drafts into production agent sessions.

### LD-3

* **Severity:** Major
* **Target:** `architecture.md` §4 (Decomposition Pipeline), §5.1 (Draft Lifecycle), §5.2 (Two-Stage Mechanical Ingestion); `technical-backlog.md` TB-2
* **Critique:** Invariant INV-1 mandates that every active functional specification and implementation task maintain an upward directed edge path to an active requirement, verified during batch promotion (`POST /api/v1/staging/approve`). However, neither `architecture.md` nor `technical-backlog.md` TB-2 defines how mechanical CommonMark AST decomposition generates edges between extracted structural blocks. If an ingested document contains nested sections (e.g. H2 Requirement containing H3 Specification), Stage 1 extracts chunk nodes but defines no edge generation rule. When Stage 2 classifies an AST block as `SPECIFICATION` or `TASK`, it has no upward parent edge to its enclosing requirement heading. Consequently, the transactional ancestor CTE check during staging approval fails and rolls back the promotion transaction, preventing any document containing specifications or tasks from ever exiting staging.
* **Proposed Alternative:** Formally specify the AST Structural Hierarchy Edge Generation Rule in TB-2 and §5.2: Stage 1 streaming AST parsing must maintain an active heading stack (H1–H4); every extracted sub-block or child heading mechanically generates a directed upward structural edge (`DERIVED_FROM`) pointing from child to its immediate parent heading node (`child_id -DERIVED_FROM-> parent_id`). Formally define that the INV-1 ancestor validation CTE traverses `FULFILLS`, `CONSTRAINED_BY`, and `DERIVED_FROM` edges upward to identify active requirement roots.

### LD-4

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (Relational Constraints: audit_ledger DDL), §6 (Interfaces & Contracts: `revert_mutation_batch`), and Decision D-45
* **Critique:** Interface `revert_mutation_batch` (§6 line 464) and Phase 2 MVD (§4 line 294) mandate the ability to roll back an atomic mutation batch by `batch_id` (e.g., `revert_mutation_batch(batch_id: "BATCH-2026-001")`). However, the concrete DDL schema for `audit_ledger` (§5.1 line 294) contains no `batch_id` or transaction correlation column. Without a correlation identifier linking events committed in the same atomic transaction or staging promotion, `revert_mutation_batch` cannot query or identify the set of events to reverse, rendering batch-level rollbacks technically unexecutable.
* **Proposed Alternative:** Add `batch_id UUID NOT NULL DEFAULT gen_random_uuid()` to `audit_ledger` with index `CREATE INDEX idx_audit_ledger_batch ON audit_ledger(batch_id, event_seq)`. During batch staging approvals (`POST /api/v1/staging/approve`) and multi-node agent mutation transactions, assign a single consistent `batch_id` to all audit event records generated by that transaction, enabling atomic point-in-time batch reversibility.

### LD-5

* **Severity:** Major
* **Target:** `architecture.md` §5.2 (Concurrency Model) and Decisions D-20, D-30, D-38; `strategic-planning-backlog.md` §7 (Sequence Diagram)
* **Critique:** The architecture specifies global transaction advisory locking (`pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`) for structural mutations and staging approvals, and native row locking (`SELECT id FROM graph_nodes WHERE id = $1 FOR UPDATE`) for leaf attribute updates. However, the sequence diagram (§7 line 510–513) depicts `propose_node_mutation` acquiring a row lock to inspect node policy *before* acquiring the global advisory lock for edge creation. Conversely, staging approvals (`POST /api/v1/staging/approve`) acquire the global advisory lock *before* updating node rows. Under concurrent multi-agent workloads, this inverted lock acquisition order will trigger frequent PostgreSQL deadlock abort cascades (`40P01 deadlock_detected`).
* **Proposed Alternative:** Specify a strict, non-negotiable lock acquisition hierarchy in §5.2: Any transaction that may perform structural edge mutations, DAG cycle checks, batch staging approvals, or rollbacks must acquire the global transaction advisory lock `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` *before* acquiring any row-level locks on `graph_nodes` or `graph_edges`.

### LD-6

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (Document Revision Reconciliation) and Decisions D-46, D-55
* **Critique:** When a revised document is ingested at `doc_path`, a new ingestion job is created and candidate draft nodes are staged. Decision D-46 purges unapproved draft nodes scoped strictly to `job_id = $1` upon approval. If a user uploads revision B while revision A is still in `STAGED` state, or if a document revision is approved, candidate draft nodes from previous non-approved ingestion jobs for that same `doc_path` remain permanently orphaned in `graph_nodes` with `lifecycle_state = 'DRAFT'`. Because their job statuses were never resolved and D-46 only deletes drafts matching the current job's ID, obsolete draft subtrees linger indefinitely in the database, violating the core invariant of zero orphaned draft residue.
* **Proposed Alternative:** In §5.1 and D-55, specify that creating or approving an ingestion job for `doc_path` executes an automatic supersession sweep: all prior open ingestion jobs for that `doc_path` (`status IN ('QUEUED', 'PROCESSING', 'STAGED')`) transition to `SUPERSEDED`, and their associated draft nodes and draft edges are atomically purged (`DELETE FROM graph_nodes WHERE job_id = ANY($superseded_job_ids) AND lifecycle_state = 'DRAFT'`).

### LD-7

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints), §6 (Interfaces & Contracts: `query_requirements`), and Decisions D-35, D-60
* **Critique:** `query_requirements` is specified to execute PostgreSQL native full-text search via `websearch_to_tsquery('english', $query)` against `graph_nodes.search_tsv`. In PostgreSQL, `websearch_to_tsquery` interprets hyphens as boolean negation (`NOT`). When an agent queries standard requirement keys (e.g. `query_requirements("REQ-CORE-001")` or `"INV-2"`), the query is parsed into `req & !core & !001`, actively excluding documents that contain `CORE` and `001`. Furthermore, `node_key` is not included in the generated `search_tsv` expression (`to_tsvector('english', coalesce(title, '') || ' ' || coalesce(content, ''))`). Consequently, searching by canonical key via `query_requirements` returns empty sets or completely incorrect results.
* **Proposed Alternative:**
  1. Update the generated column DDL in §5.1 to include `node_key`:
     `search_tsv tsvector GENERATED ALWAYS AS (to_tsvector('english', coalesce(node_key, '') || ' ' || coalesce(title, '') || ' ' || coalesce(content, ''))) STORED;`
  2. In `src/storage/envelope.rs`, sanitize search queries: if the query string matches a canonical key prefix pattern (`[A-Za-z]+-[0-9]+`), combine full-text search with an exact/prefix match query (`WHERE node_key ILIKE $query || '%' OR search_tsv @@ plainto_tsquery('english', $query)`), preventing hyphen-negation syntax corruption.

### LD-8

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (State Ownership: source_spans), Decision D-55; `technical-backlog.md` TB-7
* **Critique:** The specification contains a three-way architectural contradiction regarding document spans: §5.1 (line 219) defines `source_spans` as "immutable after creation"; Decision D-55 (line 1055) states that revision re-anchoring "creates a new entry in `source_spans`"; while TB-7 (line 122) states that span re-anchoring "updates `source_spans` in-place". Because `source_spans` lacks a unique constraint on `node_id`, following D-55 causes `source_spans` to accumulate multiple rows per node across revisions. Standard queries joining `source_spans` on `node_id` (§5.1 line 404, staging inspection, and MCP `get_document_span`) will return duplicate rows and Cartesian products.
* **Proposed Alternative:** Resolve the contradiction in favor of in-place span re-anchoring: define `node_id UUID UNIQUE NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE` on `source_spans`. Each node maintains exactly one current document span mapping (`doc_path`, `doc_hash`, `byte_start`, `byte_end`). Historical spans for previous revisions are preserved in `audit_ledger` event snapshots, guaranteeing 1-to-1 relational joins and deterministic resolution for `get_document_span`.

### LD-9

* **Severity:** Minor
* **Target:** `architecture.md` §5.1 (Document Revision Reconciliation), Decision D-55; `technical-backlog.md` TB-7
* **Critique:** Decision D-55 and TB-7 specify AST diffing re-ingested documents against existing active nodes by matching `node_key` or content hash. However, `node_key` is optional, and when a section's text is modified, its content hash changes. Without an explicit `node_key`, the decomposition worker has no structural identifier to correlate which existing active node was modified versus deleted/added, causing modified sections to lose their existing node IDs, edge relationships, and child tasks.
* **Proposed Alternative:** In TB-2 and TB-7, mandate that Stage 1 CommonMark parsing generates a deterministic hierarchical heading anchor (e.g. `doc_path#heading-1/heading-2`) stored in `graph_nodes.attributes->'ast_anchor'`. During document revision reconciliation, match existing active nodes by `node_key` first, then by `ast_anchor`, and only then by content hash, ensuring stable node correlation even when prose text is revised.

### LD-10

* **Severity:** Minor
* **Target:** `architecture.md` §5.2 (Concurrency Model), Decisions D-20, D-38
* **Critique:** Decision D-38 mandates that staging approvals acquire `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` for the entire duration of the promotion transaction. If a batch contains 60 nodes, validating INV-1 ancestor paths, calculating draft compaction summaries, and preparing audit events while holding the global lock serializes all concurrent agent mutations across the entire system.
* **Proposed Alternative:** Separate transaction read-validation from topological write commit: perform Invariant INV-1 ancestor verification CTEs and draft summary aggregation under `READ COMMITTED` first, then acquire the global advisory lock immediately prior to updating `lifecycle_state` on edges and nodes, minimizing lock hold time to $<5\text{ ms}$.

---

## Architectural Simplifications (LD-11 to LD-13)

### LD-11

* **Severity:** Major (Simplification)
* **Target:** `architecture.md` §4 (Component Topology), §5.1 (State Ownership: source_spans DDL), §7 (Technology Stack)
* **Critique:** `source_spans` is modeled as a standalone relational table (`span_id`, `node_id`, `doc_path`, `doc_hash`, `byte_start`, `byte_end`, `created_at`). Every requirement node derived from an ingested document has at most one active source document span. Maintaining a separate relational table with its own primary key, foreign key cascade, and two indexes introduces relational join overhead (`LEFT JOIN source_spans`) on every inspection query, staging list, and MCP tool call, while creating foreign key lock overhead and potential 1-to-many cardinality bugs across document revisions.
* **Proposed Alternative:** Eliminate the `source_spans` table entirely. Embed source span coordinates directly as first-class columns on `graph_nodes`:
  `doc_path VARCHAR(255)`, `doc_hash VARCHAR(64)`, `byte_start INT`, `byte_end INT`.
  Add a partial index `CREATE INDEX idx_graph_nodes_doc_path ON graph_nodes(doc_path) WHERE doc_path IS NOT NULL;`.
  This consolidates relational storage, eliminates an entire table and join from the codebase, guarantees 1-to-1 zero-duplicate joins, simplifies in-place span re-anchoring, and reduces migration and query complexity.

### LD-12

* **Severity:** Major (Simplification)
* **Target:** `architecture.md` §4 (Dedicated Git Actor Task), §5.2 (Concurrency Model), §7 (Technology Stack); `technical-backlog.md` TB-1
* **Critique:** The Dedicated Git Actor Task was introduced to serialize *writes* (`CommitCommand`) on `refs/heads/specs` and isolate blocking C calls off Tokio async threads. However, §4 and the sequence diagram (§7 line 482) route *read* commands (`ReadBlobCommand`) through the single-threaded Git actor's `mpsc` queue as well. When multiple background workers or MCP tool calls (`get_document_span`) attempt to read source text, they queue behind sequential write commits, creating a needless serialization bottleneck on immutable ODB read operations.
* **Proposed Alternative:** Restrict the Dedicated Git Actor exclusively to serializing Git *commits* and tree updates (`refs/heads/specs` writes). For read-only blob extraction (`get_document_span`, worker decomposition), allow workers and gateway handlers to open read-only repository handles or direct ODB blob lookups via `tokio::task::spawn_blocking` using `git2::Odb::read`. Because Git ODB blobs are content-addressed and strictly immutable once written, concurrent reads never block, never contend on `.git/refs/...lock`, and should not be bottlenecked behind the single-threaded Git write actor queue.

### LD-13

* **Severity:** Minor (Simplification)
* **Target:** `architecture.md` §6 (Interfaces & Contracts), §7 (Technology Stack); `strategic-planning-backlog.md` Spike 6
* **Critique:** The architecture maintains two parallel administrative rollback interfaces: `revert_mutation_batch` (taking a `batch_id`) and `revert_agent_session` (taking `agent_instance_id` and timestamp), exposed over both MCP JSON-RPC and REST (`POST /api/v1/admin/revert-session`). Both endpoints execute the exact same underlying compensating transaction logic: acquiring the global structural advisory lock, computing inverse deltas, appending compensating `REVERT` events to `audit_ledger`, marking edges `SUPERSEDED`/`REVERTED`, and executing an automated dependency sweep marking dependent children as `NEEDS_REVERIFICATION`. Maintaining two separate endpoints, schemas, and CLI commands duplicates handler logic, validation rules, and testing surface area.
* **Proposed Alternative:** Consolidate into a single polymorphic administrative endpoint and MCP tool: `revert_mutations`, accepting a structured filter: `{ "batch_id": Option<Uuid>, "agent_id": Option<String>, "since": Option<DateTime>, "event_seq_range": Option<(u64, u64)> }`. This unifies rollback mechanics into a single repository method (`src/storage/mutation.rs::revert_mutations`), eliminates redundant interface contracts, and provides greater administrative flexibility with half the code.
