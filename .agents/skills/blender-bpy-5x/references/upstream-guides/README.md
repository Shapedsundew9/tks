# Upstream guides (vendored)

Copied verbatim from the `mcp-for-blender` 2.1.9 Python package (`blender_mcp/guides/`),
© 2025 Siddharth Ahuja, MIT licence (see [LICENSE](LICENSE)). The package ships them but its
`get_guide` tool is disabled, so they are vendored here for agents to read directly.

Adaptations for this repository (the files themselves are unmodified):

- References to `get_guide("...")` mean the sibling file here (e.g. `get_guide("bpy")` → `bpy.md`).
- This repo pins **Blender 5.2.2 LTS**. Where a guide hedges across versions, follow
  [../api-changes.md](../api-changes.md), which was verified against the running build.
- Prefer the repo helpers in `scripts/blender/tks_bpy` (see [../tks_bpy.md](../tks_bpy.md))
  over re-writing the snippets shown in these guides.
