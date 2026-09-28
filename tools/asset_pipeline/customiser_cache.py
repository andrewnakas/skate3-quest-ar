"""Versioned, checksummed character stages, scoped to one portable install."""
import hashlib
import json
from pathlib import Path
import shutil
import time
from .setup_state import atomic_json, receipt, valid_receipt

SHARED = ('owned_game/**/*.py', 'vendor/utt/**/*.py', 'vendor/utt/**/*.json',
          'asset_pipeline/fast_refpack.py', 'asset_pipeline/refpack_native.rs',
          'requirements-setup.txt', 'asset_pipeline/optional_content.py')
GEOMETRY = ('asset_pipeline/character_glb.py', 'asset_pipeline/retail_character.py',
            'vendor/skate3_anim/abin_importer.py', 'vendor/skate3_anim/rx2_skeleton.py',
            'extract_default_skater.py', 'default_skater_retail_manifest.json')
SOURCES = {
    'catalog': ('asset_pipeline/customisation_catalog.py', 'asset_pipeline/customisation_native.py',
                'asset_pipeline/vlt.py', 'asset_pipeline/names.txt'),
    'library': GEOMETRY + ('asset_pipeline/customisation_library.py', 'asset_pipeline/customisation_worker.py'),
    'menu': ('asset_pipeline/customisation_profiles.py',),
    'lighting': ('asset_pipeline/customiser_lighting.py', 'asset_pipeline/environment.py',
                 'asset_pipeline/native_roster.py', 'asset_pipeline/marquee_assets.py', 'extract_default_skater.py'),
    'roster': GEOMETRY + ('asset_pipeline/native_roster.py', 'asset_pipeline/marquee_assets.py', 'mixamo_to_skate/*.py'),
}
OUTPUTS = {
    'catalog': ('catalog.json', 'native.json', 'database', 'source/data/content/recipe'),
    'library': ('library', 'decoded', 'source/data/content/createacharacter', 'library-base.json'),
    'menu': ('extra-menu.json',),
    'lighting': ('specular', 'native-lighting.json', 'library-v3.json', 'lighting-availability.json'),
    'roster': ('native-roster',),
}
REQUIRED = {
    'catalog': ('catalog.json', 'native.json', 'database/collections.json'),
    'library': ('library-base.json',),
    'menu': ('extra-menu.json',),
    'lighting': ('library-v3.json', 'native-lighting.json'),
    'roster': ('native-roster/complete.json',),
}


def versions(tools=None):
    tools = tools or Path(__file__).resolve().parents[1]
    result = {}
    for stage, patterns in SOURCES.items():
        digest = hashlib.sha256()
        for path in sorted({p for pat in SHARED+patterns for p in tools.glob(pat)
                            if p.is_file() and not p.name.startswith('test_')}):
            digest.update(path.relative_to(tools).as_posix().encode()+b'\0')
            digest.update(hashlib.sha256(path.read_bytes().replace(b'\r\n', b'\n')).digest())
        if stage != 'catalog':digest.update(result['catalog'].encode())
        if stage == 'lighting':digest.update(result['library'].encode())
        result[stage] = digest.hexdigest()
    return result


def read(path):
    try:
        data = json.loads(path.read_text())
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):return {}


def stage_valid(directory, name, saved):
    files = saved.get('files')
    return (isinstance(files, dict) and set(REQUIRED[name]).issubset(files)
            and valid_receipt(directory, files))


def stage(directory, previous, name, version, source, action, assets, report):
    start = time.perf_counter()
    marker = directory/(name+'-complete.json')
    saved = read(marker)
    if (saved.get('version') == version and saved.get('source') == source
            and stage_valid(directory, name, saved)):
        report(f'Character {name}: resumed complete stage ({time.perf_counter()-start:.2f}s)')
        return
    # Incomplete stage output must never reach an exists()-based converter.
    for relative in OUTPUTS[name]:
        path = directory/relative
        if path.is_dir():shutil.rmtree(path)
        else:path.unlink(missing_ok=True)
    cached = read(previous/(name+'-complete.json')) if previous else {}
    reused = (previous and cached.get('version') == version and cached.get('source') == source
              and stage_valid(previous, name, cached))
    if reused:
        old_prefix = previous.relative_to(assets).as_posix()+'/'
        new_prefix = directory.relative_to(assets).as_posix()+'/'
        for relative in cached['files']:
            src = previous/relative; dst = directory/relative
            dst.parent.mkdir(parents=True, exist_ok=True)
            if src.suffix == '.json':
                # Runtime references are relative to assets, including generation.
                dst.write_text(src.read_text().replace(old_prefix,new_prefix))
            else:
                try:dst.hardlink_to(src)
                except OSError:shutil.copy2(src,dst)
    else:
        action()
    if any(not (directory/path).is_file() for path in REQUIRED[name]):
        raise RuntimeError('Incomplete character stage: '+name)
    paths = []
    for relative in OUTPUTS[name]:
        path = directory/relative
        paths.extend(path.rglob('*') if path.is_dir() else [path])
    files = receipt(directory, paths)
    if not files:raise RuntimeError('Character stage produced no output: '+name)
    atomic_json(marker, dict(version=version, source=source, files=files))
    elapsed = time.perf_counter()-start
    timing = read(directory/'timings.json')
    timing[name] = {'seconds': round(elapsed,3), 'reused': bool(reused)}
    atomic_json(directory/'timings.json', timing)
    report(f'Character {name}: {"reused" if reused else "prepared"} in {elapsed:.2f}s')


def complete(directory):
    return all(stage_valid(directory, name, read(directory/(name+'-complete.json')))
               for name in SOURCES)
