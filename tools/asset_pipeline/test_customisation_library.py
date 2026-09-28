"""Offline CAC preparation checks; no window, game process, or renderer."""
import json
import os
import struct
import unittest
from pathlib import Path

import numpy as np

from tools.asset_pipeline.customisation_library import friendly, variant_label
from tools.asset_pipeline.retail_character import RX2


class LibraryTests(unittest.TestCase):
    def test_secondary_uv_is_float2_not_four_normalized_shorts(self):
        data = struct.pack('>4f', .125, .75, .875, .625)
        description = {'stride': 8, 'elements': [{'usage_name': 'TEXCOORD',
            'usage_index': 1, 'format_name': RX2.VTX_FORMATS[0x002C23A5][0], 'offset': 0}]}
        self.assertEqual(RX2._decode_vertices(data, 0, 2, description)['uvs2'],
                         [(.125, .75), (.875, .625)])

    def test_menu_names_remove_exporter_words_and_explain_wear(self):
        self.assertEqual(friendly('male_longsleeve_tshirt_stampable_sleeves_up'), 'Rolled-Sleeve Tee')
        self.assertEqual(variant_label('Feet', 'Busenitz Pro Shoes', 'busenitz_pro_material_d'), 'Worn')
        self.assertEqual(variant_label('Feet', 'Busenitz Pro Shoes', 'busenitz_pro_material'), 'New')
        self.assertEqual(friendly('tattoo_biglizard_mat'), 'Big Lizard')

    @unittest.skipUnless(os.environ.get('SKATE_CAC_TEST_LIBRARY'), 'owned library is opt-in')
    def test_owned_geometry_and_dependencies(self):
        path = Path(os.environ['SKATE_CAC_TEST_LIBRARY'])
        library = json.loads(path.read_text(encoding='utf-8'))
        assets = path.parents[2]
        self.assertEqual(library['errors'], [])
        self.assertEqual(len(library['models']), 480)
        self.assertEqual(len(library['tattoos']), 170)
        for part in library['models'].values():
            data = (assets / part['scene']).read_bytes()
            magic, version, size, text_size = struct.unpack_from('<4I', data)
            self.assertEqual((magic, version, size), (0x46546C67, 2, len(data)))
            doc = json.loads(data[20:20+text_size])
            blob = data[28+text_size:]

            def array(index):
                accessor = doc['accessors'][index]
                view = doc['bufferViews'][accessor['bufferView']]
                width = {'SCALAR':1, 'VEC2':2, 'VEC3':3, 'VEC4':4, 'MAT4':16}[accessor['type']]
                result = np.frombuffer(blob, dtype={5126:'<f4', 5123:'<u2', 5125:'<u4'}[accessor['componentType']],
                    count=accessor['count']*width, offset=view.get('byteOffset',0)+accessor.get('byteOffset',0))
                self.assertTrue(np.isfinite(result).all())
                return result.reshape(-1,width)

            for mesh in doc['meshes']:
                self.assertEqual(mesh['extras']['targetNames'], library['morphs'])
                self.assertEqual(mesh['weights'], [0.,0.]+[.25]*17+[0.]*3)
                for primitive in mesh['primitives']:
                    a = primitive['attributes']; count = len(array(a['POSITION']))
                    self.assertLess(int(array(primitive['indices']).max()), count)
                    self.assertLess(int(array(a['JOINTS_0']).max()), len(doc['skins'][0]['joints']))
                    np.testing.assert_allclose(array(a['WEIGHTS_0']).sum(1), 1., atol=1e-6)
                    self.assertEqual(len(primitive['targets']),22)
                    for target in primitive['targets']:
                        self.assertEqual(len(array(target['POSITION'])),count)
                        self.assertEqual(len(array(target['NORMAL'])),count)
                    if part['slot'] == 'Hair':
                        self.assertIn('TEXCOORD_1', a)
                        self.assertEqual(len(array(a['TEXCOORD_1'])), count)
                    if (part['slot']=='Arm' and part['flags'].get('ArmModelType')=='shoulder') or (part['slot']=='Leg' and part['flags'].get('LegModelType') in ['thigh','thighdown'] and part['flags'].get('RequiresSockStyle','none') in ['','none']):
                        uv = array(a['TEXCOORD_1'])
                        for side in ['Q3','Q4']:
                            top,bottom,left,right = map(float,part['flags']['StampUVConstraint'+side].split(','))
                            inside=(uv[:,0]>=left)&(uv[:,0]<=right)&(uv[:,1]>=1-top)&(uv[:,1]<=1-bottom)
                            self.assertTrue(inside.any(),part['name']+' '+side)
        for material in library['materials'].values():
            for channel in ['diffuse','normal','rough','opacity']:
                if material.get(channel): self.assertTrue((assets/material[channel]).is_file())
        # V3 preserves the native hair opacity texture independently of diffuse.
        hair = [m for m in library['materials'].values() if m.get('opacity')]
        self.assertEqual(len(hair), 31)
        for material in hair:
            self.assertNotEqual(material['diffuse'], material['opacity'])
            self.assertIn('/decoded/', material['diffuse'])
            self.assertTrue(material['alpha'])
        # Native moussed-up hair demonstrates why diffuse A cannot be coverage.
        from PIL import Image
        sample = library['materials']['00000c5603e38817']
        diffuse = np.asarray(Image.open(assets/sample['diffuse']).convert('RGBA'))
        opacity = np.asarray(Image.open(assets/sample['opacity']).convert('RGBA'))
        self.assertLess(np.mean(diffuse[:,:,3] >= 128), .01)
        self.assertGreater(np.mean(opacity[:,:,0] >= 128), .4)
        for tattoo in library['tattoos'].values():
            self.assertTrue((assets/tattoo['texture']).is_file())


if __name__ == '__main__':
    unittest.main()
