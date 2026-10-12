"""Live tests that execute every ``# recipe: <path> step N`` block in the scene-director skill.

Each block is sent as its own call (fresh namespace, like ``execute_blender_code``), scene names
are prefixed with ``tks_recipe_test_`` and everything created is removed afterwards, so the
skill documentation cannot drift from what actually works in the running Blender.

    cd scripts/blender && python3 -m unittest test_recipes_live -v
"""

import json
import re
import unittest
from pathlib import Path

from test_tks_bpy_live import blender_reachable, send_code

RECIPES_DIR = Path(__file__).resolve().parents[2] / ".agents/skills/blender-scene-director/references"
BLOCK = re.compile(r"```python\n(# recipe: (\w+) step (\d+)\n.*?)```", re.S)
MARK = "@@TKS_RECIPE@@"

SNAPSHOT = f"""
import bpy, json
print("{MARK}" + json.dumps({{"scene": bpy.context.window.scene.name,
                             "scenes": sorted(s.name for s in bpy.data.scenes)}}))
"""

CLEANUP = """
import bpy, json
_keep = set(json.loads({keep!r}))
bpy.context.window.scene = bpy.data.scenes[{active!r}]
for _s in [s for s in bpy.data.scenes if s.name not in _keep]:
    _colls = list(_s.collection.children_recursive)
    for _o in list(_s.objects):
        bpy.data.objects.remove(_o, do_unlink=True)
    for _c in _colls:
        if _c.name in bpy.data.collections and _c.users <= 1:
            bpy.data.collections.remove(_c)
    bpy.data.scenes.remove(_s)
bpy.data.orphans_purge(do_recursive=True)
print("{mark}" + json.dumps({{"scenes": sorted(s.name for s in bpy.data.scenes),
    "fake_user_groups": sorted(g.name for g in bpy.data.node_groups if g.use_fake_user)}}))
"""


def recipe_blocks(path):
    blocks = [(int(step), code) for code, _name, step in BLOCK.findall(path.read_text())]
    steps = [s for s, _ in blocks]
    assert steps == list(range(1, len(steps) + 1)), f"{path.name}: steps out of order {steps}"
    return [re.sub(r'new_scene\("', 'new_scene("tks_recipe_test_', code) for _, code in blocks]


def _result(text):
    line = next(ln for ln in reversed(text.splitlines()) if ln.startswith(MARK))
    return json.loads(line[len(MARK):])


@unittest.skipUnless(blender_reachable(), "Blender MCP socket not reachable")
class RecipeTests(unittest.TestCase):
    def run_recipe(self, name):
        path = RECIPES_DIR / f"{name}.md"
        blocks = recipe_blocks(path)
        self.assertTrue(blocks, f"no recipe blocks in {path}")
        before = _result(send_code(SNAPSHOT))
        try:
            for i, code in enumerate(blocks, 1):
                try:
                    send_code(code)
                except AssertionError as exc:
                    self.fail(f"{name} step {i} failed: {exc}")
        finally:
            after = _result(send_code(CLEANUP.format(keep=json.dumps(before["scenes"]),
                                                     active=before["scene"], mark=MARK)))
        self.assertEqual(after["scenes"], before["scenes"])
        self.assertEqual(after["fake_user_groups"], [])

    def test_outdoor(self):
        self.run_recipe("outdoor")

    def test_interior(self):
        self.run_recipe("interior")

    def test_product(self):
        self.run_recipe("product")

    def test_lowpoly(self):
        self.run_recipe("lowpoly")

    def test_procedural(self):
        self.run_recipe("procedural")


if __name__ == "__main__":
    unittest.main()
