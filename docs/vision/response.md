# Lead Developer Technical Critique & Alignment Response

This document contains the architectural and technical alignment critique formulated by the Lead Developer Sub-Agent following a thorough inspection of `docs/vision/architecture.md`, `docs/vision/vision.md`, `docs/vision/strategic-planning-backlog.md`, and `docs/vision/technical-backlog.md`.

---

## Substantive Findings

### LD-1

* **Severity:** Blocker
* **Target:** `architecture.md` §3 (INV-9), §5.1 (Entity Typing and Relational Constraints), §6 (Interfaces: Pre-Merge CI Test-Run Reporting), §9 (D-93, D-103)
* **Critique:** Pre-merge CI verification via `VERIFIED_BY` edge on `TASK` is relationally and structurally impossible as specified.
  1. The concrete DDL for `graph_edges` in §5.1 defines only `edge_id`, `from_node_id`, `to_node_id`, `edge_type`, `created_by`, and `lifecycle_state`. There is no `attributes` JSONB column on `graph_edges`.
  2. Every edge in `graph_edges` mandates `from_node_id NOT NULL REFERENCES graph_nodes(id)` and `to_node_id NOT NULL REFERENCES graph_nodes(id)`. If provisional commit nodes do not exist in `graph_nodes` (eliminated by D-103 to prevent graph pollution), and test runs are not nodes, what is the opposing endpoint (`to_node_id` or `from_node_id`) of this edge? An edge cannot have a single endpoint in a relational property graph.
  3. Pre-transaction edge validation (C-24, D-97) explicitly verifies that both edge endpoints exist in `graph_nodes`, which will unconditionally reject any edge creation attempt that lacks a valid registered target UUID.
* **Proposed Alternative:**
  Disambiguate pre-merge verification storage from graph topology edges:
  Record pre-merge CI test run verifications directly on the `TASK` node in `graph_nodes.attributes->'pre_merge_verification'` (e.g., `{"pr_commit_sha": "...", "status": "PASSED", "test_suite": "...", "verified_at": "..."}`), mirroring how branch commit SHAs are recorded in `attributes->'vcs_commits'`. This completely avoids ghost nodes, requires no impossible single-ended edges, and requires no schema changes to `graph_edges`.
  Alternatively, if `VERIFIED_BY` edges must be preserved as topological edges, add `attributes JSONB NOT NULL DEFAULT '{}'::jsonb` to `graph_edges`, and define that `POST /api/v1/verification/test-run` links the `TASK` node to an existing, registered `VERIFICATION` node (representing the test specification or test suite) in `graph_nodes`, passing `verification_node_id` in the payload.

---

### LD-2

* **Severity:** Blocker
* **Target:** `architecture.md` §3 (INV-1), §5.1 (Edge Relationship Rules), §9 (D-50, D-93), `strategic-planning-backlog.md` §2 (Phase 4 Deliverable 4.4), `technical-backlog.md` TB-12.2
* **Critique:** Directionality inversion and invariant breakdown for `IMPLEMENTED_BY` and `CODE_COMMIT`.
  Invariant INV-1 mandates: *"Every active functional specification, implementation task, and code artifact reference must maintain a valid directed edge path terminating at an authorized active requirement node. All governance and structural edges must point directed upward toward governing requirements (`child_id -DERIVED_FROM-> parent_id`, `Spec -CONSTRAINED_BY-> Requirement`)."* Decision D-50 standardizes that all edges point uniformly upward toward governing requirements to allow unidirectional recursive CTE traversals.
  However, §5.1 and TB-12.2 specify `IMPLEMENTED_BY` edges as *"pointing from resolved `TASK` or `SPECIFICATION` entities to the canonical `CODE_COMMIT` node"*.
  This creates two severe implementation failures:
  1. `CODE_COMMIT` is an active node in `graph_nodes` (`node_type = 'CODE_COMMIT'`, `lifecycle_state = 'ACTIVE'`), but it is a terminal sink with zero outgoing edges. An Invariant INV-1 ancestor check on a `CODE_COMMIT` node will immediately fail because no path exists from `CODE_COMMIT` to an active `REQUIREMENT`.
  2. Recursive upward CTE queries (`get_context_envelope`, ancestor validation, impact analysis) cannot traverse from a commit to a requirement without switching join directions mid-query (`from_node_id` vs `to_node_id`), violating Decision D-50's performance rationale and complicating all graph traversal CTEs.
