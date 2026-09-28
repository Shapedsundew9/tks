# Lead Developer Review: Architectural Alignment & Implementation Realities (Iteration 4)

## Executive Summary

As the Lead Developer Sub-Agent for Iteration 4, I have evaluated the updated architectural specifications (`architecture.md`), governing vision (`vision.md`), strategic planning backlog (`strategic-planning-backlog.md`), and technical implementation backlog (`technical-backlog.md`).

The architecture has matured substantially through prior iterations: the elimination of the disconnected staging queue table (`staging_queue`), the consolidation of candidate requirements directly into `graph_nodes` with `lifecycle_state = 'DRAFT'`, the standardization on 0-based byte offsets, and the adoption of native row-level locking (`SELECT ... FOR UPDATE`) for leaf attribute updates provide an exceptional, pragmatic foundation.

However, moving from architectural concept to executable code reveals two **Blockers** and six **Major** implementation friction points that must be rectified before engineering begins:

1. **Existential Data Loss Risk (Blocker):** The foreign key `job_id REFERENCES ingestion_jobs(job_id) ON DELETE CASCADE` on `graph_nodes` will cause routine operational job pruning to cascade-delete active, canonical production requirements.
2. **Constraint Collision on Edge Lifecycle (Blocker):** The composite primary key `PRIMARY KEY (from_node_id, to_node_id, edge_type)` on `graph_edges` prevents re-linking or evolving topological relationships once an edge has transitioned to `SUPERSEDED` or `REVERTED`, violating Invariant INV-2.
3. **Git Concurrency & Non-Fast-Forward Race Conditions (Major):** Committing directly to a single branch (`refs/heads/specs`) via direct ODB writes without in-process synchronization causes file lock contention (`GIT_ELOCKED`) and non-fast-forward push rejections under concurrent document ingestion calls.
4. **Batch Promotion Ancestor Validation Failure (Major):** Transactional CTE checks for Invariant INV-1 evaluate parent nodes as `DRAFT` during atomic batch approval, causing valid hierarchical specification trees to fail promotion and abort.
5. **Schema Disconnect on Source Spans (Major):** The inspection query joins `source_spans.node_id = graph_nodes.id`, but Table 5.1 omits `node_id` from `source_spans`.
6. **Undefined Search Strategy for `query_requirements` (Major):** Coupling `query_requirements` to synchronous external embedding generation would breach SLA-1 (<50ms) and introduce external API failure points on read operations.
7. **Incomplete Ingestion Job State Machine (Major):** The absence of a `STAGED` / `AWAITING_REVIEW` state prevents supervisory tools and CLI automation from distinguishing between in-progress decomposition and unapproved drafts.
8. **Lack of Graceful Degradation in Decomposition Worker (Major):** Ingestion jobs fail entirely if external LLM API credentials are not configured, breaking offline development and local test automation despite having 80%+ mechanical extraction.

Below are the 10 prioritized findings followed by 3 whole-component simplifications.

---

## Prioritized Findings (Ranked by Severity)

### LD-1

* **Severity:** `Blocker`
* **Target:** `architecture.md` §5.1 State Ownership (Table 5.1 & Relational Ingestion Unification)
* **Critique:** In Table 5.1 and Section 5.1 "Relational Ingestion Unification", the foreign key on `graph_nodes` is specified as:
  `job_id UUID REFERENCES ingestion_jobs(job_id) ON DELETE CASCADE`.
  While candidate draft nodes are initially linked to an ingestion job, once a supervisor approves them via `POST /api/v1/staging/approve`, they transition to `lifecycle_state = 'ACTIVE'` and become the permanent, canonical requirements of the project.
  In any production database, job queues like `ingestion_jobs` are transient operational tables that undergo periodic retention cleanup (e.g., pruning records older than 30 or 90 days). Under `ON DELETE CASCADE`, pruning or deleting an old ingestion job record will trigger PostgreSQL to cascade-delete all active requirements, specifications, and tasks originally ingested by that job.
  This introduces an existential operational risk of permanent data loss and directly violates Invariant INV-2 ("Destructive in-place updates on active requirements, specifications, and topological edges must not occur").
