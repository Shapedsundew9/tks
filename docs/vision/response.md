# Lead Developer Sub-Agent Technical Review & Alignment Critique

## Executive Assessment

As Lead Developer, I affirm our commitment to the Knowledge Substrate's core mission: grounding external autonomous agents to a version-governed property graph. The architectural foundations established in Phases 0 through 3—PostgreSQL single-engine topology, Git ODB integration via an isolated write actor, embedded refinery migrations, and advisory-locked DAG cycle enforcement—are robust, well-conceived, and sound.

However, comparing the architecture baseline (`architecture.md`) against the governing vision (`vision.md`), the Phase 4 strategic reprioritization (`strategic-planning-backlog.md`), and the recent empirical dogfooding review reveals several critical blockers and major architectural gaps. Specifically:

1. Concrete relational schemas (`chk_node_type`) strictly reject `CODE_COMMIT` nodes, and interfaces for pre-merge CI test run reporting are omitted, blocking closed-loop traceability.
2. The mechanical AST decomposition pipeline still relies on RFC 2119 keywords rather than structural heading depth, producing documents with zero active `REQUIREMENT` nodes and triggering immediate Invariant INV-1 ancestry validation failures out-of-the-box.
3. The cognitive planning rails (`get_elaboration_context`), camelCase MCP tool serialization (`inputSchema`), Actionable Remediation Envelopes (INV-8), and relational edge pre-validation (INV-9) are missing from the architecture and contract specifications.
4. Redundant components (such as an asynchronous `CascadeWorker` alongside synchronous single-roundtrip invalidation CTEs, and provisional commit graph nodes) add unnecessary operational friction.

The following 10 ranked findings and 3 architectural simplifications address these gaps to ensure the engineering team can build, deploy, and operate TKS cleanly.

---

## Ranked Findings (LD-1 to LD-10)

### LD-1

* **Severity:** `Blocker`
* **Target:** `architecture.md` §5.1 (Entity Typing and Relational Constraints) & §6 (Interfaces & Contracts)
* **Critique:** The governing vision (`vision.md` §5 Invariant I-9, Key Capability 1) and strategic backlog (`strategic-planning-backlog.md` Phase 4 Deliverable 4.4) establish closed-loop traceability from requirements to executable code as the central objective of Phase 4. This requires materializing canonical merge commits to `main` as `CODE_COMMIT` graph nodes linked via `IMPLEMENTED_BY` edges, and reporting CI test runs via `POST /api/v1/verification/test-run`. However, the concrete DDL constraint in `architecture.md` §5.1 defines:

  ```sql
  ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type
      CHECK (
          (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'UNCLASSIFIED'))
          OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION'))
      );
  ```

  This check constraint strictly excludes `'CODE_COMMIT'`. Any attempt by CI webhooks or release handlers to insert a `CODE_COMMIT` node fails with a fatal PostgreSQL `23514 check_violation`. Additionally, `architecture.md` §5.1 edge rules omit `IMPLEMENTED_BY`, and §6 completely omits `POST /api/v1/verification/test-run`, `POST /api/v1/vcs/commits`, and `GET /api/v1/release/readiness`. As specified, closed-loop traceability cannot be built or operated.
* **Proposed Alternative:**
  1. Update `chk_node_type` DDL in `architecture.md` §5.1 to include `'CODE_COMMIT'`:

     ```sql
     ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type
         CHECK (
             (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'CODE_COMMIT', 'UNCLASSIFIED'))
             OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'CODE_COMMIT'))
         );
     ```

  2. Formally document the `IMPLEMENTED_BY` edge type in §5.1 (connecting `TASK` / `SPECIFICATION` nodes to `CODE_COMMIT` nodes).
  3. Add the three missing traceability endpoints to §6 Interfaces & Contracts:
     * `POST /api/v1/verification/test-run`: accepts test suite run metadata and candidate PR commit SHA, creating `VERIFIED_BY` edges to active `TASK` entities.
     * `POST /api/v1/vcs/commits`: webhook creating `CODE_COMMIT` nodes and `IMPLEMENTED_BY` edges upon merge to `main`.
     * `GET /api/v1/release/readiness`: reports requirement satisfaction based on active `VERIFIED_BY` and `IMPLEMENTED_BY` paths.

---

### LD-2