* **Proposed Alternative:**
  Standardize edge orientation consistently with Invariant INV-1 and Decision D-50:
  Define the edge as pointing upward from commit to task: `CODE_COMMIT -IMPLEMENTS-> TASK` (`from_node_id = commit_id, to_node_id = task_id`), maintaining an unbroken upward path from code commit to requirement root.
  Alternatively, if the edge remains `task_id -IMPLEMENTED_BY-> commit_id`, explicitly exempt `node_type = 'CODE_COMMIT'` from Invariant INV-1 upward ancestry validation CTEs and document in §5.1 and §6 that `IMPLEMENTED_BY` is a special terminal downward reference edge queried exclusively via `idx_graph_edges_to_node_active`.

---

### LD-3

* **Severity:** Major
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints, Node Lifecycle State), §9 (D-74, D-102), `technical-backlog.md` TB-7.5
* **Critique:** Premature active node mutation and Invariant INV-1 child invalidation via `PENDING_DEPRECATION`.
  Decision D-102 was introduced to prevent premature downward invalidation of child tasks when a document is re-ingested with omitted sections. It states that omitted active sections are flagged for deprecation in staging metadata, and hard supersession only executes upon explicit supervisory approval.
  However, §5.1 adds `PENDING_DEPRECATION` directly to `graph_nodes.chk_lifecycle_state` and states that the worker updates active nodes to `PENDING_DEPRECATION`. If a background ingestion worker updates existing active nodes in place:
  1. It violates Invariant INV-2 and Decision D-74 (*"background ingestion and draft generation never mutate active graph state"*).
  2. Because `get_context_envelope` filters on `lifecycle_state = 'ACTIVE'`, modifying an active node to `PENDING_DEPRECATION` immediately hides it from external agents before supervisory approval.
  3. Active child tasks pointing to this node immediately fail Invariant INV-1 ancestor checks because their parent is no longer `ACTIVE`, breaking agent task elaboration across the entire subtree.
* **Proposed Alternative:**
  Active nodes must remain strictly in `lifecycle_state = 'ACTIVE'` during the entire background ingestion and staging window. Proposed deprecations must be stored strictly as staged metadata in `ingestion_jobs` or candidate staging attributes. Only upon explicit staging approval (`POST /api/v1/documents/ingest/{job_id}/approve`) does the approval transaction transition confirmed omitted nodes from `ACTIVE` to `SUPERSEDED` and trigger downward invalidation cascades under the global advisory lock. Remove `PENDING_DEPRECATION` from `chk_lifecycle_state` or document it strictly as an administrative post-approval state.

---

### LD-4

* **Severity:** Major
* **Target:** `architecture.md` §4 (Decomposition Pipeline), §5.1, §9 (D-98), `technical-backlog.md` TB-11.2, TB-11.5
* **Critique:** Brownfield Intent Scaffolding under Authority Inheritance breaks Invariant INV-1 due to missing root requirement synthesis.
  Decision D-98 and TB-11 state that when brownfield specifications (OpenAPI, ADRs, BDD) are ingested from pre-governed sources (`authority_source = 'BRANCH_PROTECTED_CODEOWNERS'`), candidate nodes bypass manual staging and are inserted directly into `graph_nodes` in `ACTIVE` state. TB-11 specifies that OpenAPI path operations and schemas are mapped directly to `SPECIFICATION` nodes.
  However, in an OpenAPI document, there are no `REQUIREMENT` entities. If the adapter creates `SPECIFICATION` nodes directly in `ACTIVE` state without an active `REQUIREMENT` node above them, every single extracted node immediately violates Invariant INV-1 (*"Every active functional specification... must maintain a valid directed edge path terminating at an authorized active requirement node. Orphan execution tasks [and specifications] must be rejected at the database constraint level"*).
