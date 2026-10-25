#!/bin/bash

# This script runs after the container is created.
# The 'set -e' command ensures that the script will exit immediately if a command fails.
set -e

echo "Configuring Rust toolchain..."
if command -v rustup >/dev/null 2>&1; then
    rust_components=cargo,rustc,rust-std,rustfmt,clippy,rust-analyzer,rust-src
    reinstall_stable=false
    if rustup_output=$(rustup toolchain install stable --profile minimal \
        --component "$rust_components" 2>&1); then
        printf '%s\n' "$rustup_output"
        if rustup_output=$({
            rustup run stable cargo --version && rustup run stable rustc --version
        } 2>&1); then
            printf '%s\n' "$rustup_output"
        else
            printf '%s\n' "$rustup_output" >&2
            reinstall_stable=true
        fi
    else
        printf '%s\n' "$rustup_output" >&2
        if [[ "$rustup_output" != *"failed to install component:"* ||
              "$rustup_output" != *"detected conflict:"* ]]; then
            exit 1
        fi
        reinstall_stable=true
    fi

    if [[ "$reinstall_stable" == true ]]; then
        echo "Repairing conflicting or incomplete stable toolchain files by reinstalling stable..." >&2
        rustup toolchain uninstall stable
        rustup toolchain install stable --profile minimal \
            --component "$rust_components"
        rustup run stable cargo --version
        rustup run stable rustc --version
    fi
    rustup default stable
fi

if [[ -f Cargo.toml ]]; then
    cargo check
fi

echo "Installing Playwright Chromium and browser system dependencies..."
(
    cd scripts/elicitation/gate
    npm install --no-package-lock
    npx --no-install playwright install --with-deps chromium
)

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
