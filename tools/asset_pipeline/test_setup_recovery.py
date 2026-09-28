"""Publication, interruption and cache-integrity regressions using tiny files."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from . import test_versions
from .install import install
from . import versions, customiser_cache as cache
from .setup_state import source_directory, setup_lock, receipt, valid_receipt


class SetupRecovery(unittest.TestCase):
    @unittest.skipUnless(os.name == 'nt', 'Windows directory aliases')
    def test_receipts_accept_windows_aliases_but_reject_outside_files(self):
        import _winapi
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            target = root/'long installation directory';target.mkdir()
            alias = root/'alias'
            _winapi.CreateJunction(str(target),str(alias))
            try:
                payload = target/'map.skate';payload.write_bytes(b'map')
                expected = receipt(target,[payload])
                self.assertEqual(receipt(alias,[payload]),expected)
                self.assertEqual(receipt(target,[alias/'map.skate']),expected)
                self.assertTrue(valid_receipt(alias,expected))
                outside = root/'outside';outside.write_bytes(b'outside')
                with self.assertRaises(ValueError):receipt(alias,[outside])
            finally:
                alias.rmdir()  # Remove the junction, never its target.

    def test_character_refresh_extracts_over_old_cache_without_touching_live_copy(self):
        from . import install as engine
        from tools.owned_game.big import BigArchive, BigEntry
        for fail in (False, True):
            with self.subTest(fail=fail), tempfile.TemporaryDirectory() as temp:
                base, old, source, current, marker = test_versions.AssetVersions().fixture(Path(temp))
                marker['pipelines'] = dict(current, character='old')
                (base/'installation.json').write_text(json.dumps(marker))
                relative = 'data/content/createacharacter/texture/example.rx2'
                cached = old/'assets/private/stock'/relative
                cached.parent.mkdir(parents=True);cached.write_bytes(b'old texture')
                archive = object.__new__(BigArchive)
                archive.entries = [BigEntry(0,relative,0,3,3,0)]
                archive.read = lambda _: b'new texture'
                def character(game, stage, work, report, log, converted):
                    # Use the real extractor's no-overwrite behavior, not a mock
                    # that could hide the failure seen in release builds.
                    engine.extract(game/'data/content/createacharacter.big',stage/'assets/private/stock')
                    self.assertEqual((stage/'assets/private/stock'/relative).read_bytes(),b'new texture')
                    self.assertTrue((stage/'assets/private/stock/skater-collections.json').is_file())
                    if fail:raise RuntimeError('later conversion failed')
                with patch.object(versions,'fingerprints',return_value=current), \
                     patch.object(engine,'BigArchive',return_value=archive),patch.object(engine,'run'), \
                     patch('tools.asset_pipeline.asset_exports.character',side_effect=character):
                    if fail:
                        with self.assertRaisesRegex(RuntimeError,'later conversion failed'):
                            install(source,base,Path('unused.exe'),lambda _:None,refresh=True)
                        self.assertEqual(json.loads((base/'installation.json').read_text()),marker)
                    else:
                        installed=install(source,base,Path('unused.exe'),lambda _:None,refresh=True)
                        self.assertEqual((installed/'assets/private/stock'/relative).read_bytes(),b'new texture')
                self.assertEqual(cached.read_bytes(),b'old texture')

    def test_core_refresh_starts_empty_and_failed_rebuild_preserves_live_inputs(self):
        with tempfile.TemporaryDirectory() as temp:
            base,old,source,current,marker=test_versions.AssetVersions().fixture(Path(temp))
            marker['pipelines']=dict(current,core='old')
            (base/'installation.json').write_text(json.dumps(marker))
            def core(game,stage,*args):
                self.assertFalse((stage/'assets/private/stock').exists())
                self.assertTrue((old/'assets/private/stock/skater-collections.json').is_file())
                raise RuntimeError('core extraction failed')
            with patch.object(versions,'fingerprints',return_value=current), \
                 patch('tools.asset_pipeline.asset_exports.core',side_effect=core):
                with self.assertRaisesRegex(RuntimeError,'core extraction failed'):
                    install(source,base,Path('unused.exe'),lambda _:None,refresh=True)
            self.assertEqual(json.loads((base/'installation.json').read_text()),marker)

    def test_xex_refresh_failure_never_publishes_core_or_edits_user_data(self):
        with tempfile.TemporaryDirectory() as temp:
            base, old, source, current, marker = test_versions.AssetVersions().fixture(Path(temp))
            (old/'mods').mkdir(); (old/'mods/user.lua').write_text('user mod')
            (old/'maps/user.skate').write_text('user map')
            (old/'assets/imported-profile.json').write_text('user profile')
            def fail(stage):
                self.assertEqual(json.loads((base/'installation.json').read_text()), marker)
                self.assertTrue(os.path.samefile(old/'maps/University.skate', stage/'maps/University.skate'))
                self.assertFalse(os.path.samefile(old/'maps/user.skate', stage/'maps/user.skate'))
                self.assertEqual((stage/'mods/user.lua').read_text(), 'user mod')
                (stage/'assets/imported-profile.json').write_text('staged change')
                raise RuntimeError('customiser interrupted')
            with patch.object(versions,'fingerprints',return_value=current), patch('tools.asset_pipeline.install.run'):
                with self.assertRaisesRegex(RuntimeError,'customiser interrupted'):
                    install(source/'default.xex',base,Path('unused.exe'),lambda _:None,refresh=True,finalize=fail)
            self.assertEqual(json.loads((base/'installation.json').read_text()),marker)
            self.assertEqual((old/'assets/imported-profile.json').read_text(),'user profile')

    def test_current_core_still_validates_selected_source_and_runs_finalizer(self):
        with tempfile.TemporaryDirectory() as temp:
            base, old, source, current, marker = test_versions.AssetVersions().fixture(Path(temp))
            marker['pipelines']=current
            (base/'installation.json').write_text(json.dumps(marker))
            with patch.object(versions,'fingerprints',return_value=current), patch('tools.asset_pipeline.install.run'), patch('builtins.print'):
                with self.assertRaisesRegex(RuntimeError,'character failed'):
                    install(source/'default.xex',base,Path('unused.exe'),lambda _:None,refresh=True,
                            finalize=lambda _: (_ for _ in ()).throw(RuntimeError('character failed')))
            self.assertEqual(json.loads((base/'installation.json').read_text()),marker)
            with self.assertRaisesRegex(RuntimeError,'default.xex'):
                source_directory(source/'other.xex')
            (source/'data/content/createacharacter.big').unlink()
            with self.assertRaisesRegex(RuntimeError,'createacharacter.big'):
                source_directory(source/'default.xex')

    def test_process_death_releases_lock_without_manual_file_deletion(self):
        with tempfile.TemporaryDirectory() as temp:
            base=Path(temp)
            script="from pathlib import Path; from tools.asset_pipeline.setup_state import setup_lock; import time; " \
                   "ctx=setup_lock(Path(__import__('sys').argv[1])); ctx.__enter__(); print('locked',flush=True); time.sleep(60)"
            child=subprocess.Popen([getattr(sys, '_base_executable', sys.executable),'-c',script,str(base)],stdout=subprocess.PIPE,text=True)
            try:
                self.assertEqual(child.stdout.readline().strip(),'locked')
                with self.assertRaisesRegex(RuntimeError,'already running'):
                    with setup_lock(base):pass
            finally:
                child.kill();child.wait();child.stdout.close()
            with setup_lock(base):pass

    def test_stage_reuses_only_matching_verified_outputs_and_repairs_partial_files(self):
        with tempfile.TemporaryDirectory() as temp:
            assets=Path(temp);old=assets/'old';new=assets/'new'
            old.mkdir();new.mkdir()
            def build(directory, text):
                (directory/'extra-menu.json').write_text(json.dumps({'path':directory.relative_to(assets).as_posix()+'/item', 'text':text}))
            cache.stage(old,None,'menu','v1','disc',lambda:build(old,'good'),assets,lambda _:None)
            with patch.object(cache,'shutil', wraps=cache.shutil):
                cache.stage(new,old,'menu','v1','disc',lambda:self.fail('valid cache must be reused'),assets,lambda _:None)
            self.assertEqual(json.loads((new/'extra-menu.json').read_text())['path'],'new/item')
            (new/'extra-menu.json').write_text('interrupted')
            cache.stage(new,None,'menu','v1','disc',lambda:build(new,'repaired'),assets,lambda _:None)
            self.assertEqual(json.loads((new/'extra-menu.json').read_text())['text'],'repaired')
            cache.stage(new,old,'menu','v2','disc',lambda:build(new,'new extractor'),assets,lambda _:None)
            self.assertEqual(json.loads((new/'extra-menu.json').read_text())['text'],'new extractor')
            cache.stage(new,old,'menu','v1','different disc',lambda:build(new,'new source'),assets,lambda _:None)
            self.assertEqual(json.loads((new/'extra-menu.json').read_text())['text'],'new source')

    def test_same_size_corruption_and_escaping_receipts_are_not_current(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);file=root/'asset';file.write_bytes(b'good')
            files=receipt(root,[file]);file.write_bytes(b'evil')
            self.assertFalse(valid_receipt(root,files))
            self.assertFalse(valid_receipt(root,{'../asset':files['asset']}))


if __name__=='__main__':unittest.main()
