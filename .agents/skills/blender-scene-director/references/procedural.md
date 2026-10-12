# Procedural / Geometry Nodes path

Build a custom Geometry Nodes tree from Python (Blender 5.2 interface API), drive a material
from a stored attribute, and combine it with Essentials node groups on extension-made curves.
Example brief: *"an abstract procedural landscape of glowing columns"*. Each block is one
`execute_blender_code` call (fresh namespace; re-find objects with `T.scene.find`). Blocks are
executed by `scripts/blender/test_recipes_live.py`.

**Prefer Essentials first:** `T.geonodes.list_essentials()` lists ready-made groups (Scatter on
Surface, Array, Curve to Tube, Instance on Elements, Randomize Transforms, Displace Geometry,
Smooth Geometry, hair tools…). Use `add_group_modifier(obj, "<name>", **inputs)` and
`T.geonodes.inputs(group)` to discover input names. Write a custom tree only for what they
don't cover.

## Step 1 — custom node tree: noise-driven columns

5.x API facts used here: sockets live on `tree.interface.new_socket(name, in_out, socket_type)`
(`tree.inputs/outputs` are gone); shader texture nodes such as `ShaderNodeTexNoise` and
`ShaderNodeMapRange` work inside geometry trees; an attribute stored on the `INSTANCE` domain
survives `RealizeInstances` and is readable in materials with an Attribute node.

```python
# recipe: procedural step 1
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T; T = T.reload_all()
sc = T.scene.new_scene("noise_columns", resolution=(1600, 1600), samples=64)
tree = bpy.data.node_groups.new("TKS_NoiseColumns", "GeometryNodeTree")
tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
for name, kind, default in [("Size", "NodeSocketFloat", 10.0), ("Count", "NodeSocketInt", 24),
                            ("Height", "NodeSocketFloat", 3.0), ("Noise Scale", "NodeSocketFloat", 0.25),
                            ("Material", "NodeSocketMaterial", None)]:
    item = tree.interface.new_socket(name, in_out="INPUT", socket_type=kind)
    if default is not None:
        item.default_value = default
N, L = tree.nodes, tree.links
gin, gout = N.new("NodeGroupInput"), N.new("NodeGroupOutput")
grid = N.new("GeometryNodeMeshGrid")
for axis in "XY":
    L.new(gin.outputs["Size"], grid.inputs[f"Size {axis}"])
    L.new(gin.outputs["Count"], grid.inputs[f"Vertices {axis}"])
noise = N.new("ShaderNodeTexNoise")
L.new(N.new("GeometryNodeInputPosition").outputs["Position"], noise.inputs["Vector"])
L.new(gin.outputs["Noise Scale"], noise.inputs["Scale"])
remap = N.new("ShaderNodeMapRange")  # noise Fac sits around 0.3-0.75; spread it to 0.03-1
remap.inputs["From Min"].default_value, remap.inputs["From Max"].default_value = 0.3, 0.75
remap.inputs["To Min"].default_value = 0.03
L.new(noise.outputs["Fac"], remap.inputs["Value"])
height = N.new("ShaderNodeMath")
height.operation = "MULTIPLY"
L.new(remap.outputs["Result"], height.inputs[0])
L.new(gin.outputs["Height"], height.inputs[1])
cell = N.new("ShaderNodeMath")
cell.operation = "DIVIDE"
L.new(gin.outputs["Size"], cell.inputs[0])
L.new(gin.outputs["Count"], cell.inputs[1])
scale = N.new("ShaderNodeCombineXYZ")
L.new(cell.outputs[0], scale.inputs["X"])
L.new(cell.outputs[0], scale.inputs["Y"])
L.new(height.outputs[0], scale.inputs["Z"])
lift = N.new("GeometryNodeTransform")  # unit cube with its base at z=0
lift.inputs["Translation"].default_value = (0, 0, 0.5)
L.new(N.new("GeometryNodeMeshCube").outputs["Mesh"], lift.inputs["Geometry"])
inst = N.new("GeometryNodeInstanceOnPoints")
L.new(grid.outputs["Mesh"], inst.inputs["Points"])
L.new(lift.outputs["Geometry"], inst.inputs["Instance"])
L.new(scale.outputs["Vector"], inst.inputs["Scale"])
store = N.new("GeometryNodeStoreNamedAttribute")
store.data_type, store.domain = "FLOAT", "INSTANCE"
store.inputs["Name"].default_value = "height"
L.new(inst.outputs["Instances"], store.inputs["Geometry"])
L.new(remap.outputs["Result"], store.inputs["Value"])
realize = N.new("GeometryNodeRealizeInstances")
L.new(store.outputs["Geometry"], realize.inputs["Geometry"])
set_mat = N.new("GeometryNodeSetMaterial")
L.new(realize.outputs["Geometry"], set_mat.inputs["Geometry"])
L.new(gin.outputs["Material"], set_mat.inputs["Material"])
L.new(set_mat.outputs["Geometry"], gout.inputs["Geometry"])

mat = T.materials.principled("Columns", Roughness=0.35)
nt = mat.node_tree
attr = nt.nodes.new("ShaderNodeAttribute")
attr.attribute_name = "height"
ramp = nt.nodes.new("ShaderNodeValToRGB")
ramp.color_ramp.elements[0].color, ramp.color_ramp.elements[0].position = (0.05, 0.1, 0.4, 1), 0.1
ramp.color_ramp.elements[1].color, ramp.color_ramp.elements[1].position = (1.0, 0.45, 0.1, 1), 0.9
bsdf = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
nt.links.new(attr.outputs["Fac"], ramp.inputs["Fac"])
nt.links.new(ramp.outputs["Color"], bsdf.inputs["Base Color"])

holder = T.scene.box("Columns", (0.01, 0.01, 0.01))
T.geonodes.add_group_modifier(holder, tree, **{"Size": 12.0, "Count": 30, "Height": 4.0, "Material": mat})
ev = holder.evaluated_get(bpy.context.evaluated_depsgraph_get())
print({"verts": len(ev.data.vertices), "dims": tuple(round(d, 2) for d in ev.dimensions)})
```

