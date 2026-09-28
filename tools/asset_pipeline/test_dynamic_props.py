import struct
import unittest
from unittest.mock import patch
from pathlib import Path
from tempfile import TemporaryDirectory
import numpy as np

from .dynamic_props import locators, transform_mesh, template_meshes, save_catalog, load_catalog


def resource(kind, payload):
    raw = bytearray(88+len(payload))
    raw[:7] = b'\x89RW4xb2'
    struct.pack_into('>I', raw, 32, 1)
    struct.pack_into('>I', raw, 48, 64)
    struct.pack_into('>6I', raw, 64, 88, 0, len(payload), 0, 0, kind)
    raw[88:] = payload
    return raw


class DynamicPropsTests(unittest.TestCase):
    def test_transform_does_not_read_other_meshes(self):
        class LazyArrays(dict):
            def __getitem__(self, key):
                if key.endswith('_1'):
                    raise AssertionError('Unrelated mesh was decompressed')
                return super().__getitem__(key)
        arrays = LazyArrays(vertices_0=np.zeros((3, 3)), faces_0=np.array([[0, 1, 2]]), vertices_1=None)
        result = transform_mesh(arrays, 0, np.eye(4))
        np.testing.assert_array_equal(result['faces_0'], [[0, 1, 2]])

    def test_catalog_roundtrip_preserves_double_precision_and_bindings(self):
        template = dict(matrix=np.arange(16, dtype=float).reshape(4, 4)/7,
                        model_matrix=np.eye(4), npz=Path('model.npz'),
                        meshes=[dict(retail_texture_ids={'diffuse': '0xf123456789abcdef'})],
                        asset_id='original', mesh_info=[112])
        textures={'0xf123456789abcdef': dict(width=4, height=4, rgba='original.rgba')}
        with TemporaryDirectory() as work:
            path=Path(work)/'catalog.json'
            with patch('tools.asset_pipeline.dynamic_props.catalog', return_value=({'template':template}, textures)):
                save_catalog([], path)
            templates, actual_textures=load_catalog(path)
        actual=templates['template']
        for key in ('matrix','model_matrix'):
            self.assertEqual(actual[key].tobytes(),template[key].tobytes())
        self.assertEqual(actual['meshes'],template['meshes'])
        self.assertEqual(actual_textures,textures)

    def test_locator_id_and_row_matrix(self):
        payload = bytearray(166)
        struct.pack_into('>5I', payload, 0, 0, 1, 1, 32, 160)
        m = np.eye(4); m[3, :3] = [10, 20, -30]
        struct.pack_into('>16f', payload, 32, *m.ravel())
        struct.pack_into('>3Q2I', payload, 128, 123, 456, 789, 0, 160)
        payload[160:] = b'bench\0'
        rows = locators(resource(0xEB001D, payload))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]['template_id'], '0000000000000315')
        self.assertEqual(rows[0]['matrix'][3], [10, 20, -30, 1])
        self.assertEqual(rows[0]['name'], 'bench')
        struct.pack_into('>I', payload, 16, 159)
        with self.assertRaisesRegex(ValueError, 'layout'):
            locators(resource(0xEB001D, payload))

    def test_scaled_rotation_and_normal_transform(self):
        m = np.array([[0, 0, -2, 0], [0, 3, 0, 0], [4, 0, 0, 0], [10, 20, 30, 1]], dtype=float)
        arrays = {'vertices_0': np.array([[1, 2, 3]]), 'normals_0': np.array([[0, 0, 1]]), 'faces_0': np.array([[0, 0, 0]])}
        transformed = transform_mesh(arrays, 0, m)
        np.testing.assert_allclose(transformed['vertices_0'], [[22, 26, 28]])
        np.testing.assert_allclose(transformed['normals_0'], [[1, 0, 0]])
        np.testing.assert_array_equal(arrays['vertices_0'], [[1, 2, 3]])

    def test_missing_template_reference_is_rejected(self):
        payload = bytearray(192)
        struct.pack_into('>5I', payload, 0, 0, 1, 1, 32, 192)
        struct.pack_into('>I', payload, 160, 100)
        with self.assertRaisesRegex(ValueError, 'model reference'):
            template_meshes(resource(0xEB000D, payload))


if __name__ == '__main__':
    unittest.main()
