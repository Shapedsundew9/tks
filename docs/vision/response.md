# Lead Developer Architectural Critique & Technical Alignment Response

This document records the Lead Developer Sub-Agent critique for the architectural design and technical alignment loop, evaluating `docs/vision/architecture.md`, `docs/vision/vision.md`, `docs/vision/strategic-planning-backlog.md`, and `docs/vision/technical-backlog.md`.

---

## Findings

### LD-1: `chk_node_type` Constraint Omission of Planning Rails (`SCHEMA_CONTRACT`, `CODE_BLUEPRINT`) and Governance Entities

* **Severity:** Blocker
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints), §9 (Decision D-111), and `technical-backlog.md` TB-13.2
* **Critique:** Decision D-111, §4 (Storage Repository Layer), §6 (`get_elaboration_context`), and TB-13.2 mandate that `get_elaboration_context` dynamically query first-class `SCHEMA_CONTRACT` and `CODE_BLUEPRINT` entities stored in `graph_nodes`. However, the PostgreSQL check constraint `chk_node_type` specified in §5.1 line 273 strictly restricts `node_type`:

  ```sql
  ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type
      CHECK (
          (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'CODE_COMMIT', 'UNCLASSIFIED'))
          OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'CODE_COMMIT'))
      );
  ```

  Neither `'SCHEMA_CONTRACT'` nor `'CODE_BLUEPRINT'` is included in `chk_node_type`. Any SQL insert or promotion of schema contracts or codebase blueprints will immediately fail at runtime with PostgreSQL exception `23514 check_violation`, blocking the implementation of Phase 5 Cognitive Planning Rails. Furthermore, Invariant INV-1 ancestor validation CTEs lack rules for how `SCHEMA_CONTRACT` and `CODE_BLUEPRINT` nodes relate to active requirement roots during promotion.
* **Proposed Alternative:** Update the `chk_node_type` DDL in §5.1 to explicitly include `'SCHEMA_CONTRACT'` and `'CODE_BLUEPRINT'` (as well as Phase 6 procedural entities `'PROCEDURAL_ENTITY'`, `'CHECKLIST_ITEM'`, `'GOVERNANCE_PROFILE'`), or alternatively, adopt Simplification LD-11 to model contracts and blueprints as standard `SPECIFICATION` nodes with typed JSONB attributes (`attributes->'blueprint_type'`), avoiding check constraint modifications and preserving uniform Invariant INV-1 ancestry.

---

### LD-2: Cross-Document Specification Divergence on Pre-Merge Verification and Commit Lineage

* **Severity:** Major
* **Target:** `vision.md` §3 (Physical Schema Contracts), §5 (Invariant I-9), `strategic-planning-backlog.md` §1, §4 (Phase 4 MVD), §7 (Sequence Diagram), and `architecture.md` §5.1, §9 (Decisions D-107, D-108)
* **Critique:** A direct cross-document specification conflict exists regarding pre-merge CI verification and VCS commit linking. `architecture.md` (Decisions D-107 and D-108, Invariant INV-9, TB-12) explicitly eliminated provisional commit nodes, unattributed edge schemas, and single-ended edges: pre-merge test runs record directly in `graph_nodes.attributes->'pre_merge_verification'` on active `TASK` entities (no edge created), and merge commits link upward via `CODE_COMMIT -IMPLEMENTS-> TASK`. In contrast, `vision.md` (§3, Invariant I-9) and `strategic-planning-backlog.md` (§1, MVD §4 step 7, and §7 sequence diagram item 482) still mandate creating `VERIFIED_BY` edges directly on `TASK` nodes carrying PR commit metadata, link commits via downward `IMPLEMENTED_BY` edges, and reference provisional commit nodes. An engineer implementing Phase 4 Deliverable 4.4 following `vision.md` or `strategic-planning-backlog.md` will attempt to insert `VERIFIED_BY` edges without a target node or attempt to populate non-existent edge attributes, crashing database referential integrity.
* **Proposed Alternative:** Reconcile `vision.md` (§3, §5 Invariant I-9) and `strategic-planning-backlog.md` (§1, §4, §7) with `architecture.md` Decisions D-107 and D-108: update the sequence diagram and MVD text to specify that `POST /api/v1/verification/test-run` appends test results directly to `TASK.attributes->'pre_merge_verification'` without creating graph edges, and post-merge commits link upward via `CODE_COMMIT -IMPLEMENTS-> TASK`. Eliminate all references to provisional commit nodes, `VERIFIED_BY` edges on tasks, and downward `IMPLEMENTED_BY` edges.

