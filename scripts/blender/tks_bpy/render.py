"""Preview/final rendering, compositor grading and saving."""

from contextlib import contextmanager
from pathlib import Path

import bpy

from .paths import RENDERS_DIR, SCENES_DIR


@contextmanager
def _temporary(scene, **overrides):
    """Temporarily set dotted scene attributes, restoring them afterwards."""
    saved = {}
    try:
        for dotted, value in overrides.items():
            *parts, attr = dotted.split("__")
            target = scene
            for p in parts:
                target = getattr(target, p)
            saved[dotted] = (target, attr, getattr(target, attr))
            setattr(target, attr, value)
        yield
    finally:
        for target, attr, value in saved.values():
            setattr(target, attr, value)


def _out_path(name, folder):
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    return folder / f"{name}.png"


def preview(name=None, width=480, samples=16, folder=RENDERS_DIR / "previews"):
    """Fast low-sample still (keeps GPU memory low; VRAM is shared with Ollama). Returns path."""
    sc = bpy.context.scene
    if sc.camera is None:
        raise RuntimeError("scene has no active camera; use tks_bpy.camera.rig() first")
    name = name or f"{sc.name}_preview"
    aspect = sc.render.resolution_y / sc.render.resolution_x
    out = _out_path(name, folder)
    overrides = {
        "render__resolution_x": width,
        "render__resolution_y": max(2, int(width * aspect)),
        "render__resolution_percentage": 100,
        "render__filepath": str(out),
        "render__image_settings__file_format": "PNG",
    }
    if sc.render.engine == "CYCLES":
        overrides["cycles__samples"] = samples
    elif sc.render.engine == "BLENDER_EEVEE":
        overrides["eevee__taa_render_samples"] = samples
    with _temporary(sc, **overrides):
        bpy.ops.render.render(write_still=True)
    return str(out)


def final(name=None, samples=None, folder=RENDERS_DIR):
    """Full-resolution still at the scene's own settings (optionally override samples)."""
    sc = bpy.context.scene
    if sc.camera is None:
        raise RuntimeError("scene has no active camera")
    out = _out_path(name or sc.name, folder)
    overrides = {"render__filepath": str(out), "render__image_settings__file_format": "PNG"}
    if samples is not None and sc.render.engine == "CYCLES":
        overrides["cycles__samples"] = samples
    with _temporary(sc, **overrides):
        bpy.ops.render.render(write_still=True)
    return str(out)


def grade(vignette=0.3, grain=0.0, scene=None):
    """Compositor grade with Essentials Vignette / Film Grain groups (Blender 5.x
    uses ``scene.compositing_node_group``; ``scene.node_tree`` no longer exists)."""
    from . import geonodes

    sc = scene or bpy.context.scene
    old = bpy.data.node_groups.get(f"{sc.name}_Grade")
    if old is not None:
        bpy.data.node_groups.remove(old)
    tree = bpy.data.node_groups.new(f"{sc.name}_Grade", "CompositorNodeTree")
    tree.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    rl = tree.nodes.new("CompositorNodeRLayers")
    rl.location = (-600, 0)
    out = tree.nodes.new("NodeGroupOutput")
    out.location = (600, 0)
    current = rl.outputs["Image"]
    x = -200
    if vignette > 0:
        node = tree.nodes.new("CompositorNodeGroup")
        node.node_tree = geonodes.append_group("Vignette")
        node.location = (x, 0)
        node.inputs["Factor"].default_value = vignette
        tree.links.new(current, node.inputs["Image"])
        current = node.outputs["Image"]
        x += 300
    if grain > 0:
        node = tree.nodes.new("CompositorNodeGroup")
        node.node_tree = geonodes.append_group("Film Grain")
        node.location = (x, 0)
        node.inputs["Factor"].default_value = grain
        tree.links.new(current, node.inputs[0])
        current = node.outputs[0]
    tree.links.new(current, out.inputs["Image"])
    sc.compositing_node_group = tree
    sc.render.use_compositing = True
    return tree


def save(name=None, folder=SCENES_DIR, copy=False):
    """Save the .blend into scenes/ (gitignored). ``copy=True`` keeps the current file path."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"{name or bpy.context.scene.name}.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(path), copy=copy)
    return str(path)
