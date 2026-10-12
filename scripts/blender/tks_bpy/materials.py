"""Principled BSDF material helpers that are robust to Blender 4.x/5.x socket renames."""

from pathlib import Path

import bpy

# Legacy (<=3.x / 4.0) Principled names & pythonic snake_case -> Blender 5.2 names.
SOCKET_ALIASES = {
    "Subsurface": "Subsurface Weight",
    "subsurface": "Subsurface Weight",
    "Specular": "Specular IOR Level",
    "specular": "Specular IOR Level",
    "Transmission": "Transmission Weight",
    "transmission": "Transmission Weight",
    "Clearcoat": "Coat Weight",
    "clearcoat": "Coat Weight",
    "Clearcoat Roughness": "Coat Roughness",
    "Clearcoat Normal": "Coat Normal",
    "Sheen": "Sheen Weight",
    "sheen": "Sheen Weight",
    "Sheen Tint": "Sheen Tint",
    "Emission": "Emission Color",
    "emission": "Emission Color",
    "Color": "Base Color",
    "base_color": "Base Color",
    "roughness": "Roughness",
    "metallic": "Metallic",
    "ior": "IOR",
    "alpha": "Alpha",
    "normal": "Normal",
}

PRESETS = {
    "plastic": {"Base Color": (0.8, 0.1, 0.1), "Roughness": 0.35},
    "rubber": {"Base Color": (0.02, 0.02, 0.02), "Roughness": 0.85},
    "ceramic": {"Base Color": (0.9, 0.9, 0.88), "Roughness": 0.15, "Coat Weight": 0.6},
    "metal_steel": {"Base Color": (0.56, 0.57, 0.58), "Metallic": 1.0, "Roughness": 0.3},
    "metal_gold": {"Base Color": (1.0, 0.77, 0.34), "Metallic": 1.0, "Roughness": 0.2},
    "metal_copper": {"Base Color": (0.95, 0.64, 0.54), "Metallic": 1.0, "Roughness": 0.25},
    "glass": {"Base Color": (1, 1, 1), "Roughness": 0.0, "Transmission Weight": 1.0, "IOR": 1.45},
    "water": {"Base Color": (0.8, 0.9, 1.0), "Roughness": 0.02, "Transmission Weight": 1.0, "IOR": 1.333},
    "wood": {"Base Color": (0.35, 0.2, 0.1), "Roughness": 0.6},
    "fabric": {"Base Color": (0.3, 0.3, 0.35), "Roughness": 0.9, "Sheen Weight": 0.5},
    "skin": {"Base Color": (0.8, 0.6, 0.5), "Roughness": 0.45, "Subsurface Weight": 0.3},
    "foliage": {"Base Color": (0.1, 0.3, 0.05), "Roughness": 0.5, "Subsurface Weight": 0.15},
}

# Texture map suffixes used by ambientCG / Poly Haven / most PBR libraries.
_MAP_PATTERNS = {
    "color": ("_color", "_basecolor", "_base_color", "_diff", "_diffuse", "_albedo", "_col"),
    "roughness": ("_roughness", "_rough"),
    "metalness": ("_metalness", "_metallic", "_metal"),
    "normal_gl": ("_normalgl", "_nor_gl", "_normal_gl"),
    "normal_dx": ("_normaldx", "_nor_dx", "_normal_dx"),
    "normal": ("_normal", "_nrm", "_nor"),
    "ao": ("_ambientocclusion", "_ao"),
    "displacement": ("_displacement", "_disp", "_height"),
    "opacity": ("_opacity", "_alpha", "_mask"),
    "emission": ("_emission", "_emissive"),
}
_IMAGE_EXTS = {".png", ".jpg", ".jpeg", ".exr", ".tif", ".tiff"}


def _bsdf(mat):
    for node in mat.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            return node
    raise ValueError(f"material {mat.name!r} has no Principled BSDF")


