"""tks_bpy: small, tested helpers for driving Blender 5.2 through MCP.

Usage inside ``execute_blender_code``::

    import sys; sys.path.insert(0, "/workspaces/tks/scripts/blender")
    import tks_bpy as T; T.reload_all()
    T.scene.new_scene("forest_cabin", engine="CYCLES")

All helpers target Blender 5.2 LTS. They raise ``RuntimeError`` on older builds
so agents notice API drift instead of silently using removed properties.
"""

import importlib
import sys

try:
    import bpy
except ModuleNotFoundError:  # imported outside Blender, e.g. by `unittest discover`
    bpy = None

if bpy is None:

    def load_tests(loader, _tests, _pattern):
        """No offline tests here; live tests are in scripts/blender/test_tks_bpy_live.py."""
        return loader.suiteClass()

else:
    MIN_VERSION = (5, 2, 0)
    if bpy.app.version < MIN_VERSION:
        raise RuntimeError(f"tks_bpy requires Blender >= {MIN_VERSION}, running {bpy.app.version_string}")

    from . import (  # noqa: F401
        assets,
        atmosphere,
        camera,
        extensions,
        geonodes,
        lighting,
        materials,
        render,
        scene,
        world,
    )

__all__ = [
    "assets",
    "atmosphere",
    "camera",
    "extensions",
    "geonodes",
    "lighting",
    "materials",
    "render",
    "scene",
    "world",
    "reload_all",
]


def reload_all():
    """Reload every tks_bpy module (Blender keeps modules cached between MCP calls)."""
    names = sorted(n for n in sys.modules if n.startswith(__name__ + "."))
    for name in names:
        importlib.reload(sys.modules[name])
    return importlib.reload(sys.modules[__name__])
