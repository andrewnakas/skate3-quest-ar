"""Resolve each owned CAC/Marquee model's shader and its own specular texture."""
import json
from pathlib import Path
import struct
import sys
from PIL import Image
from .environment import Collections, key_hash
from .native_roster import roster
from .marquee_assets import Resources
from .optional_content import CONTENT_ERRORS, note
from tools.owned_game.big import BigArchive
from tools.extract_default_skater import import_rx2_parser, decode_texture


def material_data(model, collections, parser, specular=None):
    parsed = parser.parse_rx2(model)
    shaders = {p.value for p in parsed.materials if p.kind == 'AttribulatorMaterialName'}
    if len(shaders) != 1:
        raise ValueError('Ambiguous character shader')
    shader = shaders.pop()
    family, key = shader.split('.', 1)
    fields, _ = collections.resolve('material_' + family, key)
    rows = fields[key_hash('m_params')]['array']['items']
    if len(rows) != 9:
        raise ValueError('Character shader must have nine parameter rows')
    return dict(shader=shader, params=[struct.unpack('>4f', bytes.fromhex(r)) for r in rows], specular=specular)


def prepare(game, assets, directory, library):
    tools = Path(__file__).resolve().parents[1]
    parser = import_rx2_parser(tools/'vendor/utt')
    sys.path.insert(0, str(tools/'vendor/utt'))
    import mdl_parser
    converted = json.loads((directory/'database/collections.json').read_text())
    collections = Collections(converted)
    catalog = json.loads((directory/'catalog.json').read_text())
    cac = BigArchive(game/'data/content/createacharacter.big')
    marquee = None
    cac_entries = {e.path.lower(): e for e in cac.entries}
    resources = None
    masks = directory/'specular'; masks.mkdir(exist_ok=True)

    def mask(archive, entries, prefix, tid):
        if not tid:
            return None
        tid = tid.removeprefix('0x')
        out = masks/(prefix+'-'+tid+'.png')
        if not out.exists():
            raw = masks/(prefix+'-'+tid+'.rx2')
            name = f'data/content/{prefix}/texture/0x{tid}.rx2'.lower()
            raw.write_bytes(resources.read(name) if archive is marquee else archive.read(entries[name]))
            decoded = masks/(prefix+'-'+tid+'-decoded.png')
            decode_texture(parser, raw, decoded)
            with Image.open(decoded) as image:
                image.convert('RGB').getchannel('R').save(out)
            raw.unlink(); decoded.unlink()
        return out.relative_to(assets).as_posix()

    for component in catalog['components']:
        for model in component['models']:
            if model['id'] not in library['models']:
                continue
            lod = next(l for l in model['lods'] if l['index'] == 0)
            raw = cac.read(cac_entries[lod['path'].lower()])
            authored = material_data(raw, collections, mdl_parser)
            for mid in library['models'][model['id']]['materials']:
                if mid not in library['materials']:
                    continue
                textures = {t['channel']: t['id'] for t in catalog['materials'][mid]['textures']}
                data = {**authored, 'specular': mask(cac, cac_entries, 'createacharacter', textures.get('specular'))}
                material = library['materials'][mid]
                if 'lighting' in material and material['lighting'] != data:
                    raise ValueError('Shared material has conflicting authored shader parameters: '+mid)
                material['lighting'] = data

    native = {}
    try:
        marquee = BigArchive(game/'data/content/marquee.big')
        resources = Resources(marquee)
    except CONTENT_ERRORS as error:
        note(directory/'lighting-availability.json', 'Pro character lighting', error)
    for item in roster(converted['collections']):
        if resources is None:break
        recipe = item['recipe']
        try:
            root = resources.recipe(recipe)
            definitions = {m.attrib['id']: m for m in root.findall('mat')}
            materials = {}
            for comp in root.findall('comp'):
                slot = comp.attrib['n']
                lod = next(l for l in comp.find('mod').findall('lod') if l.get('idx') == '0')
                raw = resources.read(f'data/content/marquee/model/{recipe}/{slot}/{lod.attrib["arenaid"]}.rx2')
                mat = definitions[lod.find('matinst/matvar').attrib['id']]
                textures = {s.attrib['chn']: s.attrib['id'] for s in mat.findall('sp')}
                materials['Retail_'+slot] = material_data(raw, collections, mdl_parser,
                    mask(marquee, resources.entries, 'marquee', textures.get('specular')))
            native[item['key']] = materials
        except CONTENT_ERRORS as error:
            note(directory/'lighting-availability.json', 'Pro character lighting', error)
            continue
    (directory/'native-lighting.json').write_text(json.dumps(native))
    (directory/'library-v3.json').write_text(json.dumps(library, separators=(',', ':')))
    return native
