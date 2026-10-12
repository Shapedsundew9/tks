"""Rock clustering and procedural bedrock rock generators."""

import math
from pathlib import Path
import random
import bpy
import bmesh
from mathutils import Vector

from . import scene as _scene
from . import assets as _assets
from . import materials as _materials


def faceted_rock(name="FacetedRock", size=(1.0, 1.0, 0.8), subdivisions=2, roughness=0.35, seed=42, collection="Environment"):
    """Procedural faceted rock generator using subdivided icosphere with vertex jitter."""
    coll = collection if isinstance(collection, bpy.types.Collection) else _scene.ensure_collection(collection)
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()

    bmesh.ops.create_icosphere(bm, subdivisions=subdivisions, radius=0.5)

    rng = random.Random(seed)
    for v in bm.verts:
        disp = (rng.random() - 0.5) * roughness
        v.co += v.co.normalized() * disp
        v.co.x *= size[0]
        v.co.y *= size[1]
        v.co.z *= size[2]

    bm.to_mesh(me)
    bm.free()

    obj = bpy.data.objects.new(name, me)
    coll.objects.link(obj)

    # Shade smooth by angle for crisp facets
    _scene.smooth_by_angle(obj, 35)
    return obj


def cluster(source_obj_or_path=None, target_center=(0.0, 0.0, 0.0), count=10,
            ring_radius=(0.55, 1.4), scale_range=(0.4, 1.2), z_sink=(0.05, 0.25),
            collection="Environment", seed=42):
    """Instantiate a natural cluster of rocks distributed around target_center."""
    coll = collection if isinstance(collection, bpy.types.Collection) else _scene.ensure_collection(collection)
    rng = random.Random(seed)

    source_mesh = None
    source_mat = None
    temp_imported = []

    # 1. Resolve source mesh
    if isinstance(source_obj_or_path, (str, Path)):
        p = Path(source_obj_or_path)
        if p.exists():
            root = _assets.import_model(p, name="RockSourceRoot", target_size=1.0, collection=coll)
            temp_imported.append(root)
            # Find the best mesh child (prefer boulder/rock in name, else largest mesh)
            mesh_children = [c for c in root.children if c.type == "MESH"]
            temp_imported.extend(root.children)
            if mesh_children:
                boulder_match = next((c for c in mesh_children if "boulder" in c.name.lower() or "rock" in c.name.lower()), None)
                chosen = boulder_match or mesh_children[0]
                source_mesh = chosen.data
                if chosen.data.materials:
                    source_mat = chosen.data.materials[0]

    elif isinstance(source_obj_or_path, bpy.types.Object):
        if source_obj_or_path.type == "MESH":
            source_mesh = source_obj_or_path.data
            if source_obj_or_path.data.materials:
                source_mat = source_obj_or_path.data.materials[0]

    if source_mesh is None:
        # Fallback to procedural faceted rock
        base_rock = faceted_rock(name="ProceduralRockSource", seed=seed, collection=coll)
        temp_imported.append(base_rock)
        source_mesh = base_rock.data
        source_mat = _materials.principled(
            "DarkSlate_Procedural",
            **{"Base Color": (0.04, 0.04, 0.05, 1.0), "Roughness": 0.42, "Metallic": 0.05}
        )

    # Restyle rock material to dark slate with wet glints if present
    if source_mat is not None and not source_mat.get("tks_restyled"):
        _materials.restyle(source_mat, saturation=0.15, value=0.22, roughness_range=(0.32, 0.52), normal_strength=1.8)
        source_mat["tks_restyled"] = True

    # 2. Spawn instances around target_center
    rocks = []
    tc = Vector(target_center)
    for i in range(count):
        # Evenly spaced angular sectors with jitter for 360-degree coverage
        base_angle = 2.0 * math.pi * i / count
        angle = base_angle + rng.uniform(-0.35, 0.35)
        r = rng.uniform(ring_radius[0], ring_radius[1])

        x = tc.x + r * math.cos(angle)
        y = tc.y + r * math.sin(angle)

        base_s = rng.uniform(scale_range[0], scale_range[1])
        sx = base_s * rng.uniform(0.85, 1.25)
        sy = base_s * rng.uniform(0.85, 1.25)
        sz = base_s * rng.uniform(0.70, 1.15)

        sink = rng.uniform(z_sink[0], z_sink[1])
        z = tc.z - sink

        rot_z = rng.uniform(0, 2.0 * math.pi)
        rot_x = rng.uniform(-0.25, 0.25)
        rot_y = rng.uniform(-0.25, 0.25)

        rock_ob = bpy.data.objects.new(f"Rock_{i}", source_mesh)
        rock_ob.location = (x, y, z)
        rock_ob.rotation_euler = (rot_x, rot_y, rot_z)
        rock_ob.scale = (sx, sy, sz)
        if source_mat and not rock_ob.data.materials:
            rock_ob.data.materials.append(source_mat)

        coll.objects.link(rock_ob)
        rocks.append(rock_ob)

    # 3. Clean up temporary source wrapper objects
    for o in temp_imported:
        if o.name in bpy.data.objects:
            bpy.data.objects.remove(o, do_unlink=True)

    return rocks
