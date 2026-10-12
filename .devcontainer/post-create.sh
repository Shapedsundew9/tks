#!/bin/bash

# This script runs after the container is created.
# The 'set -e' command ensures that the script will exit immediately if a command fails.
set -e

echo "Configuring Rust toolchain..."
if command -v rustup >/dev/null 2>&1; then
    if rustup_output=$(rustup toolchain install stable --profile minimal \
        --component rustfmt,clippy,rust-analyzer,rust-src 2>&1); then
        printf '%s\n' "$rustup_output"
    else
        printf '%s\n' "$rustup_output" >&2
        if [[ "$rustup_output" != *"failed to install component:"* ||
              "$rustup_output" != *"detected conflict:"* ]]; then
            exit 1
        fi

        echo "Repairing conflicting stable toolchain files by reinstalling stable..." >&2
        rustup toolchain uninstall stable
        rustup toolchain install stable --profile minimal \
            --component rustfmt,clippy,rust-analyzer,rust-src
    fi
    rustup default stable
fi

if [[ -f Cargo.toml ]]; then
    cargo check
fi

if [[ -x .shared/tools/scripts/configure-subtree.sh ]]; then
    .shared/tools/scripts/configure-subtree.sh
fi

echo "Initializing test database tks_test..."
if command -v psql >/dev/null 2>&1; then
    PGPASSWORD=postgres psql -h postgres -U postgres -d postgres -tc "SELECT 1 FROM pg_database WHERE datname = 'tks_test'" 2>/dev/null | grep -q 1 || \
        PGPASSWORD=postgres psql -h postgres -U postgres -d postgres -c "CREATE DATABASE tks_test;" 2>/dev/null || true
    PGPASSWORD=postgres psql -h postgres -U postgres -d tks_test -c "CREATE EXTENSION IF NOT EXISTS vector;" 2>/dev/null || true
fi

bash .shared/devcontainer/post-create.sh

if ! bash scripts/blender/setup-mcp.sh; then
    echo "Blender MCP setup failed; Rust setup is complete. Retry: bash scripts/blender/setup-mcp.sh" >&2
fi
