# Lead Developer Critique Response: Architectural & Implementation Alignment

This review evaluates the Technical Vision (`docs/vision/vision.md`), Architecture Document (`docs/vision/architecture.md`), Strategic Planning Backlog (`docs/vision/strategic-planning-backlog.md`), and Technical Implementation Backlog (`docs/vision/technical-backlog.md`) from the perspective of lead development, operational feasibility, and implementation pragmatism.

---

## Findings

### LD-1

* **Severity:** `Blocker`
* **Target:** `architecture.md` §5.1 (Document Revision Reconciliation and Active Requirement Supersession), `technical-backlog.md` TB-7 (AST Revision Diffing & 3-Tier Reconciliation)
* **Critique:** Section 5.1 (Step 4) and TB-7 specify that during background document decomposition of a revised specification at persistent `doc_path`, the decomposition worker updates `graph_nodes` embedded columns (`doc_hash`, `byte_start`, `byte_end`) *in place* on existing `ACTIVE` nodes matching the 3-tier correlation hierarchy. This mutates active canonical requirement rows in the production graph prior to human supervisory review. If the supervisor subsequently rejects the ingestion job (`tks staging reject --job-id`), cancels the review, or if AST re-anchoring falsely correlates modified chunks, the active production requirements are left permanently corrupted—pointing to the rejected Git revision blob and shifted byte offsets without an `audit_ledger` record or rollback trail. Furthermore, during this staging window, `get_document_span` fetches source text from the unapproved Git blob while `graph_nodes.content` retains old text, directly violating Invariant INV-2 ("Destructive in-place updates on active requirements... must not occur").
* **Proposed Alternative:** Defer in-place span re-anchoring strictly to the atomic staging approval transaction (`POST /api/v1/staging/approve`) under `pg_advisory_xact_lock`. The decomposition worker must only compute and stage candidate span re-anchoring coordinates (stored as candidate metadata or in `ingestion_jobs` attributes). The physical in-place update of `doc_hash`, `byte_start`, and `byte_end` on active nodes must execute atomically inside `POST /api/v1/staging/approve`, accompanied by a discrete `SPAN_REANCHORED` audit event in `audit_ledger` with monotonic `event_seq` and `batch_id`. Rejecting or superseding an ingestion job cleanly discards the proposed re-anchoring with zero impact on active production nodes.

---

### LD-2

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints), §5.2 (Context Envelope Traversal Guardrails & Quota Partitioning), §9 Decision D-63
* **Critique:** Decision D-63 and §5.2 specify caller-scoped draft query visibility: when `include_drafts: true` is passed to `get_context_envelope`, the recursive CTE traverses active nodes plus draft nodes "authored by the calling agent identity". However, the `graph_nodes` table schema in §5.1 contains no `created_by` or `actor_id` column (only `id, node_key, node_type, title, content, search_tsv, lifecycle_state, governance_policy, job_id, doc_path, doc_hash, byte_start, byte_end, attributes`). `graph_edges` likewise contains no author column. Furthermore, intermediate draft proposals do not write to `audit_ledger` (by design, to avoid audit bloat per D-16/D-61). Consequently, unapproved draft entities in `graph_nodes` and `graph_edges` are completely unattributed in the relational schema, making it impossible for the recursive CTE to filter drafts by caller identity without unindexed JSONB inspection, and directly violating Invariant INV-7 ("Every mutation submitted through the Integration Gateway must be attributable to a verified external identity").
* **Proposed Alternative:** Add a first-class `created_by VARCHAR(64) NOT NULL` column to both `graph_nodes` and `graph_edges`, along with partial indexes:
  `CREATE INDEX idx_graph_nodes_draft_author ON graph_nodes(created_by) WHERE lifecycle_state = 'DRAFT';`
  `CREATE INDEX idx_graph_edges_draft_author ON graph_edges(from_node_id, to_node_id) WHERE lifecycle_state = 'DRAFT';`
  The recursive CTE in `src/storage/envelope.rs` can then filter cleanly and performantly: `WHERE (lifecycle_state = 'ACTIVE' OR (lifecycle_state = 'DRAFT' AND created_by = $calling_actor_id))`, preserving strict multi-agent draft isolation and non-repudiable audit attribution.

---

