#!/usr/bin/env bash
set -euo pipefail

if ! AGY="$(command -v agy || command -v antigravity)"; then
    echo "Antigravity CLI is missing; install it before configuring Blender MCP." >&2
    exit 1
fi
BIN_DIR="$HOME/.local/bin"
UV="$BIN_DIR/uv"
UVX="$BIN_DIR/uvx"
SHARED_TEMP="/workspaces/tks/renders/.mcp-tmp"

if [[ ! -f /workspaces/tks/Cargo.toml ]]; then
    echo "Shared Blender workspace /workspaces/tks is missing; verify the workspace mount." >&2
    exit 1
fi
mkdir -p "$SHARED_TEMP"

if [[ ! -x "$UV" || ! -x "$UVX" ]]; then
    if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
        echo "Blender MCP bootstrap supports Linux x86_64; install official uv for this architecture first." >&2
        exit 1
    fi
    TEMP_DIR="$(mktemp -d)"
    cleanup() {
        rm -f "$TEMP_DIR/uv.tar.gz" "$TEMP_DIR/uv-x86_64-unknown-linux-gnu/uv" "$TEMP_DIR/uv-x86_64-unknown-linux-gnu/uvx"
        if [[ -d "$TEMP_DIR/uv-x86_64-unknown-linux-gnu" ]]; then
            rmdir "$TEMP_DIR/uv-x86_64-unknown-linux-gnu"
        fi
        rmdir "$TEMP_DIR"
    }
    trap cleanup EXIT
    curl --fail --show-error --location \
        https://github.com/astral-sh/uv/releases/download/0.13.0/uv-x86_64-unknown-linux-gnu.tar.gz \
        --output "$TEMP_DIR/uv.tar.gz"
    echo "1468ebd5a5541121837c5a2817b9972ba6090fa6caa3d142620850a47fb75154  $TEMP_DIR/uv.tar.gz" | sha256sum --check --strict
    tar -xzf "$TEMP_DIR/uv.tar.gz" -C "$TEMP_DIR"
    mkdir -p "$BIN_DIR"
    install -m 755 "$TEMP_DIR/uv-x86_64-unknown-linux-gnu/uv" "$UV"
    install -m 755 "$TEMP_DIR/uv-x86_64-unknown-linux-gnu/uvx" "$UVX"
fi

"$UV" python install 3.11
UV_PYTHON_PREFERENCE=only-managed BLENDER_MCP_DISABLE_TELEMETRY=1 \
    "$UVX" --python 3.11 mcp-for-blender==2.1.9 --help

backup_config() {
    local config="$1" backup
    backup="$config.before-blender.$(date -u +%Y%m%dT%H%M%S).bak"
    cp -p "$config" "$backup"
    chmod 600 "$backup"
    echo "Private Antigravity configuration backup: $backup"
}

SERVERS="$("$AGY" mcp list)"
if grep -q '^blender[[:space:]]' <<< "$SERVERS"; then
    echo "Existing Blender MCP registration preserved (including its enabled/disabled state)."
else
    for CONFIG in "$HOME/.gemini/antigravity-cli/settings.json" "$HOME/.gemini/config/mcp_config.json"; do
        if [[ -f "$CONFIG" ]]; then
            backup_config "$CONFIG"
        fi
    done
    "$AGY" mcp add \
        --env BLENDER_HOST=blender \
        --env BLENDER_PORT=9876 \
        --env UV_PYTHON_PREFERENCE=only-managed \
        --env BLENDER_MCP_DISABLE_TELEMETRY=1 \
        --env TMPDIR="$SHARED_TEMP" \
        blender "$UVX" --python 3.11 mcp-for-blender==2.1.9
    "$AGY" mcp list
fi

# Pre-approve the Blender MCP tools the repo skills rely on (other entries are preserved).
SETTINGS="$HOME/.gemini/antigravity-cli/settings.json"
ALLOW_TOOLS=(get_addon_status get_scene_info execute_blender_code look search_assets import_asset
    viewport_capture viewport_latest viewport_pick)
if [[ -f "$SETTINGS" ]]; then
    MISSING="$(python3 - "$SETTINGS" "${ALLOW_TOOLS[@]}" <<'PY'
import json, sys
settings = json.load(open(sys.argv[1]))
allow = settings.get("permissions", {}).get("allow", [])
print(" ".join(t for t in sys.argv[2:] if f"mcp(blender/{t})" not in allow))
PY
)"
    if [[ -n "$MISSING" ]]; then
        backup_config "$SETTINGS"
    fi
else
    mkdir -p "$(dirname "$SETTINGS")"
    echo '{}' > "$SETTINGS"
    chmod 600 "$SETTINGS"
    MISSING="${ALLOW_TOOLS[*]}"
fi
if [[ -n "$MISSING" ]]; then
    python3 - "$SETTINGS" $MISSING <<'PY'
import json, os, sys
path = sys.argv[1]
settings = json.load(open(path))
allow = settings.setdefault("permissions", {}).setdefault("allow", [])
allow.extend(f"mcp(blender/{t})" for t in sys.argv[2:])
tmp = f"{path}.tmp"
with open(tmp, "w") as fh:
    json.dump(settings, fh, indent=2)
    fh.write("\n")
os.chmod(tmp, os.stat(path).st_mode & 0o777)
os.replace(tmp, path)
PY
    echo "Allowed Blender MCP tools in $SETTINGS: $MISSING"
else
    echo "Blender MCP tool permissions already present in $SETTINGS."
fi
