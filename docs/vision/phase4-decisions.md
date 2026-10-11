# Phase 4 Implementation Decision Record: Foundational Ingestion & Closed-Loop Traceability

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-4.1 | WP-4.1 | Structural Heading Taxonomy, Atomization Scope & Compact Classification Schema | Specification Gap | architecture.md §5.1, technical-backlog.md TB-7 | Implemented |
| DEC-4.2 | WP-4.2 | Mechanical Root Requirement Synthesis & Canonical Key Normalization for Brownfield Scaffolding | Specification Gap | architecture.md §4, §5.1, technical-backlog.md TB-11 | Implemented |
| DEC-4.3 | WP-4.2 | Gateway Authorization Guard & Identity Attribution Protocol for Authority Auto-Promotion | Technical Trade-off | architecture.md §5.1, §9 (D-117), technical-backlog.md TB-11 | Implemented |
| DEC-4.4 | WP-4.3 | Decoupled Bare ODB Blob Ingestion & Deferred Spec Branch Commitment | Technical Trade-off | architecture.md §5.1, §9 (D-119), technical-backlog.md TB-7.5 | Implemented |
| DEC-4.5 | WP-4.3 | Omission Isolation & Atomic Two-Phase Invalidation Cascade for Living Graph Staging | Specification Gap | architecture.md §5.1, §9 (D-109), technical-backlog.md TB-7.5 | Implemented |
| DEC-4.6 | WP-4.4 | MCP Official Wire Conformance & CamelCase inputSchema Serialization | Specification Gap | architecture.md §6, §7, §9 (D-95), technical-backlog.md TB-4 | Implemented |
| DEC-4.7 | WP-4.4 | Relational Edge Target Pre-Validation & Actionable Remediation Envelopes | Architecture Invariant | architecture.md §5.2, §6.2, §9 (D-96, D-97), technical-backlog.md TB-13.3 | Implemented |
| DEC-4.8 | WP-4.4 | Asynchronous Stdio Proxy Streaming & Non-Blocking Initialization Fallback | Technical Trade-off | architecture.md §7, technical-backlog.md TB-4, TB-13.1 | Implemented |
| DEC-4.9 | WP-4.5 | Task Attribute Pre-Merge Verification Ingestion and Circular Gating Deadlock Resolution | Specification Gap | architecture.md §5.1, §9 (D-107), technical-backlog.md TB-10 | Implemented |
| DEC-4.10 | WP-4.5 | Upward Canonical Merge Commit Lineage (CODE_COMMIT -IMPLEMENTS-> TASK) and Two-Scope Release Readiness Gating | Architecture Invariant | architecture.md §5.1, §9 (D-108, D-113, D-120), technical-backlog.md TB-10, TB-13.3 | Implemented |
| DEC-4.11 | WP-4.5 | Code-Side Contract Drift Detection via Unified Diff Parsing and Specification Path Attribution | Specification Gap | architecture.md §5.1, §9 (D-114), technical-backlog.md TB-10 | Implemented |

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
 
### DEC-4.6: MCP Official Wire Conformance & CamelCase inputSchema Serialization

* **Work Package:** WP-4.4
* **Category:** Specification Gap
* **Context & Problem:** The official Model Context Protocol (MCP 2024-11-05 spec) mandates that tool parameter schemas serialize under the camelCase property key `inputSchema`. Prior implementations serialized tool schemas using Rust snake_case `input_schema` or inconsistent parameter formats, causing official MCP clients (e.g. Claude Code, Cursor, Windsurf) to fail tool discovery and schema validation during `tools/list`.
* **Options Considered:**
  * *Option A:* Maintain snake_case `input_schema` and require client-side shims or custom MCP proxies. Pros: No changes to server models. Cons: Breaks official MCP 2024-11-05 protocol conformance and ecosystem interoperability.
  * *Option B:* Enforce strict camelCase serialization via `#[serde(rename = "inputSchema")]` across all `McpToolDefinition` structs and responses, verifying zero snake_case `input_schema` occurrences in wire payloads. Pros: 100% compliant with MCP 2024-11-05 specification across all 8 knowledge substrate tools; interoperable with Claude Code, Cursor, Windsurf out of the box. Cons: None.