* **Severity:** `Blocker`
* **Target:** `architecture.md` §5.2 (Concurrency Model - Ingestion) & §9 (Decision D-15)
* **Critique:** In the Phase 4 dogfooding experiment (`phase4-experiment.sh`), AST decomposition categorized all Markdown headings as `SPECIFICATION`, producing zero active `REQUIREMENT` nodes. When the external agent attempted to elaborate tasks, Invariant INV-1 validation failed with `ERR_ANCESTRY_VALIDATION_FAILED` because child tasks had no active `REQUIREMENT` root ancestor. In `architecture.md` §5.2 and Decision D-15, Stage 1 extraction still specifies assigning `node_type = 'REQUIREMENT'` strictly upon matching RFC 2119 keywords, and `'UNCLASSIFIED'` otherwise. In real-world specifications, top-level headings and structural containers rarely contain RFC 2119 modal verbs ("SHALL", "MUST"), guaranteeing that ingested documents yield graph trees lacking root `REQUIREMENT` anchors and blocking task elaboration across the entire substrate.
* **Proposed Alternative:**
  Update §5.2 and Decision D-15 to replace keyword regex heuristics with the **Structural Heading Taxonomy** defined in `strategic-planning-backlog.md` Deliverable 4.1:
  1. During Stage 1 streaming CommonMark AST parsing (`pulldown-cmark`), top-level structural containers and Level-1 (`#`) or Level-2 (`##`) headings automatically classify as `REQUIREMENT` anchors by structural hierarchy depth, unless overridden by explicit frontmatter metadata (`type: REQUIREMENT` or `type: SPECIFICATION`).
  2. Sub-sections (Level-3+ headings) classify as `SPECIFICATION`.
  3. Non-heading paragraphs and lists inherit the enclosing section's typing or default to `'UNCLASSIFIED'` in `DRAFT`.
  4. Upward `DERIVED_FROM` hierarchy edges are mechanically generated between sub-sections and their parent headings, ensuring that every approved specification and task maintains a valid upward path to an active `REQUIREMENT` root out-of-the-box.

---

### LD-3

* **Severity:** `Major`
* **Target:** `architecture.md` §6 (Interfaces & Contracts) & §7 (Technology Stack)
* **Critique:** The Phase 4 empirical review demonstrated that `get_context_envelope` is insufficient for strategic planning or deliverable elaboration: clamped to $\le 3$ hops and a 40-node budget, it delivers narrow leaf context stripped of governing architectural invariants (`INV-*`), historical design decisions (`D-*`), database schemas, and codebase blueprints. To resolve this, `vision.md` (Key Capability 10) and `strategic-planning-backlog.md` (Phase 5 Deliverable 5.1) introduced `get_elaboration_context`. However, `get_elaboration_context` is completely missing from `architecture.md` Section 6 and Section 7. Furthermore, the empirical dogfooding experiment revealed that the MCP stdio server serialized tool parameter schemas using snake_case `input_schema` instead of the official MCP specification's camelCase `inputSchema`, causing agent harnesses (`agy`, Claude Code) to reject tool definitions. `architecture.md` fails to specify camelCase `inputSchema` serialization.
* **Proposed Alternative:**
  1. Add `get_elaboration_context` to `architecture.md` §6 as both an MCP tool and REST endpoint (`GET /api/v1/nodes/{id}/elaboration-context`), specifying its 4-axis synthesis payload: Normative Boundaries (active invariants `INV-1`..`INV-9`), Architectural Precedents (matching `D-*` records), Physical Schema Contracts (node/edge enums and check constraints), and Codebase Blueprints (canonical handler and CTE paths).
  2. Explicitly specify in §6 and §7 that the MCP server gateway strictly adheres to the official MCP 2024-11-05 wire protocol specification, serializing tool schemas with camelCase `inputSchema` to ensure seamless client permission harness handshakes.

---

### LD-4