### LD-3

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (Rollback Cascade Mechanics & Blast-Radius Containment), `strategic-planning-backlog.md` §2 (Phase 3 MVD: Automated Invalidation Cascading)
* **Critique:** When an upstream requirement is modified or reverted, automated invalidation cascades and rollback sweeps transition downstream child specifications and tasks to `lifecycle_state = 'NEEDS_REVERIFICATION'`. In Phase 3 MVD, external agents attempting to complete such tasks are blocked until reverification occurs. Furthermore, §5.2 specifies that context envelope queries filter strictly on `lifecycle_state = 'ACTIVE'`. However, neither `architecture.md` §5.1/§6 nor the backlogs define any mechanism, endpoint, or MCP tool to reverify a node or transition it from `NEEDS_REVERIFICATION` back to `ACTIVE`. Staging approval (`POST /api/v1/staging/approve`) only accepts `DRAFT` nodes linked to an ingestion job. Because agents cannot query context envelopes for non-active nodes and cannot complete them, and supervisors have no reverification endpoint, invalidated subtrees enter an irreversible operational deadlock.
* **Proposed Alternative:** Formalize the reverification transition in §5.1 and §6 by defining a dedicated reverification interface (e.g. `POST /api/v1/nodes/{id}/reverify` and corresponding MCP tool), permitting supervisors or authorized agents to submit a reverification rationale. The transaction verifies that upstream parents are `ACTIVE`, transitions the node back to `ACTIVE`, commits a `REVERIFIED` event to `audit_ledger`, and re-enables context envelope queryability and task completion. Additionally, specify that `get_context_envelope` permits retrieving context for targets in `NEEDS_REVERIFICATION` state so that agents can inspect the context needed to resolve the invalidation.

---

### LD-4

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (State Ownership), §7 (Technology Stack), §11 (Open Question Q-4)
* **Critique:** In §5.1, `node_embeddings` defines the vector column as `embedding vector` with no dimension specified, and completely omits index creation DDL for the vector column. In PostgreSQL `pgvector`, creating an HNSW or IVFFlat index (`CREATE INDEX ... USING hnsw (embedding vector_cosine_ops)`) **requires** an explicit dimension modifier (e.g. `vector(384)` or `vector(1536)`). A dimensionless `vector` column cannot be indexed by `pgvector`, forcing all vector similarity searches in `get_context_envelope` to execute as sequential full table scans. Furthermore, Open Question Q-4 leaves the model dimension undecided between 1536-d and 384-d, while Spike 8 benchmarks 384-d local models. Without an explicit dimension in the schema, database migrations cannot create the required HNSW index, risking runtime query failure or severe latency degradation ($>500\text{ ms}$) at scale.
* **Proposed Alternative:** Update the DDL in §5.1 to define explicit vector dimensions aligned with the chosen embedding provider strategy. For Phase 1, standardize on `embedding vector(384)` if adopting local CPU embeddings (`all-MiniLM-L6-v2` / `fastembed-rs` per Spike 8) or `vector(1536)` if targeting commercial APIs (`text-embedding-3-small`), and add the mandatory HNSW index DDL:

  ```sql
  CREATE INDEX idx_node_embeddings_vector
      ON node_embeddings USING hnsw (embedding vector_cosine_ops);
  ```

  Document that switching models across dimensions requires an explicit data migration and re-indexing.

---

### LD-5

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints), §5.2 (Transaction Isolation & Locking Architecture)
* **Critique:** In §5.1, the only index defined on `graph_edges` is `CREATE UNIQUE INDEX idx_graph_edges_active_unique ON graph_edges (from_node_id, to_node_id, edge_type) WHERE lifecycle_state = 'ACTIVE'`. Because structural edges uniformly point upward (`child -DERIVED_FROM-> parent`, `Spec -CONSTRAINED_BY-> Req`, `Task -FULFILLS-> Spec`), this index supports traversing upward from `from_node_id`. However: (1) Downward traversals required for automated invalidation cascading (§5.1, Phase 3 MVD) and retrieving assigned sub-tasks in context envelopes query by `to_node_id = $parent_id`, forcing full table scans on `graph_edges`. (2) The partial index filter `WHERE lifecycle_state = 'ACTIVE'` excludes `DRAFT` edges; consequently, speculative draft queries (`include_drafts: true`) must perform unindexed sequential scans across the entire edge table. (3) The foreign key on `to_node_id REFERENCES graph_nodes(id)` lacks an index, creating lock contention during node updates and deletions.
* **Proposed Alternative:** Add an index on `to_node_id` covering active edges for downward traversals, and add an index covering draft edges:

  ```sql
  CREATE INDEX idx_graph_edges_to_node_active
      ON graph_edges (to_node_id, from_node_id, edge_type)
      WHERE lifecycle_state = 'ACTIVE';

  CREATE INDEX idx_graph_edges_draft
      ON graph_edges (from_node_id, to_node_id)
      WHERE lifecycle_state = 'DRAFT';
  ```

  This guarantees sub-millisecond reverse lookups during invalidation sweeps and prevents sequential scans on draft queries.

---

