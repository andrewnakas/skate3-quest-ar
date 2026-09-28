"""Versioned character extras; independent of existing disc/map extractors."""
import hashlib
import json
from pathlib import Path
import tempfile
import uuid
import re


def fingerprint(tools=None):
    tools = tools or Path(__file__).resolve().parents[1]
    paths = {p for pattern in ('asset_pipeline/customis*.py', 'asset_pipeline/setup_state.py', 'asset_pipeline/native_roster.py',
                              'asset_pipeline/character_glb.py', 'asset_pipeline/retail_character.py',
                              'asset_pipeline/vlt.py', 'asset_pipeline/environment.py', 'asset_pipeline/marquee_assets.py',
                              'asset_pipeline/names.txt', 'asset_pipeline/optional_content.py', 'extract_default_skater.py',
                              'owned_game/**/*.py', 'vendor/utt/**/*.py', 'vendor/utt/**/*.json',
                              'asset_pipeline/fast_refpack.py', 'asset_pipeline/refpack_native.rs',
                              'vendor/skate3_anim/abin_importer.py', 'vendor/skate3_anim/rx2_skeleton.py',
                              'mixamo_to_skate/*.py', 'requirements-setup.txt',
                              'default_skater_retail_manifest.json') for p in tools.glob(pattern)
             if not p.name.startswith('test_')}
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(path.relative_to(tools).as_posix().encode()+b'\0')
        digest.update(hashlib.sha256(path.read_bytes().replace(b'\r\n', b'\n')).digest())
    return digest.hexdigest()


def source_fingerprint(game):
    digest = hashlib.sha256()
    for name in ('data/content/createacharacter.big', 'data/content/marquee.big', 'data/big/db.big',
                 'data/big/miscload.big', 'default.xex'):
        if name == 'data/content/marquee.big' and not (game/name).is_file():
            digest.update(b'missing optional archive')
            continue
        with (game/name).open('rb') as source:
            digest.update(hashlib.file_digest(source, 'sha256').digest())
    return digest.hexdigest()


def prepare(game, assets, report=print):
    from .optional_content import CONTENT_ERRORS, note
    from .setup_state import atomic_json
    base = assets/'private/customisation'
    availability = base/'customiser-availability.json'
    try:
        _prepare(game, assets, report)
        availability.unlink(missing_ok=True)
    except CONTENT_ERRORS as error:
        # Each generation is published atomically. Do not replace working
        # profiles/generation pointers when an optional preparation stage fails.
        from .customiser_cache import read, complete
        saved = read(base/'current.json')
        retained = (isinstance(saved.get('set'), str) and re.fullmatch('[0-9a-f]{32}', saved['set'])
                    and complete(base/'sets'/saved['set']))
        result = note(availability, 'Character customiser', error, bool(retained), report)
        result['fingerprint'] = fingerprint()
        if retained:result['set'] = saved['set']
        atomic_json(availability, result)
        if not retained:
            # Remove only the pointer, not the user's profiles or old files.
            (base/'current.json').unlink(missing_ok=True)


