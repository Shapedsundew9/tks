# The Knowledge Substrate (TKS)

The Knowledge Substrate (TKS) is an active, verified causal property graph and Model Context Protocol (MCP) gateway designed for continuous architectural governance, autonomous agent task elaboration, and multi-agent software co-evolution.

TKS replaces passive Markdown documents with a living property graph stored in PostgreSQL with `pgvector`, backed by an immutable bare Git document repository and an append-only audit ledger.

## Key Capabilities

* **Deterministic Markdown Decomposition:** Ingests specifications mechanically via CommonMark AST parsing (`pulldown-cmark`), establishing upward hierarchy edges (`DERIVED_FROM`) and exact 0-based byte spans anchored to the bare Git ODB (`INV-4`).
* **Graph-Bounded Context Envelopes:** Queries architectural context in $<50\text{ ms}$ (SLA-1) with depth clamped $\le 3$ and node budget capped at 40 (partitioned 30-node topological quota + 10 vector neighbors).
* **Autonomous Task Elaboration:** Verified agents elaborate tasks directly into `ACTIVE` state under parent nodes with `AUTONOMOUS_ELABORATION` policy, updating leaf execution statuses under native row locks (`FOR UPDATE`) with zero advisory lock contention.
* **Ephemeral Branch Workspaces:** Multi-agent co-evolution within isolated branch containers (`workspaces` table) with caller draft confidentiality (`INV-7`), advisory-lock-free task elaboration, and three-way topological merge CTEs.
* **Automated Invalidation Cascades:** When requirements change, a single-roundtrip multi-statement recursive CTE sweeps downstream dependencies to `NEEDS_REVERIFICATION` with shortest-path staleness propagation (`MIN(depth)`) and single-pass audit aggregation.
* **Zero-CDN Embedded Web Explorer:** Self-contained Cytoscape.js & Dagre interactive visualization served directly from the single binary (`GET /explorer`) with live real-time Server-Sent Events (SSE) telemetry.
* **Native Model Context Protocol (MCP):** Connects external AI agents (Claude Code, Cursor, Windsurf) via stdio proxy (`tks mcp-stdio`) or HTTP/SSE (`/mcp`, `/mcp/sse`).

## User Documentation

