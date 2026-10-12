---
name: blender-scene-director
description: Staged pipeline for turning a scene brief into a finished Blender render via the blender MCP server - routes the brief to one of five tested paths (outdoor/nature, interior/archviz, product/studio, stylised low-poly, procedural/geometry nodes), then blockout, assets, materials, lighting, atmosphere, camera, preview critique loops and final render. Use for any "make/build/render a scene" request.
---

# Scene director

Use with `blender-bpy-5x` (API rules + `tks_bpy`), `blender-assets` and `blender-extensions`.
Work in small, verifiable stages; never write one giant script.

## 0. Orient (once per session)

1. `get_addon_status` → Blender version (expect 5.2.x) and which asset sources are enabled.
2. `get_scene_info` → what is open. **Never** delete or clear scenes you didn't create; build in a
   new scene via `T.scene.new_scene("<short_name>")` (it refuses to clear user scenes).

## 1. Route the brief

| Brief mentions… | Path | Recipe |
|---|---|---|
| landscape, forest, mountains, field, beach, campfire, cabin exterior, weather, time of day | Outdoor / nature | [references/outdoor.md](references/outdoor.md) |
| room, interior, kitchen, office, archviz, apartment, furniture layout, window light | Interior / archviz | [references/interior.md](references/interior.md) |
| product, packshot, hero shot, studio, turntable, bottle, mug, watch, jewellery | Product / studio | [references/product.md](references/product.md) |
| low-poly, stylised, cartoon, toy, diorama, isometric, game-like, cute | Stylised low-poly | [references/lowpoly.md](references/lowpoly.md) |
| procedural, generative, pattern, abstract, scatter, geometry nodes, motion-graphics | Procedural / GN | [references/procedural.md](references/procedural.md) |

Mixed briefs: pick the path that owns the **camera and lighting** (e.g. "cabin interior at night"
→ interior) and borrow steps from others. Read the chosen recipe before writing code.

## 2. Write a scene plan (show it to the user in 5–10 lines)

Subject and story · path · style (photoreal or stylised — never both) · engine (Cycles default;
EEVEE for stylised/fast) · camera (position, lens, aspect) · key light and mood · hero assets
with real sizes and source · materials · atmosphere. If the brief is ambiguous on something that
changes the whole approach (style, time of day, aspect ratio), ask once; otherwise decide and go.

## 3. Build in stages, checking each

| Stage | Do | Checkpoint |
|---|---|---|
| Setup | `T.scene.new_scene(...)`; world placeholder | `print(T.scene.summary())` |
| Blockout | Ground/terrain/room + boxes at **real size** for every major element; camera rig **now** | `look(mode="camera")` — composition, scale, horizon |
| Environment & key light | HDRI or sky or studio; key light; sun position | `T.render.preview()` (480 px, 16 spp), view the PNG |
| Hero assets | Replace blockout boxes biggest-first (`blender-assets`); credit them | `look(mode="angles", target=[...])` — scale, grounding, facing |
| Materials | PBR (ambientCG/Poly Haven) on big surfaces, presets elsewhere | preview |
| Dressing | Secondary props, scatter (`T.geonodes.scatter`), variation | preview |
| Atmosphere & grade | fog/volume, `T.render.grade()` | preview |
| Final | Only when the user is happy or asks: raise samples/resolution, `T.render.final()`, `T.render.save()` | report paths + CC-BY credits |

Each stage = one or two `execute_blender_code` calls starting with the `tks_bpy` import header,
ending in a printed JSON summary. Fix errors at the stage they occur; don't push on.

## 4. Critique every preview (max 3 loops per stage)

Look at the actual image, then check:

- **Scale**: doors ~2 m, people 1.7 m, props sensible relative to each other.
- **Grounding**: nothing floating or sunk; contact shadows present.
- **Composition**: clear subject, rule of thirds, foreground/mid/background, horizon not centred,
  nothing important cut by the frame edge.
- **Light**: readable shape (key vs fill), no blown-out whites or crushed blacks, colour
  temperature matches the mood (warm practicals vs cool sky). Emissive surfaces should keep their
  colour (if they clip to white, lower the strength). Saturation should fit the brief (stylised →
  `"AgX - Punchy"` look).
- **Materials**: no pure black/white base colours, visible roughness variation, no stretched
  textures or obvious tiling.
- **Noise**: a noisy 16-spp preview is fine; judge composition and light, not grain.

State what you'll change, change one or two things, preview again.

## 5. Budgets and safety

- GPU memory is shared with a local LLM: previews ≤ 480 px / 16 spp, textures 1k–2k, ANT
  subdivisions ≤ 256², scatter counts in the hundreds (instances, not realised geometry).
- Finals: 1920×1080 at 128–256 spp unless the user asks for more.
- Outputs: previews `renders/previews/`, finals `renders/`, scenes `scenes/` (all gitignored).
- Downloads only through MCP tools or `scripts/blender/fetch_ambientcg.py`; record credits.
