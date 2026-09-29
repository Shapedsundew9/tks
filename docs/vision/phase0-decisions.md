# Phase 0 Implementation Decision Record: Pre-Construction Hypothesis De-risking & Prototype Spikes

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-0.1 | WP-0.1 | Containerized Database Resolution and Host Environment Leak Containment | Technical Trade-off | architecture.md §5.1, §7 | Implemented |

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
