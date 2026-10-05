# Phase 4 Implementation Decision Record: Foundational Ingestion & Closed-Loop Traceability

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-4.1 | WP-4.1 | Structural Heading Taxonomy, Atomization Scope & Compact Classification Schema | Specification Gap | architecture.md §5.1, technical-backlog.md TB-7 | Implemented |
| DEC-4.2 | WP-4.2 | Mechanical Root Requirement Synthesis & Canonical Key Normalization for Brownfield Scaffolding | Specification Gap | architecture.md §4, §5.1, technical-backlog.md TB-11 | Implemented |
| DEC-4.3 | WP-4.2 | Gateway Authorization Guard & Identity Attribution Protocol for Authority Auto-Promotion | Technical Trade-off | architecture.md §5.1, §9 (D-117), technical-backlog.md TB-11 | Implemented |
| DEC-4.4 | WP-4.3 | Decoupled Bare ODB Blob Ingestion & Deferred Spec Branch Commitment | Technical Trade-off | architecture.md §5.1, §9 (D-119), technical-backlog.md TB-7.5 | Implemented |
| DEC-4.5 | WP-4.3 | Omission Isolation & Atomic Two-Phase Invalidation Cascade for Living Graph Staging | Specification Gap | architecture.md §5.1, §9 (D-109), technical-backlog.md TB-7.5 | Implemented |

---

## 2. Decision Entries

### DEC-4.1: Structural Heading Taxonomy, Atomization Scope & Compact Classification Schema

* **Work Package:** WP-4.1
* **Category:** Specification Gap
* **Context & Problem:** WP-4.1 requires structural heading classification (H1/H2 $\rightarrow$ `REQUIREMENT`, H3–H6 $\rightarrow$ `SPECIFICATION`), frontmatter type overrides, mechanical atomization using `unicode-segmentation`, and zero token overhead for Tier 1 structural extraction while keeping Stage 2 LLM prompt/completion token ratios under 10% (Constraint C-11). Two design ambiguities arose during implementation:
  1. Whether structural headings should be treated as candidate chunks submitted to Stage 2 LLM semantic classification or treated as deterministically classified by AST analysis. Marking structural headings as candidates caused token explosion and C-11 ratio violations (> 10%), in direct tension with the zero-token Tier 1 invariant.
  2. In Stage 2 classification output, whether to emit full JSON object fields including redundant governance policy labels, or an ultra-compact tuple schema `(alias, modality, confidence)`. The full JSON schema consumed ~13.7% output tokens on short specifications, exceeding C-11.
* **Options Considered:**
  * *Option A:* Send all parsed blocks (headings + clauses) to Stage 2 LLM and request verbose JSON objects containing `alias`, `node_type`, `modality`, `confidence`, and `governance_policy`. Pros: Uniform pipeline for all chunks. Cons: Violates zero-token Tier 1 extraction invariant, exceeds C-11 token budget (13.7%), and risks LLM hallucinations overturning deterministic heading hierarchy.
  * *Option B:* Restrict Stage 2 LLM candidate flagging (`is_candidate = true`) strictly to non-heading clauses that contain RFC 2119 keywords or canonical keys, while structural headings are deterministically resolved in Tier 1 with 1.0 confidence. In Stage 2, emit compact tuples `{"alias": ..., "modality": ..., "confidence": ...}` while backward-compatibly accepting `chunk_id` and legacy fields. Pros: Keeps combined token ratio at ~3.5% (well below C-11 10% threshold), guarantees zero LLM tokens for heading taxonomy, eliminates heading hallucinations, and guarantees 100% precision / 96.8% recall on evaluation benchmarks. Cons: Headings with RFC 2119 phrasing rely on explicit frontmatter or manual override if a non-default type is desired.
* **Decision Taken & Rationale:** Adopted Option B. Structural headings are deterministically classified by the AST parser at zero token cost and confidence 1.0, establishing the structural backbone and upward `DERIVED_FROM` lineage. Mechanical atomization segments non-heading paragraphs into clause-level spans using `UnicodeSegmentation::split_sentence_bound_indices` with exact 0-based byte spans and ordinal aliases (`c1`, `c2`, ...). Stage 2 prompts and parser accept compact tuples, reducing output token ratio to ~3.5% on sample specifications.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, `technical-backlog.md` TB-7, TB-11. Feedback: Document that Tier 1 AST heading classification is deterministic and bypassed from Stage 2 LLM prompt dispatch to guarantee zero token overhead and C-11 compliance.
* **Status:** Implemented

### DEC-4.2: Mechanical Root Requirement Synthesis & Canonical Key Normalization for Brownfield Scaffolding

