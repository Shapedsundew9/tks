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
