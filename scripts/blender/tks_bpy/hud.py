"""HUD Decal Baker: procedural 2D overlays rendered in isolated ortho scenes."""

import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector

from .typography import font as get_font


def _make_emissive_material(name, color=(0.7, 0.9, 1.0), glow=2.0, alpha=1.0):
    """Create an emissive shader with optional transparency."""
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    if hasattr(mat, "surface_render_method"):
        mat.surface_render_method = "BLENDED"
    if hasattr(mat, "blend_method"):
        mat.blend_method = "BLEND"

    nt = mat.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    rgb = color[:3] if len(color) >= 3 else (0.7, 0.9, 1.0)
    em.inputs["Color"].default_value = (*rgb, 1.0)
    em.inputs["Strength"].default_value = glow

    if alpha < 0.999:
        trans = nt.nodes.new("ShaderNodeBsdfTransparent")
        mix = nt.nodes.new("ShaderNodeMixShader")
        mix.inputs["Fac"].default_value = alpha
        nt.links.new(trans.outputs["BSDF"], mix.inputs[1])
        nt.links.new(em.outputs["Emission"], mix.inputs[2])
        nt.links.new(mix.outputs["Shader"], out.inputs["Surface"])
    else:
        nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
    return mat


def bake_overlay(spec, out_path, resolution=(2048, 2048)):
    """Render 2D geometry spec to a transparent RGBA PNG in an isolated ortho EEVEE scene."""
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    orig_scene = bpy.context.window.scene
    temp_scene = bpy.data.scenes.new("TKS_HUD_Bake")
    temp_scene.render.engine = "BLENDER_EEVEE"
    temp_scene.render.resolution_x, temp_scene.render.resolution_y = resolution
    temp_scene.render.film_transparent = True
    temp_scene.render.image_settings.file_format = "PNG"
    temp_scene.render.image_settings.color_mode = "RGBA"
    temp_scene.render.filepath = str(out_path)

    # Orthographic camera framing [0, 1] x [0, 1]
    cam_data = bpy.data.cameras.new("HUD_Camera")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = 1.0
    cam_obj = bpy.data.objects.new("HUD_Camera", cam_data)
    temp_scene.collection.objects.link(cam_obj)
    cam_obj.location = (0.5, 0.5, 10.0)
    temp_scene.camera = cam_obj

    created_objects = []
    created_materials = []
    created_curves = []
    created_meshes = []

    try:
        # 1. Filaments
        for i, fil in enumerate(spec.get("filaments", [])):
            pts = fil.get("points", [])
            if len(pts) < 2:
                continue
            width = fil.get("width", 0.0015)
            color = fil.get("color", (0.35, 0.85, 1.0))
            alpha = fil.get("alpha", 0.85)
            glow = fil.get("glow", 2.5)

            mat = _make_emissive_material(f"Filament_Mat_{i}", color, glow, alpha)
            created_materials.append(mat)

            cu = bpy.data.curves.new(f"Filament_{i}", "CURVE")
            created_curves.append(cu)
            cu.dimensions = "3D"
            cu.bevel_depth = width / 2.0
            cu.bevel_resolution = 2

            sp = cu.splines.new("POLY")
            sp.points.add(len(pts) - 1)
            for p_idx, pt in enumerate(pts):
                sp.points[p_idx].co = (pt[0], pt[1], 0.0, 1.0)

            ob = bpy.data.objects.new(f"Filament_{i}", cu)
            ob.data.materials.append(mat)
            temp_scene.collection.objects.link(ob)
            created_objects.append(ob)

        # 2. Constellation Nodes
        for i, nd in enumerate(spec.get("nodes", [])):
            xy = nd.get("xy", (0.5, 0.5))
            r = nd.get("radius", 0.005)
            color = nd.get("color", (0.7, 0.95, 1.0))
            alpha = nd.get("alpha", 1.0)
            glow = nd.get("glow", 4.0)

            mat = _make_emissive_material(f"Node_Mat_{i}", color, glow, alpha)
            created_materials.append(mat)

            # Center dot
            me = bpy.data.meshes.new(f"Node_Dot_{i}")
            created_meshes.append(me)
            bm = bmesh.new()
            bmesh.ops.create_circle(bm, cap_ends=True, radius=r, segments=24)
            bm.to_mesh(me)
            bm.free()

            dot_ob = bpy.data.objects.new(f"Node_Dot_{i}", me)
            dot_ob.location = (xy[0], xy[1], 0.001)
            dot_ob.data.materials.append(mat)
            temp_scene.collection.objects.link(dot_ob)
            created_objects.append(dot_ob)

            # Optional outer ring
            if nd.get("ring", False):
                ring_r = nd.get("ring_radius", r * 2.4)
                ring_w = nd.get("ring_width", r * 0.25)
                cu_ring = bpy.data.curves.new(f"Node_Ring_{i}", "CURVE")
                created_curves.append(cu_ring)
                cu_ring.dimensions = "3D"
                cu_ring.bevel_depth = ring_w / 2.0
                cu_ring.bevel_resolution = 2

                sp = cu_ring.splines.new("POLY")
                n_ring_pts = 32
                sp.points.add(n_ring_pts - 1)
                for p_idx in range(n_ring_pts):
                    theta = 2.0 * math.pi * p_idx / n_ring_pts
                    sp.points[p_idx].co = (ring_r * math.cos(theta), ring_r * math.sin(theta), 0.0, 1.0)
                sp.use_cyclic_u = True

                ring_ob = bpy.data.objects.new(f"Node_Ring_{i}", cu_ring)
                ring_ob.location = (xy[0], xy[1], 0.001)
                ring_ob.data.materials.append(mat)
                temp_scene.collection.objects.link(ring_ob)
                created_objects.append(ring_ob)

        # 3. Panels & Data blocks
        for i, pnl in enumerate(spec.get("panels", [])):
            text = pnl.get("text", "")
            f_name = pnl.get("font", "IBMPlexMono-Regular")
            size = pnl.get("size", 0.012)
            xy = pnl.get("xy", (0.1, 0.5))
            color = pnl.get("color", (0.5, 0.85, 1.0, 0.8))
            glow = pnl.get("glow", 2.0)
            align = pnl.get("align", "LEFT")

            c_rgb = color[:3] if len(color) >= 3 else (0.5, 0.85, 1.0)
            c_a = color[3] if len(color) >= 4 else 0.85
            mat = _make_emissive_material(f"Panel_Mat_{i}", c_rgb, glow, c_a)
            created_materials.append(mat)

            text_cu = bpy.data.curves.new(f"Panel_Text_{i}", "FONT")
            created_curves.append(text_cu)
            text_cu.body = text
            text_cu.size = size
            try:
                text_cu.font = get_font(f_name)
            except Exception:
                pass
            text_cu.align_x = align.upper()
            text_cu.space_line = pnl.get("line_spacing", 1.2)

            t_ob = bpy.data.objects.new(f"Panel_Text_{i}", text_cu)
            t_ob.location = (xy[0], xy[1], 0.002)
            t_ob.data.materials.append(mat)
            temp_scene.collection.objects.link(t_ob)
            created_objects.append(t_ob)

        # 4. Etched Subtext
        for i, st in enumerate(spec.get("subtext", [])):
            text = st.get("text", "")
            f_name = st.get("font", "ShareTechMono-Regular")
            size = st.get("size", 0.016)
            xy = st.get("xy", (0.5, 0.5))
            color = st.get("color", (0.8, 0.92, 1.0, 0.9))
            glow = st.get("glow", 2.5)
            align = st.get("align", "CENTER")

            c_rgb = color[:3] if len(color) >= 3 else (0.8, 0.92, 1.0)
            c_a = color[3] if len(color) >= 4 else 0.9
            mat = _make_emissive_material(f"Subtext_Mat_{i}", c_rgb, glow, c_a)
            created_materials.append(mat)

            text_cu = bpy.data.curves.new(f"Subtext_{i}", "FONT")
            created_curves.append(text_cu)
            text_cu.body = text
            text_cu.size = size
            try:
                text_cu.font = get_font(f_name)
            except Exception:
                pass
            text_cu.align_x = align.upper()

            t_ob = bpy.data.objects.new(f"Subtext_{i}", text_cu)
            t_ob.location = (xy[0], xy[1], 0.002)
            t_ob.data.materials.append(mat)
            temp_scene.collection.objects.link(t_ob)
            created_objects.append(t_ob)

        # Render in temp scene
        bpy.context.window.scene = temp_scene
        bpy.ops.render.render(write_still=True, scene=temp_scene.name)

    finally:
        # Completely restore original scene and clean up temp assets
        bpy.context.window.scene = orig_scene
        for o in created_objects:
            if o.name in bpy.data.objects:
                bpy.data.objects.remove(o, do_unlink=True)
        if cam_obj.name in bpy.data.objects:
            bpy.data.objects.remove(cam_obj, do_unlink=True)
        for m in created_materials:
            if m.name in bpy.data.materials:
                bpy.data.materials.remove(m)
        for c in created_curves:
            if c.name in bpy.data.curves:
                bpy.data.curves.remove(c)
        for me in created_meshes:
            if me.name in bpy.data.meshes:
                bpy.data.meshes.remove(me)
        if cam_data.name in bpy.data.cameras:
            bpy.data.cameras.remove(cam_data)
        if temp_scene.name in bpy.data.scenes:
            bpy.data.scenes.remove(temp_scene)

    return out_path