* **Proposed Alternative:**
  Change the foreign key constraint on `graph_nodes.job_id` from `ON DELETE CASCADE` to `ON DELETE SET NULL`.
  To cleanly remove false positives during staging review, `POST /api/v1/staging/reject` should execute an explicit statement: `DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT'`.
  Once nodes are promoted to `ACTIVE`, their relationship to `job_id` becomes non-cascading historical provenance, guaranteeing that operational maintenance on `ingestion_jobs` can never alter or destroy active graph entities.

### LD-2

* **Severity:** `Blocker`
* **Target:** `architecture.md` §5.1 Entity Typing and Relational Constraints (lines 236–247), §9 Decision D-21
* **Critique:** Section 5.1 defines the schema for `graph_edges` as:

  ```sql
  CREATE TABLE graph_edges (
      from_node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
      to_node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
      edge_type VARCHAR(32) NOT NULL,
      lifecycle_state VARCHAR(20) NOT NULL DEFAULT 'ACTIVE'
          CHECK (lifecycle_state IN ('DRAFT', 'ACTIVE', 'SUPERSEDED', 'REVERTED')),
      PRIMARY KEY (from_node_id, to_node_id, edge_type)
  );
  ```

  Invariant INV-2 explicitly mandates that structural edges are never physically deleted, instead transitioning through `lifecycle_state` (`DRAFT`, `ACTIVE`, `SUPERSEDED`, `REVERTED`).
  However, because the primary key is strictly composite on `(from_node_id, to_node_id, edge_type)`, the table cannot store more than one historical or lifecycle state record for any edge relationship.
  If an edge from node A to node B is marked `SUPERSEDED` or `REVERTED` (for instance, during a dependency refactor or administrative session rollback), and later a new draft or revised active edge of that type is proposed between A and B, PostgreSQL immediately throws a fatal primary key violation (`duplicate key value violates unique constraint "graph_edges_pkey"`).
  Furthermore, if in-place `UPDATE` on edges is prohibited by INV-2, transitioning an edge cannot even be performed by inserting an updated state row.
* **Proposed Alternative:**
  Replace the composite primary key on `graph_edges` with a dedicated surrogate key:
  `edge_id UUID PRIMARY KEY DEFAULT gen_random_uuid()`.
  Enforce active edge uniqueness using a partial unique index:

  ```sql
  CREATE UNIQUE INDEX idx_graph_edges_active_unique
      ON graph_edges (from_node_id, to_node_id, edge_type)
      WHERE lifecycle_state = 'ACTIVE';
  ```

  This guarantees that at most one active relationship of a given type exists between any two nodes while allowing historical `SUPERSEDED` and `REVERTED` edge records to coexist safely without primary key collisions, fully satisfying Invariant INV-2.

### LD-3

* **Severity:** `Major`
* **Target:** `architecture.md` §4 Component Topology, §5.2 Concurrency Model, §7 Technology Stack, §9 Decision D-29; `technical-backlog.md` TB-1
* **Critique:** Decision D-29 and TB-1 specify that raw specification documents are committed directly to a single dedicated branch (`refs/heads/specs`) in the bare Git repository via direct ODB writes using `git2`. The architecture asserts that this eliminates `.lock` contention and loose reference bloat.
  However, in Git and `libgit2`, updating a branch reference requires acquiring a reference lock file (`.git/refs/heads/specs.lock`).
  In a concurrent multi-threaded environment (such as an Axum server handling concurrent document uploads or an ingestion retry executing alongside a new upload), multiple worker tasks attempting to commit to `refs/heads/specs` concurrently will encounter:
  1. File lock collisions (`GIT_ELOCKED`) on `.git/refs/heads/specs.lock`.
  2. Non-fast-forward push rejections, because both threads construct their tree from the same parent commit, and the second commit will attempt to overwrite the first rather than building on it.
  3. In addition, the `git2::Repository` handle in Rust contains raw C pointers and does not implement `Sync`, preventing safe shared access across Tokio worker threads without synchronization.
