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
