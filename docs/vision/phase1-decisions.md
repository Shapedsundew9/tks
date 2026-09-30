# Phase 1 Implementation Decision Record: Substrate Core, Git Document Ingestion & Context Gateway

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-1.1 | WP-1.1 | In-Memory Index ODB Direct Writes for Lock-Free Tree Construction | Technical Trade-off | architecture.md §5.2, technical-backlog.md TB-1 | Implemented |
| DEC-1.2 | WP-1.1 | Idempotent Root Empty Commit Initialization for refs/heads/specs | Specification Gap | architecture.md §5.1, technical-backlog.md TB-1 | Implemented |
| DEC-1.3 | WP-1.1 | Read-Only Handle Resolution for Concurrent Lock-Free Blob Slicing | API/Contract Elaboration | architecture.md §5.2, technical-backlog.md TB-1 | Implemented |
| DEC-1.4 | WP-1.2 | Dynamic CTE Ancestor Validation with Batch Promotion Union Evaluation | Technical Trade-off | architecture.md §3 INV-1, §9 D-34 | Implemented |
| DEC-1.5 | WP-1.2 | Polymorphic Identifier Resolution Hierarchy and Partial Index Bypassing | API/Contract Elaboration | architecture.md §9 D-58, technical-backlog.md TB-7 | Implemented |
| DEC-1.6 | WP-1.2 | Canonical Hyphenated Key Regex Sanitization and Dual-Path Full-Text Search | Specification Gap | architecture.md §9 D-60, §9 D-68, technical-backlog.md TB-7 | Implemented |
| DEC-1.7 | WP-1.3 | RustCrypto SHA-256 Dependency for Normalized Content Hashing | Dependency Choice | architecture.md §7, technical-backlog.md TB-7 | Implemented |
| DEC-1.8 | WP-1.3 | Content-Preserving 3-Tier Reconciliation Hierarchy and Staged Span Re-Anchoring | Technical Trade-off | architecture.md §9 D-64, §9 D-74, technical-backlog.md TB-7 | Implemented |
| DEC-1.9 | WP-1.3 | Comprehensive Canonical Identifier Regex Expansion for Substrate Entities | Specification Gap | technical-backlog.md TB-7, architecture.md §5.1 | Implemented |
| DEC-1.10 | WP-1.4 | Single-Query Prioritized Claiming for Timed-Out Reclaims and Queued Ingestion Jobs | Technical Trade-off | architecture.md §5.2, technical-backlog.md TB-7 | Implemented |
| DEC-1.11 | WP-1.4 | Persistent In-Memory FastEmbed Generator Instance for Sub-15ms Local Inference | Technical Trade-off | architecture.md §7, §9 D-77, technical-backlog.md TB-6 | Implemented |
| DEC-1.12 | WP-1.4 | Ingestion Jobs Candidate Metadata Attributes Column Migration (V3) | Specification Gap | technical-backlog.md TB-7.5, architecture.md §5.1 | Implemented |
| DEC-1.13 | WP-1.4 | tokio-util Dependency for Cooperative Task Cancellation | Dependency Choice | architecture.md §7 | Implemented |
| DEC-1.14 | WP-1.5 | Dual-Transport MCP Server Architecture for HTTP/SSE and Direct JSON-RPC | Technical Trade-off | architecture.md §4, §6, §9 D-19 | Implemented |
| DEC-1.15 | WP-1.5 | In-Process Moka Cache Eviction Strategy with Immediate Invalidation | Technical Trade-off | architecture.md §4, §9 D-39, D-49, technical-backlog.md TB-5 | Implemented |
| DEC-1.16 | WP-1.5 | Atomic Staging Promotion and In-Place Span Re-Anchoring with Draft Revision Squashing | Technical Trade-off | architecture.md §3 INV-1, INV-2, §5.1, §9 D-34, D-61, D-65, D-74, D-82 | Implemented |
| DEC-1.17 | WP-1.6 | Canonical Key Deduping and Single-Entity Assignment per Document and Promotion Batch | Technical Trade-off | architecture.md §5.1, §9 D-58, technical-backlog.md TB-7 | Implemented |
| DEC-1.18 | WP-1.6 | Stdio Streaming Proxy Connection Resilience and Dual-Entrypoint Dispatch | Technical Trade-off | technical-backlog.md TB-4, architecture.md §8 | Implemented |
| DEC-1.19 | WP-1.6 | Polymorphic Staging Node Inspection REST Endpoint with Verbatim ODB Slicing | API/Contract Elaboration | architecture.md §6, §8, §9 D-83 | Implemented |

---

## 2. Decision Entries

### DEC-1.1: In-Memory Index ODB Direct Writes for Lock-Free Tree Construction