Comprehensive documentation and step-by-step operational walkthroughs are available in the **[User's Guide](docs/user-guide/README.md)**:

* [Chapter 1: Mental Model & Core Architecture](docs/user-guide/01-mental-model-and-architecture.md)
* [Chapter 2: Daemon & Identity Provisioning](docs/user-guide/02-daemon-and-identities.md)
* [Chapter 3: Ingesting Governing Documents](docs/user-guide/03-ingesting-governing-documents.md)
* [Chapter 4: Context Envelopes & Requirement Retrieval](docs/user-guide/04-context-envelopes-and-querying.md)
* [Chapter 5: Task Elaboration & Autonomous Execution](docs/user-guide/05-task-elaboration-and-execution.md)
* [Chapter 6: Multi-Agent Workspaces & Collaborative Co-Evolution](docs/user-guide/06-workspaces-and-multi-agent-coevolution.md)
* [Chapter 7: Document Evolution, Invalidation Storms & Reverification](docs/user-guide/07-document-evolution-and-invalidation.md)
* [Chapter 8: Web Explorer & Real-Time Observability](docs/user-guide/08-web-explorer-and-observability.md)
* [Chapter 9: Model Context Protocol (MCP) Integration](docs/user-guide/09-mcp-agent-integration.md)
* [Chapter 10: CLI Reference & Command Cheat Sheet](docs/user-guide/10-cli-reference.md)

## Getting Started

### 1. Build the Binary

```bash
cargo build --release
```

### 2. Start the Daemon

Ensure PostgreSQL with `pgvector` is running (managed automatically via `.devcontainer/docker-compose.yml`), then start the server:

```bash
cargo run --bin tks -- serve
```

### 3. Provision an Identity

```bash
cargo run --bin tks -- identity create --name "developer" --role "HUMAN"
```

Export the generated bearer token:

```bash
export TKS_AUTH_TOKEN="tks_sec_..."
export TKS_SERVER_URL="http://127.0.0.1:8080"
```

### 4. Run the Web Explorer

Open your browser to:

```text
http://localhost:8080/explorer
```

## Testing & Verification

Run tests:

```bash
cargo test
```

Run linter checks:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Check code formatting:

```bash
cargo fmt --check
```

## Substrate Storage & Database Management

### Persistent Git Repository

The development bare Git repository is stored inside the workspace at `.substrate/git/tks.git`. Because the workspace directory is bind-mounted directly from the host filesystem, the repository persists across container rebuilds and volume pruning (`docker compose down -v`).

You can override the Git storage path via the `TKS_GIT_DIR` environment variable or `--git-dir` CLI flag:

```bash
cargo run --bin tks -- serve --git-dir .substrate/git/tks.git
```

### Database Backup and Restore

Database snapshots can be created and restored using the utility scripts in `scripts/`:

#### Back up the database

Dumps the current PostgreSQL schema and state to `.substrate/backups/tks-db-<timestamp>.sql`:

```bash
./scripts/db-backup.sh
```

You can optionally pass a custom output path:

```bash
./scripts/db-backup.sh path/to/custom-backup.sql
```

#### Restore the database

Restores from the latest backup in `.substrate/backups/` (or a specified dump file):

```bash
# Restore latest snapshot
./scripts/db-restore.sh

# Restore a specific snapshot
./scripts/db-restore.sh .substrate/backups/tks-db-20261003-180448.sql
```

## Devcontainer local LLM (Ollama)

The devcontainer compose stack includes an isolated Ollama service for local inference.

* Service name: `ollama`
* Network endpoint (compose-internal): `http://ollama:11434`
* OpenAI-compatible endpoint: `http://ollama:11434/v1`
* No host port is published by default.

### Defaults

* Primary model: `qwen3:8b`
* Alternate model: `gemma4:e4b`
* Context length: `8192`
* Parallel requests: `1`
* Keep-alive: `-1` (models stay loaded until replaced or the service restarts)
* Max loaded models: `1`
* Flash attention: enabled

All values are overridable via environment variables in [devcontainer.json](.devcontainer/devcontainer.json) and [.devcontainer/docker-compose.yml](.devcontainer/docker-compose.yml).

### GPU prerequisite

The Ollama service is configured to require NVIDIA GPU access. Verify passthrough from the Docker host:

```bash
docker run --rm --gpus all ubuntu:24.04 nvidia-smi
```

### Pull models (manual, confirmation-gated)

No large model downloads happen automatically. Pull explicitly:

```bash
scripts/llm/pull-models.sh
```

Skip prompt and pull specific model(s):

```bash
scripts/llm/pull-models.sh --yes qwen3:8b
```

### Preload the default model

On every container start, `postStartCommand` runs `scripts/llm/preload-model.sh` in the background, loading `TKS_LLM_MODEL` (default `qwen3:8b`) with `keep_alive=-1`. It is skipped silently if Ollama is unreachable or the model is not pulled. Logs go to `/tmp/ollama-preload.log`. Run manually (optionally with a model name):

```bash
scripts/llm/preload-model.sh
```

Because `OLLAMA_MAX_LOADED_MODELS=1`, requesting a different model unloads the preloaded one.

### Smoke test

After pulling models, run:

```bash
scripts/llm/smoke-test.sh
```

This checks:

* API reachability
* model presence
* OpenAI-compatible `/v1/chat/completions`
* VRAM residency signal from `/api/ps`

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Notes

This is intentionally a simple starter repository. It is designed to be easy to customize for your own Rust tools, scripts, or command-line apps.