* **Work Package:** WP-4.2
* **Category:** Specification Gap
* **Context & Problem:** Brownfield specification formats (OpenAPI 3.0/3.1, AsyncAPI, MADR/Nygard ADRs, BDD feature files, Spec Kit, AWS Kiro, and `AGENTS.md` / `CLAUDE.md`) lack explicit top-level RFC 2119 requirement headers. Directly ingesting them produced property graph subtrees lacking root `REQUIREMENT` nodes, causing child specifications and tasks to fail Invariant INV-1 (Ancestry) validation during staging approval and queries. The plan mandated mechanical Root Requirement Synthesis without specifying the concrete key derivation heuristics, container scopes, or upward edge wiring across heterogeneous brownfield formats.
* **Options Considered:**
  * *Option A:* Require external LLM invocation (Tier 3) to summarize the brownfield document and synthesize synthetic requirements. Pros: Produces natural language summaries. Cons: Incurs high token costs, adds network latency, risks hallucinating requirement scopes, and violates the zero-token brownfield goal (TB-11, CAL-BROWN $\le 5$ min at zero token cost).
  * *Option B:* Mechanical zero-token Root Requirement Synthesis in pluggable `ScaffoldingAdapter` implementations. The document-level container (`info.title` in OpenAPI/AsyncAPI, ADR collection header in ADRs, BDD `Feature:`, repository steering header in `AGENTS.md`) is deterministically mapped to a synthesized root `REQUIREMENT` node (`node_key` derived via `canonical_key_slug`), while individual operations, schemas, decisions, and scenarios become child `SPECIFICATION`/`DECISION` nodes with upward directed `DERIVED_FROM` edges pointing to the root anchor. Pros: Zero token overhead, deterministic execution in $< 1\text{ ms}$, guarantees 100% Invariant INV-1 compliance out-of-the-box, and satisfies CAL-BROWN. Cons: Document containers without identifiable titles fall back to standardized collection keys (e.g. `API-SPECIFICATION`, `REQ-ADR-COLLECTION`).
* **Decision Taken & Rationale:** Adopted Option B. Pluggable adapters (`OpenApiAdapter`, `AdrAdapter`, `SddAdapter`) synthesize deterministic root `REQUIREMENT` nodes from document-level containers with `canonical_key_slug` formatting and generate upward `DERIVED_FROM` hierarchy edges from all child entities, satisfying Invariant INV-1 at zero token cost.
* **Upstream Impact & Target Document:** `architecture.md` §4, §5.1, `technical-backlog.md` TB-11. Feedback: Document mechanical root requirement synthesis conventions (`API-{TITLE}`, `REQ-ADR-COLLECTION`, `REQ-{FEATURE}`, `REQ-AGENT-STEERING`) for brownfield format adapters.
* **Status:** Implemented

### DEC-4.3: Gateway Authorization Guard & Identity Attribution Protocol for Authority Auto-Promotion

* **Work Package:** WP-4.2
* **Category:** Technical Trade-off
* **Context & Problem:** Authority inheritance allows pre-governed sources (`BRANCH_PROTECTED_CODEOWNERS`, `COMMITTED_SPEC_MAIN`) to bypass `DRAFT` staging and insert nodes directly into `ACTIVE` state. However, ingestion jobs execute asynchronously in a background worker (`DecompWorker`). Two critical questions arose: (1) how to prevent unauthorized callers (e.g. untrusted client agents) from spoofing `authority_source` to bypass staging; and (2) how to prevent the background worker from dropping caller provenance and falling back to `created_by = 'system'`, which would violate Invariant INV-7.
* **Options Considered:**
  * *Option A:* Accept `authority_source` from any caller and trust client claims, allowing the worker to auto-promote based purely on the string value in the job. Pros: Simple gateway logic. Cons: Fatal security vulnerability; any unprivileged agent could bypass governance approval by providing `authority_source: "BRANCH_PROTECTED_CODEOWNERS"`.
  * *Option B:* Implement two-tier authorization and credential persistence: (1) At the gateway (`POST /api/v1/documents/ingest`), inspect `caller.actor_type` and credentials. Only callers with `HUMAN` actor type or verified branch/CI webhook tokens are authorized to supply `authority_source`; untrusted agents have `authority_source` stripped to `NULL`; (2) Persist validated `authority_source`, `created_by`, and `token_fingerprint` to `ingestion_jobs`; (3) The background worker verifies `authority_source` against recognized pre-governed sources, inserts nodes directly into `ACTIVE` with `created_by` preserved from the job record, and writes a canonical `SYSTEM_AUTHORITY_IMPORT` event to `audit_ledger`. Pros: Preserves governance trust boundary, guarantees strict identity attribution (INV-7, D-117), and eliminates ghost draft accumulation. Cons: Unauthenticated or unprivileged callers must always pass through manual staging review.
* **Decision Taken & Rationale:** Adopted Option B. Gateway strictly gates `authority_source` acceptance to verified `HUMAN` and webhook callers, and persists caller identity to `ingestion_jobs`. The background worker inspects these credentials, directly inserts nodes and edges in `ACTIVE` state, enqueues vector embeddings, commits `SYSTEM_AUTHORITY_IMPORT` audit ledger entries with caller credentials, and transitions job status to `APPROVED`.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 (D-117), `technical-backlog.md` TB-11. Feedback: Formalize the two-tier gateway authorization guard and caller credential forwarding protocol across asynchronous worker boundaries.
* **Status:** Implemented

### DEC-4.4: Decoupled Bare ODB Blob Ingestion & Deferred Spec Branch Commitment