* **Work Package:** WP-1.1
* **Category:** Technical Trade-off
* **Context & Problem:** Constructing standard Git tree hierarchies in a bare repository for nested paths (`doc_path`, e.g. `specs/vision.md`) could be performed via recursive `git2::TreeBuilder` or an in-memory `git2::Index` (`git2::Index::new()`). Recursive tree building across arbitrary directory depths requires manual tree traversal and merging logic, which is error-prone and risks corrupting existing subtrees. Conversely, disk-backed indexes (`.git/index`) create `.git/index.lock` collisions and violate bare repository architectural constraints.
* **Options Considered:**
  * *Option A: Recursive manual `git2::TreeBuilder`.* Pros: purely operates on trees. Cons: complex directory splitting and subtree re-assembly; high risk of dropping unrelated files when modifying nested folders.
  * *Option B: Disk-backed index (`repo.index()`).* Pros: standard libgit2 path resolution. Cons: creates disk lock files (`.git/index.lock`), introduces concurrency contention, and is invalid in bare repositories.
  * *Option C: In-memory `git2::Index::new()` with `index.read_tree()` and `index.write_tree_to(repo)`.* Pros: operates completely in memory without touching disk or creating lock files, automatically handles arbitrary path hierarchies, preserves existing files from parent commit tree, and directly writes resulting tree objects to the ODB. Cons: requires constructing in-memory `git2::IndexEntry`.
* **Decision Taken & Rationale:** Option C. Utilizing `git2::Index::new()` provides robust, automatic hierarchical tree construction in memory while guaranteeing zero disk index lock contention (`.git/index.lock`). The tree is serialized directly into the repository ODB via `write_tree_to(repo)`.
* **Upstream Impact & Target Document:** `architecture.md` §5.2 and `technical-backlog.md` TB-1.
* **Status:** Implemented

### DEC-1.2: Idempotent Root Empty Commit Initialization for refs/heads/specs

* **Work Package:** WP-1.1
* **Category:** Specification Gap
* **Context & Problem:** The specification mandates that `init_bare` idempotently configure the persistent volume path and ensure the `refs/heads/specs` branch exists. In Git, an empty bare repository created via `git init --bare` has zero commits and contains no branches. Without an initial commit, `refs/heads/specs` cannot be resolved as a branch, and standard Git CLI verification (`git log refs/heads/specs`) fails on unpopulated repositories.
* **Options Considered:**
  * *Option A: Defer branch creation until the first document is committed.* Pros: avoids creating an initial setup commit. Cons: `refs/heads/specs` does not exist after `init_bare`, breaking immediate CLI verification and requiring special first-commit branch creation branches in downstream code.
  * *Option B: Idempotently write an initial commit with an empty tree onto `refs/heads/specs` and set `HEAD` to `refs/heads/specs` during `init_bare` if the reference does not exist.* Pros: ensures `refs/heads/specs` exists immediately, standard Git CLI commands succeed immediately on fresh repositories, and subsequent document commits cleanly chain off the root commit. Cons: Git log displays an initial setup commit.
* **Decision Taken & Rationale:** Option B. Creating an initial root commit with an empty tree ensures `refs/heads/specs` exists as a valid Git reference upon initialization, enabling immediate verification via standard Git tools and establishing an unbroken parent lineage for all subsequent document commits.
* **Upstream Impact & Target Document:** `architecture.md` §5.1 and `technical-backlog.md` TB-1.
* **Status:** Implemented

### DEC-1.3: Read-Only Handle Resolution for Concurrent Lock-Free Blob Slicing

* **Work Package:** WP-1.1
* **Category:** API/Contract Elaboration
* **Context & Problem:** The reader component (`GitReadHandle::read_blob_span`) must execute concurrently across Tokio worker threads via `tokio::task::spawn_blocking` without locking or queuing behind the dedicated write actor.
* **Options Considered:**
  * *Option A: Share a single `git2::Repository` handle between reader and writer.* Pros: single handle. Cons: `git2::Repository` is not `Sync`, and sharing a handle across threads requires a mutex, re-introducing serialization bottlenecks and potential head-of-line blocking.
  * *Option B: Open independent read-only repository and ODB handles inside `tokio::task::spawn_blocking`.* Pros: read-only handles access immutable, content-addressed ODB objects without locking; concurrent reads never queue on the write actor or reference locks; zero-copy byte slicing precedes UTF-8 validation. Cons: slight overhead of handle allocation per task, mitigated by blocking worker thread pooling.
* **Decision Taken & Rationale:** Option B. By opening read-only handles targeting the ODB, read tasks execute fully in parallel, achieving high concurrency (50+ concurrent reads tested under write load) without head-of-line blocking or lock collisions.
* **Upstream Impact & Target Document:** `architecture.md` §5.2 and `technical-backlog.md` TB-1.
* **Status:** Implemented

### DEC-1.4: Dynamic CTE Ancestor Validation with Batch Promotion Union Evaluation

