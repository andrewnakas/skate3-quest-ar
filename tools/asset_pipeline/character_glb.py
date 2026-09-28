"""Write the retail modular skater directly as glTF 2.0 binary."""
from pathlib import Path
import io,json,struct
from PIL import Image
import numpy as np
from tools.asset_pipeline.retail_character import ABIN,RX2,AnimSource,SkeletonSet,quat_matrix,decode_dense_morphs,morph_weight

class Glb:
    def __init__(self):
        self.data=bytearray()
        self.doc={'asset':{'version':'2.0','generator':'Skate 3 Rust Engine'},'bufferViews':[],
                  'accessors':[],'images':[],'textures':[],'materials':[]}
    def view(self,data):
        self.data.extend(b'\0'*(-len(self.data)%4));offset=len(self.data);self.data.extend(data)
        self.doc['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':len(data)})
        return len(self.doc['bufferViews'])-1
    def accessor(self,values,kind,component=5126,bounds=False):
        a=np.asarray(values,dtype={5126:'<f4',5123:'<u2',5125:'<u4'}[component])
        if component==5126 and not np.isfinite(a).all():raise ValueError('Non-finite character data')
        item={'bufferView':self.view(a.tobytes()),'componentType':component,'count':len(a),'type':kind}
        if bounds:item.update(min=a.min(axis=0).tolist(),max=a.max(axis=0).tolist())
        self.doc['accessors'].append(item);return len(self.doc['accessors'])-1
    def texture(self,path):
        data=path if isinstance(path,bytes) else path.read_bytes()
        self.doc['images'].append({'bufferView':self.view(data),'mimeType':'image/png'})
        self.doc['textures'].append({'source':len(self.doc['images'])-1})
        return {'index':len(self.doc['textures'])-1}
    def save(self,path):
        self.doc['buffers']=[{'byteLength':len(self.data)}]
        text=json.dumps(self.doc,separators=(',',':'),allow_nan=False).encode();text+=b' '*(-len(text)%4)
        self.data.extend(b'\0'*(-len(self.data)%4))
        path.write_bytes(struct.pack('<III',0x46546c67,2,28+len(text)+len(self.data))+
                        struct.pack('<I4s',len(text),b'JSON')+text+struct.pack('<I4s',len(self.data),b'BIN\0')+self.data)

