# Chapter 2: Daemon & Identity Provisioning

This chapter covers running the Knowledge Substrate daemon server, configuring its storage dependencies, and managing authentication identities.

---

## Daemon Architecture (`tks serve`)

TKS is delivered as a single compiled Rust binary. The server daemon (`tks serve`) consolidates all substrate services into a single unified process:

* **Axum Gateway:** Serves the REST API, identity endpoints, rollback engine, task management, workspace operations, and MCP HTTP/SSE transport on a single network port (default: `8080`).
* **Git Actor Task:** A dedicated background thread owning the bare Git repository handle via libgit2 C bindings, serializing write commits with panic recovery boundaries (`catch_unwind`) while concurrent read handles serve lock-free blob slicing via `tokio::task::spawn_blocking`.
* **In-Process `GraphEventBus`:** A Tokio broadcast channel distributing graph mutation, invalidation cascade, and workspace merge events to internal consumers and real-time SSE listeners with sub-millisecond filtering.
* **Cooperative `WorkerManager`:** Runs background cooperative timer loops for Markdown decomposition (`ingestion_jobs`), asynchronous local vector embedding generation (`node_embeddings`), and invalidation sweeps (`CascadeWorker`).

---

## Starting the Daemon

### Prerequisites

1. PostgreSQL 16+ with the `pgvector` extension enabled.
2. FastEmbed local model cache or Ollama local inference container.
3. Persistent storage directories for PostgreSQL WAL and the bare Git repository.

### Launch Command

Start the server from the repository root:

```bash
cargo run --bin tks -- serve
```

Or run the compiled release binary directly:

```bash
./target/release/tks serve
```

### Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `DATABASE_URL` / `TKS_DATABASE_URL` | PostgreSQL connection string | `postgres://postgres:postgres@localhost:5432/tks` |
| `TKS_GIT_DIR` | Filesystem path to the bare Git repository | `.substrate/git/tks.git` |
| `TKS_BIND_ADDR` | Network socket address for HTTP gateway | `0.0.0.0:8080` |
| `TKS_LOG` | Tracing filter directive | `info,tks=debug` |

### Standalone Migration Mode

In CI/CD environments, init containers, or pre-deployment hooks, you can apply all embedded database migrations without binding to a network port:

```bash
cargo run --bin tks -- serve --migrate-only
```

This runs all pending refinery migrations (V1 through V4) against `$DATABASE_URL` and exits cleanly with status code `0`.

---

## Storage & Database Management

### Bare Git Repository

The substrate automatically initializes a bare Git repository at `.substrate/git/tks.git` on first boot. When documents are ingested, they are committed to the `refs/heads/specs` branch.

Because the directory resides on persistent storage, the entire commit history and Git object database (ODB) survive container restarts, rebuilds, and volume pruning.

### PostgreSQL Backups

Utility scripts in `scripts/` provide automated database backup and restore operations:

```bash
# Dump the current schema and property graph
./scripts/db-backup.sh

# Restore the most recent snapshot
./scripts/db-restore.sh
```

---

## Identity Provisioning & Security Model

In accordance with **Invariant I-7**, every mutation, draft proposal, and task elaboration must be attributed to a verified identity. TKS uses a high-performance two-tier authentication model:

* Long-lived cryptographic pre-shared keys (HMAC bearer tokens) stored in the `agent_identities` table.
* In-process LRU cache (`moka`) in `tks serve` validating credentials in sub-microsecond time.
* Immediate cache invalidation on identity revocation.

### Provisioning an Agent Identity

Use the `tks identity` CLI subcommand to provision an identity:

```bash
cargo run --bin tks -- identity create \
  --name "gemini-agent-alpha" \
  --role "AGENT" \
  --description "Autonomous coding agent for Phase 3 features"
```

Output:

```text
Identity successfully created:
  ID:          3e57f201-92be-4971-a4db-4dfbb1b37992
  Name:        gemini-agent-alpha
  Role:        AGENT
  Token:       tks_sec_38fe912a77bc40...
  Created At:  2026-10-04T15:20:00Z
```

Export the token to authenticate CLI and REST requests:

```bash
export TKS_AUTH_TOKEN="tks_sec_38fe912a77bc40..."
export TKS_SERVER_URL="http://127.0.0.1:8080"
```

### Revoking an Identity

If an agent credentials leak or a session completes, revoke the identity immediately:

```bash
cargo run --bin tks -- identity revoke 3e57f201-92be-4971-a4db-4dfbb1b37992
```

Upon revocation:

1. The identity's `status` transitions to `REVOKED` in the `agent_identities` table.
2. The daemon's in-process `moka` cache immediately evicts the token.
3. Any subsequent request using this token fails instantly with HTTP `401 Unauthorized` / `ERR_AUTH_FAILED`.
4. Monotonic audit logs preserve historical attribution for all prior actions taken by that identity.

---

## Dual Authentication Extraction

Clients can supply authentication tokens using either of two methods:

1. **Standard HTTP Bearer Header (REST and SSE):**

   ```http
   Authorization: Bearer tks_sec_38fe912a77bc40...
   ```

2. **JSON-RPC Mutation Payload Field (MCP stdio & SSE):**

   ```json
   {
     "jsonrpc": "2.0",
     "method": "tools/call",
     "params": {
       "name": "elaborate_task",
       "arguments": {
         "auth_token": "tks_sec_38fe912a77bc40...",
         "parent_id": "8fb41f11-76a0-48e2-9b24-77a83d739811",
         "title": "Verify Cycle Prevention"
       }
     },
     "id": 1
   }
   ```

Requests lacking valid credentials return standardized error code `-32000` with domain code `ERR_AUTH_FAILED`.