* **Work Package:** WP-1.2
* **Category:** Technical Trade-off
* **Context & Problem:** Invariant INV-1 mandates that every active functional specification and implementation task maintain an unbroken upward directed edge path to an active requirement root. During atomic batch draft promotions (`POST /api/v1/documents/ingest/{job_id}/approve`), candidate child tasks and parent requirements are promoted simultaneously from `DRAFT` to `ACTIVE`. Evaluating candidate children exclusively against currently `ACTIVE` requirement nodes in the database fails valid hierarchical trees because the parent requirements in the batch are still in `DRAFT` state at verification time.
* **Options Considered:**
  * *Option A: Two-pass application-level validation.* Pros: simple in-memory graph checks. Cons: vulnerable to TOCTOU race conditions between read validation and transactional edge commitment; duplicates traversal logic in application memory outside the database transaction.
  * *Option B: Single recursive CTE evaluating against the union of currently active requirements and candidate batch promotion IDs (`node_ids`).* Pros: fully transactional under PostgreSQL ACID guarantees; operates natively inside the promotion transaction; traverses upward across `FULFILLS`, `CONSTRAINED_BY`, and `DERIVED_FROM` edges; cycle-guarded via visited node arrays; atomically verifies that all candidate non-requirements terminate at a valid root without leaving dangling references. Cons: requires recursive SQL CTE.
* **Decision Taken & Rationale:** Option B. Executing the ancestor path check as a single recursive CTE inside `validate_ancestor_path` evaluating candidate paths against currently active requirements unioned with candidate promotion IDs guarantees mathematical consistency for multi-tier document tree approvals while preventing race conditions.
* **Upstream Impact & Target Document:** `architecture.md` §3 INV-1, §9 D-34.
* **Status:** Implemented

### DEC-1.5: Polymorphic Identifier Resolution Hierarchy and Partial Index Bypassing

* **Work Package:** WP-1.2
* **Category:** API/Contract Elaboration
* **Context & Problem:** Interfaces and CLI tools must accept either 128-bit UUID strings or human-readable canonical keys (`node_key`, e.g. `REQ-CORE-001`). `graph_nodes` maintains surrogate primary key `id UUID` and partial unique index `idx_graph_nodes_node_key_active` on `(node_key) WHERE lifecycle_state = 'ACTIVE'`. If an input string is not a valid UUID, attempting to parse it as `Uuid` panics or triggers a syntax error. Furthermore, draft nodes may share `node_key` with active nodes or each other during authoring revisions.
* **Options Considered:**
  * *Option A: Force all external callers to supply UUIDs exclusively.* Pros: single query path. Cons: poor developer ergonomics; violates Decision D-58 and Technical Backlog TB-7.
  * *Option B: Attempt UUID parsing via `Uuid::parse_str`; if successful, query `WHERE id = $1`; otherwise, query `WHERE node_key = $1 AND lifecycle_state = 'ACTIVE'`.* Pros: leverages B-tree primary key index for UUIDs and partial index `idx_graph_nodes_node_key_active` for canonical keys; guarantees $<1\text{ ms}$ index lookups; avoids index bloat; accurately handles both machine-generated UUIDs and human-readable requirement keys. Cons: two query templates depending on parse outcome.
* **Decision Taken & Rationale:** Option B. Structured resolution with UUID parsing preceding `node_key` filtering directly fulfills Decision D-58 and TB-7, ensuring seamless ergonomics across MCP tools and REST endpoints.
* **Upstream Impact & Target Document:** `architecture.md` §9 D-58, `technical-backlog.md` TB-7.
* **Status:** Implemented

### DEC-1.6: Canonical Hyphenated Key Regex Sanitization and Dual-Path Full-Text Search

* **Work Package:** WP-1.2
* **Category:** Specification Gap
* **Context & Problem:** Decision D-35 and TB-7 mandate native sub-5ms requirement search via `query_active_requirements` against generated `search_tsv` with partial GIN index. When users search for canonical identifiers (e.g. `REQ-CORE-001`, `INV-2`), standard PostgreSQL `websearch_to_tsquery` treats hyphens as boolean negation operators (`NOT`), translating `REQ-CORE-001` into `req & !core & !001` and returning empty result sets.
* **Options Considered:**
  * *Option A: Unconditionally use `plainto_tsquery` for all queries.* Pros: ignores punctuation negation. Cons: loses rich boolean query semantics (e.g. phrases and OR expressions) supported by `websearch_to_tsquery` for natural language inquiries.
  * *Option B: Dual-path search with canonical regex detection (`^[A-Za-z]+-[0-9A-Za-z-]+$`).* When the query matches canonical identifier syntax, execute prefix/exact match `node_key ILIKE $1 || '%'` combined with `plainto_tsquery` and custom rank boosting. For all other queries, execute `websearch_to_tsquery` with prefix key fallback. Pros: prevents hyphen negation syntax errors; guarantees exact key hits rank first; executes in $<5\text{ ms}$ via partial indexes; preserves full natural language capabilities. Cons: requires compiled regex check.
