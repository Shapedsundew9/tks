"""Scene setup, collections, blockout and placement helpers."""

import bpy
from mathutils import Vector

DEFAULT_COLLECTIONS = ("Environment", "Props", "Lighting", "Cameras", "FX")


def new_scene(name, engine="CYCLES", resolution=(1920, 1080), samples=128, clear=True,
              collections=DEFAULT_COLLECTIONS, view_transform="AgX", look="None"):
    """Create (or reset) a metric scene with sane render defaults and make it active.

    ``clear=True`` reuses an existing scene of the same name after deleting its objects, but only
    if tks_bpy created it; scenes made by the user are never cleared (a ValueError asks for
    another name instead).
    """
    sc = bpy.data.scenes.get(name)
    if sc is None:
        sc = bpy.data.scenes.new(name)
        sc["tks_managed"] = True
    elif clear:
        if not sc.get("tks_managed"):
            raise ValueError(f"scene {name!r} was not created by tks_bpy; refusing to clear it - pick another name")
        for obj in list(sc.objects):
            bpy.data.objects.remove(obj, do_unlink=True)
        nested = list(sc.collection.children_recursive)
        for coll in list(sc.collection.children):
            sc.collection.children.unlink(coll)
        for coll in nested:  # parents come first, so children lose their last user in turn
            if coll.users == 0:
                bpy.data.collections.remove(coll)
    bpy.context.window.scene = sc

    sc.unit_settings.system = "METRIC"
    sc.unit_settings.scale_length = 1.0
    sc.unit_settings.length_unit = "METERS"
    sc.render.engine = engine
    sc.render.resolution_x, sc.render.resolution_y = resolution
    sc.render.resolution_percentage = 100
    sc.view_settings.view_transform = view_transform
    sc.view_settings.look = look if look == "None" else f"{view_transform} - {look}"
    if engine == "CYCLES":
        sc.cycles.device = "GPU"
        sc.cycles.samples = samples
        sc.cycles.preview_samples = min(samples, 64)
        sc.cycles.use_denoising = True
    elif engine == "BLENDER_EEVEE":
        sc.eevee.taa_render_samples = samples

    if sc.world is None:
        sc.world = bpy.data.worlds.new(f"{name}_World")
    for cname in collections:
        ensure_collection(cname, sc)
    ensure_mcp_toggles(sc)
    return sc


def find(name, scene=None):
    """Object called ``name`` in ``scene`` (or its ``name.001``-style duplicate). Each MCP
    ``execute_blender_code`` call starts with a fresh namespace, so later steps re-find objects."""
    scene = scene or bpy.context.scene
    obj = scene.objects.get(name)
    if obj is None:
        matches = sorted(o.name for o in scene.objects if o.name.startswith(name + "."))
        if not matches:
            raise KeyError(f"no object {name!r} in scene {scene.name!r}")
        obj = scene.objects[matches[0]]
    return obj


def ensure_collection(name, scene=None, parent=None):
    """Return the collection called ``name`` in ``scene`` (created under ``parent`` if missing).

    Lookup is scoped to the scene, so another scene's (or the user's) collection of the same
    name is never shared; a new one may get a ``.001`` suffix but is still found by ``name``.
    """
    if isinstance(name, bpy.types.Collection):
        return name
    scene = scene or bpy.context.scene
    for coll in scene.collection.children_recursive:
        if coll.get("tks_name") == name or (coll.name == name and coll.get("tks_name") is None):
            return coll
    coll = bpy.data.collections.new(name)
    coll["tks_name"] = name
    (parent or scene.collection).children.link(coll)
    return coll


def link(obj, collection):
    """Move ``obj`` exclusively into ``collection`` (name or Collection)."""
    if isinstance(collection, str):
        collection = ensure_collection(collection)
    for c in list(obj.users_collection):
        c.objects.unlink(obj)
    collection.objects.link(obj)
    return obj


def ensure_mcp_toggles(scene=None):
    """Re-apply asset-source toggles (they are per-scene and reset for new scenes)."""
    import os

    scene = scene or bpy.context.scene
    toggles = {
        "blendermcp_use_polyhaven": True,
        "blendermcp_use_polypizza": bool(os.environ.get("BLENDERMCP_POLYPIZZA_API_KEY")),
        "blendermcp_use_sketchfab": bool(os.environ.get("BLENDERMCP_SKETCHFAB_API_KEY")),
    }
    for prop, value in toggles.items():
        if hasattr(scene, prop):
            setattr(scene, prop, value)
    return {k: getattr(scene, k, None) for k in toggles}


def _new_mesh_object(name, mesh, collection):
    obj = bpy.data.objects.new(name, mesh)
    link(obj, collection or bpy.context.scene.collection)
    return obj


