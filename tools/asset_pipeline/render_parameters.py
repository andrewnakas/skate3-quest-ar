"""Export complete authored parameter rows for additional presentation shaders."""
import json
import struct
from .environment import Collections, key_hash


def exposure_parameters(collections, location):
    fields, chain = collections.resolve('render_locations', location)
    key = int(fields[key_hash('Hash_C345507C4B9B6F62')]['data'][:16], 16)
    rendering, rendering_chain = collections.resolve('rendering', key)
    values = {n: struct.unpack('>f', bytes.fromhex(rendering[key_hash('auto_exposure_'+n)]['data']))[0]
              for n in ('target_luminance', 'min', 'max', 'damping')}
    return dict(values, location_chain=chain, rendering_chain=rendering_chain)


def convert(assets, converted):
    collections = Collections(converted)
    result = {}
    aliases = ('default', 'reflection', 'backlituvscroll', 'transparent',
               'flowing', 'flowingalpha', 'alpha', 'skatepark', 'videoscreen')
    for (cls, key), row in collections.rows.items():
        for family in ('water', 'ocean', 'incandescent'):
            if cls != key_hash('material_' + family):
                continue
            fields, _ = collections.resolve(cls, key)
            raw = fields.get(key_hash('m_params'), {}).get('array', {}).get('items', [])
            if not raw:
                continue
            name = next((n for n in aliases if key_hash(n) == key), row['key'])
            result[family + '.' + name] = [struct.unpack('>4f', bytes.fromhex(r)) for r in raw]
    path = assets / 'private/render-parameters.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(result), encoding='utf-8')
    fields, _ = collections.resolve('Hash_1FFDC8E3ACA07C1F', 'auto_exposure')
    exposure = {n: struct.unpack('>f', bytes.fromhex(fields[key_hash('auto_exposure_'+n)]['data']))[0]
                for n in ('target_luminance', 'min', 'max', 'damping')}
    (assets / 'private/exposure.json').write_text(json.dumps(exposure), encoding='utf-8')
    profiles = {}
    for (cls, key), row in collections.rows.items():
        if cls != key_hash('world'):
            continue
        own_stream = next((v['data'] for k, v in row['fields'].items() if key_hash(k) == key_hash('WorldStream')), None)
        if not own_stream or not own_stream.startswith('DIST_'):
            continue
        world, _ = collections.resolve(cls, key)
        location = int(world[key_hash('Hash_7E23D10785C43717')]['data'][:16], 16)
        profiles[own_stream.removeprefix('DIST_').casefold()] = exposure_parameters(collections, location)
    (assets / 'private/exposure-profiles.json').write_text(json.dumps(profiles, indent=2), encoding='utf-8')
    return len(result)