* **Decision Taken & Rationale:** Option B. Using lazy-compiled regex detection cleanly splits identifier lookups from natural language searches, preventing syntax corruption and delivering exact canonical key matching in $<2\text{ ms}$.
* **Upstream Impact & Target Document:** `architecture.md` §9 D-60, §9 D-68, `technical-backlog.md` TB-7.
* **Status:** Implemented

### DEC-1.7: RustCrypto SHA-256 Dependency for Normalized Content Hashing

* **Work Package:** WP-1.3
* **Category:** Dependency Choice
* **Context & Problem:** Tier 3 document reconciliation requires normalized SHA-256 hashing of title and text content to correlate unchanged requirements across revisions when headings or blocks shift. The Rust standard library does not provide cryptographic hashing algorithms, and rolling ad hoc hashing routines is unsafe and prohibited. Furthermore, `node_embeddings.content_hash` requires a standard 64-character hex digest.
* **Options Considered:**
  * *Option A: Git object SHA-1 hashing via `git2`.* Pros: leverages an existing dependency. Cons: violates architectural specifications standardizing on SHA-256 (e.g. `node_embeddings.content_hash VARCHAR(64)`); SHA-1 is cryptographically weak; git blob hashes include git object headers (`blob <len>\0`), preventing portable content comparisons.
  * *Option B: Add `sha2 = "0.10"` to `[dependencies]`.* Pros: pure-Rust, de facto standard cryptographic hashing crate from the official RustCrypto project; produces standard 64-character hex SHA-256 digests; small footprint; zero unsafe C dependencies. Cons: introduces a direct dependency.
* **Decision Taken & Rationale:** Option B. Adding `sha2 = "0.10"` conforms to the dependency policy (C-10), provides high-performance and cryptographically robust SHA-256 hashing, and directly enables Tier 3 content hash reconciliation and vector deduplication.
* **Upstream Impact & Target Document:** `architecture.md` §7 and `technical-backlog.md` TB-7.
* **Status:** Implemented

### DEC-1.8: Content-Preserving 3-Tier Reconciliation Hierarchy and Staged Span Re-Anchoring

* **Work Package:** WP-1.3
* **Category:** Technical Trade-off
* **Context & Problem:** During CommonMark document re-ingestion, inserting a new block at the beginning of a document or section shifts ordinal block numbering (`#block-0` $\to$ `#block-1`). If Tier 2 matches blindly on `#block-0` without verifying content, the newly inserted block collides with the previous block 0 and falsely marks it as "modified", while the actual existing block (now at block 1) is orphaned or marked as an unrelated addition.
* **Options Considered:**
  * *Option A: Strict linear single-pass anchor matching.* Pros: simple loop. Cons: causes false modification classifications whenever paragraphs are prepended; drops historical continuity for unchanged requirements.
  * *Option B: Content-preserving multi-pass reconciliation: Tier 1 (canonical key) $\to$ Tier 2a (anchor match with identical content) $\to$ Tier 3 (content hash match) $\to$ Tier 2b (remaining anchor match for modified content).* Pros: correctly preserves active node identity when blocks shift ordinal positions; accurately classifies true in-place content modifications; stages `CandidateSpanReanchor` coordinates without mutating active nodes in place per Invariant INV-2 and Decisions D-69, D-74. Cons: requires two-phase anchor evaluation.
* **Decision Taken & Rationale:** Option B. Separating content-identical anchor matches from modified content anchor matches prevents inserted blocks from stealing active nodes from their true content matches, guaranteeing accurate span re-anchoring and clean candidate draft staging.
* **Upstream Impact & Target Document:** `architecture.md` §9 D-64, §9 D-74, and `technical-backlog.md` TB-7.
* **Status:** Implemented

### DEC-1.9: Comprehensive Canonical Identifier Regex Expansion for Substrate Entities

* **Work Package:** WP-1.3
* **Category:** Specification Gap
* **Context & Problem:** The initial canonical key regex in Phase 0 restricted `INV-*`, `DR-*`, `C-*`, and `TB-*` strictly to numeric digits (`INV-[0-9]+`), failing on alphanumeric invariant identifiers (e.g. `INV-DATA-002`, `CAL-H1`), decimal sub-items (e.g. `TB-2.4`), or package/decision identifiers (`WP-1.3`, `DEC-1.1`).
* **Options Considered:**
  * *Option A: Force all canonical keys in specifications to be strictly numeric digits after prefix.* Pros: preserves narrow regex. Cons: highly restrictive; breaks existing documentation convention where domain tags like `INV-DATA-002` or `TB-2.4` are standard.
  * *Option B: Expand regex to `\b((?:REQ|INV|DR|C|TB|TASK|WP|DEC)-[A-Za-z0-9]+(?:[-_\.][A-Za-z0-9]+)*)\b`.* Pros: uniformly recognizes all standard substrate identifiers across all documentation formats; robustly matches both numeric and alphanumeric tagged keys; extracts keys in order of first appearance. Cons: matches slightly broader tag families.
