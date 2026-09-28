"""Map owned native CAC profiles into named, directly applicable menu choices."""
import json,struct,re
from pathlib import Path
from tools.asset_pipeline.customisation_native import key_hash

def generate(native):
    def page(name,children):return dict(label=name,children=children)
    def choice(name,patch,**extra):return dict(label=name,patch=patch,**extra)
    targets={key_hash(m['key']):m for m in native['morphs']}
    presets=[]
    for row in sorted(native['collections'],key=lambda r:r['key']):
        if row['class']!='cac_presets' or not re.fullmatch(r'(male|female)_\d+',row['key']):continue
        values={}
        for field in row['fields'].values():
            if field['type']!='Sk8::CAC::MorphPreset':continue
            for item in field['array']['items']:
                raw=bytes.fromhex(item);target=targets[struct.unpack_from('>Q',raw,8)[0]]
                normalized=struct.unpack_from('>f',raw,24)[0]
                if not 0<=normalized<=1:raise ValueError('Preset is outside normalized range')
                bounds=target['branch_a']
                # Inverse of CASMorph normalization8253CD18, preserving target
                # hashes rather than assuming VLT array order matches UI order.
                values[target['target']]=bounds['min']+normalized*(bounds['max']-bounds['min'])
        if len(values)!=19:raise ValueError('Incomplete native face preset')
        gender,number=row['key'].split('_')
        presets.append(choice('Face '+str(int(number)),{'morphs':values},gender=gender))
    # These indices match the runtime gesture table. Source-tree consistency
    # belongs in tests; a player's setup has no Rust checkout.
    labels=['Air guitar','Airplane','Boxing','Bruce Lee','Check the time','Devil horns','Finger guns','Dunno',
            'Finger wag','Fists','Flex','Flip the table','The Fonz','Freedom','Raised fist','Get away','Get outta here',
            'Handcuffs','High fist pump','Low fist pump','Wind up','Peace','Point','Raise the roof','Shaka','Shrug',
            'Point to the sky','Snap','Soul arch','Spock','Surf\'s up','Swing high','Swing low','Throw your arms',
            'Thumbs down','Wings','Yard sale']
    gestures=page('Gestures',[page(direction,[choice(label,{'gestures':{str(i):j}}) for j,label in enumerate(labels)])
                              for i,direction in enumerate(['Up','Down','Left','Right'])])
    skin_labels={'skin_light_3':'Porcelain','skin_light_2':'Fair','skin_light_4':'Warm',
        'skin_light_6':'Tan','skin_dark_1':'Brown','skin_dark_3':'Deep brown','skin_dark_2':'Dark'}
    skin=[];hair=[]
    for row in native['collections']:
        if row['class']!='cac_colour':continue
        fields=row['fields'];key=row['key']
        if key in skin_labels:
            rgb=struct.unpack('>4f',bytes.fromhex(fields['Hash_7827ED970A88B70C']['data']))[:3]
            skin.append(choice(skin_labels[key],{'skin':fields['Hash_AB0E65FA2F485ED8']['data'],'skin_tint':rgb}))
        elif key.endswith('_hair') and not key.startswith('default') and not key.endswith('_facial_hair'):
            rgb=struct.unpack('>4f',bytes.fromhex(fields['Hash_97462E6ABC78A423']['data']))[:3]
            hair.append(choice(key[:-5].replace('_',' ').title(),{'hair_tint':rgb}))
    skin.sort(key=lambda e: -sum(e['patch']['skin_tint']))
    hair.sort(key=lambda e:e['label'])
    return [page('Skin tone',skin),page('Hair colour',hair),page('Face presets',presets),page('Style',[gestures,
        page('Stance',[choice('Regular',{'stance':0}),choice('Goofy',{'stance':1})]),
        page('Skating style',[choice(label,{'style':i}) for i,label in enumerate(['Standard','Loose','Gonzo','Aggressive'])])])]

if __name__=='__main__':
    import sys
    root=Path(sys.argv[1]);(root/'extra-menu.json').write_text(json.dumps(generate(json.loads((root/'native.json').read_text())),indent=2))
