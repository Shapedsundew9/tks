"""Wrappers for the pinned community extensions (see .devcontainer/blender/extensions.lock.json)."""

import addon_utils
import bpy

REPO = "user_default"


def module_name(ext_id):
    return f"bl_ext.{REPO}.{ext_id}"


def status():
    """Map of extension id -> enabled for every extension in the user repo."""
    out = {}
    for mod in addon_utils.modules():
        if mod.__name__.startswith(f"bl_ext.{REPO}."):
            out[mod.__name__.rsplit(".", 1)[-1]] = addon_utils.check(mod.__name__)[1]
    return out


def require(ext_id):
    """Ensure an installed extension is enabled; raise with guidance otherwise."""
    name = module_name(ext_id)
    loaded, enabled = addon_utils.check(name)
    if not enabled:
        mod = addon_utils.enable(name, default_set=True, persistent=True)
        if mod is None:
            raise RuntimeError(
                f"extension {ext_id!r} is not installed; add it to extensions.lock.json and rebuild the image"
            )
    return name


def _new_objects(before):
    return [o for o in bpy.data.objects if o not in before]


def landscape(size=(60, 60), subdivisions=(256, 256), height=4.0, seed=0, noise_type="hetero_terrain",
              feature_size=None, island=False, island_edge=0.6, name="Terrain", collection="Environment",
              **kwargs):
    """A.N.T. Landscape terrain whose relief (max z - min z) is exactly ``height`` metres.

    ``feature_size`` is ANT's ``noise_size`` in metres (default: largest side / 4; smaller is
    rougher). The base sits at z=0. ``island=True`` fades heights to 0 towards a round coast
    (``island_edge`` = fraction of the radius used for the slope). Extra kwargs go to
    bpy.ops.mesh.landscape_add (e.g. ``noise_depth``, ``strata``, ``water_plane``).
    """
    from . import scene as _scene

    require("antlandscape")
    before = set(bpy.data.objects)
    # ANT clamps heights to [minimum, maximum] (default +-1) and execute() is a no-op unless
    # refresh=True; generate unclamped, then normalise the relief ourselves.
    params = {"height": 1.0, "maximum": 1000.0, "minimum": -1000.0, "refresh": True,
              "noise_size": feature_size or max(size) / 4.0}
    params.update(kwargs)
    result = bpy.ops.mesh.landscape_add(
        mesh_size_x=size[0], mesh_size_y=size[1], subdivision_x=subdivisions[0],
        subdivision_y=subdivisions[1], random_seed=seed, noise_type=noise_type, **params,
    )
    new = _new_objects(before)
    if not new:
        raise RuntimeError(f"landscape_add created no object: {result}")
    obj = next(o for o in new if o.type == "MESH")
    verts = obj.data.vertices
    zs = [v.co.z for v in verts]
    if island:
        base, hx, hy = min(zs), size[0] / 2, size[1] / 2
        for v in verts:
            t = max(0.0, min(1.0, (1.0 - ((v.co.x / hx) ** 2 + (v.co.y / hy) ** 2) ** 0.5) / island_edge))
            v.co.z = base + (v.co.z - base) * t * t * (3 - 2 * t)
        zs = [v.co.z for v in verts]
    lo, hi = min(zs), max(zs)
    scale = height / (hi - lo) if hi > lo else 0.0
    for v in verts:
        v.co.z = (v.co.z - lo) * scale
    obj.data.update()
    obj.name = name
    for o in new:
        _scene.link(o, collection)
    return obj


def tree(seed=0, levels=3, show_leaves=True, location=(0, 0, 0), name="Tree", collection="Environment",
         convert_to_mesh=False, bark=None, foliage=None, **kwargs):
    """Sapling Tree Gen tree (curve trunk + optional "leaves" mesh child). Returns the trunk.

    Trunks are bevelled (Sapling's default is zero-width wire curves). ``bark``/``foliage`` are
    materials. Useful kwargs: ``scale`` (height-ish, default 13), ``ratio`` (trunk thickness,
    0.015), ``leaves`` (per branch, 25), ``leafScale`` (0.17), ``shape`` ("7" tend flame,
    "1" spherical, "2" hemispherical, "3" cylindrical, "4" tapered cylinder, "0" conical).
    """
    from . import materials as _materials
    from . import scene as _scene

    require("sapling_tree_gen")
    params = {"bevel": True, "bevelRes": 1}
    params.update(kwargs)
    before = set(bpy.data.objects)
    bpy.ops.curve.tree_add(do_update=True, seed=seed, levels=levels, showLeaves=show_leaves, **params)
    new = _new_objects(before)
    trunk = next((o for o in new if o.type == "CURVE"), new[0])
    trunk.name = name
    for o in new:
        _scene.link(o, collection)
        if o.type == "MESH" and o is not trunk:
            o.name = f"{name}_Leaves"
            if foliage is not None:
                _materials.assign(o, foliage)
    if bark is not None:
        _materials.assign(trunk, bark)
    trunk.location = location
    if convert_to_mesh:
        with bpy.context.temp_override(selected_editable_objects=[trunk], active_object=trunk, object=trunk):
            bpy.ops.object.convert(target="MESH")
    return trunk


def room(width=5.0, depth=4.0, height=2.7, wall_thickness=0.1, floor=True, ceiling=False,
         baseboard=True, name="Room", collection="Environment", centred=True):
    """Archimesh rectangular room (walls + optional floor/ceiling/baseboard children).

    The room spans +X/+Y from its origin; ``centred`` moves it so the floor centre is at (0, 0, 0).
    Wall angles in Archimesh are absolute and limited to [-180, 180], hence 0/90/180/-90.
    """
    from . import scene as _scene

    require("archimesh")
    before = set(bpy.data.objects)
    bpy.ops.mesh.archimesh_room()
    new = _new_objects(before)
    obj = next(o for o in new if len(getattr(o, "RoomGenerator", [])) > 0)
    rg = obj.RoomGenerator[0]
    rg.room_height = height
    rg.wall_width = wall_thickness
    rg.baseboard = baseboard
    rg.floor = floor
    rg.ceiling = ceiling
    rg.wall_num = 4
    for i, (length, angle) in enumerate(((width, 0.0), (depth, 90.0), (width, 180.0), (depth, -90.0))):
        wall = rg.walls[i]
        wall.w = length
        wall.a = True
        wall.r = angle
    obj.name = name
    for o in [obj, *obj.children_recursive]:
        _scene.link(o, collection)
    if centred:
        obj.location = (-width / 2, -depth / 2, 0.0)
    bpy.context.view_layer.update()
    return obj


def sun_position(latitude, longitude, month, day, hour, utc_zone=0.0, sun_object=None,
                 sky_texture=None, year=2026, north_offset_deg=0.0):
    """Drive a sun light (and optionally a world Sky Texture node name) from a real location/time."""
    import math

    require("sun_position")
    sp = bpy.context.scene.sun_pos_properties
    sp.usage_mode = "NORMAL"
    sp.latitude = latitude
    sp.longitude = longitude
    sp.month = month
    sp.day = day
    sp.year = year
    sp.time = hour
    sp.UTC_zone = utc_zone
    sp.north_offset = math.radians(north_offset_deg)
    if sun_object is not None:
        sp.sun_object = sun_object
    if sky_texture is not None:
        sp.sky_texture = sky_texture
    bpy.context.view_layer.update()
    return sp
