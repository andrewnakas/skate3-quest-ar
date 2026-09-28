"""Optional-content failure isolation and publication regression checks."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from tools.asset_pipeline import customiser_setup as character, optional_content
from tools.asset_pipeline import test_versions
from tools.asset_pipeline import versions
from tools.asset_pipeline.install import install, digest


class OptionalContent(unittest.TestCase):
    def test_missing_unchanged_source_archives_do_not_block_refresh(self):
        with tempfile.TemporaryDirectory() as tmp:
            base,old,source,current,saved=test_versions.AssetVersions().fixture(Path(tmp))
            for name in ('data/content/createacharacter.big','data/content/marquee.big','data/big/miscload.big'):
                (source/name).unlink()
            with patch.object(versions,'fingerprints',return_value=current),patch('tools.asset_pipeline.install.run'):
                result=install(source,base,Path('unused.exe'),lambda _:None,refresh=True)
            self.assertTrue((result/'maps/University.skate').is_file())

    def test_failed_customiser_retains_valid_generation_and_profiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            assets=Path(tmp)/'assets';base=assets/'private/customisation';base.mkdir(parents=True)
            old={'set':'a'*32,'fingerprint':'old'}
            (base/'current.json').write_text(json.dumps(old));(base/'profile.json').write_text('user choices')
            with patch.object(character,'_prepare',side_effect=KeyError('missing CAC item')),patch('tools.asset_pipeline.customiser_cache.complete',return_value=True):
                character.prepare(Path(tmp),assets,lambda _:None)
            self.assertEqual(json.loads((base/'current.json').read_text()),old)
            self.assertEqual((base/'profile.json').read_text(),'user choices')
            self.assertEqual(json.loads((base/'customiser-availability.json').read_text())['status'],'retained')

    def test_missing_customiser_records_unavailable_but_disk_error_propagates(self):
        with tempfile.TemporaryDirectory() as tmp:
            assets=Path(tmp)/'assets'
            with patch.object(character,'_prepare',side_effect=FileNotFoundError('missing CAC')):
                character.prepare(Path(tmp),assets,lambda _:None)
            self.assertEqual(json.loads((assets/'private/customisation/customiser-availability.json').read_text())['status'],'unavailable')
            with patch.object(character,'_prepare',side_effect=OSError('disk full')):
                with self.assertRaises(OSError):character.prepare(Path(tmp),assets,lambda _:None)

    def test_summary_ignores_abandoned_generations_and_includes_item_errors(self):
        with tempfile.TemporaryDirectory() as tmp:
            stage=Path(tmp);base=stage/'assets/private/customisation';base.mkdir(parents=True)
            (base/'current.json').write_text(json.dumps({'set':'a'*32}))
            for key in ('a','b'):
                directory=base/'sets'/(key*32);directory.mkdir(parents=True)
                optional_content.note(directory/'lighting-availability.json',key,ValueError('missing'),report=lambda _:None)
            (base/'sets'/('a'*32)/'library-v3.json').write_text(json.dumps({'errors':[{'model':'broken'}]}))
            warnings=optional_content.summary(stage)
            self.assertEqual([w['component'] for w in warnings],['a','Customiser items'])

    def test_no_dangling_material_selections(self):
        from tools.asset_pipeline.customisation_library import prune_unavailable
        models={'good':{'groups':[['ok','absent']], 'materials':['ok','absent']},
                'bad':{'groups':[['absent']], 'materials':['absent']}}
        errors=[];prune_unavailable(models,{'ok':{}},errors)
        self.assertEqual(list(models),['good']);self.assertEqual(models['good']['materials'],['ok'])
        self.assertEqual(errors[0]['model'],'bad')

    def test_failed_map_uses_validated_previous_map_and_keeps_other_new_map(self):
        self.map_refresh(recover=True)

    def test_no_playable_map_preserves_installation_record(self):
        self.map_refresh(recover=False)

    def map_refresh(self,recover):
        with tempfile.TemporaryDirectory() as tmp:
            base,old,source,current,saved=test_versions.AssetVersions().fixture(Path(tmp))
            previous={'name':'University','path':'maps/University.skate','sha256':digest(old/'maps/University.skate')}
            (old/'maps.json').write_text(json.dumps([previous]));saved['pipelines']=dict(current,maps='old')
            from tools.asset_pipeline.group_receipts import record
            saved['outputs']['maps']=record(old,'maps')
            (base/'installation.json').write_text(json.dumps(saved))
            if recover:(source/'data/content/worldDIST_Test.big').write_bytes(b'other map')
            def run(args,*rest):
                args=list(map(str,args))
                if any('map_job.py' in a for a in args):
                    archive=Path(args[args.index('--archive')+1])
                    if archive.stem.endswith('University'):raise RuntimeError('bad district')
                    stage=Path(args[args.index('--stage')+1]);target=stage/'maps/Test.skate';target.write_bytes(b'valid converted map')
                    Path(args[args.index('--result')+1]).write_text(json.dumps(dict(name='Test',path='maps/Test.skate',sha256=digest(target))))
                elif '--map' in args and not recover:raise RuntimeError('old map incompatible')
            with patch.object(versions,'fingerprints',return_value=current),patch('tools.asset_pipeline.install.run',side_effect=run),patch('tools.asset_pipeline.dynamic_props.prepare_catalog',side_effect=FileNotFoundError('no props')):
                if recover:
                    result=install(source,base,Path('unused.exe'),lambda _:None,refresh=True)
                    self.assertEqual({m['name'] for m in json.loads((result/'maps.json').read_text())},{'Test','University'})
                    self.assertEqual((result/'maps/University.skate').read_bytes(),b'unchanged map')
                else:
                    with self.assertRaisesRegex(RuntimeError,'No playable map'):install(source,base,Path('unused.exe'),lambda _:None,refresh=True)
                    self.assertEqual(json.loads((base/'installation.json').read_text()),saved)