def ground(size=40.0, name="Ground", collection="Environment", material=None):
    """Flat square ground plane at z=0 (built with bmesh, no operator context needed)."""
    import bmesh

    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=size / 2)
    bm.to_mesh(mesh)
    bm.free()
    obj = _new_mesh_object(name, mesh, collection)
    if material is not None:
        obj.data.materials.append(material)
    return obj


def box(name, size=(1, 1, 1), location=(0, 0, 0), collection="Props", material=None, on_ground=True):
    """Real-size blockout box. ``size`` is in metres; ``location`` is the base centre if on_ground."""
    import bmesh

    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    if on_ground:
        bmesh.ops.translate(bm, vec=Vector((0, 0, size[2] / 2)), verts=bm.verts)
    bm.to_mesh(mesh)
    bm.free()
    obj = _new_mesh_object(name, mesh, collection)
    obj.location = location
    if material is not None:
        obj.data.materials.append(material)
    return obj


def world_bbox(objs):
    """World-space (min, max) Vectors over mesh-like objects and their children."""
    if not isinstance(objs, (list, tuple, set)):
        objs = [objs]
    deps = bpy.context.evaluated_depsgraph_get()
    lo = Vector((float("inf"),) * 3)
    hi = Vector((float("-inf"),) * 3)
    stack = list(objs)
    found = False
    while stack:
        obj = stack.pop()
        stack.extend(obj.children)
        if obj.type not in {"MESH", "CURVE", "SURFACE", "META", "FONT", "CURVES", "POINTCLOUD", "VOLUME"}:
            continue
        ev = obj.evaluated_get(deps)
        mw = ev.matrix_world
        if obj.type == "MESH":
            points = [mw @ Vector(corner) for corner in ev.bound_box]
        else:
            # Evaluated curve/text objects can report a placeholder +-1 bound_box; measure real geometry.
            try:
                me = ev.to_mesh()
                points = [mw @ v.co for v in me.vertices]
                ev.to_mesh_clear()
            except RuntimeError:
                points = []
            if not points:
                points = [obj.matrix_world @ Vector(corner) for corner in obj.bound_box]
        for p in points:
            lo = Vector(map(min, lo, p))
            hi = Vector(map(max, hi, p))
            found = True
    if not found:
        raise ValueError("no geometry found for bounding box")
    return lo, hi


def drop_to_ground(obj, z=0.0):
    """Translate ``obj`` so the lowest point of its hierarchy rests at ``z``."""
    bpy.context.view_layer.update()
    lo, _ = world_bbox(obj)
    obj.location.z += z - lo.z
    bpy.context.view_layer.update()
    return obj


def surface_height(surface, x, y):
    """World z of ``surface`` (evaluated, modifiers included) under (x, y), or None if missed."""
    deps = bpy.context.evaluated_depsgraph_get()
    ev = surface.evaluated_get(deps)
    mw = ev.matrix_world
    inv = mw.inverted()
    origin = inv @ Vector((x, y, 1.0e5))
    direction = (inv.to_3x3() @ Vector((0.0, 0.0, -1.0))).normalized()
    hit, loc, _normal, _index = ev.ray_cast(origin, direction)
    return (mw @ loc).z if hit else None


def place_on(obj, surface, x, y, sink=0.0):
    """Move ``obj`` to (x, y) resting on ``surface``; ``sink`` pushes it down (e.g. 0.05 on soil)."""
    z = surface_height(surface, x, y)
    if z is None:
        raise ValueError(f"({x}, {y}) is outside {surface.name}")
    obj.location.x, obj.location.y = x, y
    return drop_to_ground(obj, z - sink)


def _object_mode(obj):
    """Leave Edit Mode so mesh data (vertices, vertex groups) can be written."""
    if obj.mode != "OBJECT":
        with bpy.context.temp_override(active_object=obj, object=obj):
            bpy.ops.object.mode_set(mode="OBJECT")


def flatten_area(surface, x, y, radius, falloff=4.0, z=None):
    """Flatten the ``surface`` mesh to a level pad at (x, y) - for buildings and paths on terrain.

    Vertices within ``radius`` m move to ``z`` (default: current height at the centre) and blend back
    to the original terrain over ``falloff`` m. Returns the pad's world z. Call before ``place_on``.
    """
    if z is None:
        z = surface_height(surface, x, y)
        if z is None:
            raise ValueError(f"({x}, {y}) is outside {surface.name}")
    _object_mode(surface)
    mw = surface.matrix_world
    inv = mw.inverted()
    f = max(falloff, 1e-6)
    for v in surface.data.vertices:
        p = mw @ v.co
        d = ((p.x - x) ** 2 + (p.y - y) ** 2) ** 0.5
        t = max(0.0, min(1.0, (d - radius) / f))
        w = 1.0 - t * t * (3.0 - 2.0 * t)
        if w > 0.0:
            p.z += (z - p.z) * w
            v.co = inv @ p
    surface.data.update()
    return z


