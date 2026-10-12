# Interior / archviz path

Archimesh room with a real window opening, real-size furniture blockout, daylight through the
window plus a warm practical, wide eye-level camera. Example brief: *"a cosy living room in the
afternoon"*. Each block is one `execute_blender_code` call (fresh namespace; re-find objects with
`T.scene.find`). Blocks are executed by `scripts/blender/test_recipes_live.py`.

**Swap-ins:** furniture from Poly Haven / Sketchfab / Poly Pizza via `blender-assets`
(`target_size` = the blockout box's longest side, then delete the box); wall/floor PBR from
ambientCG (`fetch_ambientcg.py download WoodFloor051`) + `T.materials.pbr_from_folder`;
an interior HDRI from Poly Haven as the outside view instead of `sky`.

## Step 1 — room shell with a window opening

The room is centred: floor x ∈ [-w/2, w/2], y ∈ [-d/2, d/2], walls on the outside of that.
`cut_opening` cuts a hole with a Boolean; the large `Outside` ground fills the view through it.

```python
# recipe: interior step 1
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
sc = T.scene.new_scene("living_room")
room = T.extensions.room(width=5, depth=4, height=2.7, ceiling=True)
T.materials.assign(room, T.materials.principled("Plaster", **{"Base Color": (0.8, 0.78, 0.74), "Roughness": 0.85}))
floor = next(c for c in room.children if c.name.startswith("Floor"))
T.materials.assign(floor, T.materials.principled("Oak", preset="wood"))
T.scene.cut_opening(room, (0.6, 1.6, 1.4), (-2.5, 0.2, 1.55), name="WindowHole")
outside = T.scene.ground(400, name="Outside", material=T.materials.principled("Lawn", **{"Base Color": (0.1, 0.18, 0.05), "Roughness": 0.9}))
outside.location.z = -0.02
print(T.scene.summary())
```

## Step 2 — furniture blockout at real size + camera

Typical sizes (m): sofa 2.0×0.9×0.8, coffee table 1.1×0.6×0.42, dining table 1.6×0.9×0.75,
chair seat 0.45 high, bed 2.0×1.6×0.5, shelf 0.35 deep × 1.9 high, door 0.9×2.1, worktop 0.9 high.
Interior cameras: 18–24 mm (`"wide"`), 1.2–1.6 m high, placed in a corner looking across the room.

```python
# recipe: interior step 2
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
wood = T.materials.principled("Walnut", **{"Base Color": (0.15, 0.08, 0.04), "Roughness": 0.5})
T.scene.box("Sofa", (2.0, 0.9, 0.8), (0.4, 1.45, 0), material=T.materials.principled("SofaFabric", preset="fabric"))
T.scene.box("CoffeeTable", (1.1, 0.6, 0.42), (0.4, 0.3, 0), material=wood)
T.scene.box("Rug", (2.6, 1.8, 0.01), (0.4, 0.5, 0), material=T.materials.principled("Rug", **{"Base Color": (0.5, 0.15, 0.1), "Roughness": 1.0}))
T.scene.box("Shelf", (0.35, 1.2, 1.9), (2.3, 0.6, 0), material=wood)
T.scene.box("FloorLamp", (0.3, 0.3, 1.6), (-1.6, 1.6, 0), material=T.materials.principled("LampShade", **{"Base Color": (0.9, 0.85, 0.7), "Roughness": 0.6}))
cam, focus = T.camera.rig((2.1, -1.7, 1.5), (-0.6, 0.8, 0.9), lens="wide")
print({"camera": tuple(cam.location)})
```

## Step 3 — daylight + practical light, exposure

Sun through the opening for a light patch, a cool area light just inside the window for soft
fill (interiors otherwise go dark and noisy), and a warm low-wattage practical. White walls
bounce a lot: balance with lower sky strength and `exposure` around -1 rather than more lights.

```python
# recipe: interior step 3
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T
sc = bpy.context.scene
T.world.sky(sun_elevation_deg=25, sun_rotation_deg=-90, strength=0.15)
T.lighting.sun(elevation_deg=25, azimuth_deg=-70, strength=3, color=(1, 0.92, 0.8))
T.lighting.light("AREA", "WindowFill", (-2.3, 0.2, 1.55), 25, size=1.4, target=(1, 0.2, 1.0), temperature_k=7000)
T.lighting.light("POINT", "LampBulb", (-1.6, 1.6, 1.75), 40, size=0.1, temperature_k=2700)
sc.view_settings.exposure = -1.0
sc.view_settings.look = "AgX - Medium High Contrast"
print(T.render.preview("living_room_light"))
```

## Step 4 — grade and preview

```python
# recipe: interior step 4
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
T.render.grade(vignette=0.2)
print(T.render.preview("living_room"))
```

## Critique hints for this path

- Verticals vertical: keep the camera level (target at a similar height) or use a shift-lens
  (`cam.data.shift_y`) rather than tilting.
- Furniture against walls should touch them (no gaps); rugs under the front legs of sofas.
- Too bright/flat → lower sky/fill, keep the sun patch; too dark/noisy → raise the window fill,
  not the exposure.
- Add a few small props (books, plant, cushions) for scale before declaring it done.
