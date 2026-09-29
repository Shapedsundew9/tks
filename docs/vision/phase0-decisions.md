# Phase 0 Implementation Decision Record: Pre-Construction Hypothesis De-risking & Prototype Spikes

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-0.1 | WP-0.1 | Containerized Database Resolution and Host Environment Leak Containment | Technical Trade-off | architecture.md §5.1, §7 | Implemented |
| DEC-0.2 | WP-0.2 | Enclosing Section Heading Attribution for Non-Heading Structural Chunks | API/Contract Elaboration | architecture.md §5.2, technical-backlog.md TB-2 | Implemented |
| DEC-0.3 | WP-0.2 | Deterministic AST Anchor Generation and Root Pre-Heading Scope Indexing | Specification Gap | technical-backlog.md TB-2.4, architecture.md §5.2 | Implemented |
| DEC-0.4 | WP-0.2 | Standalone Benchmark Harness Execution on Stable Rust Toolchain | Technical Trade-off | technical-backlog.md TB-2.7, phase0-plan.md WP-0.2 | Implemented |

---

## 2. Decision Entries

### DEC-0.1: Containerized Database Resolution and Host Environment Leak Containment

* **Work Package:** WP-0.1
* **Category:** Technical Trade-off
* **Context & Problem:** The devcontainer environment configuration permitted VS Code's `${localEnv:DATABASE_URL}` to forward the developer workstation's host environment variable into the development container. In local environments where the host machine runs an external PostgreSQL server without the `pgvector` extension (e.g. host LAN IP), database migrations failed on `CREATE EXTENSION IF NOT EXISTS vector;` and `vector(384)` column definitions. Additionally, inside Docker Compose container networks, default `localhost:5432` binds to the container's loopback rather than the adjacent `postgres` container service.
* **Options Considered:**
  * *Option A:* Require developers to explicitly set `DATABASE_URL=postgresql://postgres:postgres@postgres:5432/postgres` before every test run or CLI command. (Pros: Zero codebase logic. Cons: Fragile developer ergonomics, breaks standard `cargo test` and `cargo run -- --migrate-only` quality gates).
  * *Option B:* Hardcode the database connection string strictly to `postgresql://postgres:postgres@postgres:5432/postgres` without environment variable support. (Pros: Enforces container networking. Cons: Inflexible; violates Twelve-Factor App principles and prohibits custom CI/CD or staging database URLs).
  * *Option C:* Implement intelligent database URL resolution in `src/db.rs` (`resolve_database_url()`) that checks `TKS_DATABASE_URL` first, accepts valid `DATABASE_URL` values while filtering leaked host LAN addresses without pgvector, and dynamically resolves the containerized `postgres:5432` service with fallback to `localhost:5432`; update `.devcontainer/devcontainer.json` to decouple from `${localEnv:DATABASE_URL}` and configure default `DATABASE_URL` in `.devcontainer/docker-compose.yml`. (Pros: Seamless out-of-the-box local developer ergonomics, robust quality gate execution, preserves Twelve-Factor configurability for CI/CD and production).
* **Decision Taken & Rationale:** Adopted Option C. Ensures robust automated test and migration runs without manual developer intervention, preserves compatibility across both containerized devcontainers and native host environments, and prevents non-pgvector external databases from breaking schema migrations.
* **Upstream Impact & Target Document:** `architecture.md` §5.1 and §7 (document database connection string resolution hierarchy and devcontainer network isolation).
* **Status:** Implemented

### DEC-0.2: Enclosing Section Heading Attribution for Non-Heading Structural Chunks

