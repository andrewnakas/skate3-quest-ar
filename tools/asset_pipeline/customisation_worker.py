"""Private, on-demand retail CAC assembly. Never starts a game or an editor.

The protocol is JSON on stdin/stdout. Paths belong to a local worker.json,
not the saved outfit. Failed assemblies never replace an existing GLB.
"""
from __future__ import annotations
import argparse, copy, hashlib, json, math, os, shutil, sys
from pathlib import Path

from tools.asset_pipeline.customisation_catalog import write_private, selected_paths

ROOT = Path(__file__).resolve().parents[2]


def flag(item, key):
    return item['flags'].get('cas.' + key, '')


def label(name):
    return name.replace('_', ' ').strip()


def default_profile():
    recipe = json.loads((ROOT/'tools/default_skater_retail_manifest.json').read_text())
    return {'selections': {c['slot']: {'asset_id': c['asset_id'], 'material_id': c['material_id']}
                           for c in recipe['components']}, 'morphs': {},
            'truck': 0.7, 'wheel': 0.7, 'posture': 0}


def menu_index(catalog, native):
    """Use authored names and IDs; no generated merchandise or synthetic presets."""
    mats = catalog['materials']
    slots = {c['slot']: c['models'] for c in catalog['components']}
    def page(name, children=None, **extra):
        return dict(label=name, children=children or [], **extra)
    def unavailable(name, reason):
        return page(name, note=reason)
    def models(name, slot, predicate=lambda m: True):
        children = []
        if slot in {'Hat', 'Hair', 'Glasses', 'Jewellery', 'WristItem', 'Sock', 'Accessory'}:
            children.append(page('None', patch={'selections': {slot: None}}))
        for m in slots[slot]:
            if flag(m, 'Gender') not in {'', 'male', 'unisex'} or not predicate(m):
                continue
            lod = next(l for l in m['lods'] if l['index'] == 0)
            if len(lod['material_instances']) != 1:
                continue
            variants = []
            for v in lod['material_instances'][0]:
                if not any(t['channel'] == 'diffuse' for t in mats[v['id']]['textures']):
                    continue
                variants.append(page(label(v['name']), patch={'selections': {slot: {
                    'asset_id': m['id'], 'material_id': v['id']}}}))
            if variants:
                children.append(page(label(m['name']), variants))
        return page(name, children)
    morphs = []
    for m in native['morphs']:
        r = m['branch_a']
        morphs.append(page(label(m['key']), scalar='morphs.'+m['target'],
                           minimum=r['min'], maximum=r['max'], initial=r['default'], step=0.025))
    skin = sorted({flag(m, 'SkinTone') for m in mats.values()} - {''})
    beard = sorted({flag(m, 'FacialHairStyle') for m in mats.values()} - {''})
    body = page('Body mods', [
        unavailable('Gender', 'Male is supported in this test. Female material groups are still being decoded.'),
        page('Skin', [page(x, patch={'skin': x}) for x in skin]), models('Hair', 'Hair'),
        page('Body shape', morphs[:2]),
        unavailable('Facial presets', 'Native preset arrays are indexed; preset application is not yet connected.'),
        page('Face modification', morphs[2:]),
        page('Facial hair', [page(x, patch={'beard': x}) for x in beard]),
        unavailable('Upper body tattoo', 'Tattoo stamp compositing is not yet implemented.'),
        unavailable('Lower body tattoo', 'Tattoo stamp compositing is not yet implemented.')])
    merch = [models('Hats', 'Hat')]
    for title, kinds in [('Tshirts', {'tshirt','ls_tshirt','tanktop'}), ('Button shirts', {'buttonshirt'}),
                         ('Hoodies', {'hoody'}), ('Jackets', {'jacket'}), ('Sweaters', {'sweater'})]:
        merch.append(models(title, 'OuterTorso', lambda m, k=kinds: flag(m,'TopType') in k))
    merch += [models('Pants','Pants'), models('Shoes','Feet'), models('Socks','Sock'),
              page('Accessories', [models(s, s) for s in ['Glasses','Jewellery','WristItem','Accessory']]),
              page('Skateboards', [models('Deck','SkateBoard'),models('Trucks','SkateTruck'),models('Wheels','SkateWheel')])]
    style = page('Edit style', [unavailable('Gestures', 'Gesture selection is not yet connected.'),
            unavailable('Stance', 'Changing the native stance publication is not yet connected.'),
            unavailable('Style', 'Native style-to-animation selection is still being decoded.'),
            page('Posture', [page(x, patch={'posture':i}) for i,x in enumerate(['Default','Stiff','Slouch','Buff'])])])
    return page('Character customiser', [body, page('Merchandise', merch), style,
        page('Trucks adjustment',scalar='truck',minimum=0.,maximum=1.,initial=.7,step=.1),
        page('Wheels adjustment',scalar='wheel',minimum=0.,maximum=1.,initial=.7,step=.1)])


