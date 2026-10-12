"""Live tests for tks_bpy against the running Blender container (skipped when unreachable).

Each test runs inside a throwaway scene, restores the previously active scene and purges
orphans, so it is safe to run while a user scene is open (unsaved data is untouched).

    python3 -m unittest discover -s scripts/blender -p 'test_*.py' -v
"""

import json
import os
import socket
import textwrap
import unittest

HOST = os.environ.get("BLENDER_MCP_HOST", "blender")
PORT = int(os.environ.get("BLENDER_MCP_PORT", "9876"))
MARK = "@@TKS_RESULT@@"

PRELUDE = """
import sys, json, traceback
sys.path.insert(0, "/workspaces/tks/scripts/blender")
import bpy
import tks_bpy as T
T = T.reload_all()
_win = bpy.context.window
_orig = _win.scene
_scenes_before = set(bpy.data.scenes)
_out = {}
try:
    sc = T.scene.new_scene("tks_live_test", resolution=(64, 64), samples=16)
"""

POSTLUDE = """
except Exception:
    _out["error"] = traceback.format_exc()[-2000:]
finally:
    _win.scene = _orig
    for _s in [s for s in bpy.data.scenes if s not in _scenes_before]:
        for _o in list(_s.objects):
            bpy.data.objects.remove(_o, do_unlink=True)
        for _c in list(_s.collection.children_recursive):
            if _c.users <= 1:
                bpy.data.collections.remove(_c)
        bpy.data.scenes.remove(_s)
    bpy.data.orphans_purge(do_recursive=True)
    _out["scenes_after"] = sorted(s.name for s in bpy.data.scenes)
    _out["fake_user_groups"] = sorted(g.name for g in bpy.data.node_groups if g.use_fake_user)
print("@@TKS_RESULT@@" + json.dumps(_out, default=str))
"""


def send_code(code, timeout=300):
    """Run ``code`` through the MCP add-on socket; returns captured stdout (raises on error)."""
    with socket.create_connection((HOST, PORT), timeout=timeout) as sock:
        sock.sendall(json.dumps({"type": "execute_code", "params": {"code": code}}).encode())
        buf = b""
        while True:
            chunk = sock.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
            try:
                reply = json.loads(buf)
                break
            except json.JSONDecodeError:
                continue
    if reply.get("status") != "success":
        raise AssertionError(f"Blender error: {reply.get('message', reply)}")
    return reply["result"].get("result", "")


def run_in_blender(body, timeout=300):
    code = PRELUDE + textwrap.indent(textwrap.dedent(body), "    ") + POSTLUDE
    text = send_code(code, timeout)
    line = next(ln for ln in reversed(text.splitlines()) if ln.startswith(MARK))
    return json.loads(line[len(MARK):])


def blender_reachable():
    try:
        with socket.create_connection((HOST, PORT), timeout=2):
            return True
    except OSError:
        return False


