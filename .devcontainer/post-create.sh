#!/bin/bash

# This script runs after the container is created.
# The 'set -e' command ensures that the script will exit immediately if a command fails.
set -e

echo "Configuring Rust toolchain..."
if command -v rustup >/dev/null 2>&1; then
    rustup update stable
    rustup default stable
    rustup component add rustfmt clippy rust-analyzer rust-src
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
