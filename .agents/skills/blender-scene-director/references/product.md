# Product / studio path

Seamless cyclorama, a hero object at true scale, three-point area lighting, long lens with depth
of field, framed automatically. Example brief: *"a ceramic coffee mug product shot"*.
Each block is one `execute_blender_code` call (fresh namespace; re-find objects with
`T.scene.find`). Blocks are executed by `scripts/blender/test_recipes_live.py`.

**Swap-ins:** a downloaded model as the hero — `T.assets.import_model(path, target_size=<real
height>, axis="z")` (Poly Haven / Sketchfab / Poly Pizza via `blender-assets`); a studio HDRI from
Poly Haven for reflections; a textured surface (ambientCG `Marble`/`Wood`) on a box plinth.

## Step 1 — studio: cyclorama + dark world

Square or 4:5 frames suit packshots. The world stays dark so the lights define the look.

```python
# recipe: product step 1
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
sc = T.scene.new_scene("mug_shot", resolution=(1600, 1600))
T.scene.cyclorama(width=3, depth=2.5, height=1.5, radius=0.5,
                  material=T.materials.principled("Backdrop", **{"Base Color": (0.55, 0.6, 0.65), "Roughness": 0.6}))
T.world.solid((0.02, 0.02, 0.02))
print(T.scene.summary())
```

## Step 2 — hero object at real size

Modelled from primitives here: open cylinder + Solidify (wall thickness) + Bevel (catches
highlights; real objects have no razor edges) + torus handle. A mug is ~95 mm tall, 85 mm wide.
Operators such as `primitive_torus_add` work from the MCP socket; the new object is
`bpy.context.active_object`.

```python
# recipe: product step 2
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import math
import bmesh
import bpy
import tks_bpy as T
me = bpy.data.meshes.new("Mug")
bm = bmesh.new()
bmesh.ops.create_cone(bm, cap_ends=True, segments=64, radius1=0.042, radius2=0.042, depth=0.095)
top = max(bm.faces, key=lambda f: f.calc_center_median().z)
bmesh.ops.delete(bm, geom=[top], context="FACES_ONLY")
bmesh.ops.translate(bm, vec=(0, 0, 0.0475), verts=bm.verts)
bm.to_mesh(me)
bm.free()
mug = bpy.data.objects.new("Mug", me)
T.scene.link(mug, "Props")
wall = mug.modifiers.new("Wall", "SOLIDIFY")
wall.thickness, wall.offset = 0.004, -1
bevel = mug.modifiers.new("Bevel", "BEVEL")
bevel.width, bevel.segments, bevel.limit_method = 0.0015, 3, "ANGLE"
T.scene.smooth_by_angle(mug, 40)
bpy.ops.mesh.primitive_torus_add(major_radius=0.026, minor_radius=0.0055, major_segments=48,
                                 minor_segments=16, location=(0.048, 0, 0.05), rotation=(math.radians(90), 0, 0))
handle = bpy.context.active_object
handle.name = "MugHandle"
handle.parent = mug
T.scene.link(handle, "Props")
T.scene.smooth_by_angle(handle, 40)
ceramic = T.materials.principled("Ceramic", preset="ceramic", **{"Base Color": (0.6, 0.12, 0.06)})
T.materials.assign(mug, ceramic)
T.materials.assign(handle, ceramic)
mug.rotation_euler.z = math.radians(-35)
print(T.scene.summary())
```

## Step 3 — three-point light + long-lens camera, auto-framed

`three_point` places key (front-left, big soft), fill (front-right, 35%) and rim (behind, for
the silhouette). Scale `distance`/`key_energy` with object size. `frame_objects` dollies the
camera along its view direction so the objects fill the frame (`margin` 1.2–1.5).

```python
# recipe: product step 3
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T
mug, handle = T.scene.find("Mug"), T.scene.find("MugHandle")
T.lighting.three_point(target=(0, 0, 0.05), distance=1.2, key_energy=80)
cam, focus = T.camera.rig((0.35, -0.55, 0.3), (0, 0, 0.045), lens="product", dof=True, fstop=5.6)
T.camera.frame_objects(cam, [mug, handle], margin=1.35)
bpy.context.scene.view_settings.look = "AgX - Medium High Contrast"
print(T.render.preview("mug_shot_light", width=400))
```

## Step 4 — grade and preview

```python
# recipe: product step 4
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
T.render.grade(vignette=0.3)
print(T.render.preview("mug_shot", width=400))
```

## Critique hints for this path

- Subject fills 60–80% of the frame; slightly above eye level shows the top/opening.
- Highlights should show the form (a long soft streak on cylinders); move/scale the key, not exposure.
- Rim light separates the silhouette from the backdrop; the backdrop gradient should be smooth.
- DoF: f/4–8 at product distances keeps the whole object sharp; focus empty on the front face.
- Saturated colours desaturate under bright light/AgX: pick a deeper base colour.