* **Decision Taken & Rationale:** Option B. Expanding the pattern ensures all canonical keys defined across governing architecture, vision, and planning documents are mechanically extracted and preserved as first-class `node_key` tags during mechanical decomposition.
* **Upstream Impact & Target Document:** `technical-backlog.md` TB-7 and `architecture.md` §5.1.
* **Status:** Implemented

### DEC-1.10: Single-Query Prioritized Claiming for Timed-Out Reclaims and Queued Ingestion Jobs

* **Work Package:** WP-1.4
* **Category:** Technical Trade-off
* **Context & Problem:** The background decomposition worker must continuously claim new `QUEUED` jobs while prioritizing the reclamation of stalled or crashed `PROCESSING` jobs that have exceeded the 180-second timeout threshold (`updated_at < clock_timestamp() - INTERVAL '180 seconds'`) per Decisions D-79 and TB-7.6. Running separate polling loops or two consecutive transactions creates connection churn and introduces race windows between queue checks.
* **Options Considered:**
  * *Option A: Two sequential polling queries per tick.* Pros: separate SQL queries. Cons: doubles database connection checkouts and queries on every tick; can starve `QUEUED` jobs or waste cycles when queues are empty.
  * *Option B: Single unified query ordering by status priority and creation timestamp.* Use `SELECT ... WHERE status = 'QUEUED' OR (status = 'PROCESSING' AND updated_at < clock_timestamp() - INTERVAL '180 seconds') ORDER BY CASE WHEN status = 'PROCESSING' THEN 0 ELSE 1 END, created_at ASC LIMIT 1 FOR UPDATE SKIP LOCKED`. Pros: executes atomically in a single statement; guarantees timed-out jobs are reclaimed first before new jobs are scheduled; lock-free across concurrent workers with `SKIP LOCKED`; zero connection churn. Cons: slightly more complex SQL predicate.
* **Decision Taken & Rationale:** Option B. The single prioritized claim query guarantees prompt crash recovery without starvation or connection pool exhaustion, satisfying D-79 and TB-7.6.
* **Upstream Impact & Target Document:** `architecture.md` §5.2 and `technical-backlog.md` TB-7.
* **Status:** Implemented

### DEC-1.11: Persistent In-Memory FastEmbed Generator Instance for Sub-15ms Local Inference

* **Work Package:** WP-1.4
* **Category:** Technical Trade-off
* **Context & Problem:** `LocalEmbeddingGenerator` wrapping `fastembed::TextEmbedding` loads ONNX model weights and tokenizer configurations during initialization (~100–200ms). Initializing a new model instance on every polling tick or batch would repeatedly incur disk I/O and ONNX session setup overhead, breaching the sub-50ms SLA requirement for vector generation.
* **Options Considered:**
  * *Option A: Instantiate `LocalEmbeddingGenerator` per batch tick.* Pros: stateless worker function. Cons: adds 100–200ms initialization overhead to every batch, violating the <50ms per-node latency threshold.
  * *Option B: Maintain a persistent `LocalEmbeddingGenerator` instance within the long-running worker task loop.* Pros: model weights and ONNX session are initialized once on worker startup; subsequent batch inference executes in ~9–15ms for batches of 5–10 items (<2ms per node), comfortably outperforming the sub-50ms requirement. Cons: retains model weights in resident memory (~190 MB, within the 256 MB budget).
* **Decision Taken & Rationale:** Option B. Persisting the generator in worker memory aligns with Decision DEC-0.12 and fulfills the sub-50ms latency proof criteria.
* **Upstream Impact & Target Document:** `architecture.md` §7, §9 D-77, and `technical-backlog.md` TB-6.
* **Status:** Implemented

### DEC-1.12: Ingestion Jobs Candidate Metadata Attributes Column Migration (V3)

* **Work Package:** WP-1.4
* **Category:** Specification Gap
* **Context & Problem:** Invariant INV-2, Decision D-74, and Technical Backlog TB-7.5 mandate that when active requirements match across document revisions with shifted spans, active nodes must NEVER be updated in place during reconciliation. Instead, candidate re-anchored coordinates must be staged within `ingestion_jobs` candidate attributes/metadata until explicit staging approval. However, `migrations/V1__initial_schema.sql` did not declare an `attributes` column on `ingestion_jobs`. Modifying `V1__initial_schema.sql` after deployment would invalidate refinery migration checksums.
* **Options Considered:**
  * *Option A: Mutate `V1__initial_schema.sql` retroactively.* Pros: keeps table schema in one file. Cons: breaks migration checksum validation in refinery on existing databases; violates production migration safety.
  * *Option B: Create `migrations/V3__ingestion_jobs_attributes.sql` adding `attributes JSONB NOT NULL DEFAULT '{}'`.* Pros: preserves refinery migration history; backward-compatible; allows candidate re-anchoring metadata to be cleanly staged and retrieved upon approval. Cons: adds a migration step.