* **Severity:** `Major`
* **Target:** `architecture.md` §3 (Architectural Invariants) & §6.2 (Failure Semantics & Error Taxonomy)
* **Critique:** `vision.md` §5 defines nine governing invariants, including `Invariant I-8 (Actionable Diagnostics & Self-Healing Remediation Envelopes)` and `Invariant I-9 (Physical Edge Relational Integrity & Pre-Merge Verification Gating)`. However, `architecture.md` Section 3 lists only `INV-1` through `INV-7`. In §6.2, ancestry validation failure is modeled as a bare string error (`ERR_ANCESTRY_VALIDATION_FAILED`), which the dogfooding experiment proved deadlocks external agents because they receive zero diagnostic details about where the ancestor chain broke or how to fix it within governance rules.
* **Proposed Alternative:**
  1. Incorporate `INV-8` and `INV-9` directly into `architecture.md` Section 3 to ensure alignment with `vision.md`.
  2. Formalize the Actionable Remediation Envelope schema in §6.2: when an ancestry validation check fails, the gateway must return HTTP 422 with a structured payload:

     ```json
     {
       "code": "ERR_INVALID_ANCESTOR_PATH",
       "message": "Invariant INV-1 violated: task lacks path to active REQUIREMENT root",
       "remediation": {
         "target_node_id": "<uuid>",
         "terminal_node_id": "<uuid>",
         "terminal_node_type": "SPECIFICATION",
         "suggested_actions": [
           {
             "action": "PROMOTE_ANCESTOR",
             "node_id": "<uuid>",
             "target_type": "REQUIREMENT",
             "authorized": true
           }
         ]
       }
     }
     ```

  3. Specify gateway handling for executing authorized remediation actions within the caller's governance scope.

---

### LD-5

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (Entity Typing) & §6.2 (Failure Semantics & Error Taxonomy)
* **Critique:** In the Phase 4 dogfooding review, external agents attempting task elaboration submitted edge targets containing raw string literals (such as external Git commit SHAs, e.g. `c3a1f9e...`) instead of valid registered node UUIDs. In `architecture.md`, edge insertion transactions execute raw SQL without prior endpoint validation, causing unhandled PostgreSQL foreign key violations (`23503 foreign_key_violation`) that abort transactions and return uninformative 500 or 400 errors. `architecture.md` §6.2 lacks the `ERR_INVALID_EDGE_TARGET` error definition and does not mandate pre-transaction endpoint resolution.
* **Proposed Alternative:**
  1. Update §5.2 to mandate that `propose_node_mutation` and workspace task elaboration execute a pre-transaction validation pass verifying that all proposed edge endpoints (`from_node_id`, `to_node_id`) are valid UUIDs that exist in `graph_nodes`.
  2. Add `ERR_INVALID_EDGE_TARGET` (HTTP 422 Unprocessable Entity) to §6.2 Error Taxonomy table, specifying that the response payload returns the offending string value and field name.

---

### LD-6

* **Severity:** `Major`
* **Target:** `architecture.md` §4 (Component Topology) & §5.1 (State Ownership)
* **Critique:** `vision.md` (§1, §2 Key Capability 2, §3) and `strategic-planning-backlog.md` (Phase 4 Deliverable 4.5) introduce Brownfield Intent Scaffolding (extracting baseline nodes from OpenAPI/Protobuf, ADRs, BDD feature files, Spec Kit, Kiro, `AGENTS.md`) and the Authority Inheritance Principle to eliminate the cold-start adoption barrier. However, `architecture.md` assumes that all ingested artifacts are Markdown documents parsed by `pulldown-cmark`. It provides no component topology, adapter abstraction, or data model support for non-Markdown schemas, and no mechanism for pre-governed brownfield artifacts (e.g. committed schemas on protected branches) to inherit authoritative status without manual staging review.
* **Proposed Alternative:**
  1. In §4 Component Topology, define a pluggable `ScaffoldingAdapter` trait within the Ingestion Pipeline supporting zero-token mechanical parsers for OpenAPI, ADRs, and SDD files.
  2. In §5.1 `ingestion_jobs`, add `authority_source VARCHAR(64)`. When an ingestion job is verified against a pre-governed repository source (e.g. `BRANCH_PROTECTED_CODEOWNERS`), candidate nodes inherit baseline authority and can be automatically promoted to `ACTIVE` upon ingestion, bypassing manual staging while recording provenance in `audit_ledger`.

---

### LD-7

