"""Retain foliage in authored global world models outside district streams."""
import json
import hashlib
import struct
import sys
import tempfile
from pathlib import Path

from .environment import Collections, field, key_hash
from .map_writer import write
from .sky import _texture
from tools.owned_game.big import BigArchive


def texture_groups(raw, table, channels):
    """Read binary channel GUIDs; display-name suffixes are not resource IDs."""
    groups = []
    for section in table.entries:
        if section.type_id != 0x00eb0005:
            continue
        o = section.f0
        h = struct.unpack_from('>8I', raw, o)
        if not h[1] or (h[4] - h[3]) != h[1] * 32:
            raise ValueError('Unexpected global model material layout')
        for j in range(h[1]):
            v = struct.unpack_from('>8I', raw, o+h[3]+j*32)
            at = o+v[0]
            kind = raw[at:raw.index(b'\0', at)].decode('ascii')
            if kind == 'Name':
                groups.append({})
            if kind in channels:
                groups[-1][kind] = (v[4] << 32) | v[5]
    return groups


def convert(game_root, assets, converted):
    vendor = Path(__file__).resolve().parents[1] / 'vendor'
    sys.path.insert(0, str(vendor / 'utt'))
    sys.path.insert(0, str(vendor / 'university/tools/vanilla_map_extraction/tools'))
    import numpy as np
    import mdl_parser
    import rx2_parser
    import prepare_hawaiian_dream as prep
    from retail_lightmap_uv import decode_lightmap_uvs
    from retail_texture_decode import B5G6R5_FORMAT_ID, decode_b5g6r5

    archive = BigArchive(game_root / 'data/big/miscload.big')
    entries = {e.path: e for e in archive.entries}
    collections = Collections(converted)
    output = assets / 'private/native-backdrops'
    output.mkdir(parents=True, exist_ok=True)
    available = {p.stem.removeprefix('world') for p in (game_root/'data/content').glob('worldDIST_*.big')}
    count = 0
    for (cls, _), row in collections.rows.items():
        if cls != key_hash('world'):
            continue
        own_stream = next((v['data'] for k, v in row['fields'].items()
                           if key_hash(k) == key_hash('WorldStream')), None)
        if own_stream not in available:
            continue
        fields, _ = collections.resolve('world', row['key'])
        # These global presentation resources are separate from WorldStream.
        if key_hash('Hash_951898F6C0FA6856') not in fields:
            continue
        model_name = field(fields, 'Hash_951898F6C0FA6856')
        if not model_name:
            continue
        texture_name = field(fields, 'Hash_CA5A157A65E75934')
        raw = archive.read(entries['data/content/' + model_name + '.rx2'])
        model = mdl_parser.parse_rx2(raw)
        groups, bindings = prep._bind_material_groups_by_guid(
            raw, prep._group_material_parameters(model.materials), len(model.meshes),
            allow_import_order_fallback=False)
        selected = [(i, prep._material_metadata(groups, i)) for i in range(len(model.meshes))]
        selected = [(i, m) for i, m in selected if m['shader_name'] in ('tree.default', 'animated.tree', 'ocean.reflection', 'environment.reflective_simple')]
        if not selected:
            continue
        textures = rx2_parser.parse_rx2(archive.read(entries['data/content/' + texture_name + '.rx2']))
        table = rx2_parser.RX2File(raw)
        table.parse()
        channel_groups = texture_groups(raw, table, prep.RETAIL_TEXTURE_CHANNELS)
        with tempfile.TemporaryDirectory(prefix='skate-backdrop-') as temporary:
            root = Path(temporary)
            arrays, meshes, exported_textures = {}, [], {}
            for i, material in selected:
                mesh = model.meshes[i]
                binding = bindings[i]
                meshes.append(dict(material, index=i, name=groups[i]['Name'][0],
                    retail_material_guid=f"0x{binding['material_guid']:016X}",
                    retail_material_handle=f"0x{binding['material_handle']:08X}",
                    retail_material_group_index=binding['group_index'], source_offsets=mesh.source_offsets))
                arrays[f'vertices_{i}'] = mesh.vertices
                arrays[f'faces_{i}'] = mesh.faces
                arrays[f'uvs_{i}'] = mesh.uvs
                arrays[f'normals_{i}'] = mesh.normals
                lm = decode_lightmap_uvs(raw, vertex_buffer_offset=mesh.source_offsets['vertex_buffer'],
                    vertex_count=mesh.vertex_count, vertex_stride=mesh.vertex_stride, attributes=mesh.attributes)
                if lm is not None:
                    arrays[f'lightmap_uvs_{i}'] = lm.values
                for role, name in material['retail_texture_ids'].items():
                    if name in exported_textures:
                        continue
                    texture = _texture(textures, channel_groups[binding['group_index']][role])
                    rgba = texture.rgba
                    if texture.fmt_id == B5G6R5_FORMAT_ID:
                        rgba = decode_b5g6r5(textures.data[texture.data_offset:texture.data_offset+texture.buffer_size],
                                            texture.width, texture.height)
                    path = name + '.rgba'
                    (root/path).write_bytes(rgba)
                    exported_textures[name] = dict(width=texture.width, height=texture.height, rgba=path)
            np.savez(root/'model.npz', **arrays)
            name = own_stream.removeprefix('DIST_')
            manifest = dict(map_name=name, district_name=own_stream, models=[dict(
                asset_id='0x'+hashlib.sha256(raw).hexdigest()[:16], npz='model.npz', meshes=meshes)],
                textures=exported_textures, normal_texture_policy=dict(excluded_texture_ids=[]), grind_splines=[],
                source_model=model_name, source_textures=texture_name, source_sha256=hashlib.sha256(raw).hexdigest())
            path = root/'manifest.json'
            path.write_text(json.dumps(manifest))
            write(path, output/(name+'.skate'), None, render_only=True)
            count += 1
    return count


if __name__ == '__main__':
    import argparse
    from .vlt import convert as convert_vlt
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-root', type=Path, required=True)
    parser.add_argument('--assets', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='skate-backdrop-db-') as work:
        database = BigArchive(args.game_root/'data/big/db.big')
        needed = {'skaterschema.bin', 'skaterschema.vlt', 'skatercollections.bin', 'skatercollections.vlt'}
        database.extract_entries([e for e in database.entries if Path(e.path).name.lower() in needed], Path(work))
        stem = Path(work)/'data/db'
        names = (Path(__file__).parent/'names.txt').read_text(encoding='utf-8').splitlines()
        converted = convert_vlt(stem/'skaterschema', stem/'skatercollections', names)
        print('Prepared', convert(args.game_root, args.assets, converted), 'authored foliage backdrops')
