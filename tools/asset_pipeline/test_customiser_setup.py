"""Fresh/update character publication without a game process or retail data."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from tools.asset_pipeline import customiser_setup as s


class CharacterSetup(unittest.TestCase):
    def test_gesture_menu_matches_runtime_table_without_reading_source(self):
        import re
        from tools.asset_pipeline.customisation_profiles import generate
        source = Path(__file__).resolve().parents[2]/'crates/skate-game/src/graph_host/motion_character_gesture.rs'
        count = len(re.findall(r'"B_GSTR_([A-Z_]+)"', source.read_text()))
        with patch.object(Path, 'read_text', side_effect=AssertionError('setup must not read Rust source')):
            menu = generate({'morphs': [], 'collections': []})
        self.assertEqual(len(menu[-1]['children'][0]['children'][0]['children']), count)

    def setUp(self):
        source = patch.object(s, 'source_fingerprint', return_value='owned-disc')
        source.start()
        self.addCleanup(source.stop)

    def test_failed_generation_preserves_previous_selection_and_assets(self):
        with tempfile.TemporaryDirectory() as temp:
            assets = Path(temp)/'assets'
            base = assets/'private/customisation'; base.mkdir(parents=True)
            old = {'set': 'a'*32, 'fingerprint': 'old'}
            (base/'current.json').write_text(json.dumps(old))
            with patch('tools.asset_pipeline.customisation_catalog.prepare', side_effect=RuntimeError('bad source')):
                with self.assertRaises(RuntimeError): s._prepare(Path(temp)/'source', assets, lambda _: None)
            self.assertEqual(json.loads((base/'current.json').read_text()), old)

    def test_crash_after_publication_cannot_resume_inside_the_live_generation(self):
        with tempfile.TemporaryDirectory() as temp:
            assets=Path(temp)/'assets';base=assets/'private/customisation'
            live=base/'sets'/('a'*32);live.mkdir(parents=True)
            (live/'catalog.json').write_text('working output')
            old={'set':'a'*32,'fingerprint':'v','source':'owned-disc'}
            (base/'current.json').write_text(json.dumps(old))
            (base/'pending.json').write_text(json.dumps(old))
            with patch.object(s,'fingerprint',return_value='v'), \
                 patch('tools.asset_pipeline.customisation_catalog.prepare',side_effect=RuntimeError('failed')):
                with self.assertRaises(RuntimeError):s._prepare(Path(temp)/'source',assets,lambda _:None)
            self.assertEqual((live/'catalog.json').read_text(),'working output')
            self.assertEqual(json.loads((base/'current.json').read_text()),old)
            self.assertNotEqual(json.loads((base/'pending.json').read_text())['set'],old['set'])

    def test_fresh_generation_contains_customiser_profiles_lighting_and_native_roster(self):
        with tempfile.TemporaryDirectory() as temp:
            assets = Path(temp)/'assets'
            data = dict(models={'body': {}}, materials={'cloth': {}}, errors=[],
                        defaults={'male': {'selections': {'Body': {'asset_id': 'body', 'material_id': 'cloth'}}}})
            def catalog(game, out):
                (out/'native.json').write_text('{}')
                (out/'catalog.json').write_text('{}')
                (out/'database').mkdir(exist_ok=True)
                (out/'database/collections.json').write_text('{}')
            def library(config):
                (Path(config['directory'])/config['library_index']).write_text(json.dumps(data))
                return data
            def lighting(game, assets, directory, data):
                (directory/'native-lighting.json').write_text('{"pro":{}}')
                (directory/'library-v3.json').write_text(json.dumps(data))
            def roster(game, assets, library, collections, work):
                library.mkdir(exist_ok=True)
                return roster_results.pop(0)
            roster_results = [[{'status': 'ready', 'key': 'pro'},
                                  {'status': 'unavailable', 'key': 'dem_bones', 'name': 'Dem Bones', 'error': 'missing head'}]]
            with patch('tools.asset_pipeline.customisation_catalog.prepare', side_effect=catalog), \
                 patch('tools.asset_pipeline.customisation_library.prepare', side_effect=library), \
                 patch('tools.asset_pipeline.customisation_profiles.generate', return_value=[]), \
                 patch('tools.asset_pipeline.customiser_lighting.prepare', side_effect=lighting), \
                 patch('tools.asset_pipeline.native_roster.prepare', side_effect=roster):
                s.prepare(Path(temp)/'source', assets, lambda _: None)
            base = assets/'private/customisation'
            current = json.loads((base/'current.json').read_text())
            generation = base/'sets'/current['set']
            for path in ('library-v3.json', 'extra-menu.json', 'native-lighting.json', 'native-roster/complete.json'):
                self.assertTrue((generation/path).is_file(), path)
            completeness=json.loads((generation/'native-roster/complete.json').read_text())
            self.assertEqual(completeness['characters'],1)
            self.assertEqual(completeness['unavailable'][0]['key'],'dem_bones')
            with patch('tools.asset_pipeline.customisation_catalog.prepare', side_effect=AssertionError('must reuse')):
                s.prepare(Path(temp)/'source', assets, lambda _: None)

    def test_character_only_update_reuses_core_installation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); source = root/'source'; source.mkdir()
            base = root/'data'; installed = base/'installations/old'
            def core_install(*args, **kwargs):
                kwargs['finalize'](installed)
                return installed
            with patch('tools.asset_pipeline.setup_state.source_directory', return_value=source), \
                 patch('tools.asset_pipeline.install._install', side_effect=core_install) as install, \
                 patch.object(s, 'prepare') as prepare:
                s.install(source, base, Path('unused.exe'), lambda _: None, refresh=True)
            self.assertTrue(install.call_args.kwargs['refresh'])
            self.assertEqual(install.call_args.kwargs['game_root'], source)
            prepare.assert_called_once()
            self.assertEqual(prepare.call_args.args[:2], (source, installed/'assets'))


if __name__ == '__main__': unittest.main()