---

### LD-3: Missing Submitter Identity and Token Credentials in `ingestion_jobs` for Asynchronous Authority Auto-Promotion

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (Ingestion jobs schema, Audit Ledger schema), §9 (Decision D-98), and `technical-backlog.md` TB-11.5
* **Critique:** Under Authority Inheritance (D-98, TB-11.5), when a specification is ingested from a pre-governed source (e.g. `BRANCH_PROTECTED_CODEOWNERS`), the background worker automatically promotes candidate nodes directly to `ACTIVE` upon ingestion, bypassing manual staging and writing audit events directly to `audit_ledger`. However, the `ingestion_jobs` table DDL (§5.1 line 380) lacks columns for the submitting caller's identity or credentials. Meanwhile, `audit_ledger` DDL (§5.1 line 340) enforces `actor_id VARCHAR(64) NOT NULL`, `actor_type VARCHAR(16) NOT NULL`, and `token_fingerprint VARCHAR(64) NOT NULL`, and Invariant INV-7 strictly forbids committing mutations without verified identity attribution. When the asynchronous background worker executes auto-promotion in a detached thread, it lacks the original caller's actor ID and token fingerprint, causing either a fatal `23502 not_null_violation` in PostgreSQL or forcing the worker to fabricate unverified dummy credentials.
* **Proposed Alternative:** Add `created_by VARCHAR(64) NOT NULL` and `token_fingerprint VARCHAR(64) NOT NULL` to the `ingestion_jobs` table DDL in §5.1. When `POST /api/v1/documents/ingest` validates caller credentials via Tower middleware, persist `actor_id` and `token_fingerprint` onto the job record. When the background worker executes authority inheritance auto-promotion, pass these authenticated credentials to `graph_nodes.created_by` and the resulting `audit_ledger` promotion events.

---

### LD-4: In-Place Edge Mutation in `reverify_node` Re-Parenting Violating Invariant INV-2 Immutability

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (Reverification Interface & Operational Unblocking), §6 (`reverify_node`), and `technical-backlog.md` TB-8
* **Critique:** In §5.1 (line 472), `reverify_node` specifies that when re-parenting a degraded task whose upstream requirement was superseded, the reverification engine "atomically updates active upward edges to point to the new active parent requirement before verifying upstream dependencies." Updating `to_node_id` in-place on existing rows in `graph_edges` is a destructive in-place update (`UPDATE graph_edges SET to_node_id = ...`), which directly violates Invariant INV-2 ("Destructive in-place updates on active requirements, specifications, and topological edges must not occur... edges are never physically deleted"). In-place mutation destroys historical edge topology and breaks deterministic point-in-time graph reconstruction from `audit_ledger`.
* **Proposed Alternative:** Explicitly specify that re-parenting during reverification adheres strictly to non-destructive edge lifecycle semantics: the existing active upward edge is transitioned to `lifecycle_state = 'SUPERSEDED'`, and a new upward edge is inserted with a fresh surrogate `edge_id`, `lifecycle_state = 'ACTIVE'`, and `to_node_id = $new_parent_id`. An explicit `EDGE_REPARENTED` audit event correlating the old and new edge IDs must be recorded in `audit_ledger`.

---

### LD-5: Premature Commit on `refs/heads/specs` During Unapproved Document Ingestion Corrupting Git History on Rejection

