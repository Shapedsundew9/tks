import importlib
import json
from pathlib import Path
import shutil

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


@persistent
def configure_connection(_unused):
    scene = bpy.context.scene
    scene.blendermcp_port = 9876
    scene.blendermcp_auto_start_server = True


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
    configure_gpu()
    if not bpy.data.filepath:
        bpy.context.scene.render.engine = "CYCLES"
        bpy.context.scene.cycles.device = "GPU"
    bpy.ops.wm.save_userpref()
    print("TKS Blender: GPU preferences and network MCP bootstrap ready", flush=True)


if __name__ == "__main__":
    main()
