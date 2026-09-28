"""Resolve authored world/environment references without inventing frame rows."""
import math
import struct
from .vlt import hash64

# Names recovered from schema/debug strings; unnamed fields keep their hashes.
NAMES = ('world', 'render_locations', 'material_sky', 'material_fog',
         'sun_position', 'm_params', 'fog_far', 'fog_near', 'fog_colour',
         'fog_power', 'fog_max')


def key_hash(name):
    return int(name[5:], 16) if name.startswith('Hash_') else hash64(name)


class Collections:
    def __init__(self, data):
        self.rows = {(key_hash(r['class']), key_hash(r['key'])): r
                     for r in data['collections']}

    def resolve(self, cls, key):
        cls = key_hash(cls) if isinstance(cls, str) else cls
        key = key_hash(key) if isinstance(key, str) else key
        fields, chain, seen = {}, [], set()
        while key:
            if key in seen:
                raise ValueError('Cyclic environment inheritance')
            seen.add(key)
            row = self.rows[cls, key]
            chain.append(row['key'])
            for name, value in row['fields'].items():
                fields.setdefault(key_hash(name), value)
            key = key_hash(row['parent'])
        return fields, chain


def field(fields, name):
    return fields[key_hash(name)]['data']


def floats(raw, count):
    values = struct.unpack('>' + 'f' * count, bytes.fromhex(raw))
    if not all(math.isfinite(v) for v in values):
        raise ValueError('Non-finite environment value')
    return list(values)


def world_environment(collections, world_key):
    world, world_chain = collections.resolve('world', world_key)
    location_key = struct.unpack('>Q', bytes.fromhex(
        field(world, 'Hash_7E23D10785C43717'))[:8])[0]
    location, location_chain = collections.resolve('render_locations', location_key)
    sun = floats(field(location, 'sun_position'), 3)
    length = math.sqrt(sum(v*v for v in sun))
    if length < 1e-6:
        raise ValueError('Invalid authored sun direction')
    fog_cls, fog_key, _ = struct.unpack('>3Q', bytes.fromhex(field(location, 'material_fog')))
    fog, fog_chain = collections.resolve(fog_cls, fog_key)
    return {
        'world_chain': world_chain, 'location_chain': location_chain,
        'sun_direction': [v/length for v in sun],
        'anchor_height': floats(field(location, 'Hash_2E9AAECD1C29F81F'), 1)[0],
        'sky_model': field(world, 'Hash_C7A0A84F018E87BA'),
        'sky_textures': field(world, 'Hash_2A0BF629355AFB15'),
        # Keep the authored values alongside the derived native shader rows.
        'fog': {'chain': fog_chain, **{n: floats(field(fog, n), 4 if n == 'fog_colour' else 1)
                for n in ('fog_near', 'fog_far', 'fog_colour', 'fog_power', 'fog_max')}},
    }


def sky_parameters(collections, shader):
    effect, key = shader.split('.', 1)
    if effect != 'sky':
        raise ValueError('Not a sky material: ' + shader)
    fields, chain = collections.resolve('material_sky', key)
    params = fields[hash64('m_params')].get('array')
    if not params or not params['items']:
        raise ValueError('Sky parameters require complete VLT array payloads')
    row = floats(params['items'][0], 4)
    if row[0] <= 0 or row[1] < 0:
        raise ValueError('Invalid authored sky parameters')
    return {'material_chain': chain, 'sun_scale': row[0], 'multiplier': row[1]}


def fog_parameters(fog):
    """TU3 0x828012D0: schema offsets 20/28/16/24 -> ramp and colour.

    0x82F825F0 initializes the negative-one vector used for colour alpha.
    Colour RGB is already linear here; do not square it again.
    """
    near, far, power, maximum = (fog[n][0] for n in
        ('fog_near', 'fog_far', 'fog_power', 'fog_max'))
    colour = fog['fog_colour']
    if (len(colour) != 4 or not all(math.isfinite(v) for v in
            [near, far, power, maximum, *colour]) or far <= near or near < 0
            or power <= 0 or not 0 <= maximum <= 1):
        raise ValueError('Invalid authored fog range/colour')
    return {'ramp': [1 / (far - near), -near / (far - near), power, 0],
            'colour': [v * maximum for v in colour[:3]] + [-maximum]}