* **Decision Taken & Rationale:** Adopted Option B. All tool definitions serialize parameter schemas strictly under `inputSchema`, verified by regression test `test_mcp_wire_protocol_camel_case_input_schema`.
* **Upstream Impact & Target Document:** `architecture.md` §6, §7, §9 (D-95), `technical-backlog.md` TB-4.
* **Status:** Implemented

### DEC-4.7: Relational Edge Target Pre-Validation & Actionable Remediation Envelopes

* **Work Package:** WP-4.4
* **Category:** Architecture Invariant
* **Context & Problem:** When clients or sub-agents invoke task creation or draft proposal APIs referencing raw Git commit SHAs (e.g. `c0ffee...`) or non-existent entity IDs, executing PostgreSQL transactions without pre-validation caused foreign-key constraint violations, transaction aborts, and opaque database error logs. Invariants INV-8 (Actionable Remediation Envelopes) and INV-9 (Endpoint Integrity) mandate that invalid identifiers be detected and rejected before lock acquisition and transaction boundary entry.
* **Options Considered:**
  * *Option A:* Rely on PostgreSQL foreign key constraints to catch invalid IDs on INSERT, trapping the error and mapping to HTTP 400. Pros: Minimal application code. Cons: Incurs database transaction abort overhead, acquires advisory locks before failure, and yields opaque relational errors without candidate repair guidance.
  * *Option B:* Pre-transaction validation query verifying that `from_node_id` and `to_node_id` parse as valid UUIDs and exist in `graph_nodes`. If invalid, immediately return HTTP 422 / JSON-RPC error `-32000` with code `ERR_INVALID_EDGE_TARGET` containing an Actionable Remediation Envelope (`invalid_field`, `invalid_value`, `suggested_repairs`, `valid_target_candidates`). Pros: Zero lock contention, prevents database error log pollution, satisfies Invariant INV-8 and INV-9, and provides automated repair options to LLM callers. Cons: Extra read query before edge insertion.
* **Decision Taken & Rationale:** Adopted Option B. Lightweight endpoint check before transaction opening guarantees relational integrity and returns clear structured remediation envelopes.
* **Upstream Impact & Target Document:** `architecture.md` §5.2, §6.2, §9 (D-96, D-97), `technical-backlog.md` TB-13.3.
* **Status:** Implemented

### DEC-4.8: Asynchronous Stdio Proxy Streaming & Non-Blocking Initialization Fallback

* **Work Package:** WP-4.4
* **Category:** Technical Trade-off
* **Context & Problem:** `tks mcp-stdio` provides a stdio JSON-RPC streaming adapter for MCP clients. When the TKS daemon is unreachable or in non-interactive testing environments, synchronous blocking on `std::io::stdin().read_line()` locked the Tokio execution thread indefinitely because standard input remained open without delivering bytes.
* **Options Considered:**
  * *Option A:* Retain synchronous stdin locking and rely on external process killing. Pros: Minimal code changes. Cons: Test harnesses and non-interactive environments hang indefinitely; blocks asynchronous tasks on executor threads.
  * *Option B:* Refactor stdio adapter to use asynchronous I/O (`tokio::io::AsyncBufReadExt`, `tokio::io::AsyncWriteExt`) and decouple stream handling into `run_mcp_stdio_io<R, W>`. In offline daemon mode, implement a 1-second timeout on initial request read; if no client request arrives within the window, emit standard JSON-RPC remediation error `-32000` with ID 1 and terminate cleanly with exit code 0. Pros: Eliminates executor thread blocking, prevents test lockups, allows unit testing via in-memory stream buffers, and gracefully handles unresponsive MCP clients. Cons: Requires generic async reader/writer abstractions.
