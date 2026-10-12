"""Volumetric atmosphere: world fog and local fog boxes."""

import bpy

from . import scene as _scene


def world_fog(density=0.01, color=(0.8, 0.85, 0.9), anisotropy=0.2, scene=None):
    """Add a Principled Volume to the world volume output (Cycles; cheap haze)."""
    scene = scene or bpy.context.scene
    world = scene.world
    world.use_nodes = True
    nt = world.node_tree
    out = next(n for n in nt.nodes if n.type == "OUTPUT_WORLD")
    vol = nt.nodes.get("TKS_WorldFog") or nt.nodes.new("ShaderNodeVolumePrincipled")
    vol.name = "TKS_WorldFog"
    vol.location = (150, -250)
    vol.inputs["Density"].default_value = density
    vol.inputs["Color"].default_value = (*color, 1.0)
    vol.inputs["Anisotropy"].default_value = anisotropy
    nt.links.new(vol.outputs["Volume"], out.inputs["Volume"])
    return vol


def fog_box(size=(40, 40, 8), location=(0, 0, 0), density=0.02, color=(0.85, 0.9, 1.0),
            anisotropy=0.3, name="FogBox"):
    """Bounded fog volume (works in both Cycles and EEVEE). Base sits on ``location``."""
    mat = bpy.data.materials.new(f"{name}_Mat")
    mat.use_nodes = True
    nt = mat.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    vol = nt.nodes.new("ShaderNodeVolumePrincipled")
    vol.inputs["Density"].default_value = density
    vol.inputs["Color"].default_value = (*color, 1.0)
    vol.inputs["Anisotropy"].default_value = anisotropy
    nt.links.new(vol.outputs["Volume"], out.inputs["Volume"])
    obj = _scene.box(name, size=size, location=location, collection="FX", material=mat)
    obj.display_type = "WIRE"
    obj.visible_shadow = False
    return obj
