import io
import struct
import tempfile
import unittest
import zlib
from pathlib import Path
from types import SimpleNamespace

import numpy as np
from PIL import Image
from .map_writer import SpawnSelector, write_textures


class MapWriterTests(unittest.TestCase):
    def test_parallel_textures_match_serial_format_exactly(self):
        with tempfile.TemporaryDirectory() as work:
            root=Path(work);textures={};expected=io.BytesIO()
            rng=np.random.default_rng(23)
            for name, cube, png in [('z-cube', True, False), ('a-rgba', False, False), ('m-png', False, True), ('b-flat', False, False)]:
                pixels=rng.integers(0,256,(24 if cube else 4,4,4),dtype=np.uint8)
                if name=='b-flat':pixels[:]=0
                path=root/(name+('.png' if png else '.rgba'))
                if png:Image.fromarray(pixels).save(path)
                else:path.write_bytes(pixels.tobytes())
                textures[name]=dict(width=4,height=len(pixels),cube_faces=6 if cube else 1,
                                    **{'png' if png else 'rgba':path.name})
            for name,entry in sorted(textures.items()):
                if 'png' in entry:
                    with Image.open(root/entry['png']) as image:pixels=np.array(image.convert('RGBA'))
                else:pixels=np.frombuffer((root/entry['rgba']).read_bytes(),dtype=np.uint8).reshape(entry['height'],4,4)
                raw=(pixels if entry['cube_faces']==6 else pixels[::-1]).tobytes()
                packed=zlib.compress(raw,1);method=1
                if len(packed)>=len(raw):packed=raw;method=0
                encoded=name.encode()
                expected.write(struct.pack('<I',len(encoded))+encoded)
                expected.write(struct.pack('<5I',4,entry['height'],1,method,len(packed))+packed)
            actual=io.BytesIO();write_textures(actual,root,textures)
            self.assertEqual(actual.getvalue(),expected.getvalue())
            empty=io.BytesIO();write_textures(empty,root,{})
            self.assertEqual(empty.getvalue(),b'')

    def test_texture_failure_is_propagated(self):
        with tempfile.TemporaryDirectory() as work:
            with self.assertRaises(FileNotFoundError):
                write_textures(io.BytesIO(),Path(work),{'missing':dict(rgba='missing',width=4,height=4)})

    def test_spawn_streaming_preserves_ties_and_university_height(self):
        def mesh(x,y,z):
            return SimpleNamespace(bounds_min=(x-10,y,z-10),bounds_max=(x+10,y,z+10),triangles=[
                SimpleNamespace(a=(x-10,y,z-10),b=(x,y,z+10),c=(x+10,y,z-10))])
        selector=SpawnSelector('DIST_Test')
        selector.consider([mesh(0,5,0),mesh(0,20,0)])
        self.assertEqual(selector.result('test'),(0.,6.,-10/3))
        selector.consider([mesh(500,30,500)])
        self.assertEqual(selector.result('test'),(0.,6.,-10/3))
        university=SpawnSelector('DIST_University')
        university.consider([mesh(330,100,-710),mesh(330,132,-710),mesh(0,132,0)])
        self.assertEqual(university.result('University'),(330.,133.,-710.))
        with self.assertRaisesRegex(ValueError,'No supported spawn'):
            SpawnSelector('DIST_Test').result('test')


if __name__=='__main__':unittest.main()
