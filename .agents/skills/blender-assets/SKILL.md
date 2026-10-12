---
name: blender-assets
description: Find, download, import and credit community 3D assets for Blender scenes in this repo - Poly Haven (HDRIs, PBR textures, models; MCP), ambientCG (CC0 PBR materials; repo fetch script), Poly Pizza and Sketchfab (MCP, need API keys), local files - with real-world sizing, style matching and the scenes/assets/credits.json licence ledger.
---

# Community assets

Blender itself has **no internet** (`online_access` off). Assets arrive through the MCP server's
`search_assets` / `import_asset` tools or through repo scripts run in the devcontainer shell.
Everything downloaded lives under `/workspaces/tks/scenes/assets/` (gitignored, shared with
Blender). Never commit assets.

## Choose a source

| Need | Source | How |
|---|---|---|
| Lighting / reflections (HDRI) | Poly Haven (CC0) | `search_assets(source="polyhaven", asset_type="hdris", query=...)` → `import_asset(source="polyhaven", id=..., asset_type="hdris", resolution="2k")` (becomes the world) |
| Surface materials for floors, walls, terrain, wood, metal | Poly Haven textures (CC0) **or** ambientCG (CC0) | Poly Haven: `import_asset(..., asset_type="textures", apply_to=["Floor"], resolution="2k")`. ambientCG: shell `python3 scripts/blender/fetch_ambientcg.py search "<words>"` then `... download <Id> --res 2K-JPG`, then in Blender `T.materials.pbr_from_folder("<Id>", "/workspaces/tks/scenes/assets/ambientcg/<Id>", scale=...)` + `T.materials.assign(obj, mat)` |
| Realistic generic props (furniture, rocks, plants, tools) | Poly Haven models (CC0) | `search_assets(source="polyhaven", asset_type="models", query=..., previews=3)` → `import_asset(..., asset_type="models")` |
| Stylised low-poly props | Poly Pizza (CC0 / CC-BY) | Needs `BLENDERMCP_POLYPIZZA_API_KEY`. `search_assets(source="polypizza", query=..., licence="CC0")` → `import_asset(source="polypizza", id=..., target_size=...)` |
| A specific real-world object (named car, landmark) | Sketchfab (licence varies) | Needs `BLENDERMCP_SKETCHFAB_API_KEY`. Accept only CC0 / CC-BY (CC-BY-SA if the user agrees); **never** NC/ND or editorial without asking. `target_size` is required. |
| Procedural/simple geometry (walls, shelves, fences, terrain, trees, gems) | Script it | `tks_bpy` + pinned extensions (see `blender-extensions`) |
| A file the user supplies (glb/gltf/fbx/obj/usd/ply/stl/blend) | Local | `T.assets.import_model(path, target_size=..., axis=...)` |
| One-off hero object from text | `generate_3d` | Only if `get_addon_status` reports a configured provider; otherwise say so and fall back to library assets |

Run `get_addon_status` first: it shows which sources are enabled. Poly Pizza and Sketchfab are
switched on automatically when their keys are present in the Blender container environment
(`.devcontainer/.env` or the caller env → `docker-compose.blender.yml`). If a key is missing,
**don't** ask the user to paste it into chat; tell them to add it to `.devcontainer/.env` and
restart the Blender service.

If a source's toggle looks off after creating a new scene, call `T.scene.ensure_mcp_toggles()`.

## Search well

- Use plain descriptive words; Poly Haven understands synonyms. Use `previews=3` to check
  thumbnails before importing — it is cheaper than importing the wrong thing.
- Large surfaces: `min_size_m=2` (Poly Haven) or ambientCG `Ground*/Rock*/Planks*/Tiles*`
  materials so tiling isn't obvious.
- Resolution: 1k–2k for background and previews, 4k only for close-up hero surfaces (VRAM is
  shared with a local LLM).
- **Match the style**: photoreal (Poly Haven, ambientCG, Sketchfab scans) *or* stylised
  (Poly Pizza, flat colours). Don't mix them in one frame.

## After every import

1. Read the bounding box from the import result (or `T.scene.world_bbox(obj)`).
2. Scale to real size: pass `target_size` (MCP) or use `T.assets.import_model(..., target_size=, axis=)`.
   Reference heights: door 2.0–2.1 m, ceiling 2.4–2.7 m, person 1.7 m, dining table 0.75 m,
   chair seat 0.45 m / back 0.9 m, sofa 0.85 m, bed 0.5 m, mug 0.1 m, bottle 0.25–0.3 m,
   phone 0.15 m, car 1.5 m tall / 4.5 m long, deciduous tree 8–15 m, conifer 10–30 m,
   bush 0.5–1.5 m, rock 0.2–2 m, campfire log 0.5–1 m.
3. Ground it: `T.scene.drop_to_ground(obj)`; rotate to face the camera; check intersections.
4. Put it in the right collection (`T.scene.link(obj, "Props")`).
5. `look(mode="angles", target=[obj.name])` once to confirm orientation and scale.
6. Repeated props: `obj.copy()` (shared mesh) or scatter a collection — never re-import.

## Credits ledger (mandatory)

Every downloaded asset must have an entry in `/workspaces/tks/scenes/assets/credits.json`:

- `fetch_ambientcg.py` records its downloads automatically.
- For MCP imports (Poly Haven, Poly Pizza, Sketchfab) record it yourself right after import:
  `T.assets.record_credit("<id>", "polyhaven", "CC0-1.0", author="<author>", url="https://polyhaven.com/a/<id>")`.
  For CC-BY assets the author and URL are **required** — copy them from the search result.
- When delivering a final render, list CC-BY credits from the ledger in your reply.

## Offline fallback

If a source is unavailable, say which and continue with what works: procedural materials
(`T.materials.principled(..., preset=...)` plus noise), scripted geometry, extensions, or
previously downloaded assets (`T.assets.local_assets()`).
