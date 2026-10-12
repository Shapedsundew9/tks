# tks_bpy API reference

Location: `scripts/blender/tks_bpy/` (same path inside the Blender container).

```python
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
```

All lengths are metres, angles degrees (unless the name says otherwise), colours linear RGB 0–1.
Every MCP `execute_blender_code` call starts with a fresh namespace: repeat the import header in
each call and re-acquire objects with `T.scene.find("Name")`.

**Data safety:** `new_scene` only clears scenes tks_bpy created; collections are looked up within
the target scene only; `principled` only reuses materials tks_bpy created. User data with the
same names is never modified.

## scene

| Function | Notes |
|---|---|
| `new_scene(name, engine="CYCLES", resolution=(1920,1080), samples=128, clear=True, view_transform="AgX", look="None")` | Creates/resets and activates a scene; metric units, GPU Cycles + denoise, collections `Environment, Props, Lighting, Cameras, FX`, re-applies MCP asset toggles. `look="Punchy"` → `"AgX - Punchy"`. Never touches other scenes. |
| `ensure_collection(name, scene=None, parent=None)` / `link(obj, collection)` | Scene-scoped lookup (may be created as `Name.001` but is still found by `name`). `link` moves an object exclusively into a collection (name or object). |
| `find(name, scene=None)` | Object in the current scene by name (or its `.001` duplicate); `KeyError` if missing. |
| `ground(size=40, material=None)` | Single-quad plane at z=0 in `Environment`. Subdivide/displace yourself if needed. |
| `box(name, size=(x,y,z), location, collection="Props", material=None, on_ground=True)` | Real-size blockout; origin at base centre. |
| `world_bbox(objs)` → `(min, max)` | Evaluated, includes children. |
| `drop_to_ground(obj, z=0)` | Moves hierarchy so its lowest point touches `z`. |
| `surface_height(surface, x, y)` | World z of the evaluated surface under (x, y) via raycast, `None` outside. |
| `place_on(obj, surface, x, y, sink=0)` | Moves `obj` to (x, y) resting on the surface (`sink` buries the base, e.g. 0.1 on slopes). |
| `flatten_area(surface, x, y, radius, falloff=4, z=None)` | Levels a pad on terrain (default: height at centre) blending out over `falloff` m; returns pad z. Use before `place_on` for buildings/paths. |
| `clearing_mask(surface, [(x, y, radius), …], name="ScatterMask", falloff=2)` | Vertex group 0 inside clearings → 1 outside; pass `mask=name` to `geonodes.scatter`. |
| `height_mask(surface, min_z, max_z=None, falloff=0.5, name="HeightMask")` | Vertex group 0 below `min_z` (e.g. waterline) ramping to 1. |
| `cut_opening(obj, size, location, name="Opening")` | Boolean window/door hole; cutter is a hidden wire box in `FX`. |
| `cyclorama(width=4, depth=3, height=2, radius=0.6, material=None)` | Seamless studio backdrop (floor curving into a back wall on +Y). |
| `smooth_by_angle(obj, angle_deg=30)` | Replacement for removed auto smooth. |
| `ensure_mcp_toggles(scene=None)` | Poly Haven on; Poly Pizza / Sketchfab only if their API key env var is set. |
| `summary(scene=None)` | JSON-friendly overview (engine, camera, collections → objects + dimensions). |

## world

`hdri(path, strength=1, rotation_deg=0)` (local .hdr/.exr — Poly Haven HDRIs via MCP `import_asset` set the world themselves),
`sky(sun_elevation_deg=15, sun_rotation_deg=0, strength=1, sky_type="MULTIPLE_SCATTERING", altitude, air_density, aerosol_density, ozone_density, sun_intensity)`,
`solid(color, strength)`. Each rebuilds the world node tree.

## materials

| Function | Notes |
|---|---|
| `principled(name, preset=None, reuse=True, **inputs)` | Reuses only tks-made materials of that name. Socket names as in 5.2; legacy names (`Specular`, `Clearcoat`, `Transmission`, `Emission`, …) are aliased. Unknown names raise `KeyError` listing valid sockets. Use `**{"Base Color": (...)}` for names with spaces. |
| `PRESETS` | `plastic, rubber, ceramic, metal_steel, metal_gold, metal_copper, glass, water, wood, fabric, skin, foliage`. |
| `emission(name, color, strength)` | Lamps, screens, embers. |
| `assign(obj, mat, replace=True)` | Empties → applies to all mesh descendants. |
| `pbr_from_folder(name, folder, scale=1, displacement=0)` | ambientCG / Poly Haven style folders; colour sRGB, data maps Non-Color, prefers NormalGL (DX auto-flipped), optional displacement (sets `displacement_method="BOTH"`). |
| `find_maps(folder)` | Debug: which file was classified as which map. |
| `set_inputs(node, **values)` | Generic setter with aliasing and RGB→RGBA. |