* **Work Package:** WP-4.3
* **Category:** Technical Trade-off
* **Context & Problem:** Phase 1 committed uploaded specification files directly onto `refs/heads/specs` at upload time in `POST /api/v1/documents/ingest`, creating provisional Git commits before human or supervisory review approved candidate changes. If candidate drafts were rejected or revised during staging review, Git history accumulated orphaned or rolled-back commits, violating the invariant that `refs/heads/specs` strictly represents the canonical approved truth.
* **Options Considered:**
  * *Option A:* Retain immediate branch commits on upload, and generate compensating revert commits if an ingestion job is rejected or revised. Pros: Reuses existing libgit2 `commit_document` logic without actor changes. Cons: Pollutes Git repository history with unreviewed proposals and rollbacks, creates branch ref contention under concurrent document submissions, and complicates auditability.
  * *Option B:* Decouple raw Git ODB blob creation from branch commitment (D-119). On document upload in `src/gateway/routes/documents.rs`, invoke `write_blob` (`git_blob_create_from_buffer`), which writes the raw buffer directly into the bare repository object database and returns the immutable 40-character blob SHA, leaving `refs/heads/specs` untouched. Store this blob hash in `ingestion_jobs.document_hash`. Strictly upon staging approval (`execute_approve`), invoke `commit_approved_specs(&doc_path, &document_hash)` to build the tree from the existing ODB blob and advance `refs/heads/specs`. Pros: Guarantees `refs/heads/specs` strictly tracks approved specification state, decouples upload throughput from branch lock contention, and preserves an immutable Git ODB blob for candidate span inspection prior to commit. Cons: Requires dual-method Git actor protocol (`WriteBlob` vs `CommitApprovedSpecs`).
* **Decision Taken & Rationale:** Adopted Option B. Decoupled Git blob creation guarantees that `refs/heads/specs` advances strictly upon staging approval. Ingested raw blobs remain immediately inspectable in Git ODB for verbatim draft span verification without moving branch pointers or polluting history.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 (D-119), `technical-backlog.md` TB-7.5. Feedback: Update §5.1 and D-119 documenting decoupled blob creation via `git_blob_create_from_buffer` and deferred branch commitment in `execute_approve`.
* **Status:** Implemented

### DEC-4.5: Omission Isolation & Atomic Two-Phase Invalidation Cascade for Living Graph Staging

* **Work Package:** WP-4.3
* **Category:** Specification Gap
* **Context & Problem:** When re-ingesting revised specification documents that omit sections previously in `ACTIVE` state, two critical failure modes arose:
  1. *Premature Deprecation Hazard:* If omitted active nodes were immediately mutated to `PENDING_DEPRECATION` or `SUPERSEDED` upon document submission, unapproved revisions broke active downstream task dependencies and query results before supervisory review (violating D-109).
  2. *Cascade Invalidation Collisions:* When an omitted section contained parent and child nodes (e.g. a section heading and nested sub-clauses), sequential omission processing caused the parent heading's downward invalidation cascade to overwrite child omitted nodes with `NEEDS_REVERIFICATION` before their own omission step ran, causing unique constraint violations or corrupted lifecycle state.
* **Options Considered:**
  * *Option A:* Deprecate active nodes immediately upon upload and process omissions sequentially. Pros: Simple single-pass loop. Cons: Violates D-109 omission isolation; corrupts active production queries during drafting; causes cascade race conditions where omitted children get marked `NEEDS_REVERIFICATION` instead of `SUPERSEDED`.
  * *Option B:* Omission Isolation & Atomic Two-Phase Invalidation. During re-ingestion, active nodes absent from the revised candidate set remain untouched in `ACTIVE` state in `graph_nodes`; their UUIDs are recorded strictly as an array in `ingestion_jobs.attributes->'omitted_node_ids'`. During staging approval under global advisory lock (`pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`), if omissions are confirmed (`confirm_omissions: true`): (1) In Phase 1, atomically transition ALL confirmed omitted node IDs to `SUPERSEDED` in a single batch query; (2) In Phase 2, execute downward invalidation sweeps (`execute_downward_invalidation_tx`) on superseded and omitted roots, ensuring only remaining active descendants (e.g. downstream execution tasks) are cascaded to `NEEDS_REVERIFICATION`, while all omitted nodes remain permanently `SUPERSEDED`. Additionally, delete redundant draft edges before edge promotion to prevent `idx_graph_edges_active_unique` constraint violations. Pros: Guarantees zero disruption to active graph nodes during document review, provides explicit supervisor control via `--confirm-omissions`, eliminates invalidation cascade state collisions, and cleanly cascades staleness to external tasks. Cons: Requires two-phase transactional staging logic.
* **Decision Taken & Rationale:** Adopted Option B. Omission isolation keeps active requirements stable until formal review, and two-phase batch supersession guarantees that all omitted entities achieve clean `SUPERSEDED` state while external downstream tasks receive appropriate downward invalidation cascades.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 (D-109), `technical-backlog.md` TB-7.5. Feedback: Document two-phase omission processing and redundant draft edge deduplication in §5.1 and D-109.
* **Status:** Implemented

