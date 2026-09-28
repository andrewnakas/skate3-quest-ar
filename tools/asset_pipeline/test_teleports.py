import struct
import unittest
import numpy as np
from .teleports import language_table, location_records
from .test_dynamic_props import resource


class TeleportTests(unittest.TestCase):
    def test_language_join_uses_keys_not_storage_order(self):
        # Offsets are relative to the chunk payload; strings need not follow
        # the same physical order as the sorted hash index.
        raw = bytearray(36 + 16 + 11)
        struct.pack_into('<5I', raw, 0, 0x39000, len(raw)-8, 2, 28, 44)
        struct.pack_into('<4I', raw, 36, 123, 6, 456, 0)
        raw[52:] = b'First\0Last\0'
        self.assertEqual(language_table(raw), {123: b'Last', 456: b'First'})
        struct.pack_into('<I', raw, 40, 999)
        with self.assertRaises(ValueError): language_table(raw)

    def test_locator_preserves_heading_and_world_space_position(self):
        payload = bytearray(165)
        struct.pack_into('>5I', payload, 0, 1, 0, 0, 32, 160)
        m = np.array([[0, 0, -1, 0], [0, 1, 0, 0], [1, 0, 0, 0], [350, 140, -725, 1]], dtype=float)
        struct.pack_into('>16f', payload, 32, *m.ravel())
        struct.pack_into('>I', payload, 136, 160)
        payload[160:] = b'spot\0'
        rows = location_records(resource(0xEB0009, payload))
        self.assertEqual(rows[0]['locator'], 'spot')
        np.testing.assert_array_equal(rows[0]['matrix'], m)
        struct.pack_into('>I', payload, 136, 32)
        with self.assertRaisesRegex(ValueError, 'overlaps'): location_records(resource(0xEB0009, payload))

    def test_bad_record_count_is_rejected(self):
        payload = bytearray(32)
        struct.pack_into('>5I', payload, 0, 1, 0, 0, 32, 160)
        with self.assertRaises(ValueError): location_records(resource(0xEB0009, payload))