def clearing_mask(surface, clearings, name="ScatterMask", falloff=2.0):
    """Vertex group on ``surface``: 0 inside each (x, y, radius) clearing, ramping to 1 over ``falloff`` m.

    Pass the name as ``mask=`` to ``geonodes.scatter`` to keep paths, buildings and the camera clear.
    """
    _object_mode(surface)
    vg = surface.vertex_groups.get(name) or surface.vertex_groups.new(name=name)
    mw = surface.matrix_world
    for v in surface.data.vertices:
        p = mw @ v.co
        weight = 1.0
        for cx, cy, radius in clearings:
            d = ((p.x - cx) ** 2 + (p.y - cy) ** 2) ** 0.5
            weight = min(weight, max(0.0, min(1.0, (d - radius) / max(falloff, 1e-6))))
        vg.add([v.index], weight, "REPLACE")
    return vg


def height_mask(surface, min_z, max_z=None, falloff=0.5, name="HeightMask"):
    """Vertex group on ``surface``: 0 below ``min_z`` (world z), ramping to 1 over ``falloff`` m
    above it (and back to 0 approaching ``max_z``) - e.g. scatter trees only above the waterline."""
    _object_mode(surface)
    vg = surface.vertex_groups.get(name) or surface.vertex_groups.new(name=name)
    mw = surface.matrix_world
    f = max(falloff, 1e-6)
    for v in surface.data.vertices:
        z = (mw @ v.co).z
        weight = (z - min_z) / f
        if max_z is not None:
            weight = min(weight, (max_z - z) / f)
        vg.add([v.index], max(0.0, min(1.0, weight)), "REPLACE")
    return vg


def cut_opening(obj, size, location, name="Opening"):
    """Cut a box-shaped hole (window/door) through ``obj`` with a Boolean modifier.

    ``location`` is the world centre of the hole; make ``size`` thicker than the wall. The cutter
    stays (hidden, in "FX") so the opening can be moved later.
    """
    cutter = box(name, size=size, location=location, collection="FX", on_ground=False)
    cutter.display_type = "WIRE"
    cutter.hide_render = True
    cutter.visible_camera = cutter.visible_shadow = False
    mod = obj.modifiers.new(name, "BOOLEAN")
    mod.operation = "DIFFERENCE"
    mod.solver = "EXACT"
    mod.object = cutter
    return cutter


def cyclorama(width=4.0, depth=3.0, height=2.0, radius=0.6, segments=12, name="Cyclorama",
              collection="Environment", material=None):
    """Seamless studio backdrop: floor curving up into a back wall (+Y side), centred on X."""
    import math

    import bmesh

    profile = [(-depth / 2, 0.0)]
    cy, cz = depth / 2 - radius, radius
    for i in range(segments + 1):
        a = -math.pi / 2 + (math.pi / 2) * i / segments
        profile.append((cy + radius * math.cos(a), cz + radius * math.sin(a)))
    profile.append((depth / 2, height))
    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    left = [bm.verts.new((-width / 2, y, z)) for y, z in profile]
    right = [bm.verts.new((width / 2, y, z)) for y, z in profile]
    for i in range(len(profile) - 1):
        bm.faces.new((left[i], right[i], right[i + 1], left[i + 1]))
    bm.normal_update()
    bm.to_mesh(mesh)
    bm.free()
    for poly in mesh.polygons:
        poly.use_smooth = True
    obj = _new_mesh_object(name, mesh, collection)
    if material is not None:
        obj.data.materials.append(material)
    return obj


def smooth_by_angle(obj, angle_deg=30.0):
    """Shade smooth with an auto-smooth angle (Mesh.use_auto_smooth was removed in 4.1)."""
    import math

    if obj.type != "MESH":
        return obj
    for poly in obj.data.polygons:
        poly.use_smooth = True
    from . import geonodes

    mod = obj.modifiers.get("Smooth by Angle")
    if mod is None:
        mod = geonodes.add_group_modifier(obj, "Smooth by Angle", name="Smooth by Angle")
    geonodes.set_input(mod, "Angle", math.radians(angle_deg))
    return obj


def summary(scene=None):
    """Compact JSON-friendly description of the scene for checkpoints."""
    scene = scene or bpy.context.scene
    out = {
        "scene": scene.name,
        "engine": scene.render.engine,
        "resolution": [scene.render.resolution_x, scene.render.resolution_y],
        "camera": scene.camera.name if scene.camera else None,
        "world": scene.world.name if scene.world else None,
        "collections": {},
    }
    for coll in scene.collection.children_recursive:
        out["collections"][coll.name] = [
            {"name": o.name, "type": o.type, "dims": [round(d, 2) for d in o.dimensions]}
            for o in coll.objects
        ][:40]
    loose = [o.name for o in scene.collection.objects]
    if loose:
        out["collections"]["<root>"] = loose[:40]
    return out