def _prepare(game, assets, report=print):
    from tools.asset_pipeline.customisation_catalog import prepare as catalog
    from tools.asset_pipeline.customisation_library import prepare as library
    from tools.asset_pipeline.customisation_profiles import generate
    from tools.asset_pipeline.customiser_lighting import prepare as lighting
    from tools.asset_pipeline.native_roster import prepare as native_roster
    from . import customiser_cache as cache
    base = assets/'private/customisation'
    source = source_fingerprint(game)
    old_directory = None
    marker = base/'current.json'
    version = fingerprint()
    if marker.is_file():
        try:
            saved = json.loads(marker.read_text())
        except (OSError, ValueError):
            saved = {}
        if (isinstance(saved, dict) and isinstance(saved.get('set'), str)
                and re.fullmatch('[0-9a-f]{32}', saved['set']) and saved.get('fingerprint') == version and saved.get('source') == source
                and all((base/'sets'/saved['set']/name).is_file() for name in
                        ('library-v3.json', 'extra-menu.json', 'native-lighting.json', 'native-roster/complete.json'))
                and cache.complete(base/'sets'/saved['set'])):
            return
    saved = cache.read(marker)
    if isinstance(saved.get('set'), str) and re.fullmatch('[0-9a-f]{32}', saved['set']):
        old_directory = base/'sets'/saved['set']
    pending = base/'pending.json'
    try:
        previous = json.loads(pending.read_text())
    except (OSError, ValueError):
        previous = {}
    reuse = (isinstance(previous, dict) and isinstance(previous.get('set'), str)
             and previous.get('fingerprint') == version and previous.get('source') == source
             and previous.get('set') != saved.get('set')
             and re.fullmatch('[0-9a-f]{32}', previous['set']))
    identity = previous['set'] if reuse else uuid.uuid4().hex
    directory = base/'sets'/identity
    directory.mkdir(parents=True, exist_ok=True)
    cache.atomic_json(pending, dict(set=identity, fingerprint=version, source=source))
    # The generation is invisible until every model/material has been prepared.
    # Old generations and user profiles remain usable after any failure.
    report('Preparing character customiser clothing, bodies and textures')
    stage_versions = cache.versions()
    def run_stage(name, action):
        def optional_action():
            from .optional_content import CONTENT_ERRORS, note
            try:action()
            except CONTENT_ERRORS as error:
                if name not in ('lighting','roster'):raise
                # Failed optional output is never published as complete artwork.
                import shutil
                for relative in cache.OUTPUTS[name]:
                    path=directory/relative
                    if not path.resolve().is_relative_to(directory.resolve()):
                        raise ValueError('Optional output escaped character generation')
                    if path.is_dir():shutil.rmtree(path)
                    else:path.unlink(missing_ok=True)
                if name=='lighting':
                    (directory/'native-lighting.json').write_text('{}')
                    shutil.copy2(directory/'library-base.json',directory/'library-v3.json')
                    availability=directory/'lighting-availability.json'
                else:
                    (directory/'native-roster').mkdir(exist_ok=True)
                    (directory/'native-roster/complete.json').write_text('{"characters":0}')
                    availability=directory/'native-roster/roster-availability.json'
                note(availability,'Character '+name,error,report=report)
        cache.stage(directory, old_directory, name, stage_versions[name], source, optional_action, assets, report)
    run_stage('catalog', lambda: catalog(game, directory))
    def build_library():
        data = library(dict(assets=str(assets), game_root=str(game), directory=str(directory), library_index='library-base.json'))
        if not data['models']:
            raise RuntimeError('Character library preparation failed: '+str(data['errors'])[:1000])
        for error in data['errors']:report('Unavailable customiser item: '+str(error))
        for profile in data['defaults'].values():
            for selection in profile['selections'].values():
                if selection['asset_id'] not in data['models'] or selection['material_id'] not in data['materials']:
                    raise RuntimeError('Incomplete default character outfit')
    run_stage('library', build_library)
    run_stage('menu', lambda: (directory/'extra-menu.json').write_text(json.dumps(generate(json.loads((directory/'native.json').read_text())))))
    report('Preparing authored clothing lighting for custom and pro skaters')
    run_stage('lighting', lambda: lighting(game, assets, directory, json.loads((directory/'library-base.json').read_text())))
    report('Preparing all owned pro and special character models')
    def build_roster():
        # Work textures were not published or receipted; discard failed work.
        import shutil
        if (directory/'roster-work').exists():shutil.rmtree(directory/'roster-work')
        (directory/'native-roster').mkdir(exist_ok=True)
        roster = native_roster(game, assets, directory/'native-roster',
                              directory/'database/collections.json', directory/'roster-work')
        unavailable = [item for item in roster if item['status'] != 'ready']
        for item in unavailable:report(f'Unavailable optional character {item["name"]}: {item["error"]}')
        (directory/'native-roster/complete.json').write_text(json.dumps({'characters': sum(item['status']=='ready' for item in roster), 'unavailable': unavailable}))
    run_stage('roster', build_roster)
    temporary = marker.with_suffix('.tmp')
    temporary.write_text(json.dumps(dict(version=1, set=identity, fingerprint=version, source=source)))
    temporary.replace(marker)
    pending.unlink(missing_ok=True)


def install(iso, base, game_exe, report, refresh=False):
    from tools.asset_pipeline import install as core
    # Extract an ISO once and share that source across core and character jobs.
    # Existing groups/fingerprints stay unchanged; a character-only update never
    # reconverts maps or overwrites settings and imported character libraries.
    from .setup_state import source_directory, setup_lock
    base.parent.mkdir(parents=True, exist_ok=True)
    selected = iso.resolve()
    with setup_lock(base), tempfile.TemporaryDirectory(prefix='character-source-', dir=base.parent) as temp:
        if selected.suffix.lower() == '.iso':
            if not selected.is_file():raise RuntimeError('Select an existing Skate 3 Xbox 360 ISO')
            source = Path(temp)/'disc'
            extractor = core.dependency(base/'tools', 'extract-xiso', core.XISO_URL, core.XISO_SHA, report)
            with (Path(temp)/'extract.log').open('w') as log:
                core.run([extractor, '-x', selected, '-d', source], log, report)
        else:
            source = source_directory(selected, require_core=not refresh)
        stage = core._install(iso, base, game_exe, report, game_root=source, refresh=refresh,
                             finalize=lambda stage: prepare(source, stage/'assets', report))
        return stage


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--fingerprint', action='store_true')
    parser.add_argument('--game', type=Path)
    parser.add_argument('--assets', type=Path)
    args = parser.parse_args()
    if args.fingerprint:
        print(fingerprint())
    else:
        from tools.asset_pipeline.setup_state import source_directory
        prepare(source_directory(args.game), args.assets)
