"""RX2 irradiance layout checks without owned-game fixtures."""
import struct
import unittest

from .irradiance import decode


def resource():
    data = bytearray(272)
    data[:7] = b'\x89RW4xb2'
    struct.pack_into('>I', data, 32, 1)
    struct.pack_into('>I', data, 48, 64)
    struct.pack_into('>6I', data, 64, 96, 0, 176, 0, 0, 0xEB0024)
    struct.pack_into('>2I', data, 96, 1, 16)
    values = [i / 4 for i in range(39)]
    struct.pack_into('>39fI', data, 112, *values, 0xDEDEDEDE)
    return data, values


class IrradianceTests(unittest.TestCase):
    def test_preserves_coefficients_position_and_link_word(self):
        data, values = resource()
        groups = decode(data)
        self.assertEqual(len(groups), 1)
        self.assertEqual(struct.unpack('<39fI', groups[0][0]), (*values, 0xDEDEDEDE))

    def test_rejects_records_outside_section(self):
        data, _ = resource()
        struct.pack_into('>I', data, 96, 2)
        with self.assertRaises(ValueError):
            decode(data)

    def test_rejects_nonfinite_sh(self):
        data, _ = resource()
        struct.pack_into('>f', data, 112, float('nan'))
        with self.assertRaises(ValueError):
            decode(data)