* **Proposed Alternative:**
  Within `tks serve`, encapsulate bare Git operations inside an in-process serialized queue or `Arc<tokio::sync::Mutex<git2::Repository>>`.
  The commit routine must sequentially:
  1. Acquire the repository mutex.
  2. Resolve the latest commit head on `refs/heads/specs`.
  3. Update the tree structure in the ODB to include the new/updated document blob alongside existing document entries.
  4. Write the commit object with the current HEAD as parent.
  5. Advance `refs/heads/specs`.
  Update §5.2 Concurrency Model and `technical-backlog.md` TB-1 to document in-process synchronization for Git branch mutations.

### LD-4

* **Severity:** `Major`
* **Target:** `architecture.md` §3 Architectural Invariants (INV-1), §5.1 Draft Lifecycle & Event Compaction, §9 Decision D-24
* **Critique:** Invariant INV-1 requires active functional specifications and tasks to maintain a directed edge path terminating at an authorized active requirement node, verified via transactional CTE ancestor checks during draft promotion (`POST /api/v1/staging/approve`).
  When a Markdown document is ingested and decomposed, it typically yields a hierarchical tree containing top-level `REQUIREMENT` nodes, child `SPECIFICATION` nodes, and leaf `TASK` nodes, all initially staged with `lifecycle_state = 'DRAFT'`.
  When a supervisor approves the job, promoting the candidate batch together, a standard transactional CTE checking `WHERE parent.lifecycle_state = 'ACTIVE'` evaluates against the pre-update state where the parent requirement is still `DRAFT`. Consequently, the ancestor check fails for all child specifications and tasks in the batch, causing the approval transaction to roll back.
* **Proposed Alternative:**
  Formally specify the batch promotion CTE semantics in §3 and §5.1: The ancestor path validation CTE must evaluate child nodes against the union of currently `ACTIVE` requirement nodes and candidate requirement nodes included in the current promotion batch (`id = ANY($approved_node_ids)`).
  This ensures that hierarchical document trees can be approved atomically in a single transaction without false invariant violations.

### LD-5

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 State Ownership (Table 5.1 & Relational Ingestion Unification)
* **Critique:** In Section 5.1, the ingestion inspection query executes:
  `LEFT JOIN source_spans s ON s.node_id = n.id`
  However, Table 5.1 defines `source_spans` as:
  `source_spans table (doc_hash, byte_start INT NOT NULL, byte_end INT NOT NULL)`
  `node_id` is completely absent from the table schema in Table 5.1, and `graph_nodes` contains no foreign key referencing `source_spans`.
  Without `node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE` on `source_spans`, the document inspection join fails with `column s.node_id does not exist`, and Invariant INV-4's requirement to cryptographically link extracted requirements to source spans cannot be fulfilled relationally.
* **Proposed Alternative:**
  Update Table 5.1 to define the concrete relational schema for `source_spans`:

  ```sql
  CREATE TABLE source_spans (
      span_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
      node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
      doc_hash VARCHAR(64) NOT NULL,
      byte_start INT NOT NULL,
      byte_end INT NOT NULL,
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
  );
  CREATE INDEX idx_source_spans_node ON source_spans(node_id);
  ```

  This matches the relational join in Section 5.1 and guarantees instant sub-millisecond inspection joins.

### LD-6

* **Severity:** `Major`
* **Target:** `architecture.md` §6 Interfaces & Contracts, §7 Technology Stack; `strategic-planning-backlog.md` §6 SLA-1
* **Critique:** The read tool `query_requirements` is an essential Phase 1 MCP interface for agents to locate existing requirements. However, Section 6 specifies neither its search mechanism nor its performance bounds, stating only "Returns empty result set on no match; error on malformed query; filters on `lifecycle_state = 'ACTIVE'`".
  If `query_requirements` is implemented using vector similarity search, every inbound query requires an on-the-fly embedding calculation. Invoking an external embedding API (e.g. OpenAI) synchronously during an MCP request introduces a 150–500ms network round-trip, immediately breaching SLA-1 (<50ms micro-reflex latency), introducing rate-limit failure modes, and breaking offline developer workflows (violating C-9).
  Conversely, PostgreSQL native full-text search (`tsvector` / `websearch_to_tsquery`) executes in <5ms, requires zero external network calls, zero API tokens, and functions fully offline.
