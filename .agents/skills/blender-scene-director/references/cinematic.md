# Cinematic hero / key art path

High-contrast key art, cinematic monolith or hero asset embedded in an atmospheric environment.
Dark studio void background, light-linked softboxes, anisotropic brushed metal, procedural boolean
engraving, cybernetic HUD decals, bedrock rock clustering, local smoke volume, and compositor bloom.
Example brief: *"a towering brushed-steel monolith in a dark atmospheric void with glowing cybernetic HUD and bedrock formation"*.

Each step is self-contained and reproducible. Validated against live Cycles rendering.

---

## Step 1 — Studio void world

A deep void that separates camera rays from reflections. Camera rays see a dark vertical gradient,
while reflection and indirect GI rays sample a dim studio HDRI for specular highlights without flattening contrast.

```python
# recipe: cinematic step 1
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
from pathlib import Path
import tks_bpy as T; T = T.reload_all()

sc = T.scene.new_scene("cinematic_hero", engine="CYCLES", resolution=(1920, 1080))
hdri = Path("/workspaces/tks/scenes/assets/hdris/monochrome_studio_03_2k.hdr")
T.world.studio_void(
    top=(0.004, 0.006, 0.010),
    bottom=(0.001, 0.001, 0.002),
    hdri=hdri if hdri.exists() else None,
    hdri_strength=0.12,
    hdri_rotation_deg=45.0,
)
```

---

## Step 2 — Hero camera & reference overlay

Calibrated perspective with focal length and depth of field. Use `reference_overlay` to view
concept art directly through the camera for silhouette alignment.

```python
# recipe: cinematic step 2
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T

cam, focus = T.camera.shot(
    "hero",
    location=(-0.95, -4.1, 0.72),
    target=(0.10, 0.0, 1.48),
    lens=62,
    fstop=2.8,
)
```

---

## Step 3 — Hero asset geometry & brushed metal

Model the hero asset at metric scale and apply anisotropic brushed metal with micro-grain bump and bevel glints.

```python
# recipe: cinematic step 3
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T

monolith = T.scene.box("Monolith", size=(1.0, 0.35, 3.2), location=(0, 0, 1.15), collection="Props", on_ground=False)
bev = monolith.modifiers.new("Bevel", "BEVEL")
bev.width = 0.006
bev.segments = 4
bev.limit_method = "ANGLE"

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
monolith.data.materials.append(mat)
```

---

## Step 4 — Procedural engraving & typography

Non-destructive Geometry Nodes modifier that carves text into the asset face with dark cavity material.

```python
# recipe: cinematic step 4
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T

monolith = T.scene.find("Monolith")
cavity_mat = T.materials.principled("Cavity_Mat", **{"Base Color": (0.02, 0.02, 0.025, 1.0), "Roughness": 0.6})
f_font = T.typography.font("archivoblack")

T.typography.engrave(
    monolith, "THE", f_font, size=0.08, depth=0.012, center_xz=(0.0, 2.05),
    axis="-Y", wall_material=cavity_mat, name="Engrave_THE",
)
T.typography.engrave(
    monolith, "KNOWLEDGE", f_font, size=0.125, depth=0.012, width_scale=1.15,
    center_xz=(0.0, 1.88), axis="-Y", wall_material=cavity_mat, name="Engrave_KNOWLEDGE",
)
```

---

## Step 5 — HUD decal baker & projection

Bake 2D cybernetic hair-line filaments, constellation nodes, and code panels in an isolated ortho EEVEE scene,
then project onto the hero asset using `materials.add_decal`.

```python
# recipe: cinematic step 5
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
from pathlib import Path
import tks_bpy as T

monolith = T.scene.find("Monolith")
overlay_path = Path("/workspaces/tks/renders/overlays/hud_cinematic.png")
spec = T.hud.substrate_hud_spec()
T.hud.bake_overlay(spec, overlay_path, resolution=(1024, 1024))

mat = monolith.data.materials[0]
T.materials.add_decal(
    mat,
    emission_png=overlay_path,
    size_m=(1.0, 3.2),
    center=(0.0, 0.0),
    axis="-Y",
    emission_strength=3.5,
)
```

---