### LD-6

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (Automatic Ingestion Job Supersession Sweep), §8 (Failure & Recovery), `technical-backlog.md` TB-7
* **Critique:** Two race conditions exist in background decomposition execution: (1) In D-67 and §5.1, re-ingesting a document at `doc_path` sweeps prior open jobs (`status IN ('QUEUED', 'PROCESSING', 'STAGED')`) to `SUPERSEDED` and deletes their draft nodes. However, if a background worker is actively executing Stage 2 LLM classification for a job in `PROCESSING`, the worker will finish and commit its candidate nodes to `graph_nodes` and set `status = 'STAGED'` *after* the deletion sweep, resurrecting the superseded job and stranding zombie drafts in `graph_nodes`. (2) In §8, if a worker crashes while in `PROCESSING` and the job is reclaimed after a 180s timeout, the restarted worker re-executes parsing from scratch without purging partial draft rows already written by the crashed worker, producing duplicate candidate draft nodes in `graph_nodes`.
* **Proposed Alternative:** (1) The worker's terminal transition to `STAGED` must execute conditionally: `UPDATE ingestion_jobs SET status = 'STAGED', updated_at = NOW() WHERE job_id = $1 AND status = 'PROCESSING'`. If zero rows are updated (because the job was superseded or cancelled), the worker must abort and execute `DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT'`. (2) Upon reclaiming a timed-out `PROCESSING` job, the worker must atomically delete any existing partial draft nodes (`DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT'`) before restarting decomposition.

---

### LD-7

* **Severity:** `Minor`
* **Target:** `architecture.md` §5.1 (Rollback Cascade Mechanics & Blast-Radius Containment), §6 (Interfaces & Contracts `revert_mutations`)
* **Critique:** Section 5.1 specifies that "if cross-agent dependent tasks exist on a node targeted for rollback, the administrative interface requires explicit confirmation flags displaying the affected topological subgraph before applying the compensating transaction." However, the schema for `revert_mutations` in §6 defines only filter criteria (`batch_id`, `agent_id`, `since`, `event_seq_range`) and provides no `dry_run: Option<bool>` parameter or preview response schema. An administrator has no mechanism to inspect which active nodes and edges will transition to `REVERTED` or `NEEDS_REVERIFICATION` before executing the irreversible compensating transaction.
* **Proposed Alternative:** Extend the `revert_mutations` request schema in §6 with `dry_run: Option<bool>` (default `false`) and `force: Option<bool>` (default `false`). When `dry_run = true`, the transaction executes the dependency sweep CTE under `READ COMMITTED` and returns the affected subgraph (list of node IDs, keys, titles, and dependent agent IDs) without committing compensating events. When `dry_run = false`, if dependent cross-agent nodes exist and `force != true`, the call returns error `ERR_CONFIRMATION_REQUIRED` containing the preview payload.

---

### LD-8

* **Severity:** `Minor`
* **Target:** `technical-backlog.md` TB-2 (Hierarchical Heading Anchor Generation), TB-7 (AST Revision Diffing & 3-Tier Reconciliation)
* **Critique:** TB-2 item 4 and TB-7 item 4 establish deterministic heading anchors (`ast_anchor`, e.g. `doc_path#heading-1/heading-2`) as Tier 2 matching keys during document revision reconciliation. However, CommonMark documents frequently contain duplicate heading titles under the same parent section (e.g. repeated "Overview", "Rationale", or "Implementation" subsections) or multiple non-heading structural blocks (bullet list items, tables) under a single heading. Without disambiguation, multiple distinct requirement chunks generate identical `ast_anchor` values, causing Tier 2 matching to map all sub-blocks to the first matching active node, corrupting requirement identity and span re-anchoring across revisions.
* **Proposed Alternative:** Update TB-2 and TB-7 to specify heading slug disambiguation and ordinal block indexing: (1) When duplicate heading titles occur within the same parent scope, append an occurrence counter (e.g. `doc#section/overview-1`, `doc#section/overview-2`). (2) For non-heading sub-blocks (lists, tables, paragraphs) extracted under a heading, append an ordinal block index (e.g. `doc#section/heading#block-0`, `#block-1`). This guarantees that every extracted AST block generates a globally unique `ast_anchor` within the document.

---

### LD-9