* **Severity:** Major
* **Target:** `architecture.md` §4 (Component Responsibilities, Git Write Actor), §5.1 (Bare Git Document Ledger), §6 (Document Ingestion contract), and `technical-backlog.md` TB-1
* **Critique:** In §6 line 604 and TB-1, `POST /api/v1/documents/ingest` dispatches a `CommitCommand` to the Git write actor to commit the uploaded document directly to `refs/heads/specs` *synchronously upon upload*, before decomposition or staging review occurs. If the document contains invalid syntax, is superseded by an author refactoring, or is explicitly rejected by the human supervisor via `tks staging reject` (`DELETE /api/v1/documents/ingest/{job_id}`), the commit on `refs/heads/specs` remains permanently at HEAD. Subsequent uploads of other specifications branch from a tree containing rejected, unapproved specification content. This corrupts standard Git history (`git log`, `git diff`) and breaks the guarantee that `refs/heads/specs` represents the authoritative version-governed ledger of approved intent.
* **Proposed Alternative:** Decouple raw blob storage from branch commitment. On initial upload (`POST /api/v1/documents/ingest`), the Git write actor writes the raw document buffer directly to the bare Git ODB via `git_blob_create_from_buffer` and stores the resulting blob hash in `ingestion_jobs.document_hash`. The decomposition worker reads the immutable blob directly from the ODB. The actual Git commit advancing `refs/heads/specs` at `doc_path` is executed strictly within the atomic staging approval transaction (`POST /api/v1/documents/ingest/{job_id}/approve` or authority auto-promotion). Rejections discard the candidate draft nodes leaving `refs/heads/specs` pristine.

---

### LD-6: Target Identification Gap in Pre-Merge Release Readiness Endpoint (`GET /api/v1/release/readiness?scope=pre-merge`)

* **Severity:** Major
* **Target:** `architecture.md` §6 (Release Readiness Evaluation), `strategic-planning-backlog.md` Deliverable 4.4, and `technical-backlog.md` TB-12.3
* **Critique:** TB-12.3 defines `GET /api/v1/release/readiness?scope=pre-merge&pr_commit_sha=<sha>` as evaluating PR readiness by "verifying that target `TASK` nodes are active and their `attributes->'pre_merge_verification'` records passing CI runs for the specified PR commit SHA." However, the query parameters (`scope`, `pr_commit_sha`, `requirement_root_id`, `milestone`) provide no mechanism to identify *which* tasks are modified or claimed by the PR. If the endpoint scans only tasks that already contain the PR commit SHA in `pre_merge_verification`, any task that was modified by the PR but omitted from CI test reporting will be silently ignored rather than flagged as unverified, creating a false-positive `READY` release gate.
* **Proposed Alternative:** Extend `GET /api/v1/release/readiness` under `scope=pre-merge` to accept an explicit `task_ids: Vec<Uuid>` list (or query active tasks where `attributes->'vcs_commits' ? pr_commit_sha`). The endpoint must verify that *all* specified tasks (or all tasks touched on the branch) contain a passing test run matching `pr_commit_sha`, returning `BLOCKED` with `unverified_task_ids` if any task lacks a passing run.

---

### LD-7: Key Sanitization Regex Flaw in `query_requirements` Causing Full-Text Search Negation on Multi-Segment Keys

* **Severity:** Major
* **Target:** `architecture.md` §6 (`query_requirements`), §9 (Decision D-68), and `technical-backlog.md` TB-7.3
* **Critique:** Decision D-68 and §6 line 592 specify sanitizing canonical key queries using regex `[A-Za-z]+-[0-9]+` to avoid PostgreSQL `websearch_to_tsquery` hyphen-negation (`NOT`). However, standard canonical keys defined throughout TKS contain multiple alphanumeric segments (e.g. `REQ-CORE-001`, `TASK-AUTH-042`, `API-GET-USERS`). These multi-segment keys do NOT match `[A-Za-z]+-[0-9]+`. Consequently, queries for standard keys fall through to `websearch_to_tsquery('english', 'REQ-CORE-001')`, which interprets `-CORE` and `-001` as negation (`req & !core & !001`), returning empty result sets and breaking offline search.
* **Proposed Alternative:** Standardize `query_requirements` on `plainto_tsquery('english', $query)` combined with an exact/prefix match on `node_key ILIKE $query || '%'`. Unlike `websearch_to_tsquery`, `plainto_tsquery` never interprets hyphens as boolean negation operators. If `websearch_to_tsquery` is retained for advanced natural language queries, sanitize queries by replacing hyphens within alphanumeric tokens (`(\w)-(\w)`) with whitespace or underscores prior to tsquery conversion.