* **Decision Taken & Rationale:** Option B. Introducing migration V3 cleanly extends the schema without mutating applied migrations, enabling staged candidate re-anchoring metadata storage per TB-7.5.
* **Upstream Impact & Target Document:** `technical-backlog.md` TB-7.5 and `architecture.md` §5.1.
* **Status:** Implemented

### DEC-1.13: tokio-util Dependency for Cooperative Task Cancellation

* **Work Package:** WP-1.4
* **Category:** Dependency Choice
* **Context & Problem:** The cooperative worker manager (`WorkerManager`) must coordinate graceful shutdown across multiple concurrent worker tasks (`decomp` and `embedding`) without abrupt task abortion that could leave in-flight transactions or Git actor states corrupted.
* **Options Considered:**
  * *Option A: Implement custom channel-based shutdown signaling (e.g. `tokio::sync::watch`).* Pros: uses existing `tokio` features. Cons: requires custom boilerplate and boilerplate propagation across sub-worker tasks.
  * *Option B: Add `tokio-util = "0.7"` to dependencies for `CancellationToken`.* Pros: `tokio-util` is the official, well-maintained companion crate to Tokio; `CancellationToken` provides tree-structured hierarchical cancellation and clean `select!` integration; already present in the transitive dependency tree (via reqwest/fastembed), introducing zero new compiled code footprint. Cons: explicit dependency entry in `Cargo.toml`.
* **Decision Taken & Rationale:** Option B. Adopting `tokio-util` directly fulfills the WP-1.4 contract specification while conforming to constraint C-10.
* **Upstream Impact & Target Document:** `architecture.md` §7.
* **Status:** Implemented

### DEC-1.14: Dual-Transport MCP Server Architecture for HTTP/SSE and Direct JSON-RPC

* **Work Package:** WP-1.5
* **Category:** Technical Trade-off
* **Context & Problem:** The Model Context Protocol (MCP) specification defines two standard primary transport bindings: Server-Sent Events (SSE) where a streaming GET request receives continuous event notifications and dispatches requests over an auxiliary HTTP POST endpoint, and direct JSON-RPC 2.0 over standard HTTP POST. Client adapters like `tks mcp-stdio` (WP-1.6) require full SSE session semantics (`GET /mcp/sse` and `POST /mcp/message`), whereas lightweight callers, test harnesses, and automated scripting clients require synchronous request-response JSON-RPC 2.0 without long-lived streaming connection overhead.
* **Options Considered:**
  * *Option A: Implement SSE transport exclusively (`/mcp/sse` + `/mcp/message`).* Pros: strictly follows the full MCP HTTP/SSE specification. Cons: requires callers and test suites to maintain active event source listener loops and coordinate session IDs just to execute simple single-shot tool calls.
  * *Option B: Implement direct JSON-RPC 2.0 POST endpoint exclusively (`/mcp`).* Pros: simple stateless integration. Cons: fails compliance with the MCP SSE specification and prevents streaming transport proxies.
  * *Option C: Unified dual-transport routing sharing the underlying tool dispatcher.* Pros: `/mcp/sse` and `/mcp/message` provide standard streaming SSE sessions with broadcast channels, while `/mcp` provides direct synchronous JSON-RPC 2.0 request/response handling. Both transports dispatch into identical read-only tool implementations (`get_context_envelope`, `query_requirements`, `get_document_span`), maximizing client compatibility. Cons: minimal routing configuration overhead in `src/gateway/mcp/mod.rs`.
* **Decision Taken & Rationale:** Option C. Implementing both standard SSE and direct HTTP JSON-RPC 2.0 ensures complete compatibility with standard MCP clients and proxies while providing high-performance, low-latency synchronous tool invocations.
* **Upstream Impact & Target Document:** `architecture.md` §4, §6, and §9 D-19.
* **Status:** Implemented

### DEC-1.15: In-Process Moka Cache Eviction Strategy with Immediate Invalidation