* **Severity:** `Major`
* **Target:** `architecture.md` §4 (Component Topology), §5.1 (State Ownership), & §8 (Operational Model)
* **Critique:** `vision.md` (Key Capability 9) and `strategic-planning-backlog.md` (Phase 6 Deliverables 6.2 & 6.3) specify a scheduled or on-demand graph maintenance auditor (`tks graph audit`) emitting findings to `relationship_review_backlog`. Crucially, to prevent unconstrained agent churn, this worker operates strictly in an advisory capacity without directly mutating active topology. However, `architecture.md` completely omits `relationship_review_backlog` from Table 5.1 State Ownership and DDL, omits the audit command from Section 8 Operational Model, and provides no interface in Section 6 for inspecting or clearing advisory backlog entries.
* **Proposed Alternative:**
  1. In §5.1, add DDL for `relationship_review_backlog`:

     ```sql
     CREATE TABLE relationship_review_backlog (
         id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
         source_node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
         target_node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
         anomaly_type VARCHAR(32) NOT NULL,
         confidence FLOAT NOT NULL,
         rationale TEXT NOT NULL,
         suggested_action VARCHAR(32) NOT NULL,
         status VARCHAR(16) NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'ACCEPTED', 'DISMISSED')),
         created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
     );
     CREATE INDEX idx_review_backlog_status ON relationship_review_backlog(status);
     ```

  2. Add `tks graph audit` to §8 Operational Model, specifying its two-step execution (deterministic structural linting first, local LLM evaluation second).
  3. Add `GET /api/v1/admin/review-backlog` and `POST /api/v1/admin/review-backlog/{id}/dismiss` to §6 Interfaces & Contracts.
  4. Codify the architectural invariant that background relationship workers operate strictly in read-only advisory mode and cannot mutate active graph edges.

---

### LD-8

* **Severity:** `Major`
* **Target:** `architecture.md` §4 (Component Topology), §7 (Technology Stack), & §9 (Decision D-87)
* **Critique:** Decision D-87 and DEC-3.13 established that invalidation cascades execute synchronously as a single-roundtrip multi-statement recursive CTE (`DOWNWARD_INVALIDATION_SQL`) in PostgreSQL within the mutation transaction holding `pg_advisory_xact_lock`, taking $<6\text{ ms}$ p95. Despite this, Section 4 Table 4.1, Section 7 (line 315), and Section 8 (line 620) continue to list an asynchronous background `CascadeWorker` loop in `WorkerManager` polling PostgreSQL via `FOR UPDATE SKIP LOCKED`. Maintaining an asynchronous background cascade worker creates an architectural contradiction, introduces a race condition where external agents can query un-invalidated stale requirements before the background loop runs, and wastes database connection pool slots.
* **Proposed Alternative:**
  Formally retire and remove `CascadeWorker` from `WorkerManager` in §4, §7, and §8. Reaffirm that invalidation cascades execute exclusively and synchronously within the mutation transaction under `pg_advisory_xact_lock`, guaranteeing immediate topological consistency across all read queries before transaction commit.

---

### LD-9

* **Severity:** `Major`
* **Target:** `architecture.md` §5.2 (Concurrency Model) & §6 (Interfaces & Contracts)
* **Critique:** `vision.md` (Key Capabilities 2 & 7) and `strategic-planning-backlog.md` (Phase 4 Deliverable 4.6) mandate a decoupled two-stage semantic ingestion pipeline: Stage A (Intra-Node Quality Assessment, Atomization into $1 \dots N$ candidate requirements, modality tagging, and `extraction_confidence` scoring) strictly preceding Stage B (Inter-Node Topological Contradiction Risk). Furthermore, staging review in CLI (`tks staging list`) and Web Explorer must preserve the source document's hierarchical narrative order by default, pairing verbatim source spans with normalized candidate statements and displaying inline visual badges. In `architecture.md` §5.2 and §6, Stage 2 remains specified as an older single-pass tuple classifier, `attributes->'extraction_metadata'` is missing from the data model, and `GET /api/v1/documents/ingest/{job_id}` lacks narrative ordering and anomaly filtering (`--triage-anomalies`).
* **Proposed Alternative:**
  1. Update §5.2 to formally define the two-stage semantic sequence: Stage A evaluates atomicity and assigns modality (`SHALL`/`SHOULD`/`MAY`) and `extraction_confidence` (0.0 to 1.0); Stage B evaluates contradiction risk against active approved requirements.
  2. Specify the `extraction_metadata` JSONB schema inside `graph_nodes.attributes` in §5.1.
  3. Update `GET /api/v1/documents/ingest/{job_id}` in §6 to return candidate nodes structured in native narrative sequence with parent heading paths, and support query parameter `triage_anomalies=true` for flat exception scanning.

---

