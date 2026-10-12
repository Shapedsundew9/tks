---
name: blender-bpy-5x
description: Write correct bpy for this repo's Blender 5.2.2 LTS (Python 3.13) via the blender MCP server. Use before any execute_blender_code call - covers the tks_bpy helper library, Blender 4.x->5.x API changes (Principled sockets, Fac->Factor, sky types, compositor, auto smooth, geometry-nodes modifier inputs), introspection, and operator context.
---

# bpy for Blender 5.2 (this repo)

The Blender container runs **Blender 5.2.2 LTS with Python 3.13**. Training data and web examples
are mostly Blender 2.8–4.x and *will* be wrong in places. Do not guess APIs: use the helper library,
the verified change list, or introspect.

## 1. Always start with the helper library

Every `execute_blender_code` call that builds scenes should begin with:

```python
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T; T = T.reload_all()
```

`tks_bpy` is tested live against this Blender (see `scripts/blender/test_tks_bpy_live.py`). It
covers scene setup, world (HDRI/sky/solid), Principled materials with legacy-name aliases, PBR from
texture folders, lights, camera rigs, fog, asset import at real-world scale, the pinned
extensions, Geometry Nodes Essentials (scatter), previews, compositor grading and saving.
Full API: [references/tks_bpy.md](references/tks_bpy.md). If a helper is missing, write plain bpy
following the rules below — do not edit `tks_bpy` mid-scene unless asked.

## 2. Version traps (verified on 5.2.2)

| Old code you will be tempted to write | Blender 5.2 reality |
|---|---|
| `bsdf.inputs["Specular"]`, `"Subsurface"`, `"Transmission"`, `"Clearcoat"`, `"Sheen"`, `"Emission"` | `"Specular IOR Level"`, `"Subsurface Weight"`, `"Transmission Weight"`, `"Coat Weight"`, `"Sheen Weight"`, `"Emission Color"` (+ `"Emission Strength"`) |
| `outputs["Fac"]` / `inputs["Fac"]` on Noise, Wave, Gradient, Mix, MixShader, ColorRamp, HueSat | `"Factor"` |
| `ShaderNodeTexMusgrave` | removed — use Noise texture (`noise_type` FBM/MULTIFRACTAL/…) |
| `sky.sky_type = "NISHITA"` | `MULTIPLE_SCATTERING` (default), `SINGLE_SCATTERING`, `PREETHAM`, `HOSEK_WILKIE` |
| `scene.node_tree` / `scene.use_nodes` for compositing | `scene.compositing_node_group = bpy.data.node_groups.new(name, "CompositorNodeTree")` with a `NodeGroupOutput` + interface output socket → use `T.render.grade()` |
| `mesh.use_auto_smooth = True` | removed — `T.scene.smooth_by_angle(obj, 30)` (Essentials "Smooth by Angle" modifier) |
| `mod["Socket_2"] = 5` on a Geometry Nodes modifier | raises *"doesn't support IDProperties"* — `T.geonodes.set_input(mod, "Name", value)` (uses `mod.properties.inputs.<identifier>.value`) |
| `"BLENDER_EEVEE_NEXT"` | `"BLENDER_EEVEE"` (engines: `BLENDER_EEVEE`, `BLENDER_WORKBENCH`, `CYCLES`) |
| `view_transform = "Filmic"` as default | default is `"AgX"`; looks are `"AgX - Punchy"`, `"AgX - High Contrast"`, … |
| `action.fcurves` | removed in 5.0 (slotted actions) — `obj.keyframe_insert(...)`; see upstream `animation.md` |
| `bpy.ops.import_scene.obj` | `bpy.ops.wm.obj_import` (also `wm.ply_import`, `wm.stl_import`, `wm.usd_import`; glTF/FBX stay `import_scene.*`) |
| `bpy.ops.preferences.addon_enable("add_mesh_extra_objects")` | legacy add-ons are **extensions** now: `bl_ext.user_default.<id>`; they are pre-installed — see the `blender-extensions` skill |
| `nodes.new("ShaderNodeBSDFPrincipled")`, `"ShaderNodeEmissionShader"` | idnames are CamelCase: `ShaderNodeBsdfPrincipled`, `ShaderNodeEmission`, `ShaderNodeBsdfGlass`, `ShaderNodeTexImage`. Prefer `T.materials.*`; to check a name: `hasattr(bpy.types, "ShaderNodeBsdfPrincipled")` |
| `mix.inputs["A"]`, `mix.inputs["B"]` on `ShaderNodeMix` | socket names are duplicated; use index access: `inputs[0]`=Factor, `inputs[2]`=A, `inputs[3]`=B for Float; `inputs[6]`=A, `inputs[7]`=B for RGBA |
| `glare.glare_type = "BLOOM"` on `CompositorNodeGlare` | glare properties are input sockets: `inputs["Type"].default_value = "Bloom"`, `inputs["Threshold"]`, `inputs["Size"]`, `inputs["Strength"]` |
| `light.light_linking` collection assignment | use `light_obj.light_linking.receiver_collection = coll` |
| `bpy.ops.export_scene.gltf(filepath=...)` | defaults `use_active_scene=False` exporting all scenes; pass `use_active_scene=True` or `use_selection=True` |
| `fill_node.mode = "NGONS"` on `GeometryNodeFillCurve` | mode is an input socket: `fill.inputs["Mode"].default_value = "N-gons"` |
| `stc.font = font_obj` on `GeometryNodeStringToCurves` | font is an input socket: `stc.inputs["Font"].default_value = font_obj`; output socket is `'Curve Instances'` |
| After a failed snippet, retrying the same `new()` calls | partial state persists (half-built materials, `.001` duplicates). Inspect with `T.scene.summary()` and delete or reuse what the failed call made |

