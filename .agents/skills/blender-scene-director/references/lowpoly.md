# Stylised low-poly path

EEVEE, flat-shaded faceted terrain with a round coast, a flat colour palette (roughness 1, no
textures), procedural faceted props instanced with Geometry Nodes, simple sun + flat sky.
Example brief: *"a stylised low-poly island"*. Each block is one `execute_blender_code` call
(fresh namespace; re-find objects with `T.scene.find`). Blocks are executed by
`scripts/blender/test_recipes_live.py`.

**Swap-ins:** Poly Pizza (CC0/CC-BY low-poly models, API key required) or Kenney-style packs
via `blender-assets` — keep the palette consistent by re-assigning `LP_*` materials after import.
Extension props: `primitive_gem_add`, `primitive_round_cube_add`, `primitive_steppyramid_add`
(extra mesh objects) are handy stylised fillers.

## Step 1 — scene, palette, island, water

Low subdivisions (30–60) give the facets; `island=True` makes a round coast. Per-face
material indices paint beaches by height without textures.

```python
# recipe: lowpoly step 1
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
sc = T.scene.new_scene("lowpoly_island", engine="BLENDER_EEVEE", samples=32, resolution=(1600, 1200))
palette = {"grass": (0.25, 0.55, 0.12), "sand": (0.85, 0.72, 0.45), "trunk": (0.35, 0.2, 0.1),
           "leaf": (0.1, 0.4, 0.15)}
pal = {k: T.materials.principled(f"LP_{k}", **{"Base Color": c, "Roughness": 1.0}) for k, c in palette.items()}
island = T.extensions.landscape(size=(40, 40), subdivisions=(40, 40), height=6.0, seed=11, feature_size=10,
                                island=True, name="Island")
T.materials.assign(island, pal["grass"])
island.data.materials.append(pal["sand"])
for poly in island.data.polygons:
    poly.use_smooth = False
    poly.material_index = 1 if poly.center.z < 1.5 else 0
water = T.scene.ground(200, name="Water", material=T.materials.principled("LP_water", **{"Base Color": (0.1, 0.45, 0.6), "Roughness": 0.15}))
water.location.z = 1.0
print(T.scene.summary())
```

## Step 2 — camera + light early

Stylised scenes read best from a raised three-quarter view with a longer lens (less distortion).

```python
# recipe: lowpoly step 2
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
T.world.solid((0.55, 0.75, 0.95), 1.0)
T.lighting.sun(elevation_deg=45, azimuth_deg=150, strength=4, angle_deg=2)
cam, focus = T.camera.rig((34, -34, 24), (0, 0, 1.0), lens=50)
print(T.render.preview("lowpoly_island_blockout"))
```

## Step 3 — faceted tree kit scattered above the waterline

Each kit object is one mesh with two material slots; canopy faces are selected from the verts
returned by `create_cone` (bmesh face order is not creation order). `height_mask` keeps trees
above the beach.

```python
# recipe: lowpoly step 3
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bmesh
import bpy
from mathutils import Matrix
import tks_bpy as T
island = T.scene.find("Island")
trunk_mat, leaf_mat = T.materials.principled("LP_trunk"), T.materials.principled("LP_leaf")
kit = T.scene.ensure_collection("LPTreeKit")
for i, (height, radius) in enumerate([(2.2, 0.7), (3.0, 0.9), (1.6, 0.6)]):
    me = bpy.data.meshes.new(f"LPTree_{i}")
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, segments=5, radius1=0.12, radius2=0.1, depth=height * 0.35,
                          matrix=Matrix.Translation((0, 0, height * 0.175)))
    canopy = bmesh.ops.create_cone(bm, cap_ends=True, segments=6, radius1=radius, radius2=0.0,
                                   depth=height * 0.75, matrix=Matrix.Translation((0, 0, height * 0.625)))
    for face in {f for v in canopy["verts"] for f in v.link_faces}:
        face.material_index = 1
    bm.to_mesh(me)
    bm.free()
    me.materials.append(trunk_mat)
    me.materials.append(leaf_mat)
    T.scene.link(bpy.data.objects.new(f"LPTree_{i}", me), kit)
bpy.context.view_layer.layer_collection.children[kit.name].exclude = True
T.scene.height_mask(island, 1.5, falloff=0.8)
T.geonodes.scatter(island, kit, count=60, seed=4, collection=True, random_scale=0.35, mask="HeightMask")
print(T.render.preview("lowpoly_island_trees"))
```

## Step 4 — hero prop from an extension + preview

```python
# recipe: lowpoly step 4
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T
island = T.scene.find("Island")
bpy.ops.mesh.primitive_gem_add()
gem = bpy.context.active_object
gem.name = "Gem"
T.scene.link(gem, "Props")
gem.scale = (0.5, 0.5, 0.5)
T.materials.assign(gem, T.materials.principled("LP_gem_glow", **{"Base Color": (0.9, 0.1, 0.5), "Roughness": 0.2,
                                                                 "Emission Color": (0.9, 0.1, 0.5), "Emission Strength": 2.0}))
T.scene.place_on(gem, island, 0, 0)
gem.location.z += 0.6
print(T.render.preview("lowpoly_island"))
```

## Critique hints for this path

- 4–6 palette colours; vary value (light/dark) more than hue. No textures, no bevels.
- Flat shading everywhere (`use_smooth = False`); avoid `smooth_by_angle` here.
- Keep facets visible: terrain subdivisions ≈ size / 1 m; props 5–8 sided.
- Scale exaggeration is fine (big trees, chunky props) but keep it consistent.
- Washed-out colours: AgX desaturates bright stylised palettes. Use
  `sc.view_settings.look = "AgX - Punchy"` (or `view_transform = "Standard"` for exact palette
  colours), and keep the sky strength low so the horizon stays blue rather than grey haze.
