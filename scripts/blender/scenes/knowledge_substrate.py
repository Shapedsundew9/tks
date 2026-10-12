"""Scene-as-code: The Knowledge Substrate v2.

Idempotent build script with modular stages. Iterations tune PARAMS and rebuild.
"""

import math
from pathlib import Path

import bpy
from mathutils import Vector

import tks_bpy as T

PARAMS = {
    "scene": "knowledge_substrate_v2",
    "reference": "/workspaces/tks/scratchpad/Gemini_Generated_Image_vlkaqsvlkaqsvlka.jpeg",
    "monolith": {
        "size": (1.0, 0.35, 3.2),
        "sink": 0.45,  # monolith sunk into bedrock by 0.45m
        "bevel": 0.006,
    },
    "rock": {
        "source": "polyhaven:boulder_01",
        "count": 8,
        "ring": (0.55, 1.4),
    },
    "shots": {
        "hero": {
            "location": (-0.95, -4.1, 0.72),
            "target": (0.10, 0.0, 1.48),
            "lens": 62,
            "fstop": 2.8,
        },
        "three_quarter": {
            "location": (2.4, -3.2, 1.4),
            "target": (0.0, 0.0, 1.5),
            "lens": 50,
            "fstop": 3.5,
        },
        "profile": {
            "location": (3.6, 0.0, 1.5),
            "target": (0.0, 0.0, 1.5),
            "lens": 50,
            "fstop": 4.0,
        },
        "low_close": {
            "location": (-0.6, -2.6, 0.9),
            "target": (0.0, -0.1, 1.7),
            "lens": 42,
            "fstop": 2.8,
        },
    },
}

ALL_STAGES = ["setup", "world", "monolith", "engrave", "hud", "rock", "lighting", "fx", "post", "shots"]


def stage_setup(params):
    sc = T.scene.new_scene(params["scene"], engine="CYCLES", resolution=(1920, 1080))
    sc.cycles.preview_samples = 16
    return {"scene": sc.name}


def stage_world(params):
    hdri_path = Path("/workspaces/tks/scenes/assets/hdris/monochrome_studio_03_2k.hdr")
    w = T.world.studio_void(
        top=(0.004, 0.006, 0.010),
        bottom=(0.001, 0.001, 0.002),
        hdri=hdri_path if hdri_path.exists() else None,
        hdri_strength=0.12,
        hdri_rotation_deg=45.0,
    )
    return {"world": w.name}


def stage_monolith(params):
    m_p = params["monolith"]
    w, d, h = m_p["size"]
    z_centre = (h / 2.0) - m_p["sink"]
    obj = bpy.data.objects.get("Monolith")
    if obj is not None:
        bpy.data.objects.remove(obj, do_unlink=True)

    obj = T.scene.box("Monolith", size=(w, d, h), location=(0, 0, z_centre), collection="Props", on_ground=False)

    # Bevel modifier for rounded edges
    bevel_w = m_p.get("bevel", 0.006)
    if bevel_w > 0:
        bev = obj.modifiers.new("Bevel", "BEVEL")
        bev.width = bevel_w
        bev.segments = 4
        bev.limit_method = "ANGLE"

    # Brushed steel material
    mat = T.materials.brushed_metal(
        "Monolith_Steel",
        metal="steel",
        roughness=(0.20, 0.42),
        anisotropy=0.85,
        brush="horizontal",
        grain_scale=750.0,
        grain_bump=0.012,
        smudge_scale=1.5,
        edge_bevel=0.002,
    )
    obj.data.materials.clear()
    obj.data.materials.append(mat)
    return {"monolith": obj.name, "location": list(obj.location), "dims": [w, d, h], "material": mat.name}


