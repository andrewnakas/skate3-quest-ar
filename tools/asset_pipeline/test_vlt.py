"""Synthetic fixtures for the binary array layout; contains no game data."""
import struct
import unittest
from .vlt import array_items, array_text_items


class ArrayLayoutTests(unittest.TestCase):
    def test_vector4_header_alignment(self):
        first = struct.pack('>4f', .75, .35, 0, 0)
        second = struct.pack('>4f', 1, 1, 1, 1)
        raw = struct.pack('>4H', 2, 2, 16, 0x8000) + bytes(8) + first + second
        result = array_items(raw, 0, 16, 4)
        self.assertEqual(result['items'], [first.hex().upper(), second.hex().upper()])
        self.assertEqual(result['alignment'], 16)

    def test_each_element_is_aligned_at_absolute_offset(self):
        a, b = struct.pack('>3f', 2, 3, 4), struct.pack('>3f', 5, 6, 7)
        raw = bytes(4) + struct.pack('>4H', 3, 2, 12, 0) + bytes(4)
        raw += a + bytes(4) + b + bytes(4) + bytes(12)
        result = array_items(raw, 4, 12, 4)
        self.assertEqual(result['capacity'], 3)
        self.assertEqual(result['items'], [a.hex().upper(), b.hex().upper()])

    def test_empty_and_pointer_arrays(self):
        self.assertEqual(array_items(struct.pack('>4H', 0, 0, 4, 0), 0, 4, 2)['items'], [])
        raw = struct.pack('>4H2I', 2, 2, 4, 0, 0x1234, 0)
        self.assertEqual(array_items(raw, 0, 4, 2)['items'], ['00001234', '00000000'])

    def test_invalid_header_or_truncated_capacity_fails(self):
        for raw in [struct.pack('>4H', 1, 2, 4, 0), struct.pack('>4H', 1, 1, 8, 0),
                    struct.pack('>4HI', 2, 1, 4, 0, 123)]:
            with self.assertRaises(ValueError):
                array_items(raw, 0, 4, 2)

    def test_text_array_resolves_relocated_pointers(self):
        binary = bytes(8) + 'test\0caf\u00e9\0'.encode()
        self.assertEqual(array_text_items(binary, ['00000008', '0000000D', '00000000']),
                         ['test', 'caf\u00e9', ''])
        with self.assertRaises(ValueError):
            array_text_items(binary, ['0000FFFF'])


if __name__ == '__main__':
    unittest.main()