* **Work Package:** WP-0.2
* **Category:** API/Contract Elaboration
* **Context & Problem:** The `ExtractedChunk` struct contract defines `heading: Option<String>`. In CommonMark documents, non-heading structural blocks (paragraphs, lists, tables, blockquotes, codeblocks) do not define their own heading text. Downstream candidate classification in Stage 2 (WP-0.3 `build_classification_prompt`), however, requires immediate section heading context so that compact LLM prompts can identify which requirement section a candidate block belongs to without performing costly recursive relational graph lookups.
* **Options Considered:**
  * *Option A:* Set `heading = None` for all non-heading structural blocks, reserving `heading` strictly for heading blocks. (Pros: Structurally literal. Cons: Strips immediate section context from extracted chunks, requiring downstream callers to perform iterative graph traversals to recover the enclosing heading title).
  * *Option B:* Populate `heading = Some(title)` for heading chunks (representing their own heading text) and inherit `heading = Some(parent_heading_title)` for non-heading structural blocks under an active heading, defaulting to `None` strictly for blocks preceding any heading in the document. (Pros: Provides $O(1)$ section provenance for downstream classification prompt formatting while preserving structural hierarchy via `parent_heading_chunk_id`; satisfies both heading identity and block context).
* **Decision Taken & Rationale:** Adopted Option B. Enables compact, self-contained candidate representation in memory and streamlines WP-0.3 classification prompt construction while preserving strict upward lineage via `parent_heading_chunk_id`.
* **Upstream Impact & Target Document:** `architecture.md` §5.2, `technical-backlog.md` TB-2.
* **Status:** Implemented

### DEC-0.3: Deterministic AST Anchor Generation and Root Pre-Heading Scope Indexing

* **Work Package:** WP-0.2
* **Category:** Specification Gap
* **Context & Problem:** Technical backlog TB-2.4 and LD-8 specify hierarchical heading anchor generation with slug disambiguation (`doc_path#section/overview-1`) and ordinal block indexing (`doc_path#section/heading#block-0`), but do not specify the anchor structure for non-heading blocks appearing prior to the first heading in a document (e.g. document-level introductory notes or abstract blocks).
* **Options Considered:**
  * *Option A:* Discard non-heading blocks appearing before any heading. (Pros: Simple. Cons: Drops document text and violates 100% roundtrip span fidelity).
  * *Option B:* Scope pre-heading blocks directly under the document path using an ordinal root counter (`doc_path#block-0`, `doc_path#block-1`) with `parent_heading_chunk_id = None`. (Pros: Guarantees 100% text coverage, preserves document root semantics, guarantees deterministic global anchor uniqueness).
* **Decision Taken & Rationale:** Adopted Option B. Guarantees that every structural block in the document has an unambiguous, unique AST anchor regardless of document heading structure.
* **Upstream Impact & Target Document:** `technical-backlog.md` TB-2.4, `architecture.md` §5.2.
* **Status:** Implemented

### DEC-0.4: Standalone Benchmark Harness Execution on Stable Rust Toolchain

* **Work Package:** WP-0.2
* **Category:** Technical Trade-off
* **Context & Problem:** Rust's built-in benchmark harness (`#[bench]`) requires the nightly compiler feature `#![feature(test)]`. Repository guidelines (`GEMINI.md` and constraint C-1) mandate stable Rust (edition 2024).
* **Options Considered:**
  * *Option A:* Introduce a third-party benchmarking dependency such as `criterion`. (Pros: Statistical regression analysis. Cons: Introduces a large dependency tree in violation of constraint C-10).
  * *Option B:* Configure `[[bench]]` with `harness = false` in `Cargo.toml` and implement a deterministic benchmark binary in `benches/ast_decomposition_bench.rs` using `std::time::Instant`. (Pros: Zero third-party dependencies, builds and executes cleanly with `cargo bench --bench ast_decomposition_bench` on stable Rust, satisfies all verification criteria with precise token reduction and latency calculations).
* **Decision Taken & Rationale:** Adopted Option B. Aligns strictly with dependency minimization (C-10) and stable Rust toolchain rules while delivering deterministic benchmark verification.
* **Upstream Impact & Target Document:** `technical-backlog.md` TB-2.7, `docs/vision/phase0-plan.md` WP-0.2.
* **Status:** Implemented
