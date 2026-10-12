# Blender and Antigravity in the TKS devcontainer

The Blender service is an additive development tool. The existing Rust app,
Postgres, Ollama, and other devcontainers do not need to be restarted to use it.
The previous standalone Blender experiment had no saved state and is not needed.

## Components

- `.devcontainer/docker-compose.blender.yml` defines service `blender`, its
  NVIDIA GPU access, and persistent `tks-blender-config` volume.
- `.devcontainer/blender/Dockerfile` derives from the recorded LinuxServer
  Blender 5.2.2 image digest and installs the checksum-verified add-on bundled
  with `mcp-for-blender==2.1.9`. No upstream source is vendored in the repository.
- `.devcontainer/blender/bootstrap.py` enables the GUI add-on automatically,
  binds its TCP socket to `0.0.0.0:9876`, disables telemetry, and selects NVIDIA
  OptiX (or CUDA if OptiX devices are unavailable). CPU Cycles devices are disabled.
- `scripts/blender/setup-mcp.sh` installs checksum-verified official uv 0.13.0
  binaries if absent, prepares managed Python 3.11, and registers the pinned
  stdio MCP tool with the existing `agy` CLI.

The add-on runs in the GUI Blender process. Running background Blender does not
start a usable MCP socket. The stdio MCP server runs inside the app and is launched
by Antigravity, not by a separate Compose service.

## Instance-specific workspace

This running workspace lives at `/workspaces/tks`, inside external Docker volume
`tks` mounted at `/workspaces` in the app. Blender mounts only that volume's `tks`
subdirectory at `/workspaces/tks`, so both processes see the same files.

This is deliberately specific to this development instance. The committed
`workspaceFolder` remains unchanged; `/workspace` is not the active shared path
here. If moving to a host-bind-based installation, first inspect the app's actual
mount and update only Blender's source mount to match it at `/workspaces/tks`.
Do not create a fresh empty `tks` volume to bypass a missing-volume error.

Use `/workspaces/tks/scenes` for scenes/assets and `/workspaces/tks/renders` for
renders. Both are ignored by Git. Blender runs with UID/GID 1000:1000 so generated
files remain writable by `vscode`.

The MCP registration also sets:

```text
BLENDER_HOST=blender
BLENDER_PORT=9876
UV_PYTHON_PREFERENCE=only-managed
BLENDER_MCP_DISABLE_TELEMETRY=1
TMPDIR=/workspaces/tks/renders/.mcp-tmp
```

**The shared `TMPDIR` is essential:** upstream viewport tools save a file in
Blender and read it from the MCP server. Separate container-local `/tmp`
directories cause screenshot failures even when the socket works.

## Start without disrupting existing services

From the repository root:

```bash
docker compose -p tks_devcontainer \
  -f .devcontainer/docker-compose.yml \
  -f .devcontainer/docker-compose.shared.yml \
  -f .devcontainer/docker-compose.blender.yml \
  config --quiet

docker compose -p tks_devcontainer \
  -f .devcontainer/docker-compose.yml \
  -f .devcontainer/docker-compose.shared.yml \
  -f .devcontainer/docker-compose.blender.yml \
  up -d --no-deps --build blender

bash scripts/blender/setup-mcp.sh
python3 .devcontainer/blender/check_ready.py --timeout 90
```

Do not run whole-stack `up`, `down`, `down -v`, volume pruning, or rebuild/reopen
the devcontainer just to install Blender. The live app has Dev Containers-generated
overrides, and applying only the base Compose files to it could recreate it
without its runtime mounts. The targeted command above changes only `blender`.

The existing default network can produce a Compose ownership warning because it
was created outside this project. Blender joins that existing network; no
replacement network or IPAM changes are necessary.

Future devcontainer creation includes `blender` in `runServices` and installs the
MCP tool through the existing post-create hook. Blender is not an app dependency:
GPU or Blender failure must not prevent Rust work. If optional MCP setup fails,
the hook reports it explicitly; rerun the setup helper to fix it.

The helper backs up existing Antigravity settings and its
`~/.gemini/config/mcp_config.json` privately before adding a new entry.
It preserves existing `codegraph`, `tks`, custom Blender registrations,
and disabled registrations. It does not alter permissions, models, or credentials.

## Desktop and Antigravity

On the Docker host, open **<https://localhost:3001>** and accept the container's
self-signed certificate. Port 3001 is published only on host loopback.
For a remote Docker host, use an authenticated tunnel, for example from your
local machine:

```bash
ssh -L 3001:127.0.0.1:3001 your-docker-host
```