### LD-10

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (Document Revision Reconciliation) & `technical-backlog.md` (TB-7.5)
* **Critique:** TB-7.5 specifies that during document re-ingestion, "For deleted sections, transition active nodes anchored to doc_path missing from the approved revision to SUPERSEDED." Automatically superseding missing nodes triggers immediate downward invalidation cascades that mark all active child tasks as `NEEDS_REVERIFICATION`. In contrast, `vision.md` (lines 305, 406) and `strategic-planning-backlog.md` (line 160) mandate that omitted sections are flagged for *supervised deprecation* rather than triggering hard orphan cascades. Automatically superseding omitted sections introduces severe operational risk: if an author re-ingests a partial document draft or renames an un-anchored section, in-flight agent tasks across entire subtrees are prematurely invalidated without human consent.
* **Proposed Alternative:**
  Harmonize §5.1 and TB-7 with `vision.md`: on document re-ingestion, active nodes anchored to `doc_path` that are absent in the revised candidate set must not be automatically superseded. Instead, the staging approval transaction flags omitted nodes as `PENDING_DEPRECATION` in staging metadata and presents them to the supervisor for explicit confirmation. Hard supersession and downward invalidation cascade execute only if the supervisor explicitly approves deprecation.

---

## Architectural Simplifications (LD-11 to LD-13)

### LD-11

* **Severity:** `Major`
* **Target:** `architecture.md` §4 (Component Topology) & §7 (Technology Stack)
* **Critique:** As detailed in LD-8, retaining an asynchronous `CascadeWorker` polling loop in `WorkerManager` when Decision D-87 already established a synchronous, single-roundtrip multi-statement CTE (`DOWNWARD_INVALIDATION_SQL`, executing in $<6\text{ ms}$ p95) introduces redundant code, connection pool churn, and potential race windows.
* **Proposed Alternative:** Retire `CascadeWorker` completely. Consolidate invalidation cascade execution into the `StorageRepo` layer as part of the synchronous structural mutation / rollback transaction.

---

### LD-12

* **Severity:** `Major`
* **Target:** `architecture.md` §5.1 (State Ownership) & `strategic-planning-backlog.md` (Phase 4 Deliverable 4.4)
* **Critique:** `strategic-planning-backlog.md` and `vision.md` describe two competing mechanisms for pre-merge CI verification gating: (1) linking test runs directly to active `TASK` entities via `VERIFIED_BY` edges carrying the PR head commit SHA in verification metadata attributes, or (2) creating ephemeral "provisional `CODE_COMMIT`" graph nodes. Provisional commit nodes pollute the property graph with unmerged branch SHAs that must later be reconciled, superseded, or garbage collected when a PR is squashed, rebased, or discarded.
* **Proposed Alternative:** Eliminate provisional `CODE_COMMIT` graph nodes entirely. Standardize pre-merge CI verification strictly on binding `VERIFIED_BY` edges directly from test runs to active `TASK` nodes, with the candidate PR branch commit SHA stored in `VERIFIED_BY` edge or verification task attributes (`attributes->'pr_commit_sha'`). Only canonical merge commits to `main` or signed release tags materialize as permanent `CODE_COMMIT` nodes in `graph_nodes`.

---

### LD-13

* **Severity:** `Major`
* **Target:** `architecture.md` §4 (Component Topology) & `strategic-planning-backlog.md` (Phase 4 Deliverable 4.6)
* **Critique:** `strategic-planning-backlog.md` outlines Tier 2 (Local LLM Pre-Analysis) and Tier 3 (Commercial LLM Escalation) within the ingestion pipeline. Designing server-side commercial LLM orchestration into `tks serve` introduces immense complexity: external API provider clients, secret management, prompt versioning, rate limiting, and timeout retries inside the server daemon, violating Invariant I-3 and Environmental Contract C-7/C-8 (externalized cognitive compute).
* **Proposed Alternative:** Keep `tks serve` completely free of commercial LLM provider SDKs. For Tier 2, maintain optional local Ollama inference over standard OpenAI-compatible REST API for background pre-analysis. For Tier 3 commercial escalation, externalize the workflow entirely to external client agents communicating over standard MCP: the supervisor or external agent queries staged candidate nodes (`tks staging list` or MCP `get_staged_candidates`), runs commercial inference within its own harness, and submits refined classifications back through `propose_node_mutation`. This eliminates server-side commercial LLM dependencies entirely while fully satisfying the vision.
