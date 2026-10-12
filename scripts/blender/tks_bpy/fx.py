"""Atmosphere and particle FX helpers (smoke volume, dust motes)."""

import math
from pathlib import Path

import bpy
from mathutils import Vector

from . import scene as _scene


def smoke_volume(size=(8, 6, 6), location=(0, 2.5, 0), density=0.05, noise_scale=0.5, detail=6,
                 ramp=(0.45, 0.8), color=(0.75, 0.82, 0.95), anisotropy=0.45, name="Smoke",
                 collection="FX"):
    """Uneven Principled Volume box with procedural noise density variation.

    Unlike uniform world fog, this produces local smoke gradients and light shafts.
    Shadow visibility is disabled to avoid dark exterior box shadows.
    """
    obj = bpy.data.objects.get(name)
    if obj is not None:
        bpy.data.objects.remove(obj, do_unlink=True)

    obj = _scene.box(name, size=size, location=location, collection=collection, on_ground=False)
    if hasattr(obj, "visible_shadow"):
        obj.visible_shadow = False

    mat = bpy.data.materials.new(f"{name}_VolumeMat")
    mat.use_nodes = True
    nt = mat.node_tree
    nt.nodes.clear()

    out = nt.nodes.new("ShaderNodeOutputMaterial")
    out.location = (400, 0)
    vol = nt.nodes.new("ShaderNodeVolumePrincipled")
    vol.location = (150, 0)
    vol.inputs["Color"].default_value = (*color[:3], 1.0)
    vol.inputs["Anisotropy"].default_value = anisotropy

    coord = nt.nodes.new("ShaderNodeTexCoord")
    coord.location = (-600, 0)

    noise = nt.nodes.new("ShaderNodeTexNoise")
    noise.location = (-400, 0)
    noise.inputs["Scale"].default_value = noise_scale
    noise.inputs["Detail"].default_value = float(detail)
    nt.links.new(coord.outputs["Object"], noise.inputs["Vector"])

    ramp_node = nt.nodes.new("ShaderNodeValToRGB")
    ramp_node.location = (-200, 0)
    ramp_node.color_ramp.elements[0].position = ramp[0]
    ramp_node.color_ramp.elements[1].position = ramp[1]
    nt.links.new(noise.outputs["Factor"], ramp_node.inputs["Fac"])

    mul = nt.nodes.new("ShaderNodeMath")
    mul.location = (0, 0)
    mul.operation = "MULTIPLY"
    mul.inputs[1].default_value = density
    nt.links.new(ramp_node.outputs["Color"], mul.inputs[0])
    nt.links.new(mul.outputs["Value"], vol.inputs["Density"])

    nt.links.new(vol.outputs["Volume"], out.inputs["Volume"])

    obj.data.materials.clear()
    obj.data.materials.append(mat)
    return obj


def dust_motes(region_size=(2.0, 1.5, 3.5), location=(0, -0.5, 1.6), count=600,
               radius=(0.0006, 0.002), emission=3.0, color=(0.8, 0.95, 1.0), seed=0,
               name="DustMotes", collection="FX"):
    """Geometry Nodes point cloud instancing icospheres for bokeh dust specks."""
    obj = bpy.data.objects.get(name)
    if obj is not None:
        bpy.data.objects.remove(obj, do_unlink=True)

    mesh = bpy.data.meshes.new(name)
    obj = bpy.data.objects.new(name, mesh)
    obj.location = Vector(location)
    _scene.link(obj, collection)

    mat_name = f"{name}_Mat"
    mat = bpy.data.materials.get(mat_name)
    if mat is None:
        mat = bpy.data.materials.new(mat_name)
        mat.use_nodes = True
        nt = mat.node_tree
        nt.nodes.clear()
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        em = nt.nodes.new("ShaderNodeEmission")
        em.inputs["Color"].default_value = (*color[:3], 1.0)
        em.inputs["Strength"].default_value = emission
        nt.links.new(em.outputs["Emission"], out.inputs["Surface"])

    tree_name = f"{name}_GN"
    old_tree = bpy.data.node_groups.get(tree_name)
    if old_tree is not None:
        bpy.data.node_groups.remove(old_tree)

    tree = bpy.data.node_groups.new(tree_name, "GeometryNodeTree")
    tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")

    pts = tree.nodes.new("GeometryNodePoints")
    pts.location = (-200, 200)
    pts.inputs["Count"].default_value = count

    rand_pos = tree.nodes.new("FunctionNodeRandomValue")
    rand_pos.location = (-450, 200)
    rand_pos.data_type = "FLOAT_VECTOR"
    rand_pos.inputs["Min"].default_value = (-region_size[0] / 2, -region_size[1] / 2, -region_size[2] / 2)
    rand_pos.inputs["Max"].default_value = (region_size[0] / 2, region_size[1] / 2, region_size[2] / 2)
    rand_pos.inputs["Seed"].default_value = seed
    tree.links.new(rand_pos.outputs["Value"], pts.inputs["Position"])

    ico = tree.nodes.new("GeometryNodeMeshIcoSphere")
    ico.location = (-200, -100)
    ico.inputs["Subdivisions"].default_value = 1
    ico.inputs["Radius"].default_value = (radius[0] + radius[1]) / 2.0

    inst = tree.nodes.new("GeometryNodeInstanceOnPoints")
    inst.location = (50, 100)
    tree.links.new(pts.outputs["Points"], inst.inputs["Points"])
    tree.links.new(ico.outputs["Mesh"], inst.inputs["Instance"])

    set_mat = tree.nodes.new("GeometryNodeSetMaterial")
    set_mat.location = (250, 100)
    set_mat.inputs["Material"].default_value = mat
    tree.links.new(inst.outputs["Instances"], set_mat.inputs["Geometry"])

    out_gn = tree.nodes.new("NodeGroupOutput")
    out_gn.location = (450, 100)
    tree.links.new(set_mat.outputs["Geometry"], out_gn.inputs["Geometry"])

    mod = obj.modifiers.new("DustGN", "NODES")
    mod.node_group = tree
    return obj
