"""Import downloaded community assets at real-world scale and track attribution."""

from pathlib import Path

import bpy
from mathutils import Vector

from . import scene as _scene
from .paths import ASSETS_DIR, CREDITS_FILE

_IMPORTERS = {
    ".glb": lambda p: bpy.ops.import_scene.gltf(filepath=p),
    ".gltf": lambda p: bpy.ops.import_scene.gltf(filepath=p),
    ".fbx": lambda p: bpy.ops.import_scene.fbx(filepath=p),
    ".obj": lambda p: bpy.ops.wm.obj_import(filepath=p),
    ".usd": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".usda": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".usdc": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".usdz": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".ply": lambda p: bpy.ops.wm.ply_import(filepath=p),
    ".stl": lambda p: bpy.ops.wm.stl_import(filepath=p),
}


def _append_blend(path):
    with bpy.data.libraries.load(str(path), link=False) as (src, dst):
        dst.objects = list(src.objects)
    objs = [o for o in dst.objects if o is not None]
    for o in objs:
        bpy.context.scene.collection.objects.link(o)
    return objs


def import_model(path, name=None, target_size=None, axis="max", location=(0, 0, 0),
                 rotation_z_deg=0.0, collection="Props", on_ground=True):
    """Import a model file and wrap it in a parent empty.

    ``target_size`` (metres) rescales so the chosen bbox ``axis`` ("x", "y", "z" or "max")
    matches; use real-world sizes (door 2.0 z, chair 0.9 z, mug 0.1 z, tree 8-15 z).
    The empty's origin is the base centre so ``location`` places the model on the ground.
    """
    import math

    path = Path(path)
    name = name or path.stem
    before = set(bpy.data.objects)
    scenes_before = set(bpy.data.scenes)
    active_scene = bpy.context.window.scene
    if path.suffix.lower() == ".blend":
        new = _append_blend(path)
    else:
        importer = _IMPORTERS.get(path.suffix.lower())
        if importer is None:
            raise ValueError(f"unsupported model format: {path.suffix}")
        result = importer(str(path))
        if "FINISHED" not in result:
            raise RuntimeError(f"import failed for {path}: {result}")
        new = [o for o in bpy.data.objects if o not in before]
    # The 5.2 glTF importer can create empty extra scenes; drop any the import added.
    bpy.context.window.scene = active_scene
    for extra in [sc for sc in bpy.data.scenes if sc not in scenes_before]:
        for o in extra.objects:
            if o not in new:
                continue
            if active_scene.collection not in o.users_collection:
                active_scene.collection.objects.link(o)
        bpy.data.scenes.remove(extra)
    if not new:
        raise RuntimeError(f"no objects imported from {path}")

    roots = [o for o in new if o.parent is None or o.parent not in new]
    bpy.context.view_layer.update()
    lo, hi = _scene.world_bbox(roots)
    dims = hi - lo
    scale = 1.0
    if target_size:
        ref = {"x": dims.x, "y": dims.y, "z": dims.z, "max": max(dims)}[axis]
        if ref <= 0:
            raise ValueError("imported model has zero extent")
        scale = target_size / ref

    parent = bpy.data.objects.new(name, None)
    parent.empty_display_type = "PLAIN_AXES"
    coll = _scene.ensure_collection(collection) if isinstance(collection, str) else collection
    coll.objects.link(parent)
    pivot = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z if on_ground else (lo.z + hi.z) / 2))
    for o in roots:
        o.parent = parent
        o.matrix_parent_inverse.identity()
        o.location -= pivot
    for o in new:
        for c in list(o.users_collection):
            c.objects.unlink(o)
        coll.objects.link(o)
    parent.scale = (scale, scale, scale)
    parent.location = location
    parent.rotation_euler.z = math.radians(rotation_z_deg)
    parent["tks_source"] = str(path)
    bpy.context.view_layer.update()
    return parent


def record_credit(asset_id, source, licence, author=None, url=None, notes=None):
    """Record attribution in scenes/assets/credits.json (mandatory for CC-BY assets)."""
    import sys

    scripts_dir = str(Path(__file__).resolve().parents[1])
    if scripts_dir not in sys.path:
        sys.path.insert(0, scripts_dir)
    import asset_credits

    return asset_credits.record_credit(asset_id, source, licence, author, url, notes, CREDITS_FILE)


def local_assets(kind=None):
    """List downloaded asset folders under scenes/assets (optionally a single source dir)."""
    root = ASSETS_DIR / kind if kind else ASSETS_DIR
    if not root.exists():
        return []
    return sorted(str(p.relative_to(ASSETS_DIR)) for p in root.iterdir() if p.is_dir())