## lighting

`light(kind="AREA"|"POINT"|"SPOT"|"SUN", name, location, energy (W; sun W/m²), color, size, target=(x,y,z) or object or None, temperature_k=None)`,
`sun(elevation_deg=35, azimuth_deg=135, strength=3, angle_deg=0.5)`,
`three_point(target, distance=4, key_energy=600, height=2.5)` → `{"key","fill","rim"}`.

## camera

`rig(location, target, lens=50 | "wide"|"standard"|"normal"|"portrait"|"tele"|"product", dof=False, fstop=2.8)` → `(camera, focus_empty)` named `name` and `f"{name}_Focus"` (default `Camera`, `Camera_Focus`); Track To the empty, which is also the DoF focus; sets `scene.camera`.
`frame_objects(cam, objs, margin=1.15)` dollies along the current view direction to fit objects.

## atmosphere

`world_fog(density=0.01, color, anisotropy=0.2)` (Cycles world volume, keep density ≤0.01 for outdoors),
`fog_box(size, location, density=0.02, …)` (bounded volume in `FX`; works in EEVEE too).

## assets

`import_model(path, name, target_size, axis="max"|"x"|"y"|"z", location, rotation_z_deg, collection="Props", on_ground=True)` → parent empty with origin at base centre, scaled so the chosen axis equals `target_size`. Formats: glb/gltf/fbx/obj/usd*/ply/stl/blend. Cleans up the stray scenes the 5.2 glTF importer creates.
`record_credit(asset_id, source, licence, author, url, notes)` → `scenes/assets/credits.json`.
`local_assets(kind=None)` lists downloaded folders under `scenes/assets/`.

## extensions

`status()` → `{id: enabled}`, `require(id)`,
`landscape(size=(60,60), subdivisions=(256,256), height=4, seed=0, noise_type="hetero_terrain", feature_size=None, island=False, island_edge=0.6, name="Terrain", **ant_kwargs)` — `height` is the exact relief in metres (base at z=0), `feature_size` the hill size in metres (default largest side / 4), `island=True` fades to a round coast,
`tree(seed, levels=3, show_leaves=True, location, collection, convert_to_mesh=False, bark=None, foliage=None, **sapling_kwargs)` → bevelled trunk curve with a `<name>_Leaves` mesh child (Sapling kwargs: `scale`, `ratio`, `leaves`, `leafScale`, `shape`, …),
`room(width=5, depth=4, height=2.7, wall_thickness=0.1, floor=True, ceiling=False, baseboard=True, centred=True)` → Archimesh room object (floor/ceiling/baseboard are children; floor centre at origin when `centred`),
`sun_position(latitude, longitude, month, day, hour, utc_zone, sun_object=None, sky_texture=None)`.

## geonodes

`list_essentials()`, `append_group(name)` (Essentials asset libraries bundled with Blender),
`inputs(group)` → `[(identifier, name, socket_type)]`, `set_input(mod, name, value, socket_type=None)` / `get_input(...)` (menu inputs take item names, e.g. `"Amount"`), `set_input_attribute(mod, name, attribute)` (drive a field input from a vertex group / attribute), `add_group_modifier(obj, group_or_name, **inputs)` (group may be your own `GeometryNodeTree`),
`scatter(surface, instance, count=None | density=None, seed, scale, random_scale=0.3, random_rotation_z_deg=180, align_to_normal=False, min_distance=0, collection=False, mask=None)` — uses "Scatter on Surface". To change the count later: `set_input(mod, "Amount", n)` (not `Count`); density: `set_input(mod, "Density", d)`. `collection=True` picks a random child of the collection per point: put each multi-object variant (trunk + leaves) in its own sub-collection, and exclude the kit collection from the view layer (`bpy.context.view_layer.layer_collection.children[kit.name].exclude = True`). For a single source object, hide it yourself (`obj.hide_render = obj.hide_viewport = True`).

## render

`preview(name=None, width=480, samples=16)` → PNG path under `renders/previews/` (settings restored afterwards),
`final(name=None, samples=None)` → `renders/<name>.png` at scene settings,
`grade(vignette=0.3, grain=0.0)` (compositor via Essentials Vignette / Film Grain; replaces a previous grade),
`save(name=None, copy=False)` → `scenes/<name>.blend`.