* **Decision Taken & Rationale:** Adopted Option B. Non-blocking async I/O with timed initial read eliminates stdin deadlocks while maintaining full MCP stdio proxy capabilities.
* **Upstream Impact & Target Document:** `architecture.md` §7, `technical-backlog.md` TB-4, TB-13.1.
* **Status:** Implemented

### DEC-4.9: Task Attribute Pre-Merge Verification Ingestion and Circular Gating Deadlock Resolution

* **Work Package:** WP-4.5
* **Category:** Specification Gap
* **Context & Problem:** In pre-merge CI pipelines, test runs and verification results must be validated against a task before pull request approval and merge. However, if pre-merge test runs materialized provisional commit nodes or graph edges (`VERIFIES`), this would either create provisional unmerged commits in the graph before merge or require a canonical Git merge commit SHA that does not yet exist. Furthermore, if release readiness gating required canonical commit nodes prior to merge, a circular dependency deadlock would occur: the merge cannot proceed without release readiness approval, but release readiness cannot pass without the merge commit.
* **Options Considered:**
  * *Option A:* Materialize provisional `TEST_RUN` or provisional `CODE_COMMIT` graph nodes and provisional `VERIFIES` edges during pre-merge CI. Pros: Retains uniform node/edge representation for all test events. Cons: Pollutes graph topology with unmerged branch states, violates Invariant INV-1 (Ancestry) because provisional commits might never be merged or rebased, and requires complex reconciliation or deletion upon PR revision.
  * *Option B:* Ingest pre-merge CI verification results directly into `TASK.attributes->'pre_merge_verification'` as an atomically appended JSONB array via `POST /api/v1/verification/test-run` and `tks task test-run` (D-107), without materializing provisional commit nodes or graph edges. Bifurcate release readiness inspection (`GET /api/v1/release/readiness`) into explicit scopes: `scope=pre-merge` checks task status and `pre_merge_verification` test passes; `scope=release` requires canonical materialized `CODE_COMMIT` nodes. Pros: Maintains clean, immutable graph topology, completely avoids dangling provisional nodes, decouples pre-merge CI gating from Git graph mutation, and resolves the circular gating deadlock. Cons: Pre-merge test runs are queried via JSONB attributes rather than graph edge traversals until canonical merge commit materialization.
* **Decision Taken & Rationale:** Adopted Option B. Recording pre-merge test verification directly in task JSONB attributes prevents premature graph pollution and guarantees CI gates can verify and enforce task test passes prior to pull request merge without circular dependency deadlocks.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 (D-107), `technical-backlog.md` TB-10. Feedback: Document JSONB array structure `TASK.attributes->'pre_merge_verification'` and pre-merge CI ingestion protocol in §5.1 and D-107.
* **Status:** Implemented

### DEC-4.10: Upward Canonical Merge Commit Lineage (CODE_COMMIT -IMPLEMENTS-> TASK) and Two-Scope Release Readiness Gating

