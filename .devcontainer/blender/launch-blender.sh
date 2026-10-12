#!/usr/bin/env bash
set -euo pipefail

mkdir -p /config/log
exec blender --python /opt/tks-blender/bootstrap.py "$@" \
    >> /config/log/blender-mcp.log 2>&1
