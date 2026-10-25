# Project Guidelines

Read `.shared/ai/core/service-instructions.md`.
Read `.shared/ai/core/common-instructions.md`.

## Maturity

- This project is in open development. There is no legacy or backwards compatibility to maintain.
- Good architecture & design practices are encouraged and expected.
- Expect frequent changes and iterations as the project evolves.

## Repository Layout

- Use `codegraph_explore` first to inspect existing Axum routes, state wrappers,
  and error envelopes before `find` or `grep`.
- This is a pure Rust package repository.
- The crate/package project is configured by the repository-root `Cargo.toml`.
- Reusable package code belongs in `src/lib.rs` (and submodules under `src/`).
- Executable entry points belong in `src/main.rs` (or `src/bin/`).
- Integration tests belong in `tests/` and should import the package as `rust_base`.
- Put one-off developer utilities or scripts in `scripts/`.
- Keep build artifacts such as `target/` out of commits unless the repository explicitly tracks them.
- Be explicitly aware that GEMINI.md is a symbolic link to .github/copilot-instructions.md

## Development Environment

- Development is containerized and configured strictly via `.devcontainer/` (`.devcontainer/devcontainer.json`, `.devcontainer/docker-compose.yml`).
- Supporting services (such as PostgreSQL 16+ with `pgvector` and persistent storage volumes) are defined and managed through the devcontainer Docker Compose configuration.
- Do not create or expect a standalone `docker-compose.yml` in the repository root; all containerized service and tool dependencies belong within `.devcontainer/`.

## Rust Development

- Use the official Cargo toolchain (stable Rust) managed via `rustup`.
- Build and verify the project from the repository root with `cargo check` and `cargo build`.
- Declare runtime and development dependencies in `Cargo.toml`; do not add ad hoc dependency files.
- Keep the supported Rust edition (`2024`), version (`rust-version`), and package metadata in `Cargo.toml` accurate.
- Follow existing code style and public APIs (`cargo fmt`, `cargo clippy`). Avoid unrelated refactors.
- Never hard-code, print, or commit credentials, API keys, tokens, or other secrets.

## Validation

- Run Rust tests with `cargo test`.
- Integration tests in `tests/` must obtain database connections via `db::ensure_test_database_ready()` or `tks::db::TestContext::new().await`. The test harness automatically provisions isolated test databases per test binary (`tks_test_<suite>`) cloned from `tks_template`, guaranteeing safe, collision-free parallel test execution.
- Run linter checks with `cargo clippy --all-targets --all-features -- -D warnings`.
- Check formatting with `cargo fmt --check`.
- When changing packaging metadata or preparing for release, package the crate with:
  `cargo package`.
- Inspect the generated package archive in `target/package/` when packaging changes affect included files or binaries.
- Run focused checks for the files and behavior changed before running broader validation.
- Keep intentional Markdown exceptions narrow and document them with targeted configuration or inline suppression.

## Dependencies

- Prefer well-established, maintained crates when a dependency is genuinely needed.
- Add every new dependency to the appropriate section of `Cargo.toml` and verify with `cargo check`.
- Avoid introducing a dependency for functionality that is small and clear to implement with the standard library.

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
- Keep-alive: `-1`
- Max loaded models: `1`
- Flash attention: enabled

## Blender scenes (MCP)

- Blender 5.2.2 LTS (Python 3.13) runs in the `blender` devcontainer service and is driven via the `blender` MCP server (`execute_blender_code`, `get_scene_info`, `look`, `search_assets`, `import_asset`, `viewport_*`). Never assume 2.8x–4.x APIs.
- Before writing scene code, load the workspace skills in `.agents/skills/`: `blender-scene-director` (any scene/render request), `blender-bpy-5x` (API facts and the `tks_bpy` helper library), `blender-extensions` (pinned community add-ons), `blender-assets` (Poly Haven, ambientCG, credits).
- Prefer `scripts/blender/tks_bpy` helpers over raw bpy; each MCP call has a fresh namespace, so re-import and look objects up with `T.scene.find`.
- Never modify or delete the user's existing scenes, objects, or materials; build in a new scene and save work under `scenes/`, renders under `renders/`.
- Record every downloaded asset in `scenes/assets/credits.json`; never commit asset API keys.
- Validate changes to `tks_bpy` or skill recipes with `python3 -m unittest discover -s scripts/blender -p 'test_*.py' -v`.
