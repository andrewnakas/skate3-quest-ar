import unittest
from types import SimpleNamespace
from .marquee_assets import Resources, MissingMarqueeAsset


MODEL = 'data/content/marquee/model/dem_bones/Rostral/0x0000128603e38811.rx2'
RECIPE = 'data/content/recipe/marquee/dem_bones.xml'
XML = b'<compositeasset><comp n="Rostral"><mod><lod idx="0" arenaid="0x0000128603e38811"/></mod></comp></compositeasset>'


def archive(files):
    return SimpleNamespace(entries=[SimpleNamespace(path=p) for p in files], read=lambda e: files[e.path])


class MarqueeAssets(unittest.TestCase):
    def test_exact_and_relocated_authored_identity(self):
        for path in (MODEL, 'data/content/marquee/shared/0x0000128603e38811.rx2'):
            resources = Resources(archive({RECIPE:XML,path:b'original'}))
            self.assertEqual(resources.recipe('dem_bones').tag,'compositeasset')
            self.assertEqual(resources.read(MODEL),b'original')

    def test_missing_head_never_uses_lower_lod(self):
        resources = Resources(archive({RECIPE:XML,MODEL.replace('1286','1287'):b'lower lod'}))
        with self.assertRaisesRegex(MissingMarqueeAsset,'0x0000128603e38811'):
            resources.recipe('dem_bones')

    def test_ambiguous_identity_is_not_silently_substituted(self):
        resources = Resources(archive({'a/0x0000128603e38811.rx2':b'a','b/0x0000128603e38811.rx2':b'b'}))
        with self.assertRaisesRegex(ValueError,'Ambiguous'):
            resources.read(MODEL)

    def test_corrupt_recipe_is_not_treated_as_missing_optional_content(self):
        import xml.etree.ElementTree as ET
        resources = Resources(archive({RECIPE:b'bad XML'}))
        with self.assertRaises(ET.ParseError):resources.recipe('dem_bones')