def resolve(catalog, profile):
    profile = copy.deepcopy(profile)
    models = {m['id']: (c['slot'],m) for c in catalog['components'] for m in c['models']}
    materials = catalog['materials']
    selections = profile['selections']
    # Validate even entries which would subsequently be removed by a dependency.
    for slot, selection in list(selections.items()):
        if selection is None:
            del selections[slot]
            continue
        if selection['asset_id'] not in models or models[selection['asset_id']][0] != slot:
            raise ValueError('Unknown model in '+slot)
        m = models[selection['asset_id']][1]
        if flag(m,'Gender') not in {'','male','unisex'}:
            raise ValueError('Female assembly is not supported by this test build')
        groups = next(l for l in m['lods'] if l['index']==0)['material_instances']
        if len(groups)!=1 or selection['material_id'] not in {v['id'] for v in groups[0]}:
            raise ValueError('Invalid material binding for '+slot)
    def model(slot):
        return models[selections[slot]['asset_id']][1] if slot in selections else None
    def choose(slot, key, value):
        if not value or value == 'none':
            selections.pop(slot, None)
            return
        current=model(slot)
        if current and flag(current,key)==value:
            return
        candidates=[m for s,m in models.values() if s==slot and flag(m,key)==value
                    and flag(m,'Gender') in {'','male','unisex'}]
        if not candidates:
            raise ValueError(f'No compatible {slot}: {key}={value}')
        m=candidates[0]
        variants=next(l for l in m['lods'] if l['index']==0)['material_instances'][0]
        v=next((v for v in variants if flag(materials[v['id']],'IsDefault').lower()=='yes'), variants[0])
        selections[slot]={'asset_id':m['id'],'material_id':v['id']}
    outer=model('OuterTorso')
    if outer:
        choose('Arm','ArmModelType',flag(outer,'ArmModelRequired'))
        inner=flag(outer,'InnerCutTypeRequired')
        choose('InnerTorso','InnerCutType',inner)
        if flag(outer,'SupportsWristItem')=='no':selections.pop('WristItem',None)
        if flag(outer,'IsNecklaceRemoved').lower()=='yes':selections.pop('Jewellery',None)
    pants=model('Pants')
    if pants:choose('Leg','LegModelType',flag(pants,'LegModelRequired'))
    leg=model('Leg')
    if leg:
        sock=selections.get('Sock')
        sock_style=flag(materials[sock['material_id']],'SockStyle') if sock else 'none'
        candidates=[m for s,m in models.values() if s=='Leg' and flag(m,'Gender') in {'male','unisex',''}
                    and flag(m,'LegModelType')==flag(leg,'LegModelType')
                    and (flag(m,'RequiresSockStyle') or 'none')==sock_style]
        if not candidates:raise ValueError('These socks do not fit the selected pants; choose another sock length')
        if leg not in candidates:
            m=candidates[0];variants=m['lods'][0]['material_instances'][0]
            previous=selections['Leg']['material_id']
            v=next((v for v in variants if v['id']==previous),variants[0])
            selections['Leg']={'asset_id':m['id'],'material_id':v['id']}
    hat=model('Hat')
    if hat:
        required=flag(hat,'RequiresHairModelType')
        hair=model('Hair')
        if required=='none':selections.pop('Hair',None)
        elif required and hair and flag(hair,'HairModelType')!=required:
            matches=[m for s,m in models.values() if s=='Hair' and flag(m,'Gender') in {'male','unisex',''}
                     and flag(m,'HairStyle')==flag(hair,'HairStyle') and flag(m,'HairModelType')==required]
            if not matches:raise ValueError('This hat needs another hair style; choose None under Hair first')
            m=matches[0];v=m['lods'][0]['material_instances'][0][0]
            selections['Hair']={'asset_id':m['id'],'material_id':v['id']}
    for slot in list(selections):
        m=model(slot)
        if m:
            for removed in flag(m,'RequiresRemovalOfComponents').split(','):
                selections.pop(removed.strip(),None)
    # Keep exposed body material variants on the selected head's authored tone.
    head=selections['Rostral']
    tone=profile.get('skin',flag(materials[head['material_id']],'SkinTone'))
    beard=profile.get('beard',flag(materials[head['material_id']],'FacialHairStyle'))
    for slot,sel in selections.items():
        old=materials[sel['material_id']]
        if not flag(old,'SkinTone'):continue
        variants=model(slot)['lods'][0]['material_instances'][0]
        candidates=[v for v in variants if flag(materials[v['id']],'SkinTone')==tone
                    and (slot!='Rostral' or flag(materials[v['id']],'FacialHairStyle')==beard)]
        if not candidates:raise ValueError(f'No authored {slot} material for skin {tone} / facial hair {beard}')
        # Preserve unrelated eyebrow, hairline and skin-normal flags where possible.
        def distance(v):
            f=materials[v['id']]['flags']
            return sum(f.get(k)!=val for k,val in old['flags'].items() if k not in {'cas.SkinTone','cas.FacialHairStyle'})
        if not any(v['id']==sel['material_id'] for v in candidates):
            sel['material_id']=min(candidates,key=distance)['id']
    allowed={'fat','thin'}|set(json.loads((ROOT/'tools/default_skater_retail_manifest.json').read_text())['morph_assembly']['face_targets'])
    for key,value in profile.get('morphs',{}).items():
        if key not in allowed:raise ValueError('Unknown morph target: '+key)
        if not isinstance(value,(int,float)) or not math.isfinite(value) or not 0<=value<=.5:
            raise ValueError('Morph value outside the native range: '+key)
    return profile


