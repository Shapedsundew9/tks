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

ANTIGRAVITY_SETTINGS="$HOME/.gemini/antigravity-cli/settings.json"
if [[ ! -f "$ANTIGRAVITY_SETTINGS" ]]; then
    mkdir -p "$(dirname "$ANTIGRAVITY_SETTINGS")"
    cat > "$ANTIGRAVITY_SETTINGS" <<'EOF'
{
    "colorScheme": "solarized dark",
    "trustedWorkspaces": [
        "/workspaces"
    ],
    "executionMode": "accept-edits",
    "model": "Gemini 3.8 Flash (High)",
    "permissions": {
        "allow": [
            "write_file(*)",
            "command(*)"
        ],
        "deny": [
            "command(rm -rf)",
            "command(sudo)"
        ],
        "ask": [
            "plan"
        ]
    }
}
EOF
fi

if ! command -v agy >/dev/null 2>&1 && ! command -v antigravity >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -fsSL https://antigravity.google/cli/install.sh | bash
fi
