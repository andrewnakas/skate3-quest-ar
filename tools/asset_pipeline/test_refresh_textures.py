import json
import struct
import tempfile
import unittest
from pathlib import Path
from .refresh_textures import refresh


class RefreshTests(unittest.TestCase):
    def test_payload_refresh_preserves_tail_and_failed_refresh_preserves_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            string = lambda x: struct.pack('<I', len(x)) + x
            prefix = b'SKATE14\0' + struct.pack('<I', 0x12345678) + string(b'fixture') + bytes(49*4)
            prefix += struct.pack('<9I', 0, 1, 0, 0, 0, 0, 0, 0, 0)
            old = bytes(16)
            header = string(b'texture') + struct.pack('<3I', 2, 2, 0)
            tail = b'geometry/collision/metadata-preservation-fixture'
            source = root/'input.skate'
            source.write_bytes(prefix + header + struct.pack('<II', 0, 16) + old + tail)
            cache = bytes(range(16))
            (root/'new.rgba').write_bytes(cache)
            manifest = root/'manifest.json'
            manifest.write_text(json.dumps({'textures': {'texture': dict(width=2, height=2, rgba='new.rgba')}}))
            output = root/'out.skate'
            report = refresh(manifest, source, output)
            self.assertEqual(report['count'], 1)
            self.assertEqual(output.read_bytes()[:len(prefix)], prefix)
            self.assertEqual(output.read_bytes()[report['new_tail_offset']:], tail)
            saved = output.read_bytes()
            (root/'new.rgba').write_bytes(b'bad')
            with self.assertRaises(ValueError): refresh(manifest, source, output)
            self.assertEqual(output.read_bytes(), saved)
            with self.assertRaises(ValueError): refresh(manifest, source, source)
            self.assertFalse(list(root.glob('*.tmp')))