def build(config, requested):
    from tools.owned_game.big import BigArchive
    from tools.extract_default_skater import import_rx2_parser, decode_texture, build_composite_images
    from tools.asset_pipeline.retail_character import RX2, decode_dense_morphs
    from tools.asset_pipeline.character_glb import convert
    assets=Path(config['assets']); cache=assets/'private/customisation'
    catalog=json.loads((cache/'catalog.json').read_text())
    profile=resolve(catalog,requested)
    key=hashlib.sha256(json.dumps(profile,sort_keys=True).encode()).hexdigest()[:24]
    work=cache/'builds'/key; output=work/'character.glb'
    if output.is_file():return {'profile':profile,'scene':output.relative_to(assets).as_posix()}
    work.mkdir(parents=True,exist_ok=True)
    indexed={m['id']:m for c in catalog['components'] for m in c['models']}
    selections=[dict(slot=s,asset_id=v['asset_id'],lod=0,material_ids=[v['material_id']])
                for s,v in profile['selections'].items()]
    paths=selected_paths(catalog,selections)
    archive=BigArchive(Path(config['game_root'])/'data/content/createacharacter.big')
    entries={e.path.lower():e for e in archive.entries}
    for path in paths:
        dest=cache/'source'/path
        if not dest.is_file():write_private(dest,archive.read(entries[path.lower()]))
    parser=import_rx2_parser(ROOT/'tools/vendor/utt')
    decoded=cache/'decoded'; decoded.mkdir(exist_ok=True)
    recipe=json.loads((ROOT/'tools/default_skater_retail_manifest.json').read_text())
    original={c['slot']:c for c in recipe['components']}
    recipe['components']=[];recipe['morph_assembly']['expected_targets']={}
    for slot,sel in profile['selections'].items():
        m=indexed[sel['asset_id']];lod=next(l for l in m['lods'] if l['index']==0)
        mat=catalog['materials'][sel['material_id']]
        textures={t['channel']:t['id'] for t in mat['textures']}
        if 'diffuse' not in textures:raise ValueError('No diffuse map for '+slot)
        for t in mat['textures']:
            dest=decoded/(t['id']+'.png')
            if not dest.is_file():decode_texture(parser,cache/'source'/t['path'],dest)
        target=work/'models'/slot/Path(lod['path']).name
        target.parent.mkdir(parents=True,exist_ok=True)
        if not target.exists():shutil.copyfile(cache/'source'/lod['path'],target)
        parsed=RX2.parse_rx2(str(target));meshes=[m for m in parsed['meshes'] if m.get('positions') and m.get('indices')]
        if len(meshes)!=1:raise ValueError('Unsupported mesh groups in '+slot)
        morphs=decode_dense_morphs(target,parsed,len(meshes[0]['positions']),RX2)
        recipe['morph_assembly']['expected_targets'][slot]=[m['name'] for m in morphs]
        base=original.get(slot,{})
        # Preserve the existing renderer and its baseline material constants.
        tint=base.get('tint',[1.,1.,1.]) if base.get('material_id')==sel['material_id'] else [1.,1.,1.]
        recipe['components'].append(dict(slot=slot,textures=textures,tint=tint,
            alpha_mode='MASK' if 'alpha' in textures else 'OPAQUE'))
    body=recipe['preset']['body_mods'];values=profile.get('morphs',{})
    body.update(fatness=values.get('fat',0.),skinniness=values.get('thin',0.),targets=values)
    material_root=work/'materials'
    build_composite_images(recipe,decoded,material_root)
    temporary=work/'character.pending.glb'
    convert(work/'models',assets/'private',recipe,output=temporary,materials=material_root)
    os.replace(temporary,output)
    (work/'recipe.json').write_text(json.dumps(recipe,indent=2))
    return {'profile':profile,'scene':output.relative_to(assets).as_posix()}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--config',type=Path,required=True)
    args=p.parse_args();config=json.loads(args.config.read_text())
    request=json.load(sys.stdin)
    try:
        import contextlib
        with contextlib.redirect_stdout(sys.stderr):
            result=build(config,request)
    except Exception as e:
        import traceback
        traceback.print_exc(file=sys.stderr)
        result={'error':str(e)}
    print(json.dumps(result))
    return 1 if 'error' in result else 0


if __name__=='__main__':sys.exit(main())
