# Outdoor / nature path

Terrain (A.N.T. Landscape), hero building placed on the terrain, a Sapling tree kit scattered by
Geometry Nodes with clearings, physical sky + sun, volumetric haze, eye-level camera.
Example brief: *"a forest cabin at dusk"*. Each block below is one `execute_blender_code` call
(fresh namespace each time — re-find objects with `T.scene.find`). Blocks are executed by
`scripts/blender/test_recipes_live.py`, so they are known to work on Blender 5.2.

**Swap-ins:** real assets via `blender-assets` (Poly Haven HDRI instead of `sky`, Poly Haven /
ambientCG ground material, Poly Haven trees/rocks, Poly Pizza/Sketchfab cabin) — keep the same
staging and replace the blockout objects biggest-first.

## Step 1 — scene + terrain blockout

`height` is relief in metres; `feature_size` is the hill size in metres (smaller = rougher).
Keep subdivisions ≤ 256² (GPU memory).

```python
# recipe: outdoor step 1
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
sc = T.scene.new_scene("forest_cabin")
terrain = T.extensions.landscape(size=(80, 80), subdivisions=(160, 160), height=6.0, seed=7, feature_size=25)
T.scene.smooth_by_angle(terrain)
T.materials.assign(terrain, T.materials.principled("Grass", **{"Base Color": (0.12, 0.2, 0.05), "Roughness": 0.9}))
print(T.scene.summary())
```

## Step 2 — hero building on the terrain + camera early

`flatten_area` levels a pad under the building (do this before placing or scattering);
`place_on` raycasts the terrain; `sink` buries the base slightly so it never floats.
Glowing windows: emission 3–15 with a saturated warm colour reads as lamplight. Higher values
clip to flat white. Add a weak warm point light inside or just in front of the window for spill.
Set the camera now and check composition with `look(mode="camera")` before adding detail.

```python
# recipe: outdoor step 2
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
terrain = T.scene.find("Terrain")
T.scene.flatten_area(terrain, 0, 0, radius=4.5, falloff=5)
cabin = T.scene.box("Cabin", (4, 3, 2.6), material=T.materials.principled("CabinWood", preset="wood"))
T.scene.place_on(cabin, terrain, 0, 0, sink=0.1)
window = T.scene.box("Window", (1.0, 0.05, 0.8), on_ground=False,
                     material=T.materials.emission("WindowGlow", (1.0, 0.55, 0.2), 15))
window.location = (0.6, cabin.location.y - 1.52, cabin.location.z + 1.4)
eye_z = T.scene.surface_height(terrain, 6, -24) + 4  # raise above the nearest crest if blocked
cam, focus = T.camera.rig(location=(6, -24, eye_z), target=(0, 0, cabin.location.z + 1.3), lens="normal")
print({"cabin": tuple(cabin.location), "camera": tuple(cam.location)})
```

## Step 3 — sky, low sun and real-world sun position

`world.sky` uses the 5.x sky types (`MULTIPLE_SCATTERING` default; `NISHITA` is gone).
`extensions.sun_position` drives the sun light from a real place/date/time (optional).

```python
# recipe: outdoor step 3
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
sun = T.lighting.sun(elevation_deg=8, azimuth_deg=240, strength=2.5, color=(1, 0.8, 0.6))
T.world.sky(sun_elevation_deg=8, sun_rotation_deg=240, strength=0.3)
# Optional: T.extensions.sun_position(47.3, 11.4, month=10, day=12, hour=18.5, utc_zone=2, sun_object=sun)
print(T.render.preview("forest_cabin_light"))
```

## Step 4 — tree kit + scatter with clearings

Build 3 variants, one sub-collection per tree (so trunk + leaves are picked together), exclude
the kit from the view layer, then scatter the kit as a collection. `clearing_mask` keeps the
building, the camera and the sightline between them free of trees.

```python
# recipe: outdoor step 4
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T
terrain = T.scene.find("Terrain")
bark = T.materials.principled("Bark", **{"Base Color": (0.08, 0.05, 0.03), "Roughness": 0.9})
leaf = T.materials.principled("Leaf", preset="foliage")
kit = T.scene.ensure_collection("TreeKit")
for i in range(3):
    sub = T.scene.ensure_collection(f"TreeKit_{i}", parent=kit)
    T.extensions.tree(seed=i, collection=sub, name=f"KitTree_{i}", bark=bark, foliage=leaf,
                      scale=9, ratio=0.03, leaves=40, leafScale=0.35, shape="4")
bpy.context.view_layer.layer_collection.children[kit.name].exclude = True
T.scene.clearing_mask(terrain, [(0, 0, 7), (3, -12, 4), (6, -24, 5)], falloff=4)
T.geonodes.scatter(terrain, kit, count=120, seed=2, collection=True, random_scale=0.4, mask="ScatterMask")
print(T.render.preview("forest_cabin_trees"))
```

## Step 5 — atmosphere + grade

Keep fog density low (0.001–0.005 over tens of metres); `world_fog` is a cheaper Cycles-only alternative.

```python
# recipe: outdoor step 5
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
T.atmosphere.fog_box(size=(80, 80, 12), density=0.002)
T.render.grade(vignette=0.25)
print(T.render.preview("forest_cabin"))
```

## Critique hints for this path

- Horizon in the upper or lower third; terrain crest should not cut the subject.
- Trees ~8–20 m; nothing between camera and subject unless intentional framing.
- Dusk: sun 2–10° elevation, warm sun colour, sky strength 0.2–0.5, warm emissive windows.
- Washed-out image → lower fog density; flat light → lower sun or rotate azimuth for side light.
