"""Independent HUD installation checks without retail data."""
import json
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parent))
import prepare_runtime_huds as huds


def scoring(game, output, *args):
    (output/'runtime').mkdir(parents=True)
    (output/'runtime/trickdisplay.json').write_text(json.dumps(dict(
        version=1, format='skate3-scoring-hud', shapes={}, fonts={})))


def marker(game, output):
    output.mkdir(parents=True)
    (output/'hud.json').write_text(json.dumps(dict(version=1, canvas=[1280,720], meshes=[{}], textures=[])))


class HudWorkspace(unittest.TestCase):
    def test_long_install_uses_short_temporary_workspace_and_keeps_outputs(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); assets=root/'assets'; seen=[]
            long_work=root/('nested-install-'*12)/'conversion/hud'
            def prepare(game, output, *args):
                seen.append(output)
                self.assertNotIn(long_work,output.parents)
                self.assertLess(len(str(output/'source-cache/assets/data/fe/source/images/buttons/xbox360/buttons/0000_button_DPad_Down_hud.Texture.texture.bin')),260)
                scoring(game,output,*args)
            with patch.object(huds,'prepare_scoring',side_effect=prepare),patch.object(huds,'prepare_marker',side_effect=marker):
                self.assertEqual(len(huds.prepare(root/'game',assets,long_work)),2)
            self.assertTrue((assets/'private/hud/runtime/trickdisplay.json').is_file())
            self.assertTrue(all(not p.exists() for p in seen))

    def test_one_missing_hud_still_installs_other_and_receipts_allow_reuse(self):
        from tools.asset_pipeline.group_receipts import record, damaged
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); assets=root/'assets'
            with patch.object(huds,'prepare_scoring',side_effect=KeyError('missing font')),patch.object(huds,'prepare_marker',side_effect=marker):
                self.assertEqual(len(huds.prepare(root/'game',assets,root/'work')),1)
            self.assertEqual(json.loads((assets/'private/hud-availability.json').read_text())['status'],'unavailable')
            receipt=record(root,'hud')
            self.assertTrue(receipt)
            self.assertEqual(damaged(root,{'outputs':{'hud':receipt}},exclude=('core','character','environment','maps')),set())

    def test_failure_keeps_verified_existing_huds(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);assets=root/'assets'
            scoring(None,assets/'private/hud');marker(None,assets/'private/session-marker')
            before=(assets/'private/hud/runtime/trickdisplay.json').read_bytes()
            with patch.object(huds,'prepare_scoring',side_effect=RuntimeError('decode failed')),patch.object(huds,'prepare_marker',side_effect=FileNotFoundError('missing bank')):
                self.assertEqual(huds.prepare(root/'game',assets,root/'work'),{})
            self.assertEqual((assets/'private/hud/runtime/trickdisplay.json').read_bytes(),before)
            for name in ('hud','session-marker'):
                self.assertEqual(json.loads((assets/'private'/(name+'-availability.json')).read_text())['status'],'retained')

    def test_disk_errors_still_abort(self):
        with tempfile.TemporaryDirectory() as temp,patch.object(huds,'prepare_scoring',side_effect=PermissionError('read denied')):
            with self.assertRaises(PermissionError):huds.prepare(Path(temp),Path(temp)/'assets',Path(temp)/'work')