---

### LD-8: Undefined Token Budget Clamping and Axis Prioritization in `get_elaboration_context`

* **Severity:** Major
* **Target:** `architecture.md` §6 (`get_elaboration_context`), §9 (Decisions D-95, D-111), and `technical-backlog.md` TB-13.2
* **Critique:** In §6 line 590, `get_elaboration_context` accepts `max_tokens: Option<u32>` and states that it "clamps output to token budget." However, the technology stack strictly disclaims commercial LLM SDKs and tokenizers (C-25), and no tokenizer dependency (e.g. `tiktoken-rs`) is declared. Furthermore, the architecture does not specify how token clamping is enforced across the 4 axes. If an elaboration context exceeds `max_tokens` and is truncated naively by characters or at arbitrary JSON boundaries, Axis 3 (Physical Schema Contracts) or Axis 1 (Normative Invariants) can be sliced mid-contract, emitting malformed DDL or partial rules that cause client agents to hallucinate.
* **Proposed Alternative:** Define deterministic clamping semantics: (1) Use a conservative character-based heuristic budget (e.g. 1 token $\approx 3.5$ characters UTF-8) to avoid heavy tokenizer dependencies; (2) Define an explicit axis truncation priority: Axis 1 (Normative Boundaries) and Axis 3 (Physical Schema Contracts) are mandatory and never truncated; Axis 2 (Architectural Precedents) and Axis 4 (Codebase Blueprints) are clamped by dropping whole entity items (rather than string slicing) if the budget is exceeded; (3) If the mandatory axes exceed `max_tokens`, return a structured error `ERR_CONTEXT_BUDGET_EXCEEDED` with the required minimum token count.

---

### LD-9: Ghost Draft Residue Race Window during Ingestion Job Supersession

* **Severity:** Minor
* **Target:** `architecture.md` §5.1 (Worker Concurrency Hardening), §9 (Decisions D-67, D-79), and `technical-backlog.md` TB-7.6
* **Critique:** When a document is re-ingested at `doc_path`, open jobs for that path are swept to `SUPERSEDED` and their drafts deleted. If a background worker was concurrently executing Stage 1 decomposition for the old job, it inserts draft nodes *after* the supersession sweep has already executed its delete. The worker then attempts to update the job to `STAGED` with `WHERE status = 'PROCESSING'`, finds 0 rows updated, and attempts to purge its drafts. If the worker process is killed or crashes at that instant, the job is left in `SUPERSEDED` status with orphan candidate draft nodes in `graph_nodes`. The 180s timeout reclaim loop only checks `WHERE status = 'PROCESSING'`, leaving these drafts permanently orphaned.
* **Proposed Alternative:** Extend the periodic worker cleanup or startup reclaim query to purge orphaned drafts for all terminal non-approved jobs: `DELETE FROM graph_nodes WHERE job_id IN (SELECT job_id FROM ingestion_jobs WHERE status IN ('SUPERSEDED', 'REJECTED', 'FAILED')) AND lifecycle_state = 'DRAFT'`.

---

### LD-10: Stale `relationship_review_backlog` References Contradicting Decision D-106

