# Blender 5.2.2 API facts (verified in this repo's container)

Verified by introspection of the running build (Blender 5.2.2 LTS, Python 3.13.13, OptiX GPU).
When something here disagrees with a tutorial, this file wins; when it is silent, introspect.

## Render / colour

- Engines: `BLENDER_EEVEE`, `BLENDER_WORKBENCH`, `CYCLES` (RNA `enum_items` lists only the static
  item; assign a bad value and read the `TypeError` for the full list).
- View transforms: `Standard`, `ACES 1.3`, `ACES 2.0`, `Khronos PBR Neutral`, `AgX` (default),
  `Filmic`, `Filmic Log`, `False Color`, `Raw`. Looks are prefixed: `"AgX - Punchy"`,
  `"AgX - High Contrast"`, `"AgX - Medium High Contrast"`, …
- Cycles defaults: 4096 samples / 1024 preview, OpenImageDenoise. Always lower them.
- Colour spaces include `sRGB` and `Non-Color`.

## Principled BSDF inputs (5.2)

Base Color, Metallic, Roughness, IOR, Alpha, Thin Wall, Normal, Diffuse Roughness, Subsurface Weight,
Subsurface Radius, Subsurface Scale, Subsurface Anisotropy, Specular IOR Level, Specular Tint,
Anisotropic, Anisotropic Rotation, Tangent, Transmission Weight, Coat Weight, Coat Roughness,
Coat IOR, Coat Tint, Coat Normal, Sheen Weight, Sheen Roughness, Sheen Tint, Emission Color,
Emission Strength, Thin Film Thickness, Thin Film IOR. (`Weight` and `Subsurface IOR` exist but
are disabled/hidden in the default mode.)

New materials and worlds already contain Principled BSDF + Material Output / Background + World Output.

## Other shader nodes

- `Fac` → `Factor` on Noise / Wave / Gradient outputs and Mix, Mix Shader, Color Ramp,
  Hue/Saturation inputs.
- Voronoi outputs: `Distance`, `Color`, `Position`. Musgrave removed (Noise texture has the modes).
- Sky texture `sky_type`: `SINGLE_SCATTERING`, `MULTIPLE_SCATTERING`, `PREETHAM`, `HOSEK_WILKIE`
  (no `NISHITA`). Properties: `sun_disc, sun_size, sun_intensity, sun_elevation, sun_rotation,
  altitude, air_density, aerosol_density, ozone_density, sun_direction, turbidity, ground_albedo`.
  Output `Color`.
- Principled Volume inputs: Color, Density, Anisotropy, Absorption Color, Emission Strength, …;
  Volume Scatter: Color, Density, Anisotropy.
- Material settings: `surface_render_method`, `displacement_method`, `blend_method`, `thickness_mode`.

## Compositor

`scene.node_tree` is gone. Create a `CompositorNodeTree` node group, add an output interface
socket (`tree.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")`),
a `CompositorNodeRLayers` and a `NodeGroupOutput`, then `scene.compositing_node_group = tree`.
Essentials compositor groups (Vignette, Film Grain, Tune Image, Chromatic Aberration, Sensor Noise,
Split Toning, Sepia, …) are appended from `bpy.utils.system_resource("DATAFILES")/assets/nodes/`
and used via `CompositorNodeGroup`. `T.render.grade()` does all of this.

## Meshes / modifiers / geometry nodes

- `Mesh.use_auto_smooth` removed: `bpy.ops.object.shade_auto_smooth()` (needs context) or the
  Essentials "Smooth by Angle" modifier (`T.scene.smooth_by_angle`).
- Geometry Nodes modifier inputs: `mod["Socket_N"]` raises; use
  `getattr(mod.properties.inputs, identifier).value = v`. Identifiers come from
  `group.interface.items_tree` (`item_type == "SOCKET"`, `in_out == "INPUT"`). Names can repeat
  with different types ("Randomize Scale" is both Vector and Float on Scatter on Surface) —
  disambiguate by `socket_type`. Menu sockets take item names as strings.
- Essentials node groups (`geometry_nodes_essentials.blend`): Scatter on Surface, Randomize
  Transforms, Smooth by Angle, Array, Displace Geometry, Instance on Elements, Curve to Tube, …;
  also `procedural_hair_node_assets.blend`, `geometry_nodes_dynamics_assets.blend`,
  `compositing_nodes_essentials.blend`, `shading_nodes_essentials.blend`. Appended asset groups
  carry fake users — clear them (`T.geonodes.append_group` does).

## Import / export

`bpy.ops.import_scene.gltf`, `import_scene.fbx`, `wm.obj_import`, `wm.usd_import`,
`wm.ply_import`, `wm.stl_import`. The 5.2 glTF importer can create extra empty scenes; remove
them (`T.assets.import_model` does).

## Camera / lights

Camera DoF: `cam.data.dof.use_dof, focus_object, focus_distance, aperture_fstop, aperture_blades`.
Light types `POINT, SUN, SPOT, AREA`; lights support `use_temperature` + `temperature` (Kelvin).

## Add-ons / extensions

Legacy bundled add-ons became extensions (`bl_ext.<repo>.<id>`). Only these remain bundled:
cycles, io_* (fbx, gltf2, svg, bvh, uv_layout), node_wrangler, pose_library, rigify, hydra_storm,
ui_translate, viewport_vr_preview. The repo pre-installs the community set listed in the
`blender-extensions` skill.

## Scripting pitfalls verified in this repo

- `depsgraph.object_instances` items are only valid during iteration — copy what you need
  (`inst.matrix_world.translation.copy()`) inside the loop, or you get
  `ReferenceError: StructRNA of type DepsgraphObjectInstance has been removed`.
- bmesh face order after several `bmesh.ops.create_*` calls is not creation order: select parts
  from the verts each op returns (`{f for v in res["verts"] for f in v.link_faces}`).
- Geometry node groups built in Python: `tree.interface.new_socket(name, in_out, socket_type)`;
  shader math/texture nodes (`ShaderNodeTexNoise`, `ShaderNodeMath`, `ShaderNodeMapRange`) work in
  geometry trees; attributes stored on the `INSTANCE` domain survive `RealizeInstances`.
- Modifier field inputs can read attributes: `item = getattr(mod.properties.inputs, identifier)`,
  `item.type = "ATTRIBUTE"; item.attribute_name = "MyVertexGroup"` (`T.geonodes.set_input_attribute`).
- `node.type` enum values differ from idnames: Principled BSDF is `BSDF_PRINCIPLED`
  (`ShaderNodeBsdfPrincipled`) but Principled Volume is `PRINCIPLED_VOLUME`
  (`ShaderNodeVolumePrincipled`). Prefer `node.bl_idname` or `T.materials` helpers.
- Shader sockets (`NodeSocketShader`) have no `default_value`; guard with
  `hasattr(sock, "default_value")` when dumping node inputs.
- Mesh-data edits (vertex groups, `v.co`) fail in Edit Mode; call
  `bpy.ops.object.mode_set(mode="OBJECT")` after edit-mode operators (UV projects, LoopTools). The
  `tks_bpy` terrain helpers do this automatically.
- The MCP `execute_blender_code` namespace is fresh on every call; variables do not persist.
