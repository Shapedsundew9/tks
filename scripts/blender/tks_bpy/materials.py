"""Principled BSDF material helpers that are robust to Blender 4.x/5.x socket renames."""

from pathlib import Path

import bpy

# Legacy (<=3.x / 4.0) Principled names -> Blender 5.2 names.
SOCKET_ALIASES = {
    "Subsurface": "Subsurface Weight",
    "Specular": "Specular IOR Level",
    "Transmission": "Transmission Weight",
    "Clearcoat": "Coat Weight",
    "Clearcoat Roughness": "Coat Roughness",
    "Clearcoat Normal": "Coat Normal",
    "Sheen": "Sheen Weight",
    "Sheen Tint": "Sheen Tint",
    "Emission": "Emission Color",
    "Color": "Base Color",
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
