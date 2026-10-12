#!/usr/bin/env python3
"""Search and download CC0 PBR materials from ambientCG (https://ambientcg.com).

Run from the devcontainer (it has internet access; Blender has online access disabled):

    python3 scripts/blender/fetch_ambientcg.py search "forest floor" --limit 5
    python3 scripts/blender/fetch_ambientcg.py download Ground037 --res 2K-JPG

Downloads extract to scenes/assets/ambientcg/<AssetId>/ (shared with the Blender
container) and are recorded in scenes/assets/credits.json. Then, inside Blender:
``tks_bpy.materials.pbr_from_folder("Ground037", "/workspaces/tks/scenes/assets/ambientcg/Ground037")``.
"""

import argparse
import io
import json
import shutil
import sys
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import asset_credits  # noqa: E402

API = "https://ambientcg.com/api/v2/full_json"
USER_AGENT = "tks-blender-assets/1.0 (+https://github.com/Shapedsundew9/tks)"
DEFAULT_DEST = asset_credits.ASSETS_DIR / "ambientcg"
RESOLUTIONS = ("1K-JPG", "2K-JPG", "4K-JPG", "8K-JPG", "1K-PNG", "2K-PNG", "4K-PNG", "8K-PNG")
MAX_UNCOMPRESSED = 2 * 1024**3
ALLOWED_SUFFIXES = {".jpg", ".jpeg", ".png", ".exr", ".tif", ".tiff", ".usda", ".usdc", ".mtlx", ".blend", ".txt"}


def _get(url, timeout=60):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read()


def _query(**params):
    params = {k: v for k, v in params.items() if v is not None}
    return json.loads(_get(f"{API}?{urllib.parse.urlencode(params)}"))


def search(query, asset_type="Material", limit=10, sort="Popular"):
    """Return [{id, name, category, tags}] for matching assets."""
    data = _query(q=query, type=asset_type, limit=limit, sort=sort, include="tagData,displayData")
    return [
        {
            "id": a["assetId"],
            "name": a.get("displayName"),
            "category": a.get("displayCategory"),
            "tags": a.get("tags", [])[:8],
        }
        for a in data.get("foundAssets", [])
    ]


def download_options(asset_id):
    """Map attribute (e.g. '2K-JPG') -> download record for ``asset_id``."""
    data = _query(id=asset_id, include="downloadData")
    assets = data.get("foundAssets", [])
    if not assets:
        raise LookupError(f"ambientCG asset {asset_id!r} not found")
    out = {}
    for folder in assets[0].get("downloadFolders", {}).values():
        for category in folder.get("downloadFiletypeCategories", {}).values():
            for item in category.get("downloads", []):
                out[item["attribute"]] = item
    return out


def safe_extract(zip_bytes, dest):
    """Extract only flat, allow-listed files; reject traversal and oversize archives."""
    dest = Path(dest)
    with zipfile.ZipFile(io.BytesIO(zip_bytes)) as zf:
        members = [m for m in zf.infolist() if not m.is_dir()]
        if sum(m.file_size for m in members) > MAX_UNCOMPRESSED:
            raise ValueError("archive too large")
        tmp = dest.with_name(dest.name + ".partial")
        shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir(parents=True)
        written = []
        for m in members:
            name = Path(m.filename)
            if name.is_absolute() or ".." in name.parts:
                raise ValueError(f"unsafe path in archive: {m.filename}")
            if name.suffix.lower() not in ALLOWED_SUFFIXES:
                continue
            target = tmp / name.name
            with zf.open(m) as src, open(target, "wb") as dst:
                shutil.copyfileobj(src, dst)
            written.append(target.name)
        shutil.rmtree(dest, ignore_errors=True)
        tmp.rename(dest)
    return sorted(written)


def download(asset_id, resolution="2K-JPG", dest_root=DEFAULT_DEST, credits_file=asset_credits.CREDITS_FILE,
             force=False):
    dest = Path(dest_root) / asset_id
    marker = dest / ".tks-source.json"
    if marker.exists() and not force:
        info = json.loads(marker.read_text())
        if info.get("resolution") == resolution:
            return {"asset_id": asset_id, "path": str(dest), "files": info["files"], "cached": True}
    options = download_options(asset_id)
    if resolution not in options:
        raise LookupError(f"{asset_id} has no {resolution}; available: {sorted(options)}")
    item = options[resolution]
    files = safe_extract(_get(item["downloadLink"], timeout=600), dest)
    url = f"https://ambientcg.com/view?id={asset_id}"
    marker.write_text(json.dumps({"resolution": resolution, "files": files, "url": url}, indent=2) + "\n")
    asset_credits.record_credit(asset_id, "ambientcg", "CC0-1.0", author="ambientCG (Lennart Demes)", url=url,
                                credits_file=credits_file)
    return {"asset_id": asset_id, "path": str(dest), "files": files, "cached": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("search", help="search assets")
    s.add_argument("query")
    s.add_argument("--type", default="Material", help="Material, HDRI, 3DModel, Decal, ... (default Material)")
    s.add_argument("--limit", type=int, default=10)
    d = sub.add_parser("download", help="download and extract an asset")
    d.add_argument("asset_id")
    d.add_argument("--res", default="2K-JPG", choices=RESOLUTIONS)
    d.add_argument("--dest", type=Path, default=DEFAULT_DEST)
    d.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    if args.cmd == "search":
        result = search(args.query, args.type, args.limit)
    else:
        result = download(args.asset_id, args.res, args.dest, force=args.force)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
