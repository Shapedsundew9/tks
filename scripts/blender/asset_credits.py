"""Stdlib-only attribution ledger shared by fetch scripts and tks_bpy (scenes/assets/credits.json)."""

import json
from datetime import date
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ASSETS_DIR = REPO_ROOT / "scenes" / "assets"
CREDITS_FILE = ASSETS_DIR / "credits.json"


def record_credit(asset_id, source, licence, author=None, url=None, notes=None, credits_file=CREDITS_FILE):
    """Insert or update ``source:asset_id`` in the credits ledger and return the entry."""
    credits_file = Path(credits_file)
    credits_file.parent.mkdir(parents=True, exist_ok=True)
    data = json.loads(credits_file.read_text()) if credits_file.exists() else {}
    key = f"{source}:{asset_id}"
    data[key] = {
        "asset_id": asset_id,
        "source": source,
        "licence": licence,
        "author": author,
        "url": url,
        "notes": notes,
        "recorded": date.today().isoformat(),
    }
    credits_file.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    return data[key]