## Step 2 — extension curve + Essentials "Curve to Tube"

`curve.spirals` (Extra Curve Objects) makes the path; `edit_mode=False` keeps the socket call in
object mode. The new object is found by diffing `bpy.data.objects`.

```python
# recipe: procedural step 2
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T
before = set(bpy.data.objects)
bpy.ops.curve.spirals(spiral_type="ARCH", turns=3, steps=64, radius=5.0, dif_z=1.0, curve_type="POLY",
                      edit_mode=False, location=(0, 0, 4.5))
spiral = next(o for o in bpy.data.objects if o not in before)
spiral.name = "Spiral"
T.scene.link(spiral, "Props")
T.geonodes.add_group_modifier(spiral, "Curve to Tube", **{"Scale": 0.12, "Profile Resolution": 16, "Shade Smooth": True})
T.materials.assign(spiral, T.materials.emission("NeonTube", (0.2, 0.9, 1.0), 8))
print(T.scene.summary())
```

## Step 3 — dark studio lighting, camera, grade

```python
# recipe: procedural step 3
import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
import tks_bpy as T
T.world.solid((0.01, 0.01, 0.015))
T.lighting.light("AREA", "Key", (6, -6, 9), 800, size=4, target=(0, 0, 1))
T.lighting.light("AREA", "Rim", (-7, 7, 5), 600, size=3, target=(0, 0, 1), temperature_k=9000)
T.camera.rig((16, -16, 13), (0, 0, 2), lens=50)
T.render.grade(vignette=0.35)
print(T.render.preview("noise_columns", width=400))
```

## Critique hints for this path

- Expose the knobs (size, count, height, scale, seed) as group inputs; tweak via
  `T.geonodes.set_input(mod, "Height", 5.0)` rather than rebuilding the tree.
- Keep instance counts in the thousands and realise only when a later node needs real geometry.
- Use stored attributes + Attribute/Color Ramp nodes for colour variation instead of many materials.
