import argparse
import asyncio
import base64
import json
import os
from pathlib import Path
import sys
import uuid

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / ".devcontainer/blender"))
from check_ready import check_ready, send_command


GPU_SMOKE = r'''
import gpu
import json
from pathlib import Path
from mathutils import Vector

output = Path(OUTPUT_PATH)
token = TEST_TOKEN
assert (output / (token + ".input")).read_text() == token, "Workspace is not shared"
preferences = bpy.context.preferences.addons["cycles"].preferences
preferences.refresh_devices()
backend = preferences.compute_device_type
assert backend in ("OPTIX", "CUDA"), "NVIDIA Cycles backend not configured"
enabled = [device for device in preferences.devices if device.use]
assert enabled and all(device.type == backend for device in enabled), "CPU or wrong GPU backend enabled"
vendor = gpu.platform.vendor_get()
renderer = gpu.platform.renderer_get()
assert "NVIDIA" in vendor and "llvmpipe" not in renderer.lower(), "Viewport is not NVIDIA accelerated"

previous_scene = bpy.context.window.scene
scene = bpy.data.scenes.new("TKS_GPU_SMOKE_" + token)
objects = []
datablocks = []
try:
    bpy.context.window.scene = scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "GPU"
    scene.cycles.samples = 8
    scene.cycles.use_denoising = False
    scene.render.resolution_x = 128
    scene.render.resolution_y = 128
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(output / (token + ".png"))
    mesh = bpy.data.meshes.new(token)
    datablocks.append((bpy.data.meshes, mesh))
    mesh.from_pydata(
        [(-1,-1,-1), (-1,-1,1), (-1,1,-1), (-1,1,1),
         (1,-1,-1), (1,-1,1), (1,1,-1), (1,1,1)],
        [],
        [(0,4,6,2), (1,3,7,5), (0,1,5,4), (2,6,7,3), (0,2,3,1), (4,5,7,6)],
    )
    cube = bpy.data.objects.new("TKS_GPU_SMOKE_Cube", mesh)
    objects.append(cube)
    scene.collection.objects.link(cube)
    camera_data = bpy.data.cameras.new(token)
    datablocks.append((bpy.data.cameras, camera_data))
    camera = bpy.data.objects.new("TKS_GPU_SMOKE_Camera", camera_data)
    objects.append(camera)
    scene.collection.objects.link(camera)
    camera.location = (5, -7, 5)
    camera.rotation_euler = (-camera.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = camera
    light_data = bpy.data.lights.new(token, "AREA")
    datablocks.append((bpy.data.lights, light_data))
    light_data.energy = 1000
    light_data.size = 5
    light = bpy.data.objects.new("TKS_GPU_SMOKE_Light", light_data)
    objects.append(light)
    scene.collection.objects.link(light)
    light.location = (2, -4, 6)
    light.rotation_euler = (-Vector(light.location)).to_track_quat("-Z", "Y").to_euler()
    bpy.ops.wm.save_as_mainfile(filepath=str(output / (token + ".blend")), copy=True)
    bpy.ops.render.render(write_still=True)
    image_header = Path(scene.render.filepath).read_bytes()[:24]
    report = {
        "backend": backend,
        "devices": [device.name for device in enabled],
        "viewport_vendor": vendor,
        "viewport_renderer": renderer,
        "render_device": scene.cycles.device,
        "image": scene.render.filepath,
        "size": [int.from_bytes(image_header[16:20], "big"),
                 int.from_bytes(image_header[20:24], "big")],
        "shared_token": token,
    }
    (output / (token + ".json")).write_text(json.dumps(report, indent=2) + "\n")
    print("TKS_GPU_RESULT=" + json.dumps(report))
finally:
    bpy.context.window.scene = previous_scene
    bpy.data.scenes.remove(scene)
    for obj in objects:
        bpy.data.objects.remove(obj, do_unlink=True)
    for collection, data in datablocks:
        collection.remove(data)
'''


def png_size(path):
    data = path.read_bytes()
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise RuntimeError(f"Not a valid PNG: {path}")
    return int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")


async def verify(args):
    print(json.dumps(check_ready(args.host, 9876, 60)))
    output = REPO_ROOT / "renders"
    output.mkdir(exist_ok=True)
    (output / ".mcp-tmp").mkdir(exist_ok=True)
    token = "blender-smoke-" + uuid.uuid4().hex[:12]
    sentinel = output / (token + ".input")
    sentinel.write_text(token)
    parameters = StdioServerParameters(
        command=str(Path.home() / ".local/bin/uvx"),
        args=["--python", "3.11", "mcp-for-blender==2.1.9"],
        env={
            **os.environ,
            "BLENDER_HOST": args.host,
            "BLENDER_PORT": "9876",
            "UV_PYTHON_PREFERENCE": "only-managed",
            "BLENDER_MCP_DISABLE_TELEMETRY": "1",
            "TMPDIR": str(output / ".mcp-tmp"),
        },
    )
    try:
        async with stdio_client(parameters) as (read, write):
            async with ClientSession(read, write) as session:
                await session.initialize()
                tools = {tool.name for tool in (await session.list_tools()).tools}
                if not {"get_scene_info", "execute_blender_code", "look"} <= tools:
                    raise RuntimeError(f"Required MCP tools missing: {tools}")
                info = await session.call_tool("get_scene_info", {})
                if info.isError:
                    raise RuntimeError(f"Scene query failed: {info.content}")
                code = (
                    f"OUTPUT_PATH = {str(output)!r}\nTEST_TOKEN = {token!r}\n"
                    + GPU_SMOKE
                )
                result = await session.call_tool("execute_blender_code", {"code": code})
                text = "\n".join(item.text for item in result.content if item.type == "text")
                if result.isError or "TKS_GPU_RESULT=" not in text:
                    raise RuntimeError(f"GPU smoke render failed: {text}")
                report = json.loads((output / (token + ".json")).read_text())
                if png_size(output / (token + ".png")) != (128, 128):
                    raise RuntimeError("GPU render has incorrect dimensions")
                if report["shared_token"] != token or report["render_device"] != "GPU":
                    raise RuntimeError(f"Unexpected GPU/shared workspace report: {report}")
                viewport = await session.call_tool("look", {"max_size": 256})
                images = [item for item in viewport.content if item.type == "image"]
                if viewport.isError or not images:
                    raise RuntimeError(f"Viewport capture failed: {viewport.content}")
                viewport_path = output / (token + "-viewport.png")
                viewport_path.write_bytes(base64.b64decode(images[0].data, validate=True))
                png_size(viewport_path)
                print(json.dumps(report, indent=2))
                print(f"Viewport capture: {viewport_path}")
        if args.load_scene:
            scene_path = output / (token + ".blend")
            send_command(args.host, 9876, {
                "type": "execute_code",
                "params": {"code": f"bpy.ops.wm.open_mainfile(filepath={str(scene_path)!r})"},
            }, timeout=30)
            print("Scene-load readiness: " + json.dumps(check_ready(args.host, 9876, 30)))
    finally:
        sentinel.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description="Verify real NVIDIA rendering through MCP stdio")
    parser.add_argument("--host", default="blender")
    parser.add_argument("--load-scene", action="store_true",
                        help="Also open the generated test scene; use only in a disposable GUI session")
    args = parser.parse_args()
    asyncio.run(asyncio.wait_for(verify(args), timeout=180))


if __name__ == "__main__":
    main()
