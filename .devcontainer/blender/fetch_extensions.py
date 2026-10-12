"""Download the pinned Blender extensions and verify their SHA-256 digests.

Runs at image build time so Blender can install them offline on startup.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import urllib.request

USER_AGENT = "tks-blender-image/1.0"


def fetch(lock_path, output_dir):
    lock = json.loads(Path(lock_path).read_text(encoding="utf-8"))
    output = Path(output_dir)
    output.mkdir(parents=True, exist_ok=True)
    for package in lock["extensions"]:
        target = output / f"{package['id']}.zip"
        request = urllib.request.Request(package["url"], headers={"User-Agent": USER_AGENT})
        with urllib.request.urlopen(request, timeout=60) as response, open(target, "wb") as handle:
            shutil.copyfileobj(response, handle)
        digest = hashlib.sha256(target.read_bytes()).hexdigest()
        if digest != package["sha256"]:
            target.unlink()
            raise SystemExit(f"{package['id']}: sha256 {digest} does not match pinned {package['sha256']}")
        print(f"{package['id']} {package['version']}: verified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lock")
    parser.add_argument("output_dir")
    args = parser.parse_args()
    fetch(args.lock, args.output_dir)


if __name__ == "__main__":
    main()
