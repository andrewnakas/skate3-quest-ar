import json
from pathlib import Path
import tempfile
import unittest

from prepare_hud import font_mapping
from asset_pipeline.vlt import hash64


class Cache:
    manifest = {'bundles': [{'name': 'fonts/original', 'font': {'family': 'Original family'}}]}

    def add_font_alias(self, name, target, provenance):
        self.alias = (name, target)


class FontIdentityTests(unittest.TestCase):
    def test_readable_and_numeric_exports_resolve_the_same_original_font(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'collections.json'
            for named_class in (True, False):
                for named_fields in (True, False):
                    fields = {'AptName': {'data': 'Original APT name'}, 'FileName': {'data': 'original'}}
                    fields.update({key: {'data': '3F800000'} for key in ('ScaleX', 'ScaleY', 'OffsetX', 'OffsetY')})
                    if not named_fields:
                        fields = {f'Hash_{hash64(key):016X}': value for key, value in fields.items()}
                    path.write_text(json.dumps({'collections': [{
                        'class': 'fe_fonts' if named_class else 'Hash_FECFBCAF356518C4',
                        'key': 'original', 'sha256': '', 'fields': fields}]}))
                    cache = Cache()
                    result = font_mapping(path, cache)
                    self.assertEqual(cache.alias, ('Original APT name', 'Original family'))
                    self.assertEqual(result['Original APT name']['native_layout']['ScaleX'], 1.0)


if __name__ == '__main__':
    unittest.main()
