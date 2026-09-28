import json
import tempfile
import unittest
from pathlib import Path
import numpy as np
from PIL import Image
from test_converter import fixture
from converter import create_profile
from library_import import import_model

class LibraryTests(unittest.TestCase):
    def test_publish_preview_deduplicate_and_failed_import(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); ref=root/'ref.glb'; src=root/'model.glb'; library=root/'library'
            fixture(ref,False); fixture(src,True,.01)
            profile=create_profile(src,ref)
            identity=import_model(src,ref,library,None,profile)
            entry=library/'entries'/identity
            self.assertTrue((entry/'source.glb').is_file())
            self.assertEqual(json.loads((entry/'manifest.json').read_text())['name'],'model')
            with Image.open(entry/'preview.png') as preview:
                self.assertEqual(preview.size,(256,320))
                self.assertGreater(len(np.unique(np.asarray(preview).reshape(-1,3),axis=0)),1)
            self.assertEqual(import_model(src,ref,library,None,profile),identity)
            bad=root/'bad.glb';bad.write_bytes(b'broken')
            with self.assertRaises(Exception):import_model(bad,ref,library,None,profile)
            self.assertEqual(len(list((library/'entries').iterdir())),1)
            converted=import_model(entry/'character.glb',ref,library,None,profile)
            self.assertTrue((library/'entries'/converted/'preview.png').is_file())

if __name__=='__main__':unittest.main()