def substrate_hud_spec():
    """Default cybernetic HUD spec matching the reference key art overlay."""
    # Monolith V-coordinates:
    # "THE" is at v ~ 0.781
    # "KNOWLEDGE" is at v ~ 0.728
    # "SUBSTRATE" is at v ~ 0.672
    spec = {
        "filaments": [
            # Top vertical power trunks descending into header bracket
            {"points": [(0.50, 0.96), (0.50, 0.85)], "width": 0.0020, "color": (0.35, 0.85, 1.0), "glow": 3.0, "alpha": 0.90},
            {"points": [(0.42, 0.95), (0.42, 0.87), (0.46, 0.85)], "width": 0.0012, "color": (0.25, 0.65, 0.95), "glow": 2.0, "alpha": 0.75},
            {"points": [(0.58, 0.95), (0.58, 0.87), (0.54, 0.85)], "width": 0.0012, "color": (0.25, 0.65, 0.95), "glow": 2.0, "alpha": 0.75},

            # Upper frame bracket framing above "THE" (v=0.84)
            {"points": [(0.06, 0.84), (0.94, 0.84)], "width": 0.0014, "color": (0.35, 0.80, 1.0), "glow": 2.2, "alpha": 0.80},
            {"points": [(0.06, 0.84), (0.06, 0.63), (0.12, 0.63)], "width": 0.0014, "color": (0.35, 0.80, 1.0), "glow": 2.2, "alpha": 0.80},
            {"points": [(0.94, 0.84), (0.94, 0.63), (0.88, 0.63)], "width": 0.0014, "color": (0.35, 0.80, 1.0), "glow": 2.2, "alpha": 0.80},

            # Constellation bus lines connecting flank nodes
            {"points": [(0.08, 0.80), (0.14, 0.76), (0.14, 0.70), (0.08, 0.66)], "width": 0.0010, "color": (0.45, 0.90, 1.0), "glow": 2.5, "alpha": 0.75},
            {"points": [(0.92, 0.80), (0.86, 0.76), (0.86, 0.70), (0.92, 0.66)], "width": 0.0010, "color": (0.45, 0.90, 1.0), "glow": 2.5, "alpha": 0.75},

            # Thin horizontal telemetry tick dividers
            {"points": [(0.18, 0.83), (0.28, 0.83)], "width": 0.0018, "color": (0.85, 0.95, 1.0), "glow": 3.5, "alpha": 0.95},
            {"points": [(0.72, 0.83), (0.82, 0.83)], "width": 0.0018, "color": (0.85, 0.95, 1.0), "glow": 3.5, "alpha": 0.95},
            {"points": [(0.30, 0.63), (0.70, 0.63)], "width": 0.0012, "color": (0.30, 0.70, 0.95), "glow": 2.0, "alpha": 0.70},

            # Lower circuit traces fading downward towards bedrock
            {"points": [(0.12, 0.63), (0.12, 0.46), (0.18, 0.40)], "width": 0.0010, "color": (0.20, 0.55, 0.85), "glow": 1.8, "alpha": 0.60},
            {"points": [(0.88, 0.63), (0.88, 0.46), (0.82, 0.40)], "width": 0.0010, "color": (0.20, 0.55, 0.85), "glow": 1.8, "alpha": 0.60},
            {"points": [(0.50, 0.63), (0.50, 0.42)], "width": 0.0012, "color": (0.25, 0.60, 0.90), "glow": 2.0, "alpha": 0.65},
        ],
        "nodes": [
            # Top header junctions
            {"xy": (0.50, 0.96), "radius": 0.0055, "ring": True, "glow": 5.0, "color": (0.85, 0.98, 1.0)},
            {"xy": (0.50, 0.85), "radius": 0.0040, "ring": True, "glow": 4.5, "color": (0.75, 0.95, 1.0)},
            {"xy": (0.42, 0.95), "radius": 0.0035, "ring": False, "glow": 3.5, "color": (0.40, 0.85, 1.0)},
            {"xy": (0.58, 0.95), "radius": 0.0035, "ring": False, "glow": 3.5, "color": (0.40, 0.85, 1.0)},

            # Flank corner nodes
            {"xy": (0.06, 0.84), "radius": 0.0045, "ring": True, "glow": 4.0, "color": (0.80, 0.95, 1.0)},
            {"xy": (0.94, 0.84), "radius": 0.0045, "ring": True, "glow": 4.0, "color": (0.80, 0.95, 1.0)},
            {"xy": (0.14, 0.76), "radius": 0.0035, "ring": False, "glow": 3.0, "color": (0.40, 0.85, 1.0)},
            {"xy": (0.86, 0.76), "radius": 0.0035, "ring": False, "glow": 3.0, "color": (0.40, 0.85, 1.0)},
            {"xy": (0.14, 0.70), "radius": 0.0035, "ring": False, "glow": 3.0, "color": (0.40, 0.85, 1.0)},
            {"xy": (0.86, 0.70), "radius": 0.0035, "ring": False, "glow": 3.0, "color": (0.40, 0.85, 1.0)},
            {"xy": (0.06, 0.63), "radius": 0.0045, "ring": True, "glow": 4.0, "color": (0.80, 0.95, 1.0)},
            {"xy": (0.94, 0.63), "radius": 0.0045, "ring": True, "glow": 4.0, "color": (0.80, 0.95, 1.0)},

            # Lower bus taps
            {"xy": (0.18, 0.40), "radius": 0.0035, "ring": True, "glow": 3.5, "color": (0.35, 0.75, 1.0)},
            {"xy": (0.82, 0.40), "radius": 0.0035, "ring": True, "glow": 3.5, "color": (0.35, 0.75, 1.0)},
            {"xy": (0.50, 0.42), "radius": 0.0040, "ring": True, "glow": 4.0, "color": (0.50, 0.85, 1.0)},
        ],
        "panels": [
            # Left code/telemetry stack flanking below SUBSTRATE
            {
                "text": "SYS.SUBSTRATE // CORE_V2\nNODE_ID: 0x8F94 // LAT: 0.00\nMEM_BUS: RUNNING [SYNTH: OK]\nENTROPY_FLUX: 0.0428 uJ\nTENSOR_LATTICE: 2048-DIM",
                "font": "IBMPlexMono-Regular",
                "size": 0.0080,
                "xy": (0.08, 0.58),
                "color": (0.45, 0.80, 1.0, 0.75),
                "glow": 1.8,
                "align": "LEFT",
                "line_spacing": 1.25,
            },
            # Right telemetry stack flanking below SUBSTRATE
            {
                "text": "STATUS: COHERENT\nBAND: 94.20 GHz\nQ-BIT ROT: +0.024\nPHASE: LOCKED\nEPOCH: 2026.10",
                "font": "IBMPlexMono-Regular",
                "size": 0.0080,
                "xy": (0.92, 0.58),
                "color": (0.45, 0.80, 1.0, 0.75),
                "glow": 1.8,
                "align": "RIGHT",
                "line_spacing": 1.25,
            },
        ],
        "subtext": [
            # Header telemetry above "THE"
            {
                "text": ":: ARTIFACT SPECIFICATION VER 2.0 // DIRECTIVE CLASSIFIED ::",
                "font": "ShareTechMono-Regular",
                "size": 0.0095,
                "xy": (0.50, 0.825),
                "color": (0.75, 0.90, 1.0, 0.85),
                "glow": 2.2,
                "align": "CENTER",
            },
            # Sub-bar telemetry below "SUBSTRATE"
            {
                "text": "PRIMARY COGNITIVE MATRIX // NEURAL ANCHOR SEQUENCE",
                "font": "ShareTechMono-Regular",
                "size": 0.0090,
                "xy": (0.50, 0.615),
                "color": (0.65, 0.85, 1.0, 0.80),
                "glow": 2.0,
                "align": "CENTER",
            },
            # Lower version watermark
            {
                "text": "PROJECT TKS / KNOWLEDGE BASE INFRASTRUCTURE",
                "font": "ShareTechMono-Regular",
                "size": 0.0085,
                "xy": (0.50, 0.39),
                "color": (0.35, 0.65, 0.90, 0.65),
                "glow": 1.5,
                "align": "CENTER",
            },
        ],
    }
    return spec