* **Severity:** Minor
* **Target:** `architecture.md` §4 (Component Responsibilities table line 224), `vision.md` §4 (Mermaid diagram item 370, 390), and `strategic-planning-backlog.md` §7 (Sequence Diagram line 514)
* **Critique:** Decision D-106 explicitly retired the `relationship_review_backlog` table in favor of direct CLI/log telemetry and existing anomaly triage (`--triage-anomalies`). However, `architecture.md` §4 (line 224: "staging issues in `relationship_review_backlog`"), `vision.md` §4 (Mermaid diagram node `Backlog`, line 370), and `strategic-planning-backlog.md` (sequence diagram step 514: "Enqueue recommendations into relationship_review_backlog") still explicitly reference writing to this retired table.
* **Proposed Alternative:** Scrub all remaining references to `relationship_review_backlog` across `architecture.md`, `vision.md`, and `strategic-planning-backlog.md`, ensuring all audit findings output directly to structured logs, CLI stdout/stderr, and `attributes->'extraction_metadata'`.

---

## Simplifications (Component Consolidations)

### LD-11: Consolidate Physical Schema Contracts and Codebase Blueprints as `SPECIFICATION` Nodes Rather Than Creating Bespoke Node Types

* **Severity:** Major
* **Target:** `architecture.md` §5.1, §9 (Decision D-111), and `technical-backlog.md` TB-13.2
* **Critique:** Decision D-111 proposed introducing `SCHEMA_CONTRACT` and `CODE_BLUEPRINT` as new first-class node types in `graph_nodes`. This introduces schema migration churn, requires altering `chk_node_type`, and complicates Invariant INV-1 ancestor validation CTEs. Furthermore, storing raw codebase blueprints as bespoke graph entities risks violating the Governance Altitude principle (modeling implementation in the graph).
* **Proposed Alternative:** Model schema contracts (OpenAPI schemas, table definitions) and codebase blueprints as standard `SPECIFICATION` nodes tagged with `attributes->'blueprint_type' = 'SCHEMA_CONTRACT' | 'CODE_BLUEPRINT'`, attached upward to the synthesized root `REQUIREMENT` of their respective modules via standard `DERIVED_FROM` edges. This achieves 100% of the functionality required by `get_elaboration_context`, fully satisfies `chk_node_type` without DDL changes, and reuses the existing `OpenApiAdapter` (TB-11) and CommonMark AST parsers with zero architectural overhead.

---

### LD-12: Reconcile `GraphEventBus` Specification and Eliminate Residual `LISTEN/NOTIFY` References

* **Severity:** Major
* **Target:** `architecture.md` §4, §6 (Event Notification Bus line 603), §7 (Graph Event & Invalidation Bus line 715), and §9 (Decision D-92)
* **Critique:** Line 603 describes `GraphEventBus` as a "dedicated asynchronous `tokio_postgres` connection listener continuously polling `LISTEN tks_graph_events`." However, line 715 and Decision D-52 explicitly rejected PostgreSQL `LISTEN/NOTIFY` due to unpooled connection overhead and trigger plumbing, and Decision D-92 established that route handlers publish directly to the in-process Tokio broadcast channel (`GraphEventBus`).
* **Proposed Alternative:** Remove the obsolete `LISTEN/NOTIFY` wording from line 603 and standardize `GraphEventBus` exclusively as an in-process Tokio broadcast channel published directly by Axum route handlers and storage transaction wrappers.

---

### LD-13: Synchronous In-Process CPU Vector Embedding for Autonomous Task Elaboration

* **Severity:** Major
* **Target:** `architecture.md` §5.2 (Asynchronous Embedding Generation), §6, §9 (Decision D-114), and `technical-backlog.md` TB-6
* **Critique:** With `fastembed-rs` executing in-process on CPU via `tokio::task::spawn_blocking` (sub-15ms latency per Spike 8 and Decision D-114), asynchronously queuing task embeddings in `node_embeddings` introduces significant accidental complexity: `get_context_envelope` must implement complex ancestor requirement fallback logic (TB-6.4) and quota reallocation (TB-6.6) because newly created tasks have `status = 'PENDING'`.
* **Proposed Alternative:** For individual autonomous task creation (`create_subtask`), execute vector inference synchronously in `tokio::task::spawn_blocking` within the task elaboration transaction, setting `status = 'COMPLETED'` immediately. Reserve the asynchronous batch polling queue exclusively for bulk document ingestion. This eliminates pending-embedding fallback code in `get_context_envelope` and ensures newly created tasks are immediately vector-queryable.