### PBR Sanity & Lookdev Rules in 5.2
- **Brushed Metal Anisotropy**: Radial Z tangent (`tangent.direction_type = "RADIAL"`, `axis = "Z"`) produces horizontal brushed sheen on vertical faces without UV unwrapping.
- **Atmosphere in Hero Void**: Never attach global `Principled Volume` to World output in dark void scenes (it veils contrast); use bounded local `fx.smoke_volume` and `fx.dust_motes`.
- **Light Linking in Cycles**: Use `lighting.link_light` to isolate high-energy rims and softboxes to hero receivers without hazing background atmosphere.

More detail and the full Principled/sky/volume socket lists: [references/api-changes.md](references/api-changes.md).

## 3. Introspect instead of guessing

```python
node.inputs.keys()                                     # sockets actually present
[p.identifier for p in bpy.types.ShaderNodeTexSky.bl_rna.properties]
[(p.identifier, p.type) for p in bpy.ops.mesh.landscape_add.get_rna_type().properties]
try: scene.render.engine = "?"                         # dynamic enums under-report via RNA;
except TypeError as e: print(e)                        # the error lists the accepted values
T.geonodes.inputs(mod.node_group)                      # [(identifier, name, socket_type)]
```

Print small JSON summaries, not whole objects. `T.scene.summary()` is a compact scene overview.

## 4. Execution rules

- Code runs in the GUI process from a timer: prefer `bpy.data` APIs over `bpy.ops`. When an
  operator needs a 3D view, wrap it in `bpy.context.temp_override(area=..., region=...)` (see
  upstream `bpy.md`). The pinned extension operators used by `tks_bpy` work without overrides.
- Units are metres; build at real-world size. Name every object; keep things in the
  `Environment / Props / Lighting / Cameras / FX` collections created by `T.scene.new_scene`.
- Keep calls small (one stage per call) and end each with a printed JSON result.
- `mathutils`, `bmesh` and `bpy_extras` are separate top-level modules (`from mathutils import
  Vector`), not `bpy.mathutils`.
- **GPU budget:** the 8 GB RTX 3080 is shared with Ollama. Use `T.render.preview()` (≤480 px,
  16 samples) while iterating; only call `T.render.final()` when the user asks for the final.
- `bpy.app.online_access` is **off** inside Blender. Downloads happen in the MCP server
  (`import_asset`) or via repo scripts in the devcontainer (`fetch_ambientcg.py`).
- Never `bpy.ops.wm.read_homefile`/`read_factory_settings` or delete scenes you didn't create —
  the user may have unsaved work. Make a new scene with `T.scene.new_scene(...)` instead.
- Save work with `T.render.save("name")` → `/workspaces/tks/scenes/name.blend` (gitignored).

## 5. Further reading

- [references/tks_bpy.md](references/tks_bpy.md) — helper API.
- [references/api-changes.md](references/api-changes.md) — verified 5.2 facts.
- [references/upstream-guides/](references/upstream-guides/README.md) — MIT guides from
  mcp-for-blender: `bpy.md`, `scene.md`, `materials.md`, `level-design.md`, `animation.md`,
  `rigging.md`, `retopology.md`.