def _resolve(node, name):
    if name in node.inputs:
        return node.inputs[name]
    alias = SOCKET_ALIASES.get(name)
    if alias and alias in node.inputs:
        return node.inputs[alias]
    valid = ", ".join(s.name for s in node.inputs if s.enabled)
    raise KeyError(f"no input {name!r} on {node.bl_idname}; valid: {valid}")


def set_inputs(node, **values):
    """Set node inputs by (case-sensitive) socket name; accepts legacy names and RGB tuples."""
    for key, value in values.items():
        sock = _resolve(node, key)
        if sock.type == "RGBA" and len(value) == 3:
            value = (*value, 1.0)
        sock.default_value = value
    return node


def principled(name, preset=None, reuse=True, **inputs):
    """Create (or reuse) a Principled BSDF material.

    Only materials made by tks_bpy are reused; a user material with the same name is left alone
    and a new one (``name.001``) is created instead.

    ``inputs`` use Blender 5.2 socket names; pass keys with spaces via dict unpacking,
    e.g. ``principled("Mug", **{"Base Color": (1, 1, 1), "Coat Weight": 0.5})``.
    """
    mat = bpy.data.materials.get(name) if reuse else None
    if mat is None or not mat.get("tks_managed"):
        mat = bpy.data.materials.new(name)
        mat["tks_managed"] = True
    mat.use_nodes = True
    bsdf = _bsdf(mat)
    values = dict(PRESETS[preset]) if preset else {}
    values.update(inputs)
    set_inputs(bsdf, **values)
    return mat


def emission(name, color=(1.0, 0.6, 0.3), strength=10.0):
    """Emissive material (lamps, screens, fire glow)."""
    return principled(name, **{"Base Color": (0, 0, 0), "Emission Color": color, "Emission Strength": strength})


def assign(obj, mat, replace=True):
    """Assign ``mat`` to ``obj`` (and mesh children if the object is an empty)."""
    targets = [obj] if obj.type != "EMPTY" else [c for c in obj.children_recursive if hasattr(c.data, "materials")]
    for target in targets:
        if not hasattr(target.data, "materials"):
            continue
        if replace:
            target.data.materials.clear()
        target.data.materials.append(mat)
    return mat


def find_maps(folder):
    """Classify PBR texture files in ``folder`` by map type."""
    found = {}
    files = sorted(p for p in Path(folder).rglob("*") if p.suffix.lower() in _IMAGE_EXTS)
    for path in files:
        stem = path.stem.lower().replace("-", "_")
        for kind, suffixes in _MAP_PATTERNS.items():
            if kind in found:
                continue
            if any(stem.endswith(s) or (s + "_") in stem for s in suffixes):
                found[kind] = path
                break
    if "normal" not in found:
        found["normal"] = found.get("normal_gl") or found.get("normal_dx")
    if found.get("normal") is None:
        found.pop("normal", None)
    return found


