import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from install_prepared_hud import install


class HudInstallationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.assets, self.score, self.marker = [root / p for p in ("assets", "score", "marker")]
        (self.score / "runtime").mkdir(parents=True)
        self.marker.mkdir()
        self.pixels = bytes([12, 34, 56, 78])
        for cache in (self.score, self.marker):
            (cache / "atlas.rgba").write_bytes(self.pixels)
        self.score_json = {"format": "skate3-scoring-hud", "version": 1,
                           "shapes": {"1": [{"texture": {"rgba": "atlas.rgba", "width": 1, "height": 1}}]},
                           "fonts": {"native": {"texture": "atlas.rgba", "definition": {
                               "textures": [{"width": 1, "height": 1}]}}}}
        (self.score / "runtime/trickdisplay.json").write_text(json.dumps(self.score_json))
        self.marker_json = {"version": 1, "canvas": [1280, 720], "meshes": [{}],
                            "textures": [{"file": "atlas.rgba", "width": 1, "height": 1,
                                          "output_sha256": hashlib.sha256(self.pixels).hexdigest()}]}
        self.write_marker()

    def write_marker(self):
        (self.marker / "hud.json").write_text(json.dumps(self.marker_json))

    def run_install(self, **kwargs):
        return install(self.assets, self.score, self.marker, **kwargs)

    def test_preserves_bytes_and_repeat_install_is_a_noop(self):
        files = self.run_install()
        before = {p: (self.assets / p).stat().st_mtime_ns for p in files}
        self.assertEqual(files, self.run_install())
        self.assertEqual(before, {p: (self.assets / p).stat().st_mtime_ns for p in files})
        self.assertEqual((self.assets / "private/hud/atlas.rgba").read_bytes(), self.pixels)
        self.assertEqual((self.assets / "private/session-marker/hud.json").read_bytes(),
                         (self.marker / "hud.json").read_bytes())

    def test_invalid_second_cache_does_not_partially_install_first(self):
        (self.marker / "atlas.rgba").write_bytes(b"bad")
        with self.assertRaisesRegex(ValueError, "RGBA size"):
            self.run_install()
        self.assertFalse(self.assets.exists())

    def test_hash_and_path_failures(self):
        (self.marker / "atlas.rgba").write_bytes(b"fake")
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            self.run_install()
        self.marker_json["textures"][0]["file"] = "../outside.rgba"
        self.write_marker()
        with self.assertRaisesRegex(ValueError, "escapes cache"):
            self.run_install()
        self.assertFalse(self.assets.exists())

    def test_differing_existing_file_requires_replace(self):
        self.run_install()
        target = self.assets / "private/hud/atlas.rgba"
        target.write_bytes(b"keep")
        with self.assertRaisesRegex(ValueError, "Different installed"):
            self.run_install()
        self.assertEqual(target.read_bytes(), b"keep")
        self.run_install(replace=True)
        self.assertEqual(target.read_bytes(), self.pixels)


if __name__ == "__main__":
    unittest.main()