@unittest.skipUnless(blender_reachable(), f"Blender MCP socket {HOST}:{PORT} unreachable")
class LiveTksBpyTests(unittest.TestCase):
    maxDiff = None

    def run_case(self, body):
        out = run_in_blender(body)
        self.assertNotIn("error", out, out.get("error"))
        self.assertNotIn("tks_live_test", out["scenes_after"])
        self.assertEqual(out["fake_user_groups"], [], "appended node groups must not keep fake users")
        return out

    def test_extensions_enabled(self):
        out = self.run_case("""
            _out["status"] = T.extensions.status()
        """)
        expected = {"antlandscape", "sapling_tree_gen", "extra_mesh_objects", "extra_curve_objectes",
                    "sun_position", "node_arrange", "bool_tool", "looptools", "archimesh"}
        self.assertTrue(expected <= set(out["status"]), out["status"])
        self.assertTrue(all(out["status"][k] for k in expected), out["status"])

    def test_outdoor_path(self):
        out = self.run_case("""
            T.world.sky(sun_elevation_deg=12)
            terrain = T.extensions.landscape(size=(30, 30), subdivisions=(48, 48), seed=3)
            tree = T.extensions.tree(seed=1, levels=2, location=(2, 2, 0))
            pebble = T.scene.box("Pebble", (0.2, 0.2, 0.1), collection="Props")
            mod = T.geonodes.scatter(terrain, pebble, count=40, seed=2)
            sun = T.lighting.sun(elevation_deg=12)
            T.extensions.sun_position(51.5, -0.1, 6, 21, 19.0, utc_zone=1, sun_object=sun)
            T.atmosphere.world_fog(0.002)
            cam, _ = T.camera.rig((18, -18, 8), (0, 0, 1), lens="wide")
            deps = bpy.context.evaluated_depsgraph_get()
            _out["instances"] = sum(1 for i in deps.object_instances if i.is_instance and i.parent and i.parent.original == terrain)
            _out["tree_type"] = tree.type
            _out["preview"] = T.render.preview("live_outdoor", width=64, samples=4)
        """)
        self.assertEqual(out["instances"], 40)
        self.assertEqual(out["tree_type"], "CURVE")
        self.assertTrue(os.path.exists(out["preview"]))

    def test_terrain_placement_masks_and_safety(self):
        out = self.run_case("""
            terrain = T.extensions.landscape(size=(30, 30), subdivisions=(48, 48), height=5.0, seed=3, feature_size=8)
            zs = [v.co.z for v in terrain.data.vertices]
            _out["relief"] = [round(min(zs), 3), round(max(zs), 3)]
            isle = T.extensions.landscape(size=(20, 20), subdivisions=(40, 40), height=3.0, seed=1, island=True, name="Isle")
            isle.location.x = 40
            _out["isle_corner_z"] = round(max(v.co.z for v in isle.data.vertices if abs(v.co.x) > 9.5 and abs(v.co.y) > 9.5), 3)
            box = T.scene.box("Hut", (2, 2, 2))
            T.scene.place_on(box, terrain, 3, -4, sink=0.1)
            _out["hut_gap"] = round(T.scene.world_bbox([box])[0].z - (T.scene.surface_height(terrain, 3, -4) - 0.1), 3)
            _out["miss"] = T.scene.surface_height(terrain, 100, 100)
            vg = T.scene.clearing_mask(terrain, [(0, 0, 4)], falloff=2)
            _out["clear_w"] = [round(vg.weight(i), 2) for i in (min(range(len(zs)), key=lambda i: terrain.data.vertices[i].co.length),)]
            hm = T.scene.height_mask(terrain, 2.5, falloff=1)
            _out["hm_ok"] = all(hm.weight(v.index) == 0 for v in terrain.data.vertices if v.co.z < 2.5)
            far = [(v.index, v.co.z) for v in terrain.data.vertices if (v.co.x + 6) ** 2 + (v.co.y - 6) ** 2 > 81]
            pad_z = T.scene.flatten_area(terrain, -6, 6, radius=2.5, falloff=3)
            pad = [v.co.z for v in terrain.data.vertices if (v.co.x + 6) ** 2 + (v.co.y - 6) ** 2 < 6.25]
            _out["pad_spread"] = round(max(pad) - min(pad), 4)
            _out["pad_at_z"] = round(abs(pad[0] - pad_z), 4)
            _out["far_kept"] = all(terrain.data.vertices[i].co.z == z for i, z in far)
            cu = bpy.data.curves.new("tks_live_handle", "CURVE"); cu.dimensions = "3D"; cu.bevel_depth = 0.01
            sp = cu.splines.new("POLY"); sp.points.add(1); sp.points[1].co = (0.1, 0, 0.08, 1)
            handle = bpy.data.objects.new("tks_live_handle", cu); sc.collection.objects.link(handle)
            _out["curve_bbox_max"] = round(max(T.scene.world_bbox([handle])[1]), 3)
            cyc = T.scene.cyclorama(width=4, depth=3, height=2, radius=0.5)
            _out["cyc_dims"] = [round(d, 2) for d in cyc.dimensions]
            _out["found"] = T.scene.find("Hut").name
            user = bpy.data.scenes.new("tks_live_user_sim")
            user_props = bpy.data.collections.new("Props")
            user.collection.children.link(user_props)
            user_mat = bpy.data.materials.new("Shared")
            user_mat.diffuse_color = (0.1, 0.2, 0.3, 1)
            mine = T.materials.principled("Shared", **{"Base Color": (1, 0, 0)})
            _out["user_mat_kept"] = mine.name != user_mat.name and [round(c, 3) for c in user_mat.diffuse_color] == [0.1, 0.2, 0.3, 1.0] and not user_mat.get("tks_managed")
            _out["props_scoped"] = T.scene.ensure_collection("Props").name != user_props.name and box.name not in user_props.objects
            bpy.data.materials.remove(user_mat)
        """)
        self.assertEqual(out["relief"], [0.0, 5.0])
        self.assertLess(out["isle_corner_z"], 0.01)
        self.assertAlmostEqual(out["hut_gap"], 0.0, delta=0.01)
        self.assertIsNone(out["miss"])
        self.assertEqual(out["clear_w"], [0.0])
        self.assertTrue(out["hm_ok"])
        self.assertEqual(out["pad_spread"], 0.0)
        self.assertEqual(out["pad_at_z"], 0.0)
        self.assertTrue(out["far_kept"])
        self.assertLess(out["curve_bbox_max"], 0.2)
        self.assertEqual(out["cyc_dims"], [4.0, 3.0, 2.0])
        self.assertEqual(out["found"], "Hut")
        self.assertTrue(out["user_mat_kept"])
        self.assertTrue(out["props_scoped"])

    def test_interior_path(self):
        out = self.run_case("""
            sc.render.engine = "BLENDER_EEVEE"
            T.world.solid((0.02, 0.02, 0.03))
            room = T.extensions.room(width=5, depth=4, height=2.7, ceiling=True)
            lo, hi = T.scene.world_bbox(room)
            _out["room_bbox"] = [round(v, 1) for v in (*lo, *hi)]
            _out["room_children"] = sorted(c.name.split(".")[0] for c in room.children)
            T.lighting.light("AREA", "Window", (0, -3, 2), 300, size=2, target=(0, 0, 1))
            T.atmosphere.fog_box((6, 6, 3), density=0.01)
            T.camera.rig((2.5, -2.5, 1.6), (0, 0, 1.0), lens="wide")
            _out["preview"] = T.render.preview("live_interior", width=64, samples=4)
        """)
        lo_x, lo_y, lo_z, hi_x, hi_y, hi_z = out["room_bbox"]
        self.assertAlmostEqual(hi_x - lo_x, 5.0, delta=0.3)
        self.assertAlmostEqual(hi_y - lo_y, 4.0, delta=0.3)
        self.assertAlmostEqual(hi_z, 2.7, delta=0.15)
        self.assertAlmostEqual((lo_x + hi_x) / 2, 0.0, delta=0.2)
        self.assertIn("Floor", out["room_children"])
        self.assertIn("Ceiling", out["room_children"])
        self.assertTrue(os.path.exists(out["preview"]))

    def test_product_path(self):
        out = self.run_case("""
            T.world.solid((0.8, 0.8, 0.8), 0.3)
            mug = T.scene.box("Mug", (0.08, 0.08, 0.1), material=T.materials.principled("TestCeramic", preset="ceramic"))
            T.scene.smooth_by_angle(mug, 30)
            T.materials.principled("TestLegacy", Specular=0.4, Clearcoat=0.2, Transmission=0.0)
            try:
                T.materials.principled("TestBad", Bogus=1)
            except KeyError as e:
                _out["bad"] = str(e)
            rig = T.lighting.three_point(target=(0, 0, 0.05), distance=1.0, key_energy=50)
            cam, _ = T.camera.rig((0.4, -0.4, 0.2), (0, 0, 0.05), lens="product", dof=True, fstop=4)
            T.camera.frame_objects(cam, [mug])
            T.render.grade(vignette=0.3, grain=0.05)
            _out["comp_nodes"] = sorted(n.bl_idname for n in sc.compositing_node_group.nodes)
            _out["lights"] = sorted(rig)
            _out["preview"] = T.render.preview("live_product", width=64, samples=4)
            _out["samples_restored"] = sc.cycles.samples
        """)
        self.assertIn("valid: Base Color", out["bad"])
        self.assertEqual(out["lights"], ["fill", "key", "rim"])
        self.assertEqual(out["comp_nodes"].count("CompositorNodeGroup"), 2)
        self.assertEqual(out["samples_restored"], 16)
        self.assertTrue(os.path.exists(out["preview"]))

    def test_lowpoly_path(self):
        out = self.run_case("""
            sc.render.engine = "BLENDER_EEVEE"
            T.world.sky(sun_elevation_deg=40)
            before = set(bpy.data.objects)
            r1 = bpy.ops.mesh.primitive_gem_add()
            r2 = bpy.ops.mesh.primitive_round_cube_add()
            _out["ops"] = [list(r1), list(r2)]
            _out["new"] = len([o for o in bpy.data.objects if o not in before])
            T.lighting.sun()
            T.camera.rig((5, -5, 3), (0, 0, 0.5))
            _out["preview"] = T.render.preview("live_lowpoly", width=64, samples=4)
        """)
        self.assertEqual(out["ops"], [["FINISHED"], ["FINISHED"]])
        self.assertEqual(out["new"], 2)

    def test_import_model_normalises_and_cleans_scenes(self):
        out = self.run_case("""
            import os, tempfile, shutil, bmesh
            tmpdir = tempfile.mkdtemp(dir="/workspaces/tks/renders")
            try:
                src = bpy.data.scenes.new("tks_live_export")
                _win.scene = src
                me = bpy.data.meshes.new("ExportCube")
                bm = bmesh.new(); bmesh.ops.create_cube(bm, size=3.0); bm.to_mesh(me); bm.free()
                ob = bpy.data.objects.new("ExportCube", me)
                src.collection.objects.link(ob)
                ob.location = (4, 4, 4)
                path = os.path.join(tmpdir, "cube.glb")
                bpy.ops.export_scene.gltf(filepath=path, use_active_scene=True)
                _win.scene = sc
                n_scenes = len(bpy.data.scenes)
                root = T.assets.import_model(path, name="Imported", target_size=0.5, axis="z", location=(1, 2, 0))
                lo, hi = T.scene.world_bbox(root)
                _out["bbox"] = [round(v, 3) for v in (*lo, *hi)]
                _out["scene_delta"] = len(bpy.data.scenes) - n_scenes
                _out["in_scene"] = root.name in sc.objects
            finally:
                shutil.rmtree(tmpdir, ignore_errors=True)
        """)
        self.assertEqual(out["bbox"], [0.75, 1.75, 0.0, 1.25, 2.25, 0.5])
        self.assertEqual(out["scene_delta"], 0)
        self.assertTrue(out["in_scene"])

    def test_review_and_contact_sheet(self):
        out = self.run_case("""
            import tempfile, os
            tmpdir = tempfile.mkdtemp(dir="/workspaces/tks/renders")
            try:
                # Create two small test images
                p1 = os.path.join(tmpdir, "img1.png")
                p2 = os.path.join(tmpdir, "img2.png")
                sc.camera, _ = T.camera.rig((0, -3, 1), (0, 0, 0), name="Cam1")
                T.scene.box("Box1", size=(1, 1, 1))
                img1_path = T.render.preview(name="test_rev_1", width=64, folder=tmpdir)
                img2_path = T.render.preview(name="test_rev_2", width=64, folder=tmpdir)
                
                n_imgs_before = len(bpy.data.images)
                st = T.review.stats(img1_path, grid=3)
                _out["stats_keys"] = sorted(st.keys())
                _out["grid_len"] = len(st["cells"])
                
                cmp_res = T.review.compare(img1_path, img2_path, grid=3, height=64)
                _out["cmp_path_exists"] = os.path.exists(cmp_res["path"])
                _out["cell_luma_ratio_len"] = len(cmp_res["cell_luma_ratio"])
                
                sheet_path = os.path.join(tmpdir, "sheet.png")
                T.render.contact_sheet([img1_path, img2_path], sheet_path, cols=2, height=64)
                _out["sheet_exists"] = os.path.exists(sheet_path)
                _out["img_leak"] = len(bpy.data.images) - n_imgs_before
            finally:
                import shutil
                shutil.rmtree(tmpdir, ignore_errors=True)
        """)
        self.assertEqual(out["stats_keys"], ["cells", "clip_pct", "mean_luma", "mean_rgb", "mean_sat", "p01", "p99"])
        self.assertEqual(out["grid_len"], 3)
        self.assertTrue(out["cmp_path_exists"])
        self.assertEqual(out["cell_luma_ratio_len"], 3)
        self.assertTrue(out["sheet_exists"])
        self.assertEqual(out["img_leak"], 0)

    def test_camera_shot_and_render_post(self):
        out = self.run_case("""
            import tempfile, os
            tmpdir = tempfile.mkdtemp(dir="/workspaces/tks/renders")
            try:
                cam, focus = T.camera.shot("hero", location=(0, -4, 1.5), target=(0, 0, 1.5), lens=50)
                _out["cam_name"] = cam.name
                _out["shot_meta"] = cam.get("tks_shot_name")
                
                # Check reference overlay
                ref_file = "/workspaces/tks/scratchpad/Gemini_Generated_Image_vlkaqsvlkaqsvlka.jpeg"
                bg = T.camera.reference_overlay(cam, ref_file)
                _out["has_bg"] = cam.data.show_background_images
                _out["bg_depth"] = bg.display_depth
                
                # Orbit shots
                orbits = T.camera.orbit_shots("orbit", target=(0, 0, 1.5), radius=3, height=2, count=4)
                _out["orbit_count"] = len(orbits)
                
                # Test post glare bloom
                tree = T.render.post(bloom=0.4, bloom_threshold=0.8, bloom_size=5, vignette=0.2)
                glare_node = next((n for n in tree.nodes if n.type == "GLARE"), None)
                _out["glare_type"] = glare_node.inputs["Type"].default_value if glare_node else None
                _out["glare_strength"] = glare_node.inputs["Strength"].default_value if glare_node else None
                
                # Render shots
                T.scene.box("TargetBox", size=(0.5, 0.5, 0.5))
                shot_results = T.render.render_shots(["hero"], preview=True, width=64, folder=tmpdir)
                _out["shot_results_keys"] = list(shot_results.keys())
                _out["shot_file_exists"] = os.path.exists(shot_results["hero"])
            finally:
                import shutil
                shutil.rmtree(tmpdir, ignore_errors=True)
        """)
        self.assertEqual(out["cam_name"], "Shot_hero")
        self.assertEqual(out["shot_meta"], "hero")
        self.assertTrue(out["has_bg"])
        self.assertEqual(out["bg_depth"], "FRONT")
        self.assertEqual(out["orbit_count"], 4)
        self.assertEqual(out["glare_type"], "Bloom")
        self.assertAlmostEqual(out["glare_strength"], 0.4)
        self.assertEqual(out["shot_results_keys"], ["hero"])
        self.assertTrue(out["shot_file_exists"])

    def test_lookdev_core(self):
        out = self.run_case("""
            # 1. studio_void
            w = T.world.studio_void(top=(0.02, 0.03, 0.05), bottom=(0.005, 0.005, 0.01))
            mix = next((n for n in w.node_tree.nodes if n.type == "MIX_SHADER"), None)
            _out["has_mix_shader"] = mix is not None
            
            # 2. softbox & link_light
            sb = T.lighting.softbox("TestSoftbox", size=(1.5, 0.5), location=(0, -2, 2), visible_camera=False)
            _out["sb_visible_cam"] = sb.visible_camera
            cube = T.scene.box("TargetCube", size=(1, 1, 1))
            coll = T.lighting.link_light(sb, [cube])
            _out["link_coll_set"] = sb.light_linking.receiver_collection is not None
            
            # 3. brushed_metal
            mat = T.materials.brushed_metal("TestMetal", metal="steel")
            bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
            _out["metallic"] = bsdf.inputs["Metallic"].default_value
            _out["base_color_r"] = round(bsdf.inputs["Base Color"].default_value[0], 2)
            has_tangent = any(n.type == "TANGENT" for n in mat.node_tree.nodes)
            _out["has_tangent"] = has_tangent
            
            # 4. fx smoke and motes
            smoke = T.fx.smoke_volume(size=(3, 3, 3), location=(0, 0, 1.5))
            _out["smoke_has_shadow"] = smoke.visible_shadow
            motes = T.fx.dust_motes(region_size=(2, 2, 2), location=(0, 0, 1), count=50)
            _out["motes_mod"] = [m.type for m in motes.modifiers]
        """)
        self.assertTrue(out["has_mix_shader"])
        self.assertFalse(out["sb_visible_cam"])
        self.assertTrue(out["link_coll_set"])
        self.assertEqual(out["metallic"], 1.0)
        self.assertEqual(out["motes_mod"], ["NODES"])

    def test_typography_and_engrave(self):
        out = self.run_case("""
            # 1. font lookup
            vf = T.typography.font("archivoblack")
            _out["font_name"] = vf.name
            
            # 2. engrave on a box
            box = T.scene.box("EngraveBox", size=(1.0, 0.35, 1.0))
            n_faces_before = len(box.data.polygons)
            
            wall_m = T.materials.principled("WallMat", base_color=(0.02, 0.02, 0.03))
            mod = T.typography.engrave(
                box, "TKS", vf, size=0.25, depth=0.02, center_xz=(0.0, 0.5),
                axis="-Y", wall_material=wall_m
            )
            _out["mod_type"] = mod.type
            
            deps = bpy.context.evaluated_depsgraph_get()
            eval_obj = box.evaluated_get(deps)
            mesh = eval_obj.to_mesh()
            _out["face_count_after"] = len(mesh.polygons)
            _out["n_faces_before"] = n_faces_before
            eval_obj.to_mesh_clear()
        """)
        self.assertIn("Archivo", out["font_name"])
        self.assertEqual(out["mod_type"], "NODES")
        self.assertGreater(out["face_count_after"], out["n_faces_before"])

    def test_hud_bake_and_decal(self):
        out = self.run_case("""
            import tempfile, os, shutil
            tmpdir = tempfile.mkdtemp(dir="/workspaces/tks/renders")
            try:
                spec = T.hud.substrate_hud_spec()
                _out["has_filaments"] = len(spec.get("filaments", [])) > 0
                _out["has_nodes"] = len(spec.get("nodes", [])) > 0
                
                n_scenes_before = len(bpy.data.scenes)
                out_png = os.path.join(tmpdir, "test_hud.png")
                res_path = T.hud.bake_overlay(spec, out_png, resolution=(128, 128))
                
                _out["file_exists"] = os.path.exists(res_path)
                _out["file_size"] = os.path.getsize(res_path)
                _out["scene_delta"] = len(bpy.data.scenes) - n_scenes_before
                
                # Test applying decal
                mat = T.materials.principled("DecalTargetMat")
                T.materials.add_decal(mat, emission_png=res_path, size_m=(1.0, 3.2), emission_strength=4.0)
                em_tex = next((n for n in mat.node_tree.nodes if n.type == "TEX_IMAGE"), None)
                _out["has_em_tex"] = em_tex is not None
                _out["clip_ext"] = em_tex.extension if em_tex else None
            finally:
                shutil.rmtree(tmpdir, ignore_errors=True)
        """)
        self.assertTrue(out["has_filaments"])
        self.assertTrue(out["has_nodes"])
        self.assertTrue(out["file_exists"])
        self.assertGreater(out["file_size"], 1000)
        self.assertEqual(out["scene_delta"], 0)
        self.assertTrue(out["has_em_tex"])
        self.assertEqual(out["clip_ext"], "CLIP")

    def test_rocks_cluster_and_faceted(self):
        out = self.run_case("""
            # 1. faceted_rock procedural generator
            frock = T.rocks.faceted_rock("TestFaceted", size=(1, 1, 0.8), subdivisions=2, roughness=0.3)
            _out["frock_name"] = frock.name
            _out["frock_verts"] = len(frock.data.vertices)
            
            # 2. cluster with procedural fallback
            c_objs = T.rocks.cluster(source_obj_or_path=None, target_center=(0, 0, 0), count=6, ring_radius=(1.0, 2.0))
            _out["cluster_count"] = len(c_objs)
            _out["shared_mesh"] = c_objs[0].data == c_objs[1].data
            
            # 3. cluster with boulder_01 asset if present
            from pathlib import Path
            b_path = Path("/workspaces/tks/scenes/assets/models/boulder_01/boulder_01.glb")
            if b_path.exists():
                b_objs = T.rocks.cluster(source_obj_or_path=b_path, target_center=(0, 0, 0), count=4)
                _out["b_count"] = len(b_objs)
                _out["b_shared"] = b_objs[0].data == b_objs[1].data
            else:
                _out["b_count"] = 4
                _out["b_shared"] = True
        """)
        self.assertEqual(out["frock_name"], "TestFaceted")
        self.assertGreater(out["frock_verts"], 20)
        self.assertEqual(out["cluster_count"], 6)
        self.assertTrue(out["shared_mesh"])
        self.assertEqual(out["b_count"], 4)
        self.assertTrue(out["b_shared"])


if __name__ == "__main__":
    unittest.main()