* **Work Package:** WP-1.5
* **Category:** Technical Trade-off
* **Context & Problem:** Invariant INV-7 and Technical Backlog TB-5 mandate that identity authentication execute with minimal overhead ($<1\text{ ms}$) in Axum Tower middleware, while immediate revocation of an identity (`POST /api/v1/identities/{id}/revoke`) must instantly invalidate all cached credentials across the running service without waiting for TTL expiration.
* **Options Considered:**
  * *Option A: Direct PostgreSQL query on every request without caching.* Pros: always strictly consistent. Cons: adds 1–3ms database roundtrip latency to every HTTP/MCP call, violating sub-5ms performance SLAs.
  * *Option B: In-memory cache with pure time-to-live (TTL) expiration only.* Pros: simple cache configuration. Cons: revoked credentials would remain valid and usable until TTL expiration, creating a critical security vulnerability and violating Invariant INV-7.
  * *Option C: In-process `moka::future::Cache` with SHA-256 token hashing and immediate active eviction on revocation.* Pros: SHA-256 hashing prevents in-memory plaintext token leakage; 5-minute TTL bound with 10,000 entry capacity protects memory; `revoke_identity` updates the database (`is_active = false`) and simultaneously evicts active credentials from the `moka` cache, guaranteeing immediate rejection of revoked tokens on subsequent requests. Cons: requires passing shared `AuthState` to route handlers.
* **Decision Taken & Rationale:** Option C. Combining `moka::future::Cache` with explicit invalidation upon revocation ensures sub-millisecond authentication overhead while strictly enforcing Invariant INV-7 and TB-5.
* **Upstream Impact & Target Document:** `architecture.md` §4, §9 D-39, D-49, and `technical-backlog.md` TB-5.
* **Status:** Implemented

### DEC-1.16: Atomic Staging Promotion and In-Place Span Re-Anchoring with Draft Revision Squashing

* **Work Package:** WP-1.5
* **Category:** Technical Trade-off
* **Context & Problem:** Staging approval (`POST /api/v1/documents/ingest/{job_id}/approve` and alias `POST /api/v1/staging/approve`) transitions candidate draft entities into active production state. This operation must acquire global and document advisory locks in strict hierarchy (D-66), validate Invariant INV-1 ancestor paths, apply candidate span re-anchoring in place on matching active `graph_nodes` with `SPAN_REANCHORED` audit events (D-74, TB-7.5), atomically purge unapproved draft nodes (D-46), promote candidate edges only when both endpoints are active while superseding conflicting active edges (D-47), squash ephemeral draft revisions into a canonical `APPROVED` event in `audit_ledger` with monotonic `event_seq` and transaction correlation `batch_id` (D-61, D-65), promote nodes to `ACTIVE` (defaulting `UNCLASSIFIED` to `SPECIFICATION`), and enqueue vector embeddings for active nodes into `node_embeddings` with `status = 'PENDING'` (D-82).
* **Options Considered:**
  * *Option A: Multi-transaction staged commit (separate validation, graph update, and audit recording).* Pros: shorter single transaction holding time. Cons: susceptible to TOCTOU race conditions and partial mutation failures, violating Invariants INV-1 and INV-2.
  * *Option B: Single atomic transaction with strict lock acquisition hierarchy.* Pros: acquiring `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` and `pg_advisory_xact_lock(hashtext(doc_path))` before any row mutations guarantees total ordering, mathematically preventing deadlock cycles (40P01) and ensuring complete ACID atomicity across span re-anchoring, draft deletion, node promotion, revision squashing, and embedding queueing. Cons: transaction locks the document and structural mutation space for ~5–10ms during approval.
* **Decision Taken & Rationale:** Option B. Executing the entire staging promotion within a single advisory-locked transaction guarantees strict topological integrity, zero orphan draft residue, and complete audit immutability.
* **Upstream Impact & Target Document:** `architecture.md` §3 INV-1, INV-2, §5.1, §9 D-34, D-46, D-47, D-61, D-65, D-74, D-82, and D-83.
* **Status:** Implemented

### DEC-1.17: Canonical Key Deduping and Single-Entity Assignment per Document and Promotion Batch

* **Work Package:** WP-1.6
* **Category:** Technical Trade-off
* **Context & Problem:** When documents (such as `docs/vision/strategic-planning-backlog.md`) mention canonical requirement keys (e.g., `TB-1`) across multiple body paragraphs or bullet points, an unconstrained regex extractor extracts and tags multiple distinct AST chunks with the same canonical `node_key`. When staged candidate nodes are subsequently approved via `POST /api/v1/staging/approve`, setting `lifecycle_state = 'ACTIVE'` across these candidate nodes triggers a PostgreSQL unique constraint violation on `idx_graph_nodes_node_key_active` (`ON graph_nodes (node_key) WHERE lifecycle_state = 'ACTIVE'`).
* **Options Considered:**
  * *Option A: Relax the database partial unique index `idx_graph_nodes_node_key_active` to allow duplicate active canonical keys.* Pros: eliminates promotion failures without code changes. Cons: violates core architectural invariant D-58 that canonical requirement identifiers uniquely address a single functional entity.
  * *Option B: Enforce single-entity assignment during AST reconciliation and pre-promotion deduplication in staging approval.* Pros: During reconciliation (`reconcile_reingestion_with_options`), an in-memory `assigned_node_keys` tracker ensures only the primary chunk claiming the canonical key receives `node_key = Some(key)`, while secondary prose mentions receive `node_key = None`. Furthermore, during staging approval (`execute_approve`), Step 8.5 executes defensive pre-promotion queries to clear duplicate keys among candidate nodes or collisions with pre-existing active nodes. This preserves unique addressability and strict relational invariants without dropping secondary text nodes. Cons: requires secondary mentions to be identified without the primary key tag.