def stage_engrave(params):
    monolith = bpy.data.objects.get("Monolith")
    if monolith is None:
        raise RuntimeError("Monolith object not found; stage_monolith must run before stage_engrave.")

    cavity_mat = T.materials.principled(
        "Cavity_Mat",
        **{
            "Base Color": (0.02, 0.02, 0.025, 1.0),
            "Roughness": 0.6,
            "Metallic": 0.1,
        }
    )

    f_archivo = T.typography.font("archivoblack")

    # 1. "THE"
    m_the = T.typography.engrave(
        monolith,
        "THE",
        f_archivo,
        size=0.08,
        depth=0.012,
        center_xz=(0.0, 2.05),
        axis="-Y",
        wall_material=cavity_mat,
        floor_material=cavity_mat,
        name="Engrave_THE",
    )

    # 2. "KNOWLEDGE" - width_scale ~1.15, spans ~86% face width (0.86m)
    m_know = T.typography.engrave(
        monolith,
        "KNOWLEDGE",
        f_archivo,
        size=0.125,
        depth=0.012,
        width_scale=1.15,
        center_xz=(0.0, 1.88),
        axis="-Y",
        wall_material=cavity_mat,
        floor_material=cavity_mat,
        name="Engrave_KNOWLEDGE",
    )

    # 3. "SUBSTRATE" - tracking ~0.08
    m_sub = T.typography.engrave(
        monolith,
        "SUBSTRATE",
        f_archivo,
        size=0.09,
        depth=0.012,
        tracking=0.08,
        center_xz=(0.0, 1.70),
        axis="-Y",
        wall_material=cavity_mat,
        floor_material=cavity_mat,
        name="Engrave_SUBSTRATE",
    )

    return {"engravings": ["THE", "KNOWLEDGE", "SUBSTRATE"]}


def stage_hud(params):
    monolith = bpy.data.objects.get("Monolith")
    if monolith is None:
        raise RuntimeError("Monolith object not found; stage_monolith must run before stage_hud.")

    overlay_path = Path("/workspaces/tks/renders/overlays/hud_knowledge_substrate.png")
    spec = T.hud.substrate_hud_spec()
    T.hud.bake_overlay(spec, overlay_path, resolution=(2048, 2048))

    # Apply decal to monolith brushed steel material
    mat = monolith.data.materials.get("Monolith_Steel")
    if mat is None:
        for m in monolith.data.materials:
            if "Steel" in m.name:
                mat = m
                break
    if mat is None and len(monolith.data.materials) > 0:
        mat = monolith.data.materials[0]

    if mat:
        T.materials.add_decal(
            mat,
            emission_png=overlay_path,
            size_m=(1.0, 3.2),
            center=(0.0, 0.0),
            axis="-Y",
            emission_strength=3.5,
        )

    return {"hud_overlay": str(overlay_path), "resolution": [2048, 2048]}


def stage_rock(params):
    coll = T.scene.ensure_collection("Environment")
    for o in list(coll.objects):
        if o.name.startswith("Rock_") or o.name.startswith("Rock_Blockout_") or o.name == "Bedrock_Ground":
            bpy.data.objects.remove(o, do_unlink=True)

    # 1. Dark bedrock ground plane
    ground_mat = T.materials.principled(
        "Bedrock_Slate_Floor",
        **{
            "Base Color": (0.025, 0.028, 0.035, 1.0),
            "Roughness": 0.55,
            "Metallic": 0.05,
        }
    )
    ground_obj = T.scene.ground(size=30.0, name="Bedrock_Ground", collection=coll, material=ground_mat)

    # 2. Rock cluster
    r_p = params.get("rock", {})
    count = r_p.get("count", 14)
    r_min, r_max = r_p.get("ring", (0.50, 1.6))
    boulder_path = Path("/workspaces/tks/scenes/assets/models/boulder_01/boulder_01.glb")
    source = boulder_path if boulder_path.exists() else None

    # Monolith base center is (0, 0, 0) on ground plane
    rocks = T.rocks.cluster(
        source_obj_or_path=source,
        target_center=(0.0, 0.0, 0.0),
        count=count,
        ring_radius=(r_min, r_max),
        scale_range=(0.7, 1.9),
        z_sink=(0.10, 0.35),
        collection=coll,
        seed=137,
    )

    return {"ground": ground_obj.name, "rock_count": len(rocks)}


