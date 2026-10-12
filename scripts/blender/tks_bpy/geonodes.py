"""Geometry Nodes helpers built on Blender's bundled Essentials node-group assets.

Blender 5.x modifier inputs are set through ``mod.properties.inputs.<identifier>.value``;
``mod["Socket_N"]`` raises "doesn't support IDProperties".
"""

from pathlib import Path

import bpy

ESSENTIALS_FILES = (
    "geometry_nodes_essentials.blend",
    "procedural_hair_node_assets.blend",
    "geometry_nodes_dynamics.blend",
    "compositing_nodes_essentials.blend",
)


def essentials_dir():
    return Path(bpy.utils.system_resource("DATAFILES")) / "assets" / "nodes"


def list_essentials():
    """Map of .blend file name -> node-group names available for append."""
    out = {}
    for path in sorted(essentials_dir().glob("*.blend")):
        with bpy.data.libraries.load(str(path), link=False, assets_only=True) as (src, _dst):
            out[path.name] = sorted(src.node_groups)
    return out


def append_group(name):
    """Append an Essentials node group by name (idempotent; reuses an existing local copy)."""
    existing = bpy.data.node_groups.get(name)
    if existing is not None and existing.library is None:
        return existing
    for path in sorted(essentials_dir().glob("*.blend")):
        before = set(bpy.data.node_groups)
        with bpy.data.libraries.load(str(path), link=False) as (src, dst):
            found = name in src.node_groups
            if found:
                dst.node_groups = [name]
        if found:
            # Asset groups (and their dependencies) carry fake users; drop them so
            # unused copies are purged instead of accumulating.
            for ng in set(bpy.data.node_groups) - before:
                ng.use_fake_user = False
            return dst.node_groups[0]
    raise KeyError(f"node group {name!r} not found in {essentials_dir()}; see list_essentials()")


def inputs(group):
    """List [(identifier, name, socket_type)] of a node group's inputs."""
    return [
        (item.identifier, item.name, item.socket_type)
        for item in group.interface.items_tree
        if item.item_type == "SOCKET" and item.in_out == "INPUT"
    ]


def _identifier(mod, name, socket_type=None):
    matches = [i for i in inputs(mod.node_group) if i[1] == name]
    if socket_type:
        matches = [i for i in matches if i[2] == socket_type]
    if not matches:
        names = sorted({i[1] for i in inputs(mod.node_group)})
        raise KeyError(f"no input {name!r} on {mod.node_group.name}; valid: {names}")
    if len(matches) > 1:
        raise KeyError(f"input {name!r} is ambiguous {[m[2] for m in matches]}; pass socket_type=")
    return matches[0][0]


def set_input(mod, name, value, socket_type=None):
    """Set a Geometry Nodes modifier input by display name (menu inputs take item names)."""
    ident = _identifier(mod, name, socket_type)
    getattr(mod.properties.inputs, ident).value = value
    return mod


def set_input_attribute(mod, name, attribute, socket_type=None):
    """Drive a field input from a named attribute (vertex groups count), e.g. a scatter mask."""
    item = getattr(mod.properties.inputs, _identifier(mod, name, socket_type))
    item.type = "ATTRIBUTE"
    item.attribute_name = attribute
    return mod


def get_input(mod, name, socket_type=None):
    return getattr(mod.properties.inputs, _identifier(mod, name, socket_type)).value


def add_group_modifier(obj, group, name=None, **values):
    """Add a Nodes modifier using ``group`` (NodeTree or Essentials name) and set inputs by name."""
    if isinstance(group, str):
        group = append_group(group)
    mod = obj.modifiers.new(name or group.name, "NODES")
    mod.node_group = group
    for key, value in values.items():
        set_input(mod, key, value)
    return mod


def scatter(surface, instance, count=None, density=None, seed=0, scale=(1, 1, 1),
            random_scale=0.3, random_rotation_z_deg=180.0, align_to_normal=False,
            min_distance=0.0, collection=False, mask=None, name="Scatter"):
    """Scatter ``instance`` (object or collection) over ``surface`` with Essentials "Scatter on Surface".

    Give either ``count`` (total) or ``density`` (per m^2). ``mask`` names a vertex group/float
    attribute on ``surface`` (0 = nothing, 1 = full density; see ``scene.clearing_mask``).
    Hide the source instance(s) from render yourself (or exclude the kit collection).
    """
    import math

    mod = add_group_modifier(surface, "Scatter on Surface", name=name)
    if count is not None:
        set_input(mod, "Density Method", "Amount")
        set_input(mod, "Amount", int(count))
    else:
        set_input(mod, "Density Method", "Density")
        set_input(mod, "Density", float(density if density is not None else 1.0))
    set_input(mod, "Seed", seed)
    set_input(mod, "Keep Surface", True)
    if min_distance > 0:
        set_input(mod, "Minimum Distance", min_distance)
    if collection:
        set_input(mod, "Instance Type", "Collection")
        set_input(mod, "Collection", instance)
        set_input(mod, "Pick Instance", True)
    else:
        set_input(mod, "Object", instance)
    set_input(mod, "Align Rotation", align_to_normal)
    set_input(mod, "Scale", scale)
    set_input(mod, "Randomize", True)
    set_input(mod, "Randomize Rotation", (0.0, 0.0, math.radians(random_rotation_z_deg)))
    set_input(mod, "Randomize Scale", random_scale, socket_type="NodeSocketFloat")
    if mask:
        set_input_attribute(mod, "Distribution Mask", mask)
    return mod