* **Proposed Alternative:**
  Specify in TB-11 and §4 that `ScaffoldingAdapter` implementations must mechanically synthesize a root `REQUIREMENT` node from the document-level container (e.g. OpenAPI `info.title` and `info.description`, ADR category, or BDD Feature header), mirroring the Structural Heading Taxonomy (D-94). All extracted `SPECIFICATION` and `DECISION` nodes must mechanically generate upward `DERIVED_FROM` edges terminating at this root `REQUIREMENT` node, guaranteeing that direct-to-active Authority Inheritance satisfies Invariant INV-1 out-of-the-box.

---

### LD-5

* **Severity:** Major
* **Target:** `architecture.md` §6 (Interfaces: `get_elaboration_context`), §9 (D-95), `technical-backlog.md` TB-13.2
* **Critique:** Data sourcing for `get_elaboration_context` planning rails is completely unspecified and conflates TKS dogfooding specifics with the substrate engine.
  Key Capability 10, Decision D-95, and TB-13 define `get_elaboration_context` as a 4-axis planning dossier: Axis 1 (Normative Boundaries), Axis 2 (Architectural Precedents), Axis 3 (Physical Schema Contracts), and Axis 4 (Codebase Blueprints).
  TB-13 specifies that Axis 4 returns *"canonical repository implementation patterns (e.g. Axum route handler templates, CTE query conventions, test templates)"*.
  However:
  1. In a production deployment, `tks serve` only has access to PostgreSQL and the bare Git repository (`refs/heads/specs`); it has no filesystem access to the user's local working copy.
  2. The architecture provides no data model, table, schema, or configuration mechanism for how Axis 3 (schema contracts) or Axis 4 (codebase blueprints) are ingested, registered, or resolved.
  3. If Axis 4 templates are hardcoded in Rust for Axum route handlers and SQL CTEs, TKS fails its core vision as a general-purpose, open-source substrate for arbitrary external codebases.
* **Proposed Alternative:**
  Formalize the data model for planning rails. Blueprints and schema contracts must be modeled as first-class architectural entities in the graph (e.g., `SPECIFICATION` nodes with `attributes->'blueprint_type'` or a dedicated `blueprints` configuration manifest committed to `refs/heads/specs`). `get_elaboration_context` must dynamically assemble Axis 3 and Axis 4 by querying these stored graph entities rather than assuming hardcoded repository paths.

---

### LD-6

* **Severity:** Major
* **Target:** `architecture.md` §2 (C-11), §5.2, §9 (D-15, D-101), `strategic-planning-backlog.md` §2 (Phase 4 Deliverable 4.6b)
* **Critique:** Unresolvable conflict between LLM-driven requirement atomization, token minimization, and exact 0-based byte offset tracking.
  Phase 4 Deliverable 4.6b, Decision D-101, and `vision.md` state that Stage 2 semantic evaluation *"atomizes structural blocks into $1 \dots N$ candidate requirements"*.
  However:
  1. Decision D-22 and Invariant INV-4 mandate that every requirement node store exact 0-based byte offsets (`byte_start`, `byte_end`) derived from `pulldown-cmark`.
  2. Constraint C-11 mandates that LLMs output strictly compact classification tuples using ordinal aliases (`c1`, `c2`) and never echo verbatim source text.
  If an LLM receives a compound paragraph and atomizes it into multiple sub-requirements, an LLM cannot compute exact 0-based byte offsets into the raw source buffer, nor can it return them via compact ordinal tuples without echoing text.
