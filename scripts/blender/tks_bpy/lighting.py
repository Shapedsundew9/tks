"""Light creation helpers (all aim at a target via a Track To constraint)."""

import math

import bpy
from mathutils import Vector

from . import scene as _scene


def _track_to(obj, target):
    if target is None:
        return
    if isinstance(target, (tuple, list, Vector)):
        empty = bpy.data.objects.new(f"{obj.name}_Target", None)
        empty.location = Vector(target)
        _scene.link(empty, obj.users_collection[0])
        target = empty
    con = obj.constraints.new("TRACK_TO")
    con.target = target
    con.track_axis = "TRACK_NEGATIVE_Z"
    con.up_axis = "UP_Y"


def light(kind="AREA", name=None, location=(4, -4, 5), energy=500.0, color=(1, 1, 1), size=1.0,
          target=(0, 0, 0), collection="Lighting", temperature_k=None):
    """Create a light. ``kind`` is POINT, SUN, SPOT or AREA. Energy is W (sun: W/m^2 irradiance)."""
    name = name or f"{kind.title()}Light"
    data = bpy.data.lights.new(name, kind)
    data.energy = energy
    data.color = color
    if temperature_k is not None and hasattr(data, "use_temperature"):
        data.use_temperature = True
        data.temperature = temperature_k
    if kind == "AREA":
        data.size = size
    elif kind in {"POINT", "SPOT"}:
        data.shadow_soft_size = size
    elif kind == "SUN":
        data.angle = math.radians(max(size, 0.05))
    obj = bpy.data.objects.new(name, data)
    obj.location = location
    _scene.link(obj, collection)
    _track_to(obj, target)
    return obj


def sun(elevation_deg=35.0, azimuth_deg=135.0, strength=3.0, angle_deg=0.5, color=(1, 0.95, 0.9)):
    """Directional sun; azimuth measured from +Y toward +X."""
    obj = light("SUN", "Sun", location=(0, 0, 10), energy=strength, color=color, size=angle_deg, target=None)
    obj.rotation_euler = (math.radians(90 - elevation_deg), 0, math.radians(azimuth_deg))
    return obj


def three_point(target=(0, 0, 0.5), distance=4.0, key_energy=600.0, height=2.5):
    """Classic key/fill/rim area-light rig around ``target`` (product/studio shots)."""
    t = Vector(target)

    def at(az_deg, dist, h):
        a = math.radians(az_deg)
        return (t.x + dist * math.sin(a), t.y - dist * math.cos(a), t.z + h)

    return {
        "key": light("AREA", "Key", at(-45, distance, height), key_energy, size=1.5, target=t),
        "fill": light("AREA", "Fill", at(60, distance * 1.2, height * 0.6), key_energy * 0.35, size=2.5, target=t),
        "rim": light("AREA", "Rim", at(170, distance, height * 1.2), key_energy * 0.8, size=1.0, target=t),
    }


def softbox(name="Softbox", size=(2.0, 0.6), location=(0, -2.5, 3.5), target=(0, 0, 1.5), strength=8.0,
            color=(1, 1, 1), gradient=True, visible_camera=False, collection="Lighting"):
    """Emissive plane with optional gradient along its long side that reflects in surfaces."""
    import bmesh

    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=0.5)
    bmesh.ops.scale(bm, vec=Vector((size[0], size[1], 1.0)), verts=bm.verts)
    bm.to_mesh(mesh)
    bm.free()

    obj = bpy.data.objects.new(name, mesh)
    obj.location = Vector(location)
    _scene.link(obj, collection)
    _track_to(obj, target)

    if hasattr(obj, "visible_camera"):
        obj.visible_camera = visible_camera

    mat = bpy.data.materials.new(f"{name}_Mat")
    mat.use_nodes = True
    nt = mat.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    out.location = (400, 0)
    em = nt.nodes.new("ShaderNodeEmission")
    em.location = (150, 0)
    em.inputs["Strength"].default_value = strength
    em.inputs["Color"].default_value = (*color[:3], 1.0)

    if gradient:
        coord = nt.nodes.new("ShaderNodeTexCoord")
        coord.location = (-400, 0)
        sep = nt.nodes.new("ShaderNodeSeparateXYZ")
        sep.location = (-250, 0)
        ramp = nt.nodes.new("ShaderNodeValToRGB")
        ramp.location = (-100, 0)
        nt.links.new(coord.outputs["Generated"], sep.inputs["Vector"])
        nt.links.new(sep.outputs["X"], ramp.inputs["Fac"])
        ramp.color_ramp.elements[0].position = 0.0
        ramp.color_ramp.elements[0].color = (0.05, 0.05, 0.05, 1.0)
        ramp.color_ramp.elements[1].position = 1.0
        ramp.color_ramp.elements[1].color = (*color[:3], 1.0)
        nt.links.new(ramp.outputs["Color"], em.inputs["Color"])

    nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
    obj.data.materials.append(mat)
    return obj


def link_light(light_obj, receivers):
    """Cycles light linking: light only illuminates `receivers` (objects or collection)."""
    if not hasattr(light_obj, "light_linking"):
        return None
    if isinstance(receivers, bpy.types.Collection):
        coll = receivers
    elif isinstance(receivers, str):
        coll = bpy.data.collections.get(receivers)
        if coll is None:
            coll = _scene.ensure_collection(receivers)
    else:
        coll_name = f"{light_obj.name}_Receivers"
        coll = bpy.data.collections.get(coll_name)
        if coll is None:
            coll = bpy.data.collections.new(coll_name)
            bpy.context.scene.collection.children.link(coll)
        objs = receivers if isinstance(receivers, (list, tuple, set)) else [receivers]
        for o in objs:
            if o.name not in coll.objects:
                coll.objects.link(o)

    light_obj.light_linking.receiver_collection = coll
    return coll


def look_rig(pivot=(0, 0, 1.6), name="LookRig", collection="Lighting"):
    """Empty that camera-relative lights (back glow, rim) are parented to; render_shots rotates it."""
    obj = bpy.data.objects.get(name)
    if obj is None:
        obj = bpy.data.objects.new(name, None)
        obj.empty_display_type = "ARROWS"
        obj.empty_display_size = 0.5
        _scene.link(obj, collection)
    obj.location = Vector(pivot)
    return obj

