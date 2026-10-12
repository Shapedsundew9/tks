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


def post(bloom=0.4, bloom_threshold=0.9, bloom_size=7, dispersion=0.0, vignette=0.3, grain=0.03,
         lift=(1, 1, 1), gamma=(1, 1, 1), gain=(1, 1, 1), scene=None):
    """Compositor post-processing stack (Bloom glare, dispersion, color balance, vignette, grain)."""
    from . import geonodes

    sc = scene or bpy.context.scene
    old = bpy.data.node_groups.get(f"{sc.name}_Grade")
    if old is not None:
        bpy.data.node_groups.remove(old)
    tree = bpy.data.node_groups.new(f"{sc.name}_Grade", "CompositorNodeTree")
    tree.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    rl = tree.nodes.new("CompositorNodeRLayers")
    rl.location = (-800, 0)
    out = tree.nodes.new("NodeGroupOutput")
    out.location = (800, 0)
    current = rl.outputs["Image"]
    x = -500

    if bloom > 0:
        g = tree.nodes.new("CompositorNodeGlare")
        g.location = (x, 0)
        g.inputs["Type"].default_value = "Bloom"
        g.inputs["Threshold"].default_value = bloom_threshold
        g.inputs["Size"].default_value = bloom_size
        g.inputs["Strength"].default_value = bloom
        tree.links.new(current, g.inputs["Image"])
        current = g.outputs["Image"]
        x += 250

    if dispersion > 0:
        ld = tree.nodes.new("CompositorNodeLensdist")
        ld.location = (x, 0)
        ld.inputs["Dispersion"].default_value = dispersion
        if "Fit" in ld.inputs:
            ld.inputs["Fit"].default_value = True
        tree.links.new(current, ld.inputs["Image"])
        current = ld.outputs["Image"]
        x += 250

    if lift != (1, 1, 1) or gamma != (1, 1, 1) or gain != (1, 1, 1):
        cb = tree.nodes.new("CompositorNodeColorBalance")
        cb.location = (x, 0)
        cb.inputs[4].default_value = (*lift, 1.0)
        cb.inputs[6].default_value = (*gamma, 1.0)
        cb.inputs[8].default_value = (*gain, 1.0)
        tree.links.new(current, cb.inputs["Image"])
        current = cb.outputs["Image"]
        x += 250

    if vignette > 0:
        node = tree.nodes.new("CompositorNodeGroup")
        node.node_tree = geonodes.append_group("Vignette")
        node.location = (x, 0)
        node.inputs["Factor"].default_value = vignette
        tree.links.new(current, node.inputs["Image"])
        current = node.outputs["Image"]
        x += 250

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


def grade(vignette=0.3, grain=0.0, scene=None):
    """Compositor grade with Essentials Vignette / Film Grain groups (convenience wrapper around post)."""
    return post(bloom=0.0, dispersion=0.0, vignette=vignette, grain=grain, scene=scene)


def render_shots(names, preview=True, rig_pivot="LookRig", scene=None, **kw):
    """Render shot cameras by name (e.g. 'hero' or 'Shot_hero').

    For cameras with rig_azimuth=True, LookRig (if present) is rotated to match camera azimuth.
    Returns {shot_name: file_path}.
    """
    import math

    sc = scene or bpy.context.scene
    results = {}
    pivot_obj = bpy.data.objects.get(rig_pivot)

    for name in names:
        cam_obj = bpy.data.objects.get(name) or bpy.data.objects.get(f"Shot_{name}")
        if cam_obj is None or cam_obj.type != "CAMERA":
            raise ValueError(f"Shot camera '{name}' not found")
        sc.camera = cam_obj

        if cam_obj.get("tks_rig_azimuth", True) and pivot_obj is not None:
            target = cam_obj.get("tks_target", [pivot_obj.location.x, pivot_obj.location.y, pivot_obj.location.z])
            dx = cam_obj.location.x - target[0]
            dy = cam_obj.location.y - target[1]
            cam_azimuth = math.atan2(dy, dx)
            pivot_obj.rotation_euler.z = cam_azimuth + math.pi / 2
            bpy.context.view_layer.update()

        shot_label = cam_obj.name.replace("Shot_", "")
        file_name = f"{sc.name}_{shot_label}"
        if preview:
            path = globals()["preview"](name=file_name, **kw)
        else:
            path = globals()["final"](name=file_name, **kw)
        results[name] = path

    return results


def contact_sheet(paths, out, cols=3, height=360):
    """Compose multiple image files into a single contact sheet grid."""
    import math
    import numpy as np

    if not paths:
        raise ValueError("No paths provided for contact sheet")

    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)

    loaded = []
    max_w = 0
    for p in paths:
        img = bpy.data.images.load(str(p), check_existing=False)
        w = max(2, int(round(img.size[0] * height / max(1, img.size[1]))))
        img.scale(w, height)
        raw = np.empty(w * height * 4, dtype=np.float32)
        img.pixels.foreach_get(raw)
        arr = raw.reshape((height, w, 4))
        top_down = np.flipud(arr)
        bpy.data.images.remove(img)
        loaded.append(top_down)
        if w > max_w:
            max_w = w

    n_images = len(loaded)
    rows = math.ceil(n_images / cols)

    padded_cells = []
    for arr in loaded:
        h, w, c = arr.shape
        cell = np.zeros((height, max_w, 4), dtype=np.float32)
        cell[:, :, 3] = 1.0
        start_x = (max_w - w) // 2
        cell[:, start_x : start_x + w, :] = arr
        padded_cells.append(cell)

    for _ in range(rows * cols - n_images):
        cell = np.zeros((height, max_w, 4), dtype=np.float32)
        cell[:, :, 3] = 1.0
        padded_cells.append(cell)

    grid_rows = []
    for r in range(rows):
        row_cells = padded_cells[r * cols : (r + 1) * cols]
        grid_rows.append(np.concatenate(row_cells, axis=1))

    full_top_down = np.concatenate(grid_rows, axis=0)
    full_blender = np.flipud(full_top_down)

    total_h, total_w, _ = full_blender.shape
    comp_img = bpy.data.images.new("sheet_out", width=total_w, height=total_h, alpha=True)
    try:
        comp_img.pixels.foreach_set(full_blender.ravel())
        comp_img.filepath_raw = str(out)
        comp_img.file_format = "PNG"
        comp_img.save()
    finally:
        bpy.data.images.remove(comp_img)

    return str(out)


def save(name=None, folder=SCENES_DIR, copy=False):
    """Save the .blend into scenes/ (gitignored). ``copy=True`` keeps the current file path."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"{name or bpy.context.scene.name}.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(path), copy=copy)
    return str(path)