def pbr_from_folder(name, folder, scale=1.0, displacement=0.0):
    """Build a PBR material from an ambientCG/Poly Haven texture folder.

    Prefers OpenGL normals (Blender convention). Data maps load as Non-Color.
    ``displacement`` > 0 wires a Displacement node (Cycles: set material
    displacement_method to 'BOTH' and use a subdivided mesh).
    """
    maps = find_maps(folder)
    if "color" not in maps:
        raise FileNotFoundError(f"no base colour map found in {folder}: {sorted(maps)}")
    mat = principled(name)
    nt = mat.node_tree
    bsdf = _bsdf(mat)
    out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL")
    coord = nt.nodes.new("ShaderNodeTexCoord")
    coord.location = (-1100, 0)
    mapping = nt.nodes.new("ShaderNodeMapping")
    mapping.location = (-900, 0)
    mapping.inputs["Scale"].default_value = (scale, scale, scale)
    nt.links.new(coord.outputs["UV"], mapping.inputs["Vector"])

    def tex(kind, y, non_color=True):
        node = nt.nodes.new("ShaderNodeTexImage")
        node.location = (-600, y)
        node.image = bpy.data.images.load(str(maps[kind]), check_existing=True)
        if non_color:
            node.image.colorspace_settings.name = "Non-Color"
        nt.links.new(mapping.outputs["Vector"], node.inputs["Vector"])
        return node

    color = tex("color", 300, non_color=False)
    nt.links.new(color.outputs["Color"], bsdf.inputs["Base Color"])
    if "roughness" in maps:
        nt.links.new(tex("roughness", 0).outputs["Color"], bsdf.inputs["Roughness"])
    if "metalness" in maps:
        nt.links.new(tex("metalness", -300).outputs["Color"], bsdf.inputs["Metallic"])
    if "opacity" in maps:
        nt.links.new(tex("opacity", -600).outputs["Color"], bsdf.inputs["Alpha"])
    if "normal" in maps:
        nmap = nt.nodes.new("ShaderNodeNormalMap")
        nmap.location = (-250, -500)
        n = tex("normal", -900)
        nt.links.new(n.outputs["Color"], nmap.inputs["Color"])
        nt.links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
        if maps["normal"] == maps.get("normal_dx") and "normal_gl" not in maps:
            # DirectX normals: invert green channel.
            sep = nt.nodes.new("ShaderNodeSeparateColor")
            comb = nt.nodes.new("ShaderNodeCombineColor")
            inv = nt.nodes.new("ShaderNodeMath")
            inv.operation = "SUBTRACT"
            inv.inputs[0].default_value = 1.0
            nt.links.new(n.outputs["Color"], sep.inputs["Color"])
            nt.links.new(sep.outputs["Red"], comb.inputs["Red"])
            nt.links.new(sep.outputs["Green"], inv.inputs[1])
            nt.links.new(inv.outputs["Value"], comb.inputs["Green"])
            nt.links.new(sep.outputs["Blue"], comb.inputs["Blue"])
            nt.links.new(comb.outputs["Color"], nmap.inputs["Color"])
    if displacement > 0 and "displacement" in maps:
        disp = nt.nodes.new("ShaderNodeDisplacement")
        disp.location = (100, -500)
        disp.inputs["Scale"].default_value = displacement
        nt.links.new(tex("displacement", -1200).outputs["Color"], disp.inputs["Height"])
        nt.links.new(disp.outputs["Displacement"], out.inputs["Displacement"])
        mat.displacement_method = "BOTH"
    mat["tks_source"] = str(folder)
    return mat


METAL_F0 = {
    "steel": (0.56, 0.57, 0.58),
    "aluminium": (0.91, 0.92, 0.92),
    "titanium": (0.54, 0.50, 0.45),
}


