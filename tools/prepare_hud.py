"""Incrementally prepare original HUD assets from the user's extracted game.

No executable/game launch. Output contains copyrighted assets and stays private.
The vendored preview.18 UI extractor supplies the original decoding pipeline.
"""
from pathlib import Path
import argparse
import hashlib
import json
import sys
import struct

from vendor.skate3_ui.project import extract_project
from vendor.skate3_ui.scene_graph import AssetCache, SceneFlattener
from vendor.skate3_ui.actions import Actions
from asset_pipeline.vlt import hash64


def font_mapping(collections: Path, cache: AssetCache) -> dict:
    """FontManager82808AE8 table; APT-name lookup82809208, load82809308.

    Resolve the authored FileName, never infer family aliases from metrics.
    """
    data = json.loads(collections.read_text(encoding='utf-8'))
    result = {}
    for row in data['collections']:
        class_id = int(row['class'][5:], 16) if row['class'].startswith('Hash_') else hash64(row['class'])
        if class_id != 0xFECFBCAF356518C4:
            continue
        def field(name):
            return row['fields'].get(name, row['fields'].get(f'Hash_{hash64(name):016X}'))
        apt_name = field('AptName')['data']
        filename = field('FileName')['data']
        matches = [r for r in cache.manifest['bundles']
                   if 'font' in r and Path(r['name']).name == filename]
        if not matches:
            continue  # The debug font is not a packaged front-end bitmap bank.
        if len(matches) != 1 or apt_name in result:
            raise ValueError(f'Ambiguous native font record: {apt_name}')
        scale = {name: struct.unpack('>f',bytes.fromhex(field(name)['data']))[0]
                 for name in ('ScaleX','ScaleY','OffsetX','OffsetY')}
        target = matches[0]['font']['family']
        provenance = {'class': row['class'], 'key': row['key'],
                      'source_sha256': row['sha256'], 'file_name':filename,
                      'native_layout':scale}
        cache.add_font_alias(apt_name,target,provenance)
        result[apt_name] = provenance
    return result


def prepare(game: Path, output: Path, collections: Path):
    extract_project(game, output, prefixes=("data/fe/source/screens/hud2/trickdisplay2",), update=True)
    cache = AssetCache(output)
    mappings = font_mapping(collections,cache)
    name = "data/fe/source/screens/hud2/trickdisplay2"
    bundle = cache.load_bundle(name)
    apt_path = output / "raw" / (name + ".apt")
    const_path = apt_path.with_suffix('.const')
    actions = Actions(apt_path.read_bytes(), const_path.read_bytes())
    blocks = {}
    for c in bundle['characters'].values():
        for f in c.get('frames', []):
            for control in f['controls']:
                if control['type_name'] in ('do_action', 'do_init_action'):
                    offset = control.get('actions_offset', 0)
                    if offset:
                        blocks[str(offset)] = actions.stream(offset)
    shapes = {}
    fonts = {}
    for c in bundle['characters'].values():
        if c['type_name'] == 'shape':
            scene = SceneFlattener(cache, lambda *_: {}).flatten(name, c['id'])
            if scene['unresolved']:
                raise ValueError(f"Unresolved original HUD shape: {scene['unresolved']}")
            shapes[str(c['id'])] = scene['primitives']
        elif c['type_name'] == 'font':
            family = c['font']['name']
            asset = cache.font_asset(family)
            fonts[family] = asset
    result = {
        'format': 'skate3-scoring-hud', 'version': 1,
        'source': {'bundle': name,
                   'apt_sha256': hashlib.sha256(apt_path.read_bytes()).hexdigest(),
                   'const_sha256': hashlib.sha256(const_path.read_bytes()).hexdigest(),
                   'collections_sha256': hashlib.sha256(collections.read_bytes()).hexdigest()},
        'characters': list(bundle['characters'].values()),
        'shapes': shapes, 'fonts': fonts, 'actions': blocks,
        'font_mappings':mappings,
        'language': {row['label'].strip(): row['value'] for row in
                     json.loads((output / 'metadata/languages/english_global.json').read_text(encoding='utf-8'))['entries']},
        'unresolved_fonts': [family for family, asset in fonts.items() if asset is None],
    }
    target = output / 'runtime' / 'trickdisplay.json'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(result, separators=(',', ':')) + '\n', encoding='utf-8')
    print(f"Prepared {len(shapes)} original HUD shapes and {len(blocks)} action blocks: {target}")
    if result['unresolved_fonts']:
        print('Unresolved authored font families: ' + ', '.join(result['unresolved_fonts']))
    return result


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--collections', type=Path, required=True,
                   help='Owned private/stock/skater-collections.json')
    args = p.parse_args()
    prepare(args.game.resolve(), args.output.resolve(), args.collections.resolve())
