"""Prepare resident CAC geometry and shared textures outside the game loop."""
from pathlib import Path
import copy,json,shutil,sys,re,struct
import numpy as np
from PIL import Image,ImageChops
from tools.asset_pipeline.customisation_worker import ROOT,default_profile,flag
from tools.asset_pipeline.customisation_catalog import write_private
from tools.owned_game.big import BigArchive
from tools.asset_pipeline.optional_content import CONTENT_ERRORS

def friendly(name):
    text=name.lower().replace('_',' ')
    replacements=[('shortsleeve','short sleeve'),('longsleeve','long sleeve'),('tshirt','tee'),
        ('t-shirt','tee'),('buttonup','button-up'),('zipup','zip-up'),('flippedup','flipped up'),
        ('kneedown',''),('thighdown',''),('medhem',''),('lowhem',''),('highhem',''),
        ('sleeves pushed up','rolled sleeves'),('sleeves up','rolled sleeves'),('sleevup','rolled sleeves'),
        ('colourizable',''),('colorizable',''),('stampable',''),('logoable',''),('material',''),
        ('lambert1','original'),('newera','new era'),('45right','angled right'),('45left','angled left'),
        ('long sleeve tee','long-sleeve tee'),('short sleeve tee','tee'),
        ('short sleeve button-up shirt','short-sleeve shirt'),('long sleeve button-up shirt','long-sleeve shirt'),
        ('shirt ls button-up','long-sleeve shirt'),('top button only','top button'),
        ('long sleeve mens shirt vneck','long-sleeve v-neck'),('jeans straight leg baggy waistline','baggy straight jeans'),
        ('jeans straight leg baggy waistline','baggy straight jeans'),('sunglasses','shades'),('necklaces',''),('tattoo',''),('biglizard','big lizard'),
        ('blueflame','blue flame'),('radialsun','sunburst'),('tribalbody','tribal'),('pinstripe','pinstripe ')]
    for a,b in replacements:text=text.replace(a,b)
    text=re.sub(r'\b(male\d*|female\d*|unisex|mesh|mat|hmat|h|l|high|low|ropa|hem|inner)\b','',text)
    text=re.sub(r'\b(?:0x)?[a-f0-9]{8,}\b','',text)
    text=re.sub(r'([a-z])([0-9]+)\b',r'\1 \2',text)
    text=re.sub(r'\s+',' ',text).strip(' -')
    text=text.replace('jeans straight leg baggy waistline','baggy straight jeans').replace('long-sleeve tee rolled sleeves','rolled-sleeve tee')
    return text.title() or 'Original'


def variant_label(slot, model_name, native_name):
    label=friendly(native_name)
    worn=bool(re.search(r'(?:_| )d(?:_| |$)|dirty',native_name.lower()))
    label=re.sub(r'\b(Diffuse|Tga|D|Malemat|Skateboard|Featured Artist|Instance)\b','',label)
    label=re.sub(r'\b0\b','',label)
    if slot=='Feet': return 'Worn' if worn else 'New'
    model_words=set(friendly(model_name).lower().split())
    words=[w for w in label.split() if w.lower() not in model_words]
    label=' '.join(words).strip()
    for a,b in [('Ozzie Wright','Ozzie'),('Grass Roots Pullover Hoodie','Grass Roots'),
                ('Full Back New Era',''),('Hoodie Hooddown Pullover',''),('Hood Up',''),
                ('Hooddown',''),('Closed',''),('Open',''),('In4Mation Colab Zip','In4Mation'),
                ('Nightmare Catcher','Nightmare'),('Gonz Animal Bullfish','Bullfish')]: label=label.replace(a,b)
    label=re.sub(r'\s+',' ',label).strip()
    if not label: label='Original'
    return label+(' (Worn)' if worn else '')


def prune_unavailable(models, materials, errors):
    """Never expose a model selection that points at an absent material."""
    for key, model in list(models.items()):
        model['groups'] = [[mid for mid in group if mid in materials] for group in model['groups']]
        if not model['groups'] or any(not group for group in model['groups']):
            del models[key]
            errors.append(dict(model=key, error='No available material for a mesh group'))
        else:
            model['materials'] = model['groups'][0]