def brushed_metal(name="BrushedSteel", metal="steel", roughness=(0.22, 0.45), anisotropy=0.8,
                  brush="horizontal", grain_scale=700.0, grain_bump=0.015, smudge_scale=1.2,
                  edge_bevel=0.0015):
    """Physically based brushed metal with anisotropy, radial tangents, micro-grain bump, and bevel highlights."""
    base_col = METAL_F0.get(metal, METAL_F0["steel"])
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    nt.nodes.clear()

    out = nt.nodes.new("ShaderNodeOutputMaterial")
    out.location = (600, 0)
    bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (300, 0)
    bsdf.inputs["Base Color"].default_value = (*base_col[:3], 1.0)
    bsdf.inputs["Metallic"].default_value = 1.0
    bsdf.inputs["Anisotropic"].default_value = anisotropy
    if brush == "vertical":
        bsdf.inputs["Anisotropic Rotation"].default_value = 0.25

    # Radial Z tangent gives horizontal brushing on vertical faces without UV unwrapping
    tangent = nt.nodes.new("ShaderNodeTangent")
    tangent.location = (50, -250)
    tangent.direction_type = "RADIAL"
    tangent.axis = "Z"
    nt.links.new(tangent.outputs["Tangent"], bsdf.inputs["Tangent"])

    coord = nt.nodes.new("ShaderNodeTexCoord")
    coord.location = (-700, 0)

    # Micro grain noise (stretched along Z for horizontal lines on vertical faces)
    map_grain = nt.nodes.new("ShaderNodeMapping")
    map_grain.location = (-500, 200)
    map_grain.inputs["Scale"].default_value = (4.0, 4.0, grain_scale)
    nt.links.new(coord.outputs["Object"], map_grain.inputs["Vector"])

    noise_grain = nt.nodes.new("ShaderNodeTexNoise")
    noise_grain.location = (-300, 200)
    noise_grain.inputs["Scale"].default_value = 1.0
    noise_grain.inputs["Detail"].default_value = 4.0
    nt.links.new(map_grain.outputs["Vector"], noise_grain.inputs["Vector"])

    # Low frequency smudges
    noise_smudge = nt.nodes.new("ShaderNodeTexNoise")
    noise_smudge.location = (-300, -50)
    noise_smudge.inputs["Scale"].default_value = smudge_scale
    noise_smudge.inputs["Detail"].default_value = 3.0
    nt.links.new(coord.outputs["Object"], noise_smudge.inputs["Vector"])

    # Mix grain + smudges -> MapRange -> Roughness
    mix_rough = nt.nodes.new("ShaderNodeMix")
    mix_rough.location = (-100, 100)
    mix_rough.data_type = "FLOAT"
    mix_rough.inputs[0].default_value = 0.35
    nt.links.new(noise_grain.outputs["Factor"], mix_rough.inputs[2])
    nt.links.new(noise_smudge.outputs["Factor"], mix_rough.inputs[3])

    map_range = nt.nodes.new("ShaderNodeMapRange")
    map_range.location = (100, 100)
    map_range.inputs["From Min"].default_value = 0.2
    map_range.inputs["From Max"].default_value = 0.8
    map_range.inputs["To Min"].default_value = roughness[0]
    map_range.inputs["To Max"].default_value = roughness[1]
    nt.links.new(mix_rough.outputs[0], map_range.inputs["Value"])
    nt.links.new(map_range.outputs["Result"], bsdf.inputs["Roughness"])

    # Bump for micro grain
    bump = nt.nodes.new("ShaderNodeBump")
    bump.location = (100, -150)
    bump.inputs["Strength"].default_value = grain_bump
    nt.links.new(noise_grain.outputs["Factor"], bump.inputs["Height"])

    # ShaderNodeBevel for edge glints
    if edge_bevel > 0:
        bevel = nt.nodes.new("ShaderNodeBevel")
        bevel.location = (-100, -250)
        bevel.inputs["Radius"].default_value = edge_bevel
        bevel.samples = 8
        nt.links.new(bevel.outputs["Normal"], bump.inputs["Normal"])

    nt.links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
    nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
    return mat


def restyle(mat, saturation=0.15, value=0.35, roughness_range=(0.35, 0.6), normal_strength=1.5):
    """Re-grade an existing PBR material (e.g. Poly Haven scan -> dark glossy slate)."""
    nt = mat.node_tree
    bsdf = _bsdf(mat)

    # Insert Hue/Saturation node on Base Color if connected
    bc_link = next((l for l in nt.links if l.to_socket == bsdf.inputs["Base Color"]), None)
    if bc_link is not None:
        src = bc_link.from_socket
        nt.links.remove(bc_link)
        hsv = nt.nodes.new("ShaderNodeHueSaturation")
        hsv.location = (-200, 200)
        hsv.inputs["Saturation"].default_value = saturation
        hsv.inputs["Value"].default_value = value
        nt.links.new(src, hsv.inputs["Color"])
        nt.links.new(hsv.outputs["Color"], bsdf.inputs["Base Color"])
    else:
        old_col = bsdf.inputs["Base Color"].default_value
        bsdf.inputs["Base Color"].default_value = (old_col[0] * value, old_col[1] * value, old_col[2] * value, 1.0)

    # Remap roughness if connected
    r_link = next((l for l in nt.links if l.to_socket == bsdf.inputs["Roughness"]), None)
    if r_link is not None:
        src_r = r_link.from_socket
        nt.links.remove(r_link)
        mr = nt.nodes.new("ShaderNodeMapRange")
        mr.location = (-200, 50)
        mr.inputs["To Min"].default_value = roughness_range[0]
        mr.inputs["To Max"].default_value = roughness_range[1]
        nt.links.new(src_r, mr.inputs["Value"])
        nt.links.new(mr.outputs["Result"], bsdf.inputs["Roughness"])
    else:
        bsdf.inputs["Roughness"].default_value = (roughness_range[0] + roughness_range[1]) / 2.0

    # Boost Normal Map strength if present
    normal_node = next((n for n in nt.nodes if n.type == "NORMAL_MAP"), None)
    if normal_node is not None:
        normal_node.inputs["Strength"].default_value = normal_strength

    return mat