## Step 6 — Bedrock & natural rock cluster

Embed the asset into a dark slate ground plane and spawn a natural boulder cluster around the perimeter.

```python
# recipe: cinematic step 6
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
from pathlib import Path
import tks_bpy as T

coll = T.scene.ensure_collection("Environment")
ground_mat = T.materials.principled("Bedrock_Floor", **{"Base Color": (0.025, 0.028, 0.035, 1.0), "Roughness": 0.55})
T.scene.ground(size=30.0, name="Bedrock_Ground", collection=coll, material=ground_mat)

boulder_path = Path("/workspaces/tks/scenes/assets/models/boulder_01/boulder_01.glb")
T.rocks.cluster(
    source_obj_or_path=boulder_path if boulder_path.exists() else None,
    target_center=(0.0, 0.0, 0.0),
    count=12,
    ring_radius=(0.50, 1.6),
    scale_range=(0.7, 1.8),
    z_sink=(0.10, 0.35),
    collection=coll,
    seed=137,
)
```

---

## Step 7 — Light-linked softbox, rim & look rig

Controlled lighting: softbox overhead for face gradient, point light for back glow, sharp spotlight shaft,
and rim light linked exclusively to hero receivers.

```python
# recipe: cinematic step 7
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T

coll = T.scene.ensure_collection("Lighting")
sb = T.lighting.softbox("Softbox_Key", size=(2.2, 0.7), location=(-0.4, -1.8, 3.8), target=(0.0, 0.0, 2.1), strength=4.0, visible_camera=False, collection=coll)
fill = T.lighting.light("AREA", "Fill_Area", location=(-3.0, -1.8, 1.4), energy=25.0, color=(0.70, 0.82, 0.95), size=2.0, target=(0.0, 0.0, 1.2), collection=coll)

rig = T.lighting.look_rig(pivot=(0.0, 0.0, 1.6), collection=coll)
back_glow = T.lighting.light("POINT", "BackGlow", location=(0.0, 0.7, 1.9), energy=160.0, color=(0.20, 0.60, 0.95), size=0.6, collection=coll)
back_glow.parent = rig

shaft = T.lighting.light("SPOT", "LightShaft_Right", location=(2.6, -1.0, 4.0), energy=1200.0, color=(0.65, 0.85, 1.0), size=0.3, target=(0.2, 0.0, 1.4), collection=coll)
shaft.parent = rig

rim = T.lighting.light("AREA", "Rim_Right", location=(2.2, -0.4, 2.2), energy=500.0, color=(0.80, 0.92, 1.0), size=0.8, target=(0.4, 0.0, 1.8), collection=coll)
rim.parent = rig

monolith = T.scene.find("Monolith")
receivers = [monolith] + list(T.scene.ensure_collection("Environment").objects)
T.lighting.link_light(rim, receivers)
```

---

## Step 8 — Local smoke volume & dust motes

Confine atmosphere to a bounded box around the hero and instance illuminated dust motes for depth.

```python
# recipe: cinematic step 8
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T

coll = T.scene.ensure_collection("FX")
T.fx.smoke_volume(size=(7.5, 6.5, 5.5), location=(0.0, 0.4, 1.8), density=0.022, noise_scale=0.55, collection=coll)
T.fx.dust_motes(region_size=(2.5, 2.2, 3.2), location=(0.0, -0.6, 1.6), count=400, radius=(0.0006, 0.0018), emission=3.0, collection=coll)
```

---

## Step 9 — Compositor bloom, dispersion & grade

Bloom on emissive HUD nodes, subtle chromatic dispersion, and vignette.

```python
# recipe: cinematic step 9
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T

T.render.post(
    bloom=0.35,
    bloom_threshold=0.88,
    bloom_size=6,
    dispersion=0.002,
    vignette=0.28,
    grain=0.025,
    lift=(0.95, 0.98, 1.02),
)
```

---

## Step 10 — Multi-angle verification & contact sheet

Render hero shot, orbit shots, and verify in a 4-up contact sheet.

```python
# recipe: cinematic step 10
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T

shots = T.render.render_shots(["hero"], preview=True, width=64, samples=4)
print("Hero rendered:", shots["hero"])
```