def convert(models,private,recipe,*,stock=None,output=None,materials=None,live_morphs=False,geometry_only=False,reference_models=None,
            source=None,reference_skeleton=None,parsed_models=None,decoded_morphs=None):
    source=source or AnimSource((stock or private/'stock')/'data/anim/OnBoard.abin')
    skeleton=SkeletonSet(str(models),parsed_models)
    if reference_models:
        reference_skeleton=reference_skeleton or SkeletonSet(str(reference_models))
        skeleton.bind={**reference_skeleton.bind,**skeleton.bind}
    if skeleton.errors:raise ValueError(str(skeleton.errors))
    mapping=skeleton.index_map(ABIN.BONE_NAMES,source.parents)
    worlds={mapping[n]:m for n,m in skeleton.bind.items() if n in mapping}
    names=list(ABIN.BONE_NAMES)
    reference={}
    for index,parent in enumerate(source.parents):
        sqt=source.ref_pose.get(index)
        local=quat_matrix(sqt.quat,sqt.scale) if sqt else np.eye(4)
        if sqt:local[3,:3]=sqt.trans
        reference[index]=local@reference[parent] if parent>=0 else local
    for index,name in {32:'RIGHTTOEBASE_REPARENTED',33:'LEFTTOEBASE_REPARENTED',34:'LEFTHAND_REPARENTED',35:'RIGHTHAND_REPARENTED'}.items():
        worlds[index]=reference[index];names[index]=name
    # Retain the established mesh bone-local basis consumed by render_basis()
    # in the game. This algebraic change needs no editor or animation bake.
    basis_transpose=np.array([[1.,0,0,0],[0,0,-1.,0],[0,1.,0,0],[0,0,0,1.]])
    worlds={i:basis_transpose@m for i,m in worlds.items()}
    indices=sorted(worlds);joint_index={bone:i for i,bone in enumerate(indices)}
    glb=Glb();nodes=[];root_children=[]
    # RX2 matrices use row vectors. Their row-major bytes are glTF's
    # column-major representation of the equivalent column-vector matrix.
    for bone in indices:
        parent=source.parents[bone]
        while parent>=0 and parent not in worlds:parent=source.parents[parent]
        local=worlds[bone]@np.linalg.inv(worlds[parent]) if parent>=0 else worlds[bone]
        nodes.append({'name':names[bone],'matrix':local.ravel().tolist()})
        if parent<0:root_children.append(joint_index[bone])
    for bone in indices:
        parent=source.parents[bone]
        while parent>=0 and parent not in worlds:parent=source.parents[parent]
        if parent>=0:nodes[joint_index[parent]].setdefault('children',[]).append(joint_index[bone])
    inverse=glb.accessor([np.linalg.inv(worlds[i]).ravel() for i in indices],'MAT4')
    primitives=[];components={c['slot']:c for c in recipe['components']}
    meshes=skeleton.renderable_meshes(ABIN.BONE_NAMES,len(source.parents),source.parents)
    if {m['folder'] for m in meshes}!=set(components):raise ValueError('Incomplete character mesh set')
    for mesh in meshes:
        slot=mesh['folder'];component=components[slot];path=models/slot/mesh['name']
        parsed=parsed_models[str(path)] if parsed_models is not None else RX2.parse_rx2(str(path))
        raw=[m for m in parsed['meshes'] if m.get('positions') and m.get('indices')]
        if len(raw)!=1:raise ValueError('Ambiguous character mesh '+slot)
        raw=raw[0];positions=np.asarray(mesh['pos'],dtype=np.float64)
        morphs=decoded_morphs[str(path)] if decoded_morphs is not None else decode_dense_morphs(path,parsed,len(positions),RX2)
        if [m['name'] for m in morphs]!=recipe['morph_assembly']['expected_targets'][slot]:raise ValueError('Unexpected morph set '+slot)
        if 'normals' in raw:
            normals=np.asarray(raw['normals'],dtype=np.float64)
        else:
            normals=np.zeros_like(positions)
            for triangle in mesh['tris']:
                a,b,c=triangle;normal=np.cross(positions[b]-positions[a],positions[c]-positions[a])
                for index in triangle:normals[index]+=normal
            normals/=np.maximum(np.linalg.norm(normals,axis=1,keepdims=True),1e-20)
        for morph in ([] if live_morphs else morphs):
            weight=morph_weight(morph['name'],recipe)
            positions+=np.asarray(morph['deltas'])*weight
            if weight:
                normals+=np.asarray(morph['normal_deltas'])*weight
        normals/=np.maximum(np.linalg.norm(normals,axis=1,keepdims=True),1e-20)
        joints=np.zeros((len(positions),4),dtype=np.uint16);weights=np.zeros((len(positions),4),dtype=np.float32)
        if not mesh['skin']:raise ValueError('Missing skin '+slot)
        for i,influences in enumerate(mesh['skin']):
            combined={}
            for bone,w in influences:combined[bone]=combined.get(bone,0)+w
            if len(combined)>4 or not combined:raise ValueError('Invalid skin influences '+slot)
            for j,(bone,w) in enumerate(combined.items()):joints[i,j]=joint_index[bone];weights[i,j]=w
        weights/=weights.sum(axis=1,keepdims=True)
        folder=materials or private/'default_skater/textures/materials'
        pbr={'baseColorFactor':[*component['tint'],1.0],
             'metallicFactor':0.65 if slot=='SkateTruck' else 0.0,
             'roughnessFactor':0.48 if slot in {'SkateTruck','SkateWheel'} else 0.72}
        if not geometry_only:pbr['baseColorTexture']=glb.texture(folder/(slot+'_base_color.png'))
        rough=folder/(slot+'_roughness.png')
        if not geometry_only and rough.is_file():
            channel=Image.open(rough).convert('RGB').getchannel('R')
            white=Image.new('L',channel.size,255);packed=Image.merge('RGB',(white,channel,white));buffer=io.BytesIO();packed.save(buffer,format='PNG')
            pbr['metallicRoughnessTexture']=glb.texture(buffer.getvalue());pbr['roughnessFactor']=1.0
        material={'name':'Retail_'+slot,'pbrMetallicRoughness':pbr,'doubleSided':False,
                  'alphaMode':component.get('alpha_mode','OPAQUE')}
        if material['alphaMode']=='MASK':material['alphaCutoff']=component.get('alpha_cutoff',0.5)
        if not geometry_only and component['textures'].get('normal'):material['normalTexture']=glb.texture(folder/(slot+'_normal.png'))
        glb.doc['materials'].append(material)
        attributes={'POSITION':glb.accessor(positions,'VEC3',bounds=True),'NORMAL':glb.accessor(normals,'VEC3'),
                    'TEXCOORD_0':glb.accessor(raw['uvs'],'VEC2'),'JOINTS_0':glb.accessor(joints,'VEC4',5123),
                    'WEIGHTS_0':glb.accessor(weights,'VEC4')}
        if live_morphs and 'uvs2' in raw:
            attributes['TEXCOORD_1']=glb.accessor(raw['uvs2'],'VEC2')
        primitives.append({'attributes':attributes,'indices':glb.accessor(np.asarray(mesh['tris']).ravel(),'SCALAR',5125),
                           'material':len(glb.doc['materials'])-1})
        if live_morphs:
            names_morph=recipe['morph_assembly'].get('live_targets',['fat','thin']+recipe['morph_assembly']['face_targets'])
            zero=glb.accessor(np.zeros_like(positions),'VEC3',bounds=True)
            targets={}
            for morph in morphs:
                name=morph['name']
                key='fat' if name.startswith('fat') else 'thin' if name.startswith('thin') else name
                if key not in names_morph:raise ValueError('Unmapped live morph '+name)
                targets[key]={'POSITION':glb.accessor(morph['deltas'],'VEC3',bounds=True),
                              'NORMAL':glb.accessor(morph['normal_deltas'],'VEC3')}
            primitives[-1]['targets']=[targets.get(n,{'POSITION':zero,'NORMAL':zero}) for n in names_morph]
    mesh_node=len(nodes);nodes.append({'name':'Skate3_SkaterAndBoard','mesh':0,'skin':0})
    root=len(nodes);nodes.append({'name':'Skate3_RX2_Rig','children':root_children+[mesh_node]})
    glb.doc.update(nodes=nodes,meshes=[{'primitives':primitives}],skins=[{'joints':list(range(len(indices))),
                    'inverseBindMatrices':inverse}],scenes=[{'nodes':[root]}],scene=0)
    if live_morphs:
        glb.doc['meshes'][0].update(weights=[.25 if n in recipe['morph_assembly']['face_targets'] else 0. for n in names_morph],
                                  extras={'targetNames':names_morph})
    glb.save(output or private/'skater.glb')

if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--models',type=Path,required=True)
    parser.add_argument('--private',type=Path,required=True)
    parser.add_argument('--recipe',type=Path,required=True)
    args=parser.parse_args()
    convert(args.models,args.private,json.loads(args.recipe.read_text()))
    print('CHARACTER_GLB_READY')
