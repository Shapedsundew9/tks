#!/usr/bin/env python3
"""Search and download curated OFL fonts from Google Fonts repository.

Run from the devcontainer (internet access; Blender has online access disabled):

    python3 scripts/blender/fetch_fonts.py list
    python3 scripts/blender/fetch_fonts.py download archivoblack
    python3 scripts/blender/fetch_fonts.py download all

Downloads save to scenes/assets/fonts/<Family>/<file>.ttf (shared with Blender)
and record attribution in scenes/assets/credits.json.
"""

import argparse
import json
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import asset_credits  # noqa: E402

BASE_URL = "https://raw.githubusercontent.com/google/fonts/main/"
USER_AGENT = "tks-blender-fonts/1.0 (+https://github.com/Shapedsundew9/tks)"
DEFAULT_DEST = asset_credits.ASSETS_DIR / "fonts"

CATALOGUE = {
    "archivoblack": {
        "family": "ArchivoBlack",
        "file": "ArchivoBlack-Regular.ttf",
        "path": "ofl/archivoblack/ArchivoBlack-Regular.ttf",
        "author": "Omnibus-Type",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/Archivo+Black",
    },
    "michroma": {
        "family": "Michroma",
        "file": "Michroma-Regular.ttf",
        "path": "ofl/michroma/Michroma-Regular.ttf",
        "author": "Vernon Adams",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/Michroma",
    },
    "kronaone": {
        "family": "KronaOne",
        "file": "KronaOne-Regular.ttf",
        "path": "ofl/kronaone/KronaOne-Regular.ttf",
        "author": "Yvonne Schüttler",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/Krona+One",
    },
    "barlowcondensed-regular": {
        "family": "BarlowCondensed",
        "file": "BarlowCondensed-Regular.ttf",
        "path": "ofl/barlowcondensed/BarlowCondensed-Regular.ttf",
        "author": "Jeremy Tribby",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/Barlow+Condensed",
    },
    "barlowcondensed-medium": {
        "family": "BarlowCondensed",
        "file": "BarlowCondensed-Medium.ttf",
        "path": "ofl/barlowcondensed/BarlowCondensed-Medium.ttf",
        "author": "Jeremy Tribby",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/Barlow+Condensed",
    },
    "ibmplexmono": {
        "family": "IBMPlexMono",
        "file": "IBMPlexMono-Regular.ttf",
        "path": "ofl/ibmplexmono/IBMPlexMono-Regular.ttf",
        "author": "Mike Abbink, Bold Monday",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/IBM+Plex+Mono",
    },
    "sharetechmono": {
        "family": "ShareTechMono",
        "file": "ShareTechMono-Regular.ttf",
        "path": "ofl/sharetechmono/ShareTechMono-Regular.ttf",
        "author": "Carrois Apostrophe",
        "licence": "OFL-1.1",
        "url": "https://fonts.google.com/specimen/Share+Tech+Mono",
    },
}


def _get(url, timeout=60):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read()


def list_fonts():
    """Return catalogue entries."""
    return [
        {
            "key": k,
            "family": v["family"],
            "file": v["file"],
            "author": v["author"],
            "licence": v["licence"],
            "url": v["url"],
        }
        for k, v in CATALOGUE.items()
    ]


def resolve_key(key):
    """Normalize input key or filename stem to catalogue key."""
    norm = key.strip().lower().replace("_", "").replace("-", "")
    for k, v in CATALOGUE.items():
        if k.replace("-", "") == norm:
            return k
        stem = Path(v["file"]).stem.lower().replace("-", "")
        if stem == norm:
            return k
        fam = v["family"].lower().replace("-", "")
        if fam == norm:
            return k
    raise LookupError(f"Font key '{key}' not found in catalogue. Available: {list(CATALOGUE.keys())}")


def download(key, dest_root=DEFAULT_DEST, credits_file=asset_credits.CREDITS_FILE, timeout=60):
    """Download a font from Google Fonts and record its attribution in credits.json."""
    cat_key = resolve_key(key)
    entry = CATALOGUE[cat_key]

    target_dir = Path(dest_root) / entry["family"]
    target_path = target_dir / entry["file"]

    if target_path.exists() and target_path.stat().st_size > 0:
        asset_credits.record_credit(
            f"font:{cat_key}",
            "google_fonts",
            entry["licence"],
            author=entry["author"],
            url=entry["url"],
            credits_file=credits_file,
        )
        return {"key": cat_key, "path": target_path, "cached": True}

    target_dir.mkdir(parents=True, exist_ok=True)
    url = BASE_URL + entry["path"]
    data = _get(url, timeout=timeout)
    if not data or len(data) < 100:
        raise ValueError(f"Downloaded font from {url} is empty or corrupt")

    target_path.write_bytes(data)

    asset_credits.record_credit(
        f"font:{cat_key}",
        "google_fonts",
        entry["licence"],
        author=entry["author"],
        url=entry["url"],
        credits_file=credits_file,
    )
    return {"key": cat_key, "path": target_path, "cached": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("list", help="List curated OFL fonts")

    dl = sub.add_parser("download", help="Download a font by key or 'all'")
    dl.add_argument("key", help="Catalogue key (e.g. archivoblack, ibmplexmono, or 'all')")
    dl.add_argument("--dest", type=Path, default=DEFAULT_DEST)

    args = parser.parse_args()

    if args.cmd == "list":
        for f in list_fonts():
            print(f"{f['key']:<24} {f['family']:<16} {f['file']:<28} {f['author']}")
    elif args.cmd == "download":
        keys = list(CATALOGUE.keys()) if args.key == "all" else [args.key]
        for k in keys:
            res = download(k, dest_root=args.dest)
            status = "cached" if res["cached"] else "downloaded"
            print(f"[{status}] {res['key']} -> {res['path']}")


if __name__ == "__main__":
    main()
