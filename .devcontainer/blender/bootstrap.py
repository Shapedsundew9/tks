import importlib
import json
import os
from pathlib import Path
import shutil
import tomllib

import addon_utils
import bpy
from bpy.app.handlers import persistent


def configure_gpu():
    preferences = bpy.context.preferences.addons["cycles"].preferences
    for backend in ("OPTIX", "CUDA"):
        try:
            preferences.compute_device_type = backend
        except TypeError as error:
            print(f"TKS Blender: {backend} unavailable: {error}", flush=True)
            continue
        preferences.refresh_devices()
        devices = [device for device in preferences.devices if device.type == backend]
        if not devices:
            print(f"TKS Blender: no {backend} devices found", flush=True)
            continue
        for device in preferences.devices:
            device.use = device.type == backend
        print(
            "TKS Blender GPU: " + json.dumps({
                "backend": backend,
                "devices": [device.name for device in devices],
            }),
            flush=True,
        )
        return backend
    raise RuntimeError("No NVIDIA OptiX or CUDA Cycles device found; GPU setup failed")


EXTENSIONS_LOCK = Path("/opt/tks-blender/extensions.lock.json")
EXTENSIONS_DIR = Path("/opt/tks-blender/extensions")
BUNDLED_ADDONS = ("node_wrangler",)
# Scene property -> environment variable holding the key it needs (None: keyless).
ASSET_SOURCES = {
    "blendermcp_use_polyhaven": None,
    "blendermcp_use_polypizza": "BLENDERMCP_POLYPIZZA_API_KEY",
    "blendermcp_use_sketchfab": "BLENDERMCP_SKETCHFAB_API_KEY",
}


def installed_version(repo_dir, package_id):
    manifest = repo_dir / package_id / "blender_manifest.toml"
    if not manifest.is_file():
        return None
    with manifest.open("rb") as handle:
        return tomllib.load(handle).get("version")


def install_extensions():
    """Install pinned extensions offline and enable them; never blocks MCP startup."""
    if not EXTENSIONS_LOCK.is_file():
        print("TKS Blender: no extensions lock file; skipping extensions", flush=True)
        return
    lock = json.loads(EXTENSIONS_LOCK.read_text(encoding="utf-8"))
    repo = lock["repository"]
    repo_dir = Path(bpy.utils.user_resource("EXTENSIONS", path=repo, create=True))
    report = {}
    for package in lock["extensions"]:
        package_id = package["id"]
        try:
            if installed_version(repo_dir, package_id) != package["version"]:
                bpy.ops.extensions.package_install_files(
                    filepath=str(EXTENSIONS_DIR / f"{package_id}.zip"),
                    repo=repo,
                    enable_on_install=True,
                )
            module = f"bl_ext.{repo}.{package_id}"
            if module not in bpy.context.preferences.addons:
                addon_utils.enable(module, default_set=True, persistent=True)
            report[package_id] = installed_version(repo_dir, package_id)
        except Exception as error:  # noqa: BLE001 - report and keep MCP available
            report[package_id] = f"FAILED: {error}"
    for module in BUNDLED_ADDONS:
        if module not in bpy.context.preferences.addons:
            addon_utils.enable(module, default_set=True, persistent=True)
    print("TKS Blender extensions: " + json.dumps(report), flush=True)


@persistent
def configure_connection(_unused):
    scene = bpy.context.scene
    scene.blendermcp_port = 9876
    scene.blendermcp_auto_start_server = True
    for prop, key_env in ASSET_SOURCES.items():
        if hasattr(scene, prop):
            setattr(scene, prop, key_env is None or bool(os.environ.get(key_env)))


def main():
    addons_path = Path(bpy.utils.user_resource("SCRIPTS", path="addons", create=True))
    source = Path("/opt/tks-blender/blender_mcp.py")
    destination = addons_path / "blender_mcp.py"
    if not destination.exists() or destination.read_bytes() != source.read_bytes():
        shutil.copyfile(source, destination)
    bpy.utils.refresh_script_paths()
    addon_utils.enable("blender_mcp", default_set=True, persistent=True)
    addon = importlib.import_module("blender_mcp")

    # Upstream listens on localhost. All auto-start/operator-created servers
    # must instead accept connections from the app on the Compose network.
    class NetworkServer(addon.BlenderMCPServer):
        def __init__(self, host="0.0.0.0", port=9876):
            super().__init__(host=host, port=port)

    addon.BlenderMCPServer = NetworkServer
    bpy.context.preferences.addons["blender_mcp"].preferences.telemetry_consent = False
    configure_connection(None)
    if configure_connection not in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.insert(0, configure_connection)
    bpy.context.preferences.inputs.use_mouse_continuous = False
    install_extensions()
    configure_gpu()
    if not bpy.data.filepath:
        bpy.context.scene.render.engine = "CYCLES"
        bpy.context.scene.cycles.device = "GPU"
    bpy.ops.wm.save_userpref()
    print("TKS Blender: GPU preferences and network MCP bootstrap ready", flush=True)


if __name__ == "__main__":
    main()