def prepare(config):
    from tools.extract_default_skater import import_rx2_parser,decode_texture,parse_fallback_recipe
    from tools.asset_pipeline.retail_character import RX2,decode_dense_morphs,AnimSource,SkeletonSet
    from tools.asset_pipeline.character_glb import convert
    assets=Path(config['assets']);cache=Path(config.get('directory',assets/'private/customisation'));out=cache/'library';out.mkdir(exist_ok=True)
    catalog=json.loads((cache/'catalog.json').read_text());archive=BigArchive(Path(config['game_root'])/'data/content/createacharacter.big')
    entries={e.path.lower():e for e in archive.entries}
    def extract(path):
        dest=cache/'source'/path
        if not dest.exists():write_private(dest,archive.read(entries[path.lower()]))
        return dest
    parser=import_rx2_parser(ROOT/'tools/vendor/utt');decoded=cache/'decoded';decoded.mkdir(exist_ok=True)
    def texture(tid,kind='raw'):
        src=decoded/(tid+'.png')
        if not src.exists():decode_texture(parser,extract('data/content/createacharacter/texture/0x'+tid+'.rx2'),src)
        if kind=='raw':return src.relative_to(assets).as_posix()
        dest=out/'textures'/(tid+'_'+kind+'.png');dest.parent.mkdir(exist_ok=True)
        if not dest.exists():
            image=Image.open(src).convert('RGBA');a=np.array(image,dtype=np.float64)
            if kind=='normal':
                x=a[:,:,3]/127.5-1.;y=a[:,:,1]/127.5-1.;scale=np.maximum(np.sqrt(x*x+y*y),1.);x/=scale;y/=scale
                z=np.sqrt(np.maximum(0.,1.-x*x-y*y));image=Image.fromarray(np.rint((np.stack((x,y,z),axis=2)*.5+.5)*255.).astype('uint8'))
            elif kind=='rough':
                grey=ImageChops.invert(image.convert('L'));white=Image.new('L',image.size,255);image=Image.merge('RGB',(white,grey,white))
            image.save(dest)
        return dest.relative_to(assets).as_posix()
    model_data={};mat_data={};errors=[]
    recipe_base=json.loads((ROOT/'tools/default_skater_retail_manifest.json').read_text())
    recipe_base['morph_assembly']['live_targets']=['fat','thin']+recipe_base['morph_assembly']['face_targets']+['global_ethnicity_african','global_ethnicity_asian','global_ethnicity_caucasian']
    reference=out/'reference'
    for component in recipe_base['components']:
        slot=component['slot'];model=next(m for p in catalog['components'] if p['slot']==slot for m in p['models'] if m['id']==component['asset_id'])
        src=extract(next(l for l in model['lods'] if l['index']==0)['path']);dest=reference/slot/src.name
        dest.parent.mkdir(parents=True,exist_ok=True)
        if not dest.exists():shutil.copyfile(src,dest)
    defaults={x['material_id']:x for x in recipe_base['components']}
    # Read-only shared rig inputs live only for this preparation invocation.
    # Per-item meshes and morphs are discarded after conversion, bounding RAM.
    animation=AnimSource(assets/'private/stock/data/anim/OnBoard.abin')
    reference_skeleton=SkeletonSet(str(reference))
    for part in catalog['components']:
        for model in part['models']:
            if part['slot']=='Misc':continue  # Non-geometric stamp catalogue, indexed below.
            slot=part['slot'];key=model['id'];path=out/(key+'_v4.glb')
            try:
                lod=next(l for l in model['lods'] if l['index']==0)
                if not path.exists():
                    source=extract(lod['path']);folder=out/'work'/key/'models'/slot;folder.mkdir(parents=True,exist_ok=True)
                    mesh_file=folder/source.name
                    if not mesh_file.exists():shutil.copyfile(source,mesh_file)
                    parsed=RX2.parse_rx2(str(mesh_file));mesh=next(m for m in parsed['meshes'] if m.get('positions') and m.get('indices'))
                    morphs=decode_dense_morphs(mesh_file,parsed,len(mesh['positions']),RX2)
                    recipe=copy.deepcopy(recipe_base);recipe['components']=[dict(slot=slot,textures={},tint=[1.,1.,1.])]
                    recipe['morph_assembly']['expected_targets']={slot:[m['name'] for m in morphs]}
                    temporary=path.with_suffix('.tmp')
                    convert(folder.parent,assets/'private',recipe,output=temporary,geometry_only=True,live_morphs=True,reference_models=reference,
                            source=animation,reference_skeleton=reference_skeleton,
                            parsed_models={str(mesh_file):parsed},decoded_morphs={str(mesh_file):morphs})
                    temporary.replace(path)
                groups=[[v['id'] for v in group] for group in lod['material_instances']]
                model_data[key]=dict(slot=slot,name=friendly(model['name']),flags={k[4:]:v for k,v in model['flags'].items() if v},
                    materials=groups[0],groups=groups,scene=path.relative_to(assets).as_posix())
                for v in (v for group in lod['material_instances'] for v in group):
                    try:
                        mid=v['id']
                        if mid in mat_data:continue
                        mat=catalog['materials'][mid];tex={t['channel']:t['id'] for t in mat['textures']}
                        if 'diffuse' not in tex:continue
                        diffuse=texture(tex['diffuse'])
                        if 'alpha' in tex and slot!='Hair':
                            alpha_path=assets/texture(tex['alpha']);combined=out/'textures'/(tex['diffuse']+'_'+tex['alpha']+'.png')
                            if not combined.exists():
                                im=Image.open(assets/diffuse).convert('RGBA');alpha=Image.open(alpha_path).convert('RGBA')
                                alpha=alpha.resize(im.size,Image.Resampling.LANCZOS);mask=alpha.getchannel('A')
                                if mask.getextrema()==(255,255):mask=alpha.convert('L')
                                im.putalpha(ImageChops.multiply(im.getchannel('A'),mask));im.save(combined)
                            diffuse=combined.relative_to(assets).as_posix()
                        mat_data[mid]=dict(name=variant_label(slot,model['name'],v['name']),flags={k[4:]:val for k,val in mat['flags'].items() if val},
                            diffuse=diffuse,normal=texture(tex['normal'],'normal') if 'normal' in tex else None,
                            rough=texture(tex['specular'],'rough') if 'specular' in tex else None,
                            opacity=texture(tex['alpha']) if slot=='Hair' and 'alpha' in tex else None,
                            alpha='alpha' in tex,tint=defaults.get(mid,{}).get('tint',
                                [0.72,0.57,0.49] if mat['flags'].get('cas.SkinTone')=='light' else
                                [0.33,0.26,0.23] if mat['flags'].get('cas.SkinTone')=='dark' else
                                [0.02,0.01,0.01] if slot=='Hair' else [1.,1.,1.]),
                            metallic=.65 if slot=='SkateTruck' else 0.,roughness=.48 if slot in {'SkateTruck','SkateWheel'} else .72)
                    except CONTENT_ERRORS as error:
                        errors.append(dict(model=key,material=v.get('id'),slot=slot,error=str(error)))
            except CONTENT_ERRORS as e:errors.append(dict(model=key,slot=slot,error=str(e)))
        print('Prepared',part['slot'],len(model_data),'models',len(mat_data),'materials',flush=True)
    tattoos={}
    misc=next((p for p in catalog['components'] if p['slot']=='Misc'),None)
    variants=(v for m in (misc or {}).get('models',[]) for lod in m.get('lods',[])
              for group in lod.get('material_instances',[]) for v in group)
    for variant in variants:
        try:
            mid=variant['id'];mat=catalog['materials'][mid]
            if not mat['flags'].get('cas.TattooCategory'):continue
            tex={t['channel']:t['id'] for t in mat['textures']}
            tattoos[mid]=dict(name=friendly(variant['name']),texture=texture(tex['decal']),
                bounds=[float(v) for v in mat['flags']['cas.StampBorderConstraint'].split(',')])
        except CONTENT_ERRORS as error:
            errors.append(dict(tattoo=variant.get('id'),error=str(error)))
    prune_unavailable(model_data,mat_data,errors)
    male=default_profile();male['gender']='male'
    female=copy.deepcopy(male);female['gender']='female';female['selections']={}
    raw=parse_fallback_recipe(assets/'private/stock/data/cacrecipes/SavedRecipeFallbackFemale.bin',16)
    for slot,items in raw['asset_lists'].items():
        if items:
            item=items[0];female['selections'][slot]={'asset_id':item['asset_id'],'material_id':item['models'][0]['material_id']}
    colours=[]
    native=json.loads((cache/'native.json').read_text())
    for row in native['collections']:
        if row['class']=='cac_colour' and row['key'].endswith('_clothing') and row['key']!='default_clothing':
            colours.append(dict(name=row['key'][:-9].replace('_',' ').title(),
                rgb=struct.unpack('>4f',bytes.fromhex(row['fields']['Hash_7827ED970A88B70C']['data']))[:3]))
    colours.sort(key=lambda c:c['name'])
    library=dict(version=3,colours=colours,tattoos=tattoos,models=model_data,materials=mat_data,defaults={'male':male,'female':female},
                 morphs=recipe_base['morph_assembly']['live_targets'],errors=errors)
    (cache/config.get('library_index','library.json')).write_text(json.dumps(library,separators=(',',':')))
    print('LIBRARY_READY',len(model_data),len(mat_data),'errors',errors,flush=True)
    return library

if __name__=='__main__':prepare(json.loads(Path(sys.argv[1]).read_text()))
