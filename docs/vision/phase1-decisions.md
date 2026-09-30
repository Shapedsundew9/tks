# Phase 1 Implementation Decision Record: Substrate Core, Git Document Ingestion & Context Gateway

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-1.1 | WP-1.1 | In-Memory Index ODB Direct Writes for Lock-Free Tree Construction | Technical Trade-off | architecture.md §5.2, technical-backlog.md TB-1 | Implemented |
| DEC-1.2 | WP-1.1 | Idempotent Root Empty Commit Initialization for refs/heads/specs | Specification Gap | architecture.md §5.1, technical-backlog.md TB-1 | Implemented |
| DEC-1.3 | WP-1.1 | Read-Only Handle Resolution for Concurrent Lock-Free Blob Slicing | API/Contract Elaboration | architecture.md §5.2, technical-backlog.md TB-1 | Implemented |

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