* **Proposed Alternative:**
  Specify that requirement atomization is performed *mechanically* during Stage 1 prior to LLM classification. The mechanical parser must segment structural blocks into sentence/clause atoms using deterministic sentence boundary rules (e.g. `unicode-segmentation`), assigning each atom an exact `(byte_start, byte_end)` span and ordinal alias (`c1`, `c2`, ...). The Tier 2 LLM or classifier then evaluates these pre-atomized chunks strictly for normative modality and confidence, preserving exact byte provenance, zero echoing, and token minimization.

---

### LD-7

* **Severity:** Major
* **Target:** `architecture.md` §6 (Interfaces: `GET /api/v1/release/readiness`), §9 (D-93, D-103), `technical-backlog.md` TB-12.3
* **Critique:** Release readiness evaluation creates a circular gating deadlock between pre-merge CI and post-merge commits.
  TB-12.3 and §6 specify that `GET /api/v1/release/readiness` verifies that *"every leaf requirement maintains an active path through `SPECIFICATION` and `TASK` nodes terminating at verified test runs (`VERIFIED_BY`) and canonical commits (`IMPLEMENTED_BY`)"*.
  However, Decision D-103 explicitly established that canonical `CODE_COMMIT` nodes and `IMPLEMENTED_BY` edges are created *exclusively* upon canonical branch merge to `main`.
  If CI/CD pipelines query `GET /api/v1/release/readiness` to gate whether a PR can be merged to `main`, the query will ALWAYS return `BLOCKED` because the canonical merge commit cannot exist prior to merge.
* **Proposed Alternative:**
  Disambiguate readiness gating into two distinct operational scopes:
  1. `GET /api/v1/release/readiness?scope=pre-merge&pr_commit_sha=<sha>`: Evaluates strictly that all scoped `TASK` entities have passing `VERIFIED_BY` test runs matching the PR commit SHA.
  2. `GET /api/v1/release/readiness?scope=release&milestone=<tag>`: Evaluates that all scoped tasks are both verified (`VERIFIED_BY`) AND materialized in canonical merge commits (`IMPLEMENTED_BY`).

---

### LD-8

* **Severity:** Major
* **Target:** `architecture.md` §2 (C-22), §4 (WorkerManager, Embedding Worker), §5.1 (DDL), §9 (D-77), `technical-backlog.md` TB-6
* **Critique:** Architecture conflates in-process CPU vector inference (`fastembed-rs`) with external HTTP API rate-limiting.
  Decision D-77 and Phase 0 Spike 8 (DEC-0.12) formally confirmed that vector embeddings are generated locally on CPU using `fastembed-rs` executing `all-MiniLM-L6-v2` (384 dimensions) for 100% offline self-sufficiency.
  However, `architecture.md` §4, §5.1, and §5.2 (line 570) still specify:
  *"On HTTP 429 rate limits or transient errors, the worker increments `retry_count`, calculates exponential backoff delay... completely eliminating tight-loop API hammering."*
  An in-process CPU library (`fastembed-rs`) executed via `spawn_blocking` does not emit HTTP 429 status codes or suffer from external network rate limits. Describing embedding generation as an HTTP rate-limited provider API creates confusion in worker implementation and error handling.
* **Proposed Alternative:**
  Update the Embedding Worker specification in §4 and §5.2 to state that the default embedding engine is the in-process `fastembed-rs` provider running in `tokio::task::spawn_blocking` with standard task error handling (memory/panic catch). Clarify that HTTP rate-limit handling and exponential backoff on `scheduled_at` apply exclusively if an optional remote embedding API is explicitly configured.

---

### LD-9

* **Severity:** Minor
* **Target:** `architecture.md` §5.1 (line 450), §9 (D-82)
* **Critique:** Residual reference to retired `embedding_queue` table in draft approval documentation.
  Line 450 of `architecture.md` states: *"Promoted `ACTIVE` nodes are enqueued into `embedding_queue` with `scheduled_at = NOW()`."*
  However, Decision D-82 and §5.1 explicitly retired `embedding_queue` and consolidated all queue state into `node_embeddings`.