def stage_lighting(params):
    coll = T.scene.ensure_collection("Lighting")
    for o in list(coll.objects):
        if o.name in {"Softbox_Key", "Fill_Area", "BackGlow", "Rim_Right", "LightShaft_Right", "LookRig"}:
            bpy.data.objects.remove(o, do_unlink=True)

    # 1. Softbox key overhead-front: soft gradient reflection on steel face
    sb = T.lighting.softbox(
        "Softbox_Key",
        size=(2.2, 0.7),
        location=(-0.4, -1.8, 3.8),
        target=(0.0, 0.0, 2.1),
        strength=4.0,
        gradient=True,
        visible_camera=False,
        collection=coll,
    )

    # 2. Dim fill light on camera left for subtle side-face reading
    fill = T.lighting.light(
        "AREA",
        "Fill_Area",
        location=(-3.0, -1.8, 1.4),
        energy=25.0,
        color=(0.70, 0.82, 0.95),
        size=2.0,
        target=(0.0, 0.0, 1.2),
        collection=coll,
    )

    # 3. LookRig empty for camera-relative lights
    rig = T.lighting.look_rig(pivot=(0.0, 0.0, 1.6), collection=coll)

    # 4. Cool back-glow halo directly behind monolith
    back_glow = T.lighting.light(
        "POINT",
        "BackGlow",
        location=(0.0, 0.7, 1.9),
        energy=160.0,
        color=(0.20, 0.60, 0.95),
        size=0.6,
        target=None,
        collection=coll,
    )
    back_glow.parent = rig

    # 5. Right rim / shaft light: high-angle spotlight from top right creating the visible light shaft
    shaft = T.lighting.light(
        "SPOT",
        "LightShaft_Right",
        location=(2.6, -1.0, 4.0),
        energy=1200.0,
        color=(0.65, 0.85, 1.0),
        size=0.3,
        target=(0.2, 0.0, 1.4),
        collection=coll,
    )
    if hasattr(shaft.data, "spot_size"):
        shaft.data.spot_size = math.radians(45)
        shaft.data.spot_blend = 0.5
    shaft.parent = rig

    # 6. Sharp right edge rim light
    rim = T.lighting.light(
        "AREA",
        "Rim_Right",
        location=(2.2, -0.4, 2.2),
        energy=500.0,
        color=(0.80, 0.92, 1.0),
        size=0.8,
        target=(0.4, 0.0, 1.8),
        collection=coll,
    )
    rim.parent = rig

    # Light-link rim to monolith and rocks
    monolith = bpy.data.objects.get("Monolith")
    receivers = [monolith] if monolith else []
    env_coll = bpy.data.collections.get("Environment")
    if env_coll:
        receivers.extend(list(env_coll.objects))
    if receivers:
        T.lighting.link_light(rim, receivers)

    return {"softbox": sb.name, "fill": fill.name, "back_glow": back_glow.name, "shaft": shaft.name, "rim": rim.name}


def stage_fx(params):
    coll = T.scene.ensure_collection("FX")
    for o in list(coll.objects):
        if o.name in {"Smoke", "DustMotes"}:
            bpy.data.objects.remove(o, do_unlink=True)

    smoke = T.fx.smoke_volume(
        size=(7.5, 6.5, 5.5),
        location=(0.0, 0.4, 1.8),
        density=0.022,
        noise_scale=0.55,
        detail=5,
        ramp=(0.45, 0.85),
        color=(0.70, 0.80, 0.95),
        anisotropy=0.55,
        name="Smoke",
        collection=coll,
    )

    motes = T.fx.dust_motes(
        region_size=(2.5, 2.2, 3.2),
        location=(0.0, -0.6, 1.6),
        count=450,
        radius=(0.0006, 0.0018),
        emission=3.0,
        color=(0.7, 0.9, 1.0),
        seed=42,
        name="DustMotes",
        collection=coll,
    )
    return {"smoke": smoke.name, "motes": motes.name}


def stage_post(params):
    tree = T.render.post(
        bloom=0.35,
        bloom_threshold=0.88,
        bloom_size=6,
        dispersion=0.002,
        vignette=0.28,
        grain=0.025,
        lift=(0.95, 0.98, 1.02),
        gamma=(1.0, 1.0, 1.0),
        gain=(1.0, 1.0, 1.0),
    )
    return {"grade_tree": tree.name}


def stage_shots(params):
    shots_cfg = params["shots"]
    cams = {}
    ref_img = params.get("reference")
    for name, cfg in shots_cfg.items():
        cam, focus = T.camera.shot(
            name,
            location=cfg["location"],
            target=cfg["target"],
            lens=cfg.get("lens", 50),
            fstop=cfg.get("fstop", 4.0),
        )
        if name == "hero" and ref_img and Path(ref_img).exists():
            T.camera.reference_overlay(cam, ref_img, opacity=0.45)
        cams[name] = cam.name

    hero_cam = bpy.data.objects.get("Shot_hero")
    if hero_cam:
        bpy.context.scene.camera = hero_cam
    return cams


STAGE_DISPATCH = {
    "setup": stage_setup,
    "world": stage_world,
    "monolith": stage_monolith,
    "engrave": stage_engrave,
    "hud": stage_hud,
    "rock": stage_rock,
    "lighting": stage_lighting,
    "fx": stage_fx,
    "post": stage_post,
    "shots": stage_shots,
}


def build(stages=None, params=None):
    if stages is None:
        stages = ALL_STAGES
    if params is None:
        params = PARAMS

    summary = {}
    for st in stages:
        fn = STAGE_DISPATCH.get(st)
        if fn:
            res = fn(params)
            summary[st] = res
    return summary