* **Proposed Alternative:**
  Formally specify that `query_requirements` operates via PostgreSQL native full-text search using a generated `tsvector` column and GIN index on `graph_nodes(title, content)`.
  Vector similarity search in `node_embeddings` is reserved strictly for context envelope neighbor enrichment where node embeddings have already been generated asynchronously out-of-band (per D-18 and D-28).

### LD-7

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 State Ownership (Table 5.1), §6 Interfaces & Contracts; `strategic-planning-backlog.md` §2 Phase 1
* **Critique:** In Table 5.1 and Section 6, the lifecycle states for `ingestion_jobs` are defined strictly as:
  `QUEUED` → `PROCESSING` → `COMPLETED` or `FAILED`.
  When the decomposition worker finishes Stage 1 AST parsing and Stage 2 classification, candidate nodes are inserted into `graph_nodes` with `lifecycle_state = 'DRAFT'`.
  Transitioning the job status directly to `COMPLETED` at this stage causes two operational breakdowns:
  1. An external client or automated script polling `GET /api/v1/documents/ingest/{job_id}` sees `COMPLETED` and cannot distinguish whether the candidate requirements are awaiting human supervisory sign-off or are already approved and active.
  2. The supervisory CLI command `tks staging list` cannot query `ingestion_jobs` for jobs currently pending human review without scanning all jobs or executing expensive joins across `graph_nodes`.
* **Proposed Alternative:**
  Expand the `ingestion_jobs.status` state machine to:
  `QUEUED` → `PROCESSING` → `STAGED` (decomposition finished, candidates ready for review) → `APPROVED` (candidates promoted to `ACTIVE`) or `REJECTED` (all candidates discarded), with `FAILED` for unrecoverable errors.
  This establishes an unambiguous operational contract across the API, CLI, and supervisory portal.

### LD-8

* **Severity:** `Major`
* **Target:** `architecture.md` §4 Component Topology, §5.2 Concurrency Model, §7 Technology Stack, §9 Decision D-15
* **Critique:** Decision D-15 establishes that Stage 1 CommonMark AST parsing (`pulldown-cmark`) mechanically extracts $\ge 80\%$ of requirement chunks and RFC 2119 keywords without LLM tokens, while Stage 2 invokes an external LLM solely to classify ambiguous fragments into compact tuples.
  However, the operational specification dictates that any Stage 2 failure transitions the entire ingestion job to `FAILED`.
  If a developer or CI pipeline runs `tks serve` without an external LLM API key configured (or when the external API returns 429 rate-limit or 503 outage errors), document ingestion fails entirely—preventing even the mechanical 80% extraction from reaching `DRAFT` status.
  This creates unnecessary onboarding friction and violates the "start small" / constrained resource model (C-9).
* **Proposed Alternative:**
  Specify graceful degradation in the Decomposition Worker: If external LLM API credentials are not configured or the provider request fails/times out, Stage 1 mechanical extraction still commits candidate chunks to `graph_nodes` as `DRAFT` requirements with default typing (`node_type = 'REQUIREMENT'` for RFC 2119 matches, or `'UNCLASSIFIED'`), recording a warning in `ingestion_jobs.error_message`.
  Supervisors can then adjust types during staging review (`tks staging approve`), ensuring full local utility without mandatory external API keys.

### LD-9

* **Severity:** `Minor`
* **Target:** `architecture.md` §5.2 Concurrency Model (Topological Mutations & Global Advisory Locking), §6 Interfaces & Contracts
* **Critique:** Section 5.2 states that structural mutations (`propose_node_mutation`, edge creation, deletions/reversions, and cycle checks) acquire the global transaction advisory lock:
  `SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));`
  However, Section 5.1 and Section 6 do not explicitly specify that `POST /api/v1/staging/approve` (which transitions candidate draft edges to `ACTIVE`) and `revert_mutation_batch` (which transitions edges to `REVERTED` or `SUPERSEDED`) acquire this lock.
  If an agent runs `propose_node_mutation` concurrently with a staging approval or rollback, the agent's cycle check could execute concurrently with edge state transitions, resulting in cycle race conditions.
