# Rust Template

A lightweight Rust project template for building small CLI-based applications and package-friendly code.

## Overview

This repository provides a minimal starting point with:

- a Rust package layout under `src/`
- a simple CLI entry point in `src/main.rs`
- reusable crate library code in `src/lib.rs`
- project configuration and dependencies in `Cargo.toml`
- toolchain specifications in `rust-toolchain.toml`
- a docs folder for project notes and style guidance

## Project Structure

```text
.
├── .devcontainer/
├── .github/
├── docs/
├── src/
│   ├── lib.rs
│   └── main.rs
├── tests/
│   └── cli.rs
├── Cargo.toml
├── GEMINI.md
├── LICENSE
├── .gitignore
├── .markdownlint-cli2.jsonc
├── README.md
└── rust-toolchain.toml
```

## Getting Started

1. Ensure the Rust toolchain (via [rustup](https://rustup.rs/)) is installed and up to date.
2. Build the project:

```bash
cargo build
```

1. Run the CLI:

```bash
cargo run
```

This currently prints:

```text
Hello World!
```

Alternatively, install the binary locally to invoke it directly:

```bash
cargo install --path .
protoproject
```

## Development

You can extend the template by adding modules under `src/` and updating dependencies in `Cargo.toml`.

### Example

```rust
use rust_base::run;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run()
}
```

## Testing and Verification

Run tests:

```bash
cargo test
```

Run the linter:

```bash
cargo clippy
```

Check code formatting:

```bash
cargo fmt --check
```

## Scripts and Tools

The project is configured with a binary entry point:

- `protoproject` -> `src/main.rs`

This makes it easy to run the CLI directly via Cargo:

```bash
cargo run --bin protoproject
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

- Service name: `ollama`
- Network endpoint (compose-internal): `http://ollama:11434`
- OpenAI-compatible endpoint: `http://ollama:11434/v1`
- No host port is published by default.

### Defaults

- Primary model: `qwen3:8b`
- Alternate model: `gemma4:e4b`
- Context length: `8192`
- Parallel requests: `1`
- Keep-alive: `-1` (models stay loaded until replaced or the service restarts)
- Max loaded models: `1`
- Flash attention: enabled

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

- API reachability
- model presence
- OpenAI-compatible `/v1/chat/completions`
- VRAM residency signal from `/api/ps`

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Notes

This is intentionally a simple starter repository. It is designed to be easy to customize for your own Rust tools, scripts, or command-line apps.