* **Severity:** `Minor`
* **Target:** `architecture.md` §3 (INV-1, INV-5), §5.1 (Disambiguated Mutation Pathways)
* **Critique:** When a verified agent elaborates an execution sub-task (`node_type = 'TASK'`) directly into `ACTIVE` state under a parent node with `governance_policy = 'AUTONOMOUS_ELABORATION'`, `graph_nodes.governance_policy` must be populated with a valid policy satisfying `chk_governance_policy` (`AUTONOMOUS_ELABORATION`, `HUMAN_REVIEW_REQUIRED`, `LOCKED`). The architecture does not specify whether newly elaborated tasks inherit the parent's governance policy or default to a specific policy. If an agent creates a task without specifying `governance_policy`, or if inheritance is undefined, the task might default to `HUMAN_REVIEW_REQUIRED` (preventing the agent from updating its own task status) or trigger a NOT NULL constraint violation.
* **Proposed Alternative:** Formally specify in §5.1 and `technical-backlog.md` that tasks created via autonomous elaboration inherit the parent's `governance_policy` (`AUTONOMOUS_ELABORATION`) by default, unless explicitly overridden in the mutation payload, ensuring the agent retains permission to update task status to `IN_PROGRESS` and `COMPLETED` without supervisory intervention.

---

### LD-10

* **Severity:** `Minor`
* **Target:** `strategic-planning-backlog.md` §6 (Quantitative Operational Targets & Metric Calibrations)
* **Critique:** In §6, metric `CAL-H1` targets $\ge 40\%$ reduction in constraint violations during Phase 2 controlled trials, whereas Spike 0 in §5 and Risk R-1 in `architecture.md` §10 define the Phase 0 directional threshold as $\ge 30\%$ reduction with a partial validation band between 15% and 29%. While a tighter target in Phase 2 is reasonable as tooling matures, the relationship between Phase 0 spike calibration and Phase 2 controlled trial targets is not explicitly cross-referenced, which could lead to confusion over whether a 30% reduction in Spike 0 satisfies Phase 1 gating.
* **Proposed Alternative:** Add an explicit cross-reference in §6 clarifying that the $\ge 30\%$ threshold in Spike 0 serves as the Phase 0 directional greenlight gate for prototype de-risking, while $\ge 40\%$ (`CAL-H1`) represents the full-system production target evaluated during Phase 2 controlled trials.

---

## Component Simplifications

### LD-11

* **Severity:** `Major` (Component Consolidation)
* **Target:** `architecture.md` §4 (Component Topology), §5.1 (State Ownership), §7 (Technology Stack)
* **Critique:** The architecture maintains two separate 1-to-1 relational tables for node embeddings: `node_embeddings` (storing the vector and content hash) and `embedding_queue` (storing the queue status, retry count, and `scheduled_at` timestamp). Both tables share `node_id UUID PRIMARY KEY REFERENCES graph_nodes(id) ON DELETE CASCADE` and duplicate `content_hash VARCHAR(64)`. Maintaining two separate tables requires cross-table transactions, duplicate index maintenance, separate foreign key cascade locks, and explicit deletion from `embedding_queue` upon updating `node_embeddings`.
* **Proposed Alternative:** Consolidate `embedding_queue` directly into `node_embeddings`. Add `status VARCHAR(20) NOT NULL DEFAULT 'PENDING'`, `retry_count INT NOT NULL DEFAULT 0`, and `scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW()` directly to `node_embeddings`. When a node activates, upsert into `node_embeddings` with `status = 'PENDING'`. The worker claims rows with `WHERE status = 'PENDING' AND scheduled_at <= NOW() FOR UPDATE SKIP LOCKED` and updates `embedding = $vector, status = 'COMPLETED'`. A partial HNSW index on `embedding WHERE status = 'COMPLETED'` indexes only active vectors. This eliminates an entire database table, eliminates duplicate foreign keys, simplifies worker transaction logic, and preserves 100% of the asynchronous retry backoff semantics.

---

### LD-12

* **Severity:** `Minor` (API Consolidation)
* **Target:** `architecture.md` §6 (Interfaces & Contracts), `technical-backlog.md` TB-3 / TB-7
* **Critique:** The architecture defines separate REST endpoints for staging approval (`POST /api/v1/staging/approve`) and staging rejection (`POST /api/v1/staging/reject`), with overlapping polymorphic argument payloads (`job_id`, `rejected_node_ids`, `approved_node_ids`, `--only`, `--exclude`). Staging rejection simply deletes draft rows from `graph_nodes` and transitions `ingestion_jobs.status` to `REJECTED`. Having dedicated bespoke RPC endpoints for rejection duplicates routing, middleware handling, and CLI command mapping.
* **Proposed Alternative:** Consolidate staging lifecycle operations into standard RESTful resource actions on `/api/v1/documents/ingest/{job_id}`: `POST /api/v1/staging/approve` handles promotion, while rejection is modeled as `DELETE /api/v1/documents/ingest/{job_id}` (or `PATCH /api/v1/documents/ingest/{job_id}` with `{"status": "REJECTED"}`). This eliminates a standalone RPC endpoint and aligns the API with standard HTTP semantics.