Then open the same local HTTPS URL. VS Code forwarding a port from the app
container does not automatically forward this sibling container's host port.

The desktop is also reachable at `https://blender:3001` from the app's network.
Only use that address from a client that can reach the Compose network.

Launch a new interactive `agy` session, or restart your current session to pick
up the new registration. Ask it to inspect the Blender scene and approve the
Blender MCP tool when prompted. Existing interactive permissions are retained.
A headless `agy --print` call may auto-deny MCP access because it cannot prompt;
this is not a socket failure. Do not disable all permissions to work around it.

The desktop and MCP socket are unauthenticated within the trusted container
network, and Blender tools can execute Python and modify shared files. Port 9876
is never published to the host; it is raw TCP, **not** an HTTP MCP endpoint.
Do not expose either interface to the public Internet.

## GPU rendering

The host NVIDIA driver and Container Toolkit are already working with Ollama.
The Blender service reuses them and provides driver capabilities for compute,
graphics, display, video, and diagnostics. No host driver installation is needed.

The pinned Blender image was verified to enumerate the RTX 3080 through OptiX
and produce a GPU-only Cycles render. Although the LinuxServer documentation
does not promise GPU rendering, this pinned build works with this host driver.
Revalidate actual rendering before changing the base image or host driver.

New startup scenes use Cycles and `scene.cycles.device = "GPU"`. Loading an
existing scene does not rewrite its renderer/device setting; select Cycles/GPU
explicitly for scenes that were saved with CPU rendering. The add-on connection
is reconfigured after scene load, and GPU preferences persist in `/config`.

Ollama and Blender share the RTX 3080's 8 GiB VRAM. Compose GPU reservation grants
access, not exclusive ownership or a VRAM quota. Keep simultaneous jobs modest.
If rendering runs out of GPU memory, report the failure rather than silently
falling back to CPU; do not unload Ollama models or change its reservation
without approval.

## Verification and diagnostics

Focused standard-library tests:

```bash
python3 -m unittest discover -s scripts/blender -p 'test_*.py' -v
bash -n scripts/blender/setup-mcp.sh .devcontainer/blender/launch-blender.sh
```

Live MCP/GPU test, using the installed tool dependencies:

```bash
UV_PYTHON_PREFERENCE=only-managed BLENDER_MCP_DISABLE_TELEMETRY=1 \
  "$HOME/.local/bin/uv" run --python 3.11 \
  --with mcp-for-blender==2.1.9 python scripts/blender/verify.py
```

This test initializes MCP over stdio, queries the scene, checks the NVIDIA viewport
renderer and GPU-only Cycles device selection, renders a 128x128 test image,
reads a shared token in Blender and its output in the app, and captures the
viewport. It creates a temporary scene and restores the original scene afterward.
It retains uniquely named PNG, JSON, `.blend`, and viewport proof files in `renders/`.
The input token is removed after the test.

Use `--load-scene` only in a disposable GUI session: it additionally opens the
generated `.blend` and checks the add-on after file load, replacing the live scene.
Save user work first.

To verify restart persistence, save any live work and restart only Blender:

```bash
docker compose -p tks_devcontainer \
  -f .devcontainer/docker-compose.yml \
  -f .devcontainer/docker-compose.shared.yml \
  -f .devcontainer/docker-compose.blender.yml \
  restart blender
python3 .devcontainer/blender/check_ready.py --timeout 90
```

Repeat the live GPU test afterward. The Compose healthcheck verifies the GUI
add-on's protocol and response, not a render; TCP success alone is not acceptance.

Diagnostics:

```bash
docker logs --tail 100 tks_devcontainer-blender-1
docker exec tks_devcontainer-blender-1 tail -n 80 /config/log/blender-mcp.log
docker exec tks_devcontainer-blender-1 nvidia-smi
agy mcp list
```

The GUI log is in the persistent config volume. GPU discovery failure is logged
there explicitly. An AMD HIP-library warning is irrelevant to NVIDIA OptiX.

## Blender-only rollback

Stop only Blender:

```bash
docker compose -p tks_devcontainer \
  -f .devcontainer/docker-compose.yml \
  -f .devcontainer/docker-compose.shared.yml \
  -f .devcontainer/docker-compose.blender.yml \
  stop blender
```

If this setup created the CLI entry and you want it removed, run
`agy mcp remove blender`. Do not remove a registration created by someone else,
or restore the entire settings backup over later unrelated changes.

Retain the config volume and shared outputs. Remove Blender from `runServices`
if you do not want it started on future devcontainer creation. Do not tear down
the entire Compose project or prune volumes.
