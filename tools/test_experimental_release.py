"""Publisher checks without network or a game process."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
import publish_experimental as p
from test_updater import metadata


class RollingReleaseTests(unittest.TestCase):
    def test_only_complete_build_sets_are_retained(self):
        assets = [dict(name=n, state='uploaded') for n in (
            'release-3.json', 'skate3rust-windows-x64-build-3.zip',
            'skate3rust-windows-x64-build-3.zip.sha256', 'release-4.json')]
        self.assertEqual(p.complete_builds(assets), [3])
        assets[0]['state'] = 'new'
        self.assertEqual(p.complete_builds(assets), [])

    def test_package_validation_and_corruption(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            meta = {**metadata(), 'tag': 'experimental'}
            meta['files'] = {n: hashlib.sha256(b'program').hexdigest() for n in meta['files']}
            (root/'release.json').write_text(json.dumps(meta))
            with zipfile.ZipFile(root/p.PACKAGE, 'w') as z:
                z.writestr(p.PREFIX+'release.json', json.dumps(meta))
                for name in meta['files']:
                    z.writestr(p.PREFIX+name, b'program')
            digest = hashlib.sha256((root/p.PACKAGE).read_bytes()).hexdigest()
            checksum = root/(p.PACKAGE+'.sha256')
            checksum.write_text(digest+'  '+p.PACKAGE)
            self.assertEqual(p.validate(root), (meta, digest))
            checksum.write_text('0'*64+'  '+p.PACKAGE)
            with self.assertRaisesRegex(ValueError, 'checksum'):
                p.validate(root)


if __name__ == '__main__':
    unittest.main()
