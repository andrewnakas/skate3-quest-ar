"""Synthetic environment references, independent of owned game assets."""
import struct
import unittest
from .environment import Collections, field, key_hash, sky_parameters, world_environment, fog_parameters
from .render_parameters import exposure_parameters


def raw(*values):
    return struct.pack('>'+'f'*len(values), *values).hex()


def row(cls, key, parent='', **fields):
    return {'class': cls, 'key': key, 'parent': parent,
            'fields': {k: {'type': 'fixture', 'data': v} for k, v in fields.items()}}


class EnvironmentTests(unittest.TestCase):
    def test_exposure_follows_location_reference_and_inheritance(self):
        rows = [row('render_locations', 'park', Hash_C345507C4B9B6F62=struct.pack('>2Q', key_hash('park_exposure'), 0).hex()),
                row('rendering', 'base', auto_exposure_target_luminance=raw(.25), auto_exposure_min=raw(.75), auto_exposure_max=raw(2.5), auto_exposure_damping=raw(.5)),
                row('rendering', 'park_exposure', 'base', auto_exposure_max=raw(1.5))]
        result = exposure_parameters(Collections({'collections': rows}), 'park')
        self.assertEqual(result['max'], 1.5)
        self.assertEqual(result['min'], .75)
        self.assertEqual(result['rendering_chain'], ['park_exposure', 'base'])
    def test_world_reference_and_inheritance_select_default_fog(self):
        rows = [
            row('world', 'base', Hash_C7A0A84F018E87BA='world/models/sky',
                Hash_2A0BF629355AFB15='world/models/sky_Textures',
                Hash_7E23D10785C43717=struct.pack('>2Q', key_hash('park'), 0).hex()),
            row('world', 'test', 'base'),
            row('render_locations', 'default', sun_position=raw(1, 0, 0),
                Hash_2E9AAECD1C29F81F=raw(150),
                material_fog=struct.pack('>3Q', key_hash('material_fog'), key_hash('default'), 0).hex()),
            row('render_locations', 'park', 'default', sun_position=raw(0, 3, 4)),
            row('material_fog', 'default', fog_near=raw(20), fog_far=raw(1000),
                fog_colour=raw(.1, .2, .3, 1), fog_power=raw(2), fog_max=raw(.5)),
            row('material_fog', 'fog_default', 'default', fog_far=raw(200)),
        ]
        env = world_environment(Collections({'collections': rows}), 'test')
        self.assertEqual(env['sun_direction'], [0, .6, .8])
        self.assertEqual(env['anchor_height'], 150)
        self.assertEqual(env['location_chain'], ['park', 'default'])
        self.assertEqual(env['fog']['fog_far'], [1000])
        self.assertEqual(env['sky_model'], 'world/models/sky')

    def test_native_fog_rows_and_distance_boundaries(self):
        fog = dict(fog_near=[100.], fog_far=[2000.], fog_power=[2.],
                   fog_max=[.5], fog_colour=[.1, .2, .3, 1.])
        frame = fog_parameters(fog)
        self.assertEqual(frame['colour'], [.05, .1, .15, -.5])
        def opacity(distance):
            x = max(0., min(1., distance * frame['ramp'][0] + frame['ramp'][1]))
            return .5 * x ** frame['ramp'][2]
        self.assertEqual(opacity(0.), 0.)
        self.assertAlmostEqual(opacity(100.), 0.)
        self.assertAlmostEqual(opacity(1050.), .125)
        self.assertAlmostEqual(opacity(2000.), .5)
        self.assertAlmostEqual(opacity(3000.), .5)
        fog['fog_far'] = [100.]
        with self.assertRaises(ValueError): fog_parameters(fog)

    def test_unnamed_hashes_and_cycle_rejection(self):
        cls = f'Hash_{key_hash("world"):016X}'
        c = Collections({'collections': [row(cls, 'a', 'b', value='child'),
                                         row(cls, 'b', value='parent')]})
        values, _ = c.resolve('world', 'a')
        self.assertEqual(field(values, 'value'), 'child')
        c.rows[key_hash('world'), key_hash('b')]['parent'] = 'a'
        with self.assertRaises(ValueError):
            c.resolve('world', 'a')

    def test_sky_requires_complete_array_and_shader_collection(self):
        r = row('material_sky', 'default')
        r['fields']['m_params'] = {'type': 'Attrib::Types::Vector4', 'data': 'header'}
        c = Collections({'collections': [r]})
        with self.assertRaises(ValueError):
            sky_parameters(c, 'sky.default')
        r['fields']['m_params']['array'] = {'items': [raw(.5, .25, 0, 0)]}
        self.assertEqual(sky_parameters(c, 'sky.default')['multiplier'], .25)
        with self.assertRaises(KeyError):
            sky_parameters(c, 'sky.missing')


if __name__ == '__main__':
    unittest.main()