* **Proposed Alternative:**
  Explicitly mandate in §5.2 and §6 that `POST /api/v1/staging/approve`, `revert_mutation_batch`, and `revert_agent_session` acquire `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` for the duration of the promotion or rollback transaction.

### LD-10

* **Severity:** `Minor`
* **Target:** `architecture.md` §8 Operational Model (Failure & Recovery), §5.1 State Ownership
* **Critique:** Section 8 states that in-progress ingestion jobs interrupted by a server crash are reclaimed upon restart via timeout detection (`updated_at < NOW() - INTERVAL '180s'`).
  However, if a crashed worker had already inserted candidate draft nodes into `graph_nodes` before crashing, restarting the worker on that job will attempt to re-insert candidate nodes, resulting in duplicate draft requirements or unique constraint errors.
* **Proposed Alternative:**
  Specify that upon reclaiming a `PROCESSING` job, the worker must execute an idempotent cleanup statement:
  `DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT';`
  prior to re-running the decomposition pipeline.

---

## Architectural Simplifications (Consolidation of Components)

### LD-11 (Simplification)

* **Severity:** `Major` (Consolidation)
* **Target:** `architecture.md` §4 Component Topology, §7 Technology Stack
* **Critique:** In Section 4 Component Topology and Table 4.1, `AuthN: Identity Validator` is depicted as an independent middle-tier architectural component with cache notes. In a single-binary Axum application, this represents unnecessary architectural ceremony and conceptual indirection.
* **Proposed Alternative:**
  Consolidate `Identity Validator` directly into an Axum Tower middleware and `FromRequestParts` extractor (`src/gateway/auth.rs`).
  The extractor validates bearer tokens or API keys against `agent_identities` (with an in-process `moka` LRU cache) and attaches the verified `ActorClaims` directly to the request extensions. This eliminates an artificial component boundary while simplifying testing and middleware chaining.

### LD-12 (Simplification)

* **Severity:** `Major` (Consolidation)
* **Target:** `architecture.md` §4 Component Topology, §7 Technology Stack, §8 Operational Model
* **Critique:** Section 4 depicts two separate, uncoordinated background worker components (`DecompWorker` and `EmbedWorker`), each running independent polling loops against PostgreSQL (`ingestion_jobs` and `embedding_queue`) with `FOR UPDATE SKIP LOCKED`.
  In a resource-constrained single-process daemon (`tks serve`), running dual polling loops causes unnecessary connection pool churn and duplicate failure-handling logic.
* **Proposed Alternative:**
  Consolidate background task handling into a single cooperative worker manager (`src/worker/mod.rs`) running inside `tks serve`.
  The worker manager polls pending jobs using a unified cooperative loop or PostgreSQL `LISTEN/NOTIFY` triggers on `ingestion_jobs` and `embedding_queue`, reducing idle database polling queries and centralizing telemetry, backoff, and graceful shutdown handling.

### LD-13 (Simplification)

* **Severity:** `Minor` (Workflow Simplification)
* **Target:** `architecture.md` §6 Interfaces & Contracts (`POST /api/v1/staging/reject`); `strategic-planning-backlog.md` §2 Phase 1
* **Critique:** The staging rejection interface currently requires passing individual `rejected_node_ids: Vec<Uuid>`. If a decomposed document contains dozens of false positives or the user wishes to discard the entire candidate batch, requiring individual UUID extraction and rejection creates significant manual friction during staging review.
* **Proposed Alternative:**
  Expand `POST /api/v1/staging/reject` and the CLI command `tks staging reject` to support atomic job-level rejection:
  `tks staging reject --job-id <job_id> [--reason <reason>]`.
  The endpoint executes `DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT'` and marks the job `REJECTED`, discarding the entire candidate batch in one operational step.