def add_decal(mat, emission_png=None, etch_png=None, size_m=(1.0, 3.2), center=(0, 0), axis="-Y",
              emission_strength=3.0, etch_color=(0.7, 0.72, 0.75), etch_roughness=0.6, facing_only=True):
    """Project planar decals (baked HUD emission and/or etched subtext) in object space."""
    nt = mat.node_tree
    bsdf = _bsdf(mat)
    coord = next((n for n in nt.nodes if n.type == "TEX_COORD"), None)
    if coord is None:
        coord = nt.nodes.new("ShaderNodeTexCoord")
        coord.location = (-1200, 300)

    # Map Object X, Z to UV (0 to 1)
    sep_obj = nt.nodes.new("ShaderNodeSeparateXYZ")
    sep_obj.location = (-1000, 200)
    nt.links.new(coord.outputs["Object"], sep_obj.inputs["Vector"])

    # u = (X - center[0]) / size_m[0] + 0.5
    sub_x = nt.nodes.new("ShaderNodeMath")
    sub_x.operation = "SUBTRACT"
    sub_x.inputs[1].default_value = center[0]
    nt.links.new(sep_obj.outputs["X"], sub_x.inputs[0])

    div_x = nt.nodes.new("ShaderNodeMath")
    div_x.operation = "DIVIDE"
    div_x.inputs[1].default_value = max(1e-4, size_m[0])
    nt.links.new(sub_x.outputs["Value"], div_x.inputs[0])

    add_u = nt.nodes.new("ShaderNodeMath")
    add_u.operation = "ADD"
    add_u.inputs[1].default_value = 0.5
    nt.links.new(div_x.outputs["Value"], add_u.inputs[0])

    # v = (Z - center[1]) / size_m[1] + 0.5
    sub_z = nt.nodes.new("ShaderNodeMath")
    sub_z.operation = "SUBTRACT"
    sub_z.inputs[1].default_value = center[1]
    nt.links.new(sep_obj.outputs["Z"], sub_z.inputs[0])

    div_z = nt.nodes.new("ShaderNodeMath")
    div_z.operation = "DIVIDE"
    div_z.inputs[1].default_value = max(1e-4, size_m[1])
    nt.links.new(sub_z.outputs["Value"], div_z.inputs[0])

    add_v = nt.nodes.new("ShaderNodeMath")
    add_v.operation = "ADD"
    add_v.inputs[1].default_value = 0.5
    nt.links.new(div_z.outputs["Value"], add_v.inputs[0])

    comb_uv = nt.nodes.new("ShaderNodeCombineXYZ")
    comb_uv.location = (-750, 200)
    nt.links.new(add_u.outputs["Value"], comb_uv.inputs["X"])
    nt.links.new(add_v.outputs["Value"], comb_uv.inputs["Y"])

    # Facing mask: Normal.Y < -0.5 for -Y face
    geom = nt.nodes.new("ShaderNodeNewGeometry")
    geom.location = (-750, -200)
    sep_norm = nt.nodes.new("ShaderNodeSeparateXYZ")
    sep_norm.location = (-600, -200)
    nt.links.new(geom.outputs["Normal"], sep_norm.inputs["Vector"])

    math_face = nt.nodes.new("ShaderNodeMath")
    math_face.location = (-450, -200)
    math_face.operation = "LESS_THAN"
    math_face.inputs[1].default_value = -0.5
    nt.links.new(sep_norm.outputs["Y"], math_face.inputs[0])

    if emission_png and Path(emission_png).exists():
        tex_em = nt.nodes.new("ShaderNodeTexImage")
        tex_em.location = (-500, 400)
        tex_em.image = bpy.data.images.load(str(emission_png), check_existing=True)
        tex_em.extension = "CLIP"
        nt.links.new(comb_uv.outputs["Vector"], tex_em.inputs["Vector"])

        mul_em = nt.nodes.new("ShaderNodeMath")
        mul_em.location = (-250, 400)
        mul_em.operation = "MULTIPLY"
        nt.links.new(tex_em.outputs["Alpha"], mul_em.inputs[0])
        nt.links.new(math_face.outputs["Value"], mul_em.inputs[1])

        mul_str = nt.nodes.new("ShaderNodeMath")
        mul_str.location = (-100, 400)
        mul_str.operation = "MULTIPLY"
        mul_str.inputs[1].default_value = emission_strength
        nt.links.new(mul_em.outputs["Value"], mul_str.inputs[0])

        nt.links.new(tex_em.outputs["Color"], bsdf.inputs["Emission Color"])
        nt.links.new(mul_str.outputs["Value"], bsdf.inputs["Emission Strength"])

    if etch_png and Path(etch_png).exists():
        tex_etch = nt.nodes.new("ShaderNodeTexImage")
        tex_etch.location = (-500, 100)
        tex_etch.image = bpy.data.images.load(str(etch_png), check_existing=True)
        tex_etch.extension = "CLIP"
        nt.links.new(comb_uv.outputs["Vector"], tex_etch.inputs["Vector"])

        mul_etch = nt.nodes.new("ShaderNodeMath")
        mul_etch.location = (-250, 100)
        mul_etch.operation = "MULTIPLY"
        nt.links.new(tex_etch.outputs["Alpha"], mul_etch.inputs[0])
        nt.links.new(math_face.outputs["Value"], mul_etch.inputs[1])

        mix_col = nt.nodes.new("ShaderNodeMix")
        mix_col.location = (-50, 50)
        mix_col.data_type = "RGBA"
        nt.links.new(mul_etch.outputs["Value"], mix_col.inputs[0])

        bc_link = next((l for l in nt.links if l.to_socket == bsdf.inputs["Base Color"]), None)
        if bc_link:
            nt.links.new(bc_link.from_socket, mix_col.inputs[6])
            nt.links.remove(bc_link)
        else:
            mix_col.inputs[6].default_value = bsdf.inputs["Base Color"].default_value
        mix_col.inputs[7].default_value = (*etch_color[:3], 1.0)
        nt.links.new(mix_col.outputs[2], bsdf.inputs["Base Color"])

        mix_r = nt.nodes.new("ShaderNodeMix")
        mix_r.location = (-50, -100)
        mix_r.data_type = "FLOAT"
        nt.links.new(mul_etch.outputs["Value"], mix_r.inputs[0])
        r_link = next((l for l in nt.links if l.to_socket == bsdf.inputs["Roughness"]), None)
        if r_link:
            nt.links.new(r_link.from_socket, mix_r.inputs[2])
            nt.links.remove(r_link)
        else:
            mix_r.inputs[2].default_value = bsdf.inputs["Roughness"].default_value
        mix_r.inputs[3].default_value = etch_roughness
        nt.links.new(mix_r.outputs[0], bsdf.inputs["Roughness"])

    return mat