* **Work Package:** WP-4.5
* **Category:** Architecture Invariant
* **Context & Problem:** Tracking post-merge code lineage and verifying complete traceability from code to root requirements requires materializing canonical merge commits as graph nodes. The directionality of the commit-task relationship must satisfy Invariant INV-1 (Ancestry must strictly flow upward toward root `REQUIREMENT` nodes via structural edge types). In addition, release readiness must distinguish between PR pre-merge CI validation (which tests candidate branch code before merge) and release deployment gating (which requires merged canonical `CODE_COMMIT` nodes to implement all tasks fulfilling requirements).
* **Options Considered:**
  * *Option A:* Downward edges from `TASK` to `CODE_COMMIT` (`TASK -IMPLEMENTS-> CODE_COMMIT` or `TASK -COMMITTED_IN-> CODE_COMMIT`) with a single readiness evaluation endpoint. Pros: Intuitive task-to-artifact mapping. Cons: Violates Invariant INV-1 graph traversal, where ancestry walks trace from downstream artifacts upward to root requirements; breaks structural DAG cycle detection and ancestry recursion. A single readiness mode also confuses pre-merge checks (where commits are not yet materialized) with post-merge release gating.
  * *Option B:* Upward directed `IMPLEMENTS` edges from `CODE_COMMIT` to `TASK` (`CODE_COMMIT -IMPLEMENTS-> TASK`, D-108). Materialize canonical commits via `POST /api/v1/vcs/commits` inside an atomic structural mutation transaction that creates the `CODE_COMMIT` node, creates `IMPLEMENTS` edges, updates `vcs_commits.graph_node_id`, logs `COMMIT_MATERIALIZED` audit event, and verifies INV-1 ancestry to root requirements. Implement two explicit evaluation scopes in `GET /api/v1/release/readiness?scope=pre-merge|release` (D-113, D-120): `pre-merge` checks task status and `pre_merge_verification` attribute; `release` checks full downward CTE ensuring every requirement has active specifications, completed tasks, and materialized `CODE_COMMIT` nodes. Edge target pre-validation enforces INV-8/INV-9 by rejecting non-existent tasks or raw commit SHAs with HTTP 422 `InvalidEdgeTargetEnvelope`. Pros: Strictly preserves Invariant INV-1 upward ancestry walk consistency across the entire property graph, provides clear separation between pre-merge gating and post-merge deployment readiness, and provides actionable remediation envelopes for invalid target references. Cons: Requires `is_structural_edge_type`, `check_dag_cycle`, and `validate_ancestor_path` to include `IMPLEMENTS`.
* **Decision Taken & Rationale:** Adopted Option B. Materializing canonical merge commits with upward `IMPLEMENTS` edges to tasks guarantees unbroken INV-1 ancestry paths to root requirements, while two-scope release readiness gating cleanly enforces both pre-merge CI quality gates and post-merge deployment verification.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 (D-108, D-113, D-120), `technical-backlog.md` TB-10, TB-13.3. Feedback: Document canonical merge commit materialization, `IMPLEMENTS` edge directionality, and `GET /api/v1/release/readiness?scope=pre-merge|release` semantics.
* **Status:** Implemented

### DEC-4.11: Code-Side Contract Drift Detection via Unified Diff Parsing and Specification Path Attribution

* **Work Package:** WP-4.5
* **Category:** Specification Gap
* **Context & Problem:** Agents and developers may modify public interfaces or contract files without updating the corresponding specification documents or tasks in the knowledge graph. The system required a fast, zero-token, code-side contract drift detection engine capable of analyzing git diffs or commit changesets and detecting unmapped public interface modifications or specification path mismatches before or during merge.
* **Options Considered:**
  * *Option A:* Send every git diff patch to an LLM to evaluate whether code modifications alter contracts or drift from specifications. Pros: Semantically aware drift reasoning. Cons: Excessive latency, high token cost, non-deterministic drift reports, and violates zero-token Tier 1 extraction principles.
  * *Option B:* Deterministic code-side contract drift engine using unified diff parsing and regex-based interface inspection (`POST /api/v1/verification/check-drift` and CLI `tks verification check-drift`, D-114). Parses diff touched files, matches contract file patterns (`specs/`, `proto/`, `openapi/`, `schema/`, `contract/`), detects public export alterations (`pub fn`, `pub struct`, `export interface`, `public class`), and queries `graph_nodes` to verify whether modified files correspond to active specification nodes or tasks. Detects contract drift and identifies unmapped changes with actionable remediation envelopes (`ERR_CONTRACT_DRIFT`). Pros: Deterministic, sub-10ms execution, zero token cost, reproducible across CI pipelines, and provides actionable remediation guidance for contract updates. Cons: Relies on pattern heuristics for interface export identification rather than full AST compilation.
* **Decision Taken & Rationale:** Adopted Option B. Deterministic diff analysis provides instantaneous (<10ms), zero-token, reliable detection of public contract modifications and unmapped file drifts, enforcing rigorous specification-first governance.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 (D-114), `technical-backlog.md` TB-10. Feedback: Document deterministic unified diff drift detection heuristics and `ERR_CONTRACT_DRIFT` remediation envelopes in §5.1 and D-114.
* **Status:** Implemented


