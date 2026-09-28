"""Regression fixtures for padded Xbox tiles and BC colour palettes."""
import importlib.util
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch

UTT = Path(__file__).resolve().parents[1] / 'vendor/utt'
sys.path.insert(0, str(UTT))
import rx2_fast

# Exercise the standalone fallback as well as the NumPy implementation.
spec = importlib.util.spec_from_file_location('rx2_scalar_test', UTT / 'rx2_parser.py')
scalar = importlib.util.module_from_spec(spec)
with patch.dict(sys.modules, {'rx2_fast': None}):
    spec.loader.exec_module(scalar)


class TextureDecodeTests(unittest.TestCase):
    def test_padding_cannot_overwrite_visible_rows(self):
        for width, pitch in ((w, p) for w in (4, 8, 16, 32) for p in (2, 4, 8, 16)):
            with self.subTest(width=width, pitch=pitch):
                units = max(32*32, 4096//pitch)
                raw = bytearray(units*pitch)
                for offset in range(units):
                    x = scalar._x360_tiled_x(offset, width, pitch)
                    y = scalar._x360_tiled_y(offset, width, pitch)
                    if x < width and y < 32:
                        raw[offset*pitch:offset*pitch+pitch] = bytes([1+y])*pitch
                expected = b''.join(bytes([1+y])*(width*pitch) for y in range(32))
                for decoder in (scalar, rx2_fast):
                    self.assertEqual(decoder._untile360(raw, width, pitch)[:len(expected)], expected)

    def test_bc1_keeps_transparent_palette(self):
        colour = struct.pack('>HH', 0x001f, 0xf800) + b'\xff'*4
        self.assertEqual(scalar._dxt1_block(colour, True), bytes(64))
        self.assertEqual(rx2_fast._decode(colour, 4, 4, 1, False), bytes(64))

    def test_bc2_bc3_use_four_colours_with_reversed_endpoints(self):
        colour = struct.pack('>HH', 0x001f, 0xf800) + b'\xff'*4
        for kind, alpha, block_decoder in ((3, b'\xaa'*8, scalar._dxt3_block),
                                           (5, b'\x00\xaa'+bytes(6), scalar._dxt5_block)):
            expected = bytes((170, 0, 85, 170))*16
            self.assertEqual(block_decoder(alpha+colour), expected)
            self.assertEqual(rx2_fast._decode(alpha+colour, 4, 4, kind, False), expected)
