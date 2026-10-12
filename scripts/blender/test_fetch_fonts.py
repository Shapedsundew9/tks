"""Offline unit tests for fetch_fonts.py (no network)."""

import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fetch_fonts  # noqa: E402


def temp_dir(case):
    path = Path(tempfile.mkdtemp())
    case.addCleanup(shutil.rmtree, path, ignore_errors=True)
    return path


class FetchFontsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = temp_dir(self)
        self.credits = self.tmp / "credits.json"
        self.calls = []

        def fake_get(url, timeout=60):
            self.calls.append(url)
            # Dummy font file bytes (> 100 bytes)
            return b"dummy_font_bytes_" * 10

        patcher = mock.patch.object(fetch_fonts, "_get", side_effect=fake_get)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_list_fonts(self):
        fonts = fetch_fonts.list_fonts()
        self.assertGreaterEqual(len(fonts), 7)
        keys = [f["key"] for f in fonts]
        self.assertIn("archivoblack", keys)
        self.assertIn("ibmplexmono", keys)

    def test_resolve_key(self):
        self.assertEqual(fetch_fonts.resolve_key("archivoblack"), "archivoblack")
        self.assertEqual(fetch_fonts.resolve_key("ArchivoBlack-Regular"), "archivoblack")
        self.assertEqual(fetch_fonts.resolve_key("ArchivoBlack"), "archivoblack")
        self.assertEqual(fetch_fonts.resolve_key("IBMPlexMono"), "ibmplexmono")
        with self.assertRaises(LookupError):
            fetch_fonts.resolve_key("NonExistentFont")

    def test_download_and_record_credit(self):
        res = fetch_fonts.download("archivoblack", dest_root=self.tmp, credits_file=self.credits)
        self.assertFalse(res["cached"])
        self.assertTrue(res["path"].exists())
        self.assertIn("ArchivoBlack-Regular.ttf", res["path"].name)

        credits_data = json.loads(self.credits.read_text())
        self.assertIn("google_fonts:font:archivoblack", credits_data)
        entry = credits_data["google_fonts:font:archivoblack"]
        self.assertEqual(entry["licence"], "OFL-1.1")
        self.assertEqual(entry["author"], "Omnibus-Type")

        # Second download should use cache
        n_calls = len(self.calls)
        cached_res = fetch_fonts.download("archivoblack", dest_root=self.tmp, credits_file=self.credits)
        self.assertTrue(cached_res["cached"])
        self.assertEqual(len(self.calls), n_calls)


if __name__ == "__main__":
    unittest.main()
