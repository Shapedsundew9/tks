"""Typography and procedural engraving helpers."""

import math
from pathlib import Path

import bpy

from .paths import ASSETS_DIR

FONT_DIR = ASSETS_DIR / "fonts"


def font(name_or_path):
    """Load a VectorFont by catalogue key, file stem, family name, or path."""
    p = Path(name_or_path)
    if p.exists() and p.is_file():
        return bpy.data.fonts.load(str(p), check_existing=True)

    norm = str(name_or_path).lower().replace(" ", "").replace("-", "").replace("_", "")

    # Check already loaded fonts
    for f in bpy.data.fonts:
        f_norm = f.name.lower().replace(" ", "").replace("-", "").replace("_", "")
        if norm in f_norm:
            return f

    # Search in FONT_DIR
    if FONT_DIR.exists():
        for ttf in FONT_DIR.rglob("*.ttf"):
            stem_norm = ttf.stem.lower().replace(" ", "").replace("-", "").replace("_", "")
            parent_norm = ttf.parent.name.lower().replace(" ", "").replace("-", "").replace("_", "")
            if norm == stem_norm or norm == parent_norm or norm in stem_norm:
                return bpy.data.fonts.load(str(ttf), check_existing=True)

    available = [p.name for p in FONT_DIR.rglob("*.ttf")] if FONT_DIR.exists() else []
    raise LookupError(f"Font '{name_or_path}' not found. Available in {FONT_DIR}: {available}")


