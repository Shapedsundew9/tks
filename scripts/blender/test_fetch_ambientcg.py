"""Offline unit tests for fetch_ambientcg.py and asset_credits.py (no network)."""

import io
import json
import shutil
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import asset_credits  # noqa: E402
import fetch_ambientcg as acg  # noqa: E402


def temp_dir(case):
    path = Path(tempfile.mkdtemp())
    case.addCleanup(shutil.rmtree, path, ignore_errors=True)
    return path


def make_zip(entries):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        for name, data in entries.items():
            zf.writestr(name, data)
    return buf.getvalue()


API_RESPONSE = {
    "foundAssets": [
        {
            "assetId": "Wood001",
            "displayName": "Wood 001",
            "displayCategory": "Wood",
            "tags": ["wood", "planks"],
            "downloadFolders": {
                "default": {
                    "downloadFiletypeCategories": {
                        "zip": {
                            "downloads": [
                                {"attribute": "1K-JPG", "downloadLink": "https://example/1k.zip"},
                                {"attribute": "2K-JPG", "downloadLink": "https://example/2k.zip"},
                            ]
                        }
                    }
                }
            },
        }
    ]
}


class SafeExtractTests(unittest.TestCase):
    def setUp(self):
        self.tmp = temp_dir(self)

    def test_extracts_allowed_files_flat(self):
        data = make_zip({"Wood001_1K-JPG_Color.jpg": b"c", "sub/Wood001_1K-JPG_NormalGL.jpg": b"n", "evil.sh": b"x"})
        files = acg.safe_extract(data, self.tmp / "Wood001")
        self.assertEqual(files, ["Wood001_1K-JPG_Color.jpg", "Wood001_1K-JPG_NormalGL.jpg"])
        self.assertFalse((self.tmp / "Wood001" / "evil.sh").exists())
        self.assertFalse((self.tmp / "Wood001.partial").exists())

    def test_rejects_traversal(self):
        data = make_zip({"../escape.jpg": b"x"})
        with self.assertRaises(ValueError):
            acg.safe_extract(data, self.tmp / "bad")
        self.assertFalse((self.tmp / "escape.jpg").exists())
        self.assertFalse((self.tmp / "bad").exists())

    def test_rejects_oversize(self):
        data = make_zip({"a.jpg": b"x" * 10})
        with mock.patch.object(acg, "MAX_UNCOMPRESSED", 5), self.assertRaises(ValueError):
            acg.safe_extract(data, self.tmp / "big")


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.tmp = temp_dir(self)
        self.credits = self.tmp / "credits.json"
        self.zip = make_zip({"Wood001_1K-JPG_Color.jpg": b"c"})
        self.calls = []

        def fake_get(url, timeout=60):
            self.calls.append(url)
            if url.startswith(acg.API):
                return json.dumps(API_RESPONSE).encode()
            return self.zip

        patcher = mock.patch.object(acg, "_get", side_effect=fake_get)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_download_records_credit_and_caches(self):
        result = acg.download("Wood001", "1K-JPG", self.tmp, credits_file=self.credits)
        self.assertFalse(result["cached"])
        self.assertTrue((self.tmp / "Wood001" / "Wood001_1K-JPG_Color.jpg").exists())
        entry = json.loads(self.credits.read_text())["ambientcg:Wood001"]
        self.assertEqual(entry["licence"], "CC0-1.0")
        self.assertIn("1k.zip", self.calls[-1])

        n = len(self.calls)
        again = acg.download("Wood001", "1K-JPG", self.tmp, credits_file=self.credits)
        self.assertTrue(again["cached"])
        self.assertEqual(len(self.calls), n)

    def test_unknown_resolution(self):
        with self.assertRaises(LookupError):
            acg.download("Wood001", "8K-PNG", self.tmp, credits_file=self.credits)

    def test_search_shapes_results(self):
        result = acg.search("wood", limit=1)
        self.assertEqual(result, [{"id": "Wood001", "name": "Wood 001", "category": "Wood", "tags": ["wood", "planks"]}])


class CreditsTests(unittest.TestCase):
    def test_upsert(self):
        path = temp_dir(self) / "nested" / "credits.json"
        asset_credits.record_credit("a", "polyhaven", "CC0-1.0", credits_file=path)
        asset_credits.record_credit("a", "polyhaven", "CC0-1.0", author="X", credits_file=path)
        asset_credits.record_credit("b", "polypizza", "CC-BY-3.0", author="Y", credits_file=path)
        data = json.loads(path.read_text())
        self.assertEqual(sorted(data), ["polyhaven:a", "polypizza:b"])
        self.assertEqual(data["polyhaven:a"]["author"], "X")


if __name__ == "__main__":
    unittest.main()
