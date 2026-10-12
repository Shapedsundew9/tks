---
name: blender-extensions
description: Community Blender extensions pre-installed and enabled in this repo's Blender container (A.N.T. Landscape, Sapling Tree Gen, Extra Mesh/Curve Objects, Sun Position, Archimesh, Bool Tool, LoopTools, Node Arrange, Node Wrangler) - what each is for, how to drive them from execute_blender_code, gotchas, and how to add a new one.
---

# Pinned Blender extensions

Blender 4.2+ moved the old bundled add-ons to extensions.blender.org. This repo pins a core set
in `.devcontainer/blender/extensions.lock.json` (version + sha256), bakes the zips into the image,
and `bootstrap.py` installs and enables them offline at every Blender start. **Do not** try to
download or enable add-ons yourself: Blender has `online_access` off and the set is already live.
Check with:

```python
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
print(T.extensions.status())   # {"antlandscape": True, ...}
```

Module names are `bl_ext.user_default.<id>`. Bundled `node_wrangler` is also enabled.

## What to use when

| id | Use for | Drive it with |
|---|---|---|
| `antlandscape` (A.N.T. Landscape) | Terrain, hills, mountains, islands, dunes | `T.extensions.landscape(size, subdivisions, height, seed, noise_type, feature_size, island, **kw)`; raw `bpy.ops.mesh.landscape_add(..., refresh=True)` |
| `sapling_tree_gen` (Sapling) | Hero / mid-ground trees with leaves | `T.extensions.tree(seed, levels, show_leaves, location, collection, bark, foliage, **kw)`; raw `bpy.ops.curve.tree_add(do_update=True, ...)` |
| `sun_position` | Physically placed sun from lat/long/date/time; golden hour | `T.extensions.sun_position(lat, lon, month, day, hour, utc_zone, sun_object=sun)` |
| `archimesh` | Rooms, doors, windows, stairs, shelves, kitchens, roofs, columns, books, blinds | `T.extensions.room(width, depth, height, ceiling=True)`; `bpy.ops.mesh.archimesh_door/window/stairs/shelves/kitchen/books/roof/column/light/venetian/roller/japan/winpanel()` |
| `extra_mesh_objects` | Gems, diamonds, gears, round cube, teapot, torus knot, pipe joints, step pyramid, math surfaces | `bpy.ops.mesh.primitive_gem_add()`, `primitive_diamond_add`, `primitive_brilliant_add`, `primitive_gear`, `primitive_worm_gear`, `primitive_round_cube_add`, `primitive_teapot_add`, `primitive_torusknot_add`, `primitive_supertoroid_add`, `primitive_steppyramid_add`, `primitive_*_joint_add`, `primitive_z_function_surface` |
| `extra_curve_objectes` (sic, upstream id) | Spirals, torus knots, braids, celtic links, curly/simple curves | `bpy.ops.curve.spirals()`, `curve.torus_knot_plus()`, `curve.simple()`, `curve.add_braid()`, `curve.curveaceous_galore()` |
| `bool_tool` | Hard-surface boolean workflows | `bpy.ops.object.boolean_auto_union/difference/intersect/slice()` (needs selection: active = base, selected = cutters) |
| `looptools` | Edit-mode cleanup: relax, circle, flatten, bridge, space | `bpy.ops.mesh.looptools_relax/circle/flatten/bridge/space/curve/gstretch()` (edit mode + selection) |
| `node_arrange` | Tidy generated node trees for the user | `bpy.ops.node.na_arrange_selected()` (needs a node editor context) |
| `node_wrangler` (bundled) | Interactive node helpers for the user | UI only; don't call from scripts |

Prefer the `tks_bpy` wrappers; they are covered by live tests. Raw operators listed above run
from the MCP timer context without overrides unless noted (edit-mode / editor-context ones need
`bpy.context.temp_override` and a selection — see `blender-bpy-5x`).

## Gotchas (verified)

- **ANT Landscape** `execute()` returns `PASS_THROUGH` and creates nothing unless `refresh=True`.
  Raw heights are clamped to `minimum`/`maximum` (default ±1, so a big `height` gives flat
  plateaus) and `noise_size` is in world units (at 60 m, 1 ≈ 36° mean slope, 30 ≈ 10°).
  `T.extensions.landscape` handles all of this: it generates unclamped, then rescales so
  `height` = exact relief in metres with the base at z=0; `feature_size` = noise size in metres
  (default largest side / 4); `island=True` fades to a round coast. Keep `subdivisions` ≤ 256²
  while iterating (30–60 for faceted low-poly). Useful `noise_type` values: `hetero_terrain`,
  `hybrid_multi_fractal`, `ridged_multi_fractal`, `multi_fractal`, `planet_noise`; list them with
  `bpy.ops.mesh.landscape_add.get_rna_type().properties["noise_type"].enum_items`.
- **Sapling** makes a curve trunk with a `leaves` mesh child; its default `bevel=False` renders
  as hair-thin wires, so the wrapper sets `bevel=True` and takes `bark`/`foliage` materials.
  Good forest kwargs: `scale=9, ratio=0.03, leaves=40, leafScale=0.35, shape="4"`. Keep `levels`
  2–3. For forests, make 2–4 variants, each in its own sub-collection of an excluded kit
  collection, then `T.geonodes.scatter(terrain, kit, collection=True, mask=...)` — see the
  outdoor recipe in `blender-scene-director`.
- **Archimesh** operators take no arguments; parameters live on the created object
  (`obj.RoomGenerator[0]`, `DoorObjectGenerator`, …) and edits regenerate the mesh. Room wall
  angles are absolute in [-180, 180].
- **Sun Position** drives `sun_object` rotation (and a Sky Texture node if given); usage mode must
  be `NORMAL`. Combine with `T.world.sky()` or a Poly Haven HDRI.
- Extension properties persist in the `.blend`; scenes saved here need the same extensions
  when opened elsewhere.

## Adding or upgrading an extension

1. Find the id and archive in the API (slugs on the website differ from ids):
   `curl -s 'https://extensions.blender.org/api/v1/extensions/?blender_version=5.2.0&platform=linux-x64'`
   → `id`, `version`, `archive_url`, `archive_hash`, `license`.
2. Add/update the entry in `.devcontainer/blender/extensions.lock.json` (sha256 without the
   `sha256:` prefix).
3. Bump the image tag in `.devcontainer/docker-compose.blender.yml` and rebuild only Blender:
   `docker compose -p tks_devcontainer -f .devcontainer/docker-compose.yml -f .devcontainer/docker-compose.shared.yml -f .devcontainer/docker-compose.blender.yml up -d --no-deps --build blender`
   (warn the user first — restarting Blender discards unsaved work).
4. Add a wrapper + live test in `scripts/blender/` if agents should use it routinely, and
   document it in this table.

All pinned extensions are GPL; they run inside Blender only and are not redistributed by this
repository (the image downloads them at build time).