def engrave(target, text, font_obj, size, depth, center_xz, *, axis="-Y", tracking=0.0,
            width_scale=1.0, line_spacing=1.0, align="CENTER", wall_material=None,
            floor_material=None, solver="EXACT", name=None, space="WORLD"):
    """Procedural, non-destructive engraving as a Geometry Nodes modifier on target."""
    if isinstance(font_obj, (str, Path)):
        font_obj = font(font_obj)

    # Ensure assigned materials exist in target's material slots for Cycles
    for m in (wall_material, floor_material):
        if m is not None and m.name not in target.data.materials:
            target.data.materials.append(m)

    mod_name = name or f"Engrave_{text[:10]}"
    tree_name = f"{mod_name}_GN"
    old_tree = bpy.data.node_groups.get(tree_name)
    if old_tree is not None:
        bpy.data.node_groups.remove(old_tree)

    tree = bpy.data.node_groups.new(tree_name, "GeometryNodeTree")
    tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")

    in_node = tree.nodes.new("NodeGroupInput")
    out_node = tree.nodes.new("NodeGroupOutput")

    # 1. String to Curves
    stc = tree.nodes.new("GeometryNodeStringToCurves")
    stc.location = (-800, 200)
    stc.inputs["String"].default_value = text
    stc.inputs["Size"].default_value = size
    stc.inputs["Font"].default_value = font_obj
    align_x = "Center" if align.upper() == "CENTER" else ("Left" if align.upper() == "LEFT" else "Right")
    stc.inputs["Align X"].default_value = align_x
    stc.inputs["Align Y"].default_value = "Middle"
    stc.inputs["Character Spacing"].default_value = 1.0 + tracking
    stc.inputs["Line Spacing"].default_value = line_spacing

    # 2. Realize Instances
    realize = tree.nodes.new("GeometryNodeRealizeInstances")
    realize.location = (-600, 200)
    tree.links.new(stc.outputs["Curve Instances"], realize.inputs["Geometry"])

    # 3. Transform (width scale)
    curr_geo = realize.outputs["Geometry"]
    if abs(width_scale - 1.0) > 1e-4:
        scale_trans = tree.nodes.new("GeometryNodeTransform")
        scale_trans.location = (-450, 200)
        scale_trans.inputs["Scale"].default_value = (width_scale, 1.0, 1.0)
        tree.links.new(curr_geo, scale_trans.inputs["Geometry"])
        curr_geo = scale_trans.outputs["Geometry"]

    # 4. Fill Curve (N-gons)
    fill = tree.nodes.new("GeometryNodeFillCurve")
    fill.location = (-300, 200)
    fill.inputs["Mode"].default_value = "N-gons"
    tree.links.new(curr_geo, fill.inputs["Curve"])

    # 5. Extrude Mesh (cutter depth + 5mm overshoot)
    overshoot = 0.005
    extrude = tree.nodes.new("GeometryNodeExtrudeMesh")
    extrude.location = (-100, 200)
    extrude.inputs["Offset Scale"].default_value = depth + overshoot
    tree.links.new(fill.outputs["Mesh"], extrude.inputs["Mesh"])

    # 6. Material assignments on cutter (walls and floor)
    cutter_mesh = extrude.outputs["Mesh"]
    if wall_material is not None:
        set_wall = tree.nodes.new("GeometryNodeSetMaterial")
        set_wall.location = (100, 250)
        set_wall.inputs["Material"].default_value = wall_material
        tree.links.new(cutter_mesh, set_wall.inputs["Geometry"])
        tree.links.new(extrude.outputs["Side"], set_wall.inputs["Selection"])
        cutter_mesh = set_wall.outputs["Geometry"]

    if floor_material is not None:
        set_floor = tree.nodes.new("GeometryNodeSetMaterial")
        set_floor.location = (100, 100)
        set_floor.inputs["Material"].default_value = floor_material
        tree.links.new(cutter_mesh, set_floor.inputs["Geometry"])
        tree.links.new(extrude.outputs["Top"], set_floor.inputs["Selection"])
        cutter_mesh = set_floor.outputs["Geometry"]
    elif wall_material is not None:
        # Default floor inherits wall material if floor not specified
        set_floor = tree.nodes.new("GeometryNodeSetMaterial")
        set_floor.location = (100, 100)
        set_floor.inputs["Material"].default_value = wall_material
        tree.links.new(cutter_mesh, set_floor.inputs["Geometry"])
        tree.links.new(extrude.outputs["Top"], set_floor.inputs["Selection"])
        cutter_mesh = set_floor.outputs["Geometry"]

    # 7. Close the cutter manifold with flipped bottom cap
    flip = tree.nodes.new("GeometryNodeFlipFaces")
    flip.location = (-100, 0)
    tree.links.new(fill.outputs["Mesh"], flip.inputs["Mesh"])

    join = tree.nodes.new("GeometryNodeJoinGeometry")
    join.location = (300, 200)
    tree.links.new(cutter_mesh, join.inputs["Geometry"])
    tree.links.new(flip.outputs["Mesh"], join.inputs["Geometry"])

    merge = tree.nodes.new("GeometryNodeMergeByDistance")
    merge.location = (450, 200)
    merge.inputs["Distance"].default_value = 0.0001
    tree.links.new(join.outputs["Geometry"], merge.inputs["Geometry"])

    # 8. Transform cutter to target face
    # Target face -Y:
    # We rotate -90 around X so +Z extrusion points into +Y (into the monolith).
    # Y position: face_y - overshoot
    # Target bounds: target dimensions
    dims = target.dimensions
    face_y = -dims[1] / 2.0 if axis == "-Y" else (dims[1] / 2.0 if axis == "+Y" else 0.0)

    if space.upper() == "WORLD":
        loc_x = center_xz[0] - target.location.x
        loc_z = center_xz[1] - target.location.z
    else:
        loc_x = center_xz[0]
        loc_z = center_xz[1]

    trans = tree.nodes.new("GeometryNodeTransform")
    trans.location = (600, 200)
    if axis == "-Y":
        trans.inputs["Rotation"].default_value = (math.radians(90), 0, 0)
        trans.inputs["Translation"].default_value = (loc_x, face_y + depth, loc_z)
    else:
        trans.inputs["Rotation"].default_value = (math.radians(-90), 0, math.radians(180))
        trans.inputs["Translation"].default_value = (loc_x, face_y - depth, loc_z)
    tree.links.new(merge.outputs["Geometry"], trans.inputs["Geometry"])

    # 9. Mesh Boolean DIFFERENCE
    bool_node = tree.nodes.new("GeometryNodeMeshBoolean")
    bool_node.location = (800, 0)
    bool_node.operation = "DIFFERENCE"
    bool_node.solver = solver.upper()
    tree.links.new(in_node.outputs["Geometry"], bool_node.inputs["Mesh 1"])
    tree.links.new(trans.outputs["Geometry"], bool_node.inputs["Mesh 2"])

    out_node.location = (1000, 0)
    tree.links.new(bool_node.outputs["Mesh"], out_node.inputs["Geometry"])

    # Check if modifier already exists on target
    mod = target.modifiers.get(mod_name)
    if mod is None:
        mod = target.modifiers.new(mod_name, "NODES")
    mod.node_group = tree
    return mod