* **Proposed Alternative:**
  Update line 450 to reference `node_embeddings` with `status = 'PENDING'`, aligning with D-82 and eliminating contradictory table references.

---

## Component Simplifications

### LD-10

* **Severity:** Major (Simplification)
* **Target:** `architecture.md` §4, §5.1, §6, `technical-backlog.md` TB-1.7
* **Critique:** Branch workspaces duplicate caller-scoped draft capabilities and add an unexposed, complex 3-way merge subsystem.
  Branch workspaces (`workspaces` table, `src/storage/workspace.rs`, `src/storage/conflict.rs`) were introduced to enable lock-free parallel agent exploration (D-88).
  However:
  1. Workspaces are exposed exclusively via REST endpoints (`/api/v1/workspaces/...`). The MCP tool suite in §6 contains NO tools to create, list, rebase, or promote workspaces. External agent harnesses (Claude Code, Cursor, Windsurf, agy) communicating via MCP stdio or HTTP/SSE cannot utilize branch workspaces.
  2. In §5.1 and Decision D-75, TKS already implemented caller-scoped draft isolation via `created_by VARCHAR(64) NOT NULL` on `graph_nodes` and `graph_edges`, allowing agents to create and inspect their own `DRAFT` entities without locking or leaking drafts.
  3. Staging batch approvals (`POST /api/v1/documents/ingest/{job_id}/approve`) already handle ancestor validation, squashing, and promotion.
  Maintaining a complete parallel Git-like branch-merge engine in PostgreSQL (with three-way combined subgraph CTEs, auto-reparenting, and rebase endpoints) that is inaccessible over MCP adds massive implementation and maintenance overhead.
* **Proposed Alternative:**
  Consolidate branch workspaces into caller-scoped draft batches (`created_by`). Retire the separate `workspaces` table and complex 3-way merge CTEs in favor of standard caller-isolated draft staging and batch promotion.

---

### LD-11

* **Severity:** Major (Simplification)
* **Target:** `strategic-planning-backlog.md` §2 (Phase 6 Deliverable 6.1)
* **Critique:** Premature enterprise ALM interoperability protocols (ReqIF / OSLC) in Phase 6.
  Phase 6 specifies implementing export and synchronization adapters for enterprise ALM formats (OMG ReqIF XML and OASIS OSLC Linked Data). ReqIF and OSLC are notoriously complex enterprise standards requiring hundreds of pages of XML/RDF schema mapping. TKS explicitly disclaims being an enterprise ALM replacement (non-goal #3) and already provides open data export via SQL, JSON-LD, and REST. Forcing a solo developer or small team (C-9, DR-7) to build and maintain ReqIF/OSLC adapters creates massive implementation drag with near-zero utility for agentic software engineering.
* **Proposed Alternative:**
  Retire ReqIF and OSLC synchronization adapters from Phase 6. Standardize enterprise export exclusively on standard JSON-LD and SQL property graph dumps, preserving engineering focus on core graph governance and agent cognitive rails.

---

### LD-12

* **Severity:** Minor (Simplification)
* **Target:** `architecture.md` §4, §5.1, §6, §9 (D-99)
* **Critique:** Standalone `relationship_review_backlog` table creates an unnecessary review queue and redundant CRUD layer.
  Introducing a dedicated relational table `relationship_review_backlog`, separate status state machine (`PENDING`, `DISMISSED`, `RESOLVED`), and dedicated REST endpoints (`/api/v1/admin/review-backlog`) for advisory graph audits violates the Supervisory Queue Economy (vision §2 Key Capabilities (7)). It creates an isolated review silo for developers to monitor.
* **Proposed Alternative:**
  Eliminate the standalone `relationship_review_backlog` table. Direct `tks graph audit` to output human-readable diagnostic reports directly to CLI/logs, and surface graph anomalies directly through the existing anomaly triage interface (`--triage-anomalies` / `attributes->'extraction_metadata'`) or as candidate tasks in `NEEDS_REVERIFICATION`.