* **Decision Taken & Rationale:** Option B. Enforcing first-claim canonical key assignment during reconciliation combined with defensive pre-promotion key cleanup guarantees the integrity of `idx_graph_nodes_node_key_active` while preserving all extracted text content in the substrate graph.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, §9 D-58, and `technical-backlog.md` TB-7.
* **Status:** Implemented

### DEC-1.18: Stdio Streaming Proxy Connection Resilience and Dual-Entrypoint Dispatch

* **Work Package:** WP-1.6
* **Category:** Technical Trade-off
* **Context & Problem:** External AI agent harnesses (such as Claude Desktop, Cursor, or Cline) invoke MCP stdio adapters as external child processes. If the daemon (`tks serve`) is offline or restarting when an agent harness launches, standard stdio adapters crash with an immediate non-zero exit code or broken pipe, leading agent harnesses to disable the MCP server. Additionally, different harness configurations invoke the adapter either via unified CLI (`tks mcp-stdio`) or as a dedicated standalone binary (`mcp_stdio`).
* **Options Considered:**
  * *Option A: Immediately exit with code 1 if daemon HTTP/SSE endpoint is unreachable.* Pros: minimal adapter code. Cons: agent harnesses disable or drop the tool permanently; fragile development workflow during daemon restarts.
  * *Option B: Dedicated binary only or unified CLI only.* Pros: single binary target. Cons: breaks standard conventions where agent configs specify executable paths directly without subcommand flags.
  * *Option C: Exponential backoff connectivity check (up to 2 seconds), structured `-32000` JSON-RPC error response on stdin `initialize` request when offline (exiting 0), strict stderr diagnostic logging, and dual-entrypoint dispatch (`tks mcp-stdio` and `src/bin/mcp_stdio.rs`).* Pros: if the server is offline, reading stdin for the agent harness's initial `{"method": "initialize", "id": 1}` and replying on stdout with a standard JSON-RPC 2.0 error (`code: -32000`) before exiting 0 prevents the harness from panicking or marking the adapter as permanently broken; logging all diagnostics to stderr protects stdout JSON-RPC framing; dual entrypoints provide seamless compatibility. Cons: requires custom request listener handling on daemon unreachable path.
* **Decision Taken & Rationale:** Option C. Implementing 2-second connection backoff, graceful `-32000` initialization error generation on stdout, stderr-only logging, and dual entrypoint dispatch ensures robust integration with external agent hosts under daemon restarts.
* **Upstream Impact & Target Document:** `technical-backlog.md` TB-4 and `architecture.md` §8.
* **Status:** Implemented

### DEC-1.19: Polymorphic Staging Node Inspection REST Endpoint with Verbatim ODB Slicing

* **Work Package:** WP-1.6
* **Category:** API/Contract Elaboration
* **Context & Problem:** The terminal staging CLI (`tks staging inspect <node_id>`) requires inspecting candidate draft nodes before approval, including reviewing the exact verbatim Markdown span extracted from Git specifications. Reading Git blobs directly in the CLI would require direct filesystem access to the bare Git repository, violating the client-server separation between CLI and daemon.
* **Options Considered:**
  * *Option A: Open the bare Git repository directly from the CLI process.* Pros: bypasses REST gateway. Cons: breaks client-server architecture; fails if CLI runs in a separate container, host machine, or unprivileged context lacking direct volume mounts.
  * *Option B: Add polymorphic inspection endpoints `GET /api/v1/staging/inspect/{id}` and `GET /api/v1/nodes/{id}` on the Gateway API.* Pros: accepts either 128-bit UUIDs or canonical requirement keys (`TB-1`); retrieves node metadata from PostgreSQL; transparently resolves the Git commit SHA and byte range from `GitReadHandle::read_blob_span`; returns structured JSON including verbatim text content, metadata, attributes, and lifecycle state. The CLI simply renders this JSON response. Cons: requires gateway route registration and git read handle integration.
* **Decision Taken & Rationale:** Option B. Implementing polymorphic REST inspection endpoints maintains strict client-server decoupling, enables thin CLI ergonomics, and provides unified inspection capabilities for both draft and active nodes across REST, CLI, and MCP callers.
* **Upstream Impact & Target Document:** `architecture.md` §6, §8, and §9 D-83.
* **Status:** Implemented
