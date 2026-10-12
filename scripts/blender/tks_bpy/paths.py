"""Shared repository paths (identical inside the Blender container and the devcontainer)."""

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
SCENES_DIR = REPO_ROOT / "scenes"
ASSETS_DIR = SCENES_DIR / "assets"
RENDERS_DIR = REPO_ROOT / "renders"
CREDITS_FILE = ASSETS_DIR / "credits.json"
