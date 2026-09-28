"""Export authored character shader parameters and default SH coefficients."""
import json
import struct
import sys
from PIL import Image
from pathlib import Path
from .environment import Collections, key_hash


def convert(models, private, converted):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'vendor/utt'))
    import mdl_parser
    collections = Collections(converted)
    result = {'materials': {}}
    recipe = json.loads((Path(__file__).resolve().parents[1] / 'default_skater_retail_manifest.json').read_text())
    components = {c['slot']: c for c in recipe['components']}
    for slot in sorted(models.iterdir()):
        if not slot.is_dir():
            continue
        files = list(slot.glob('*.rx2'))
        if len(files) != 1:
            raise ValueError('Ambiguous character lighting model: '+slot.name)
        model = mdl_parser.parse_rx2(files[0].read_bytes())
        shaders = {p.value for p in model.materials if p.kind == 'AttribulatorMaterialName'}
        if len(shaders) != 1:
            raise ValueError('Ambiguous character shader: '+slot.name)
        shader = shaders.pop()
        family, key = shader.split('.', 1)
        fields, _ = collections.resolve('material_'+family, key)
        rows = fields[key_hash('m_params')]['array']['items']
        result['materials']['Retail_'+slot.name] = dict(shader=shader,
            params=[struct.unpack('>4f', bytes.fromhex(row)) for row in rows])
        specular = components[slot.name]['textures'].get('specular')
        if specular:
            # The native skin shader consumes R, not luminance or roughness.
            source = private / 'default_skater/textures/decoded' / (specular+'.png')
            destination = private / 'native-character' / (slot.name+'_specular.png')
            destination.parent.mkdir(parents=True, exist_ok=True)
            with Image.open(source) as image:
                image.convert('RGB').getchannel('R').save(destination)
            result['materials']['Retail_'+slot.name]['specular'] = 'private/native-character/'+destination.name
    fields, _ = collections.resolve('Hash_6A2B5946514B8490', 'freeskate')
    result['default_sh'] = [struct.unpack('>3f', bytes.fromhex(row))
        for row in fields[key_hash('Hash_F45DFCAB6EC195CD')]['array']['items']]
    (private/'character-lighting.json').write_text(json.dumps(result))


if __name__ == '__main__':
    import argparse
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--models', type=Path, required=True)
    p.add_argument('--private', type=Path, required=True)
    p.add_argument('--collections', type=Path, required=True)
    a = p.parse_args()
    convert(a.models, a.private, json.loads(a.collections.read_text()))
