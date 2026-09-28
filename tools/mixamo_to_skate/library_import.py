"""Game-facing import job. File chooser and CPU thumbnail run outside the game."""
import hashlib
import json
import os
import shutil
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
from PIL import Image

from converter import convert_file, sha, validate_output
from glb import Document, require


def thumbnail(path, output, width=256, height=320):
    doc=Document(path)
    items=[]
    for node in doc.doc['nodes']:
        if 'mesh' not in node:
            continue
        for p in doc.doc['meshes'][node['mesh']]['primitives']:
            pos=doc.accessor(p['attributes']['POSITION'])
            idx=doc.accessor(p['indices']).ravel().reshape(-1,3).astype(int)
            material=doc.doc.get('materials',[])[p['material']] if 'material' in p else {}
            pbr=material.get('pbrMetallicRoughness',{})
            color=np.asarray(pbr.get('baseColorFactor',[.65,.72,.8,1]))
            texture=None
            if 'baseColorTexture' in pbr:
                import io
                data,_=doc.image(doc.doc['textures'][pbr['baseColorTexture']['index']]['source'])
                with Image.open(io.BytesIO(data)) as image:
                    image.thumbnail((1024,1024))
                    texture=np.asarray(image.convert('RGBA'))/255.
            uv=doc.accessor(p['attributes']['TEXCOORD_0']) if 'TEXCOORD_0' in p['attributes'] else None
            items.append((pos,idx,color,texture,uv,material))
    require(items,'Cannot preview an empty model')
    angle=.22
    basis=np.array([[np.cos(angle),0,-np.sin(angle)], [0,1,0], [np.sin(angle),0,np.cos(angle)]])
    all_points=np.concatenate([p for p,*_ in items])@basis.T
    low,high=all_points.min(0),all_points.max(0)
    scale=min((width-24)/max(high[0]-low[0],.01),(height-24)/max(high[1]-low[1],.01))
    centre=(high+low)/2
    rgb=np.zeros((height,width,3),dtype=float)+np.array([.055,.075,.11])
    depth=np.full((height,width),-np.inf)
    light=np.array([.3,.6,1]);light/=np.linalg.norm(light)
    for pos,idx,color,texture,uv,material in items:
        world=pos@basis.T
        points=(world-centre)*scale
        points[:,0]+=width/2;points[:,1]=height/2-points[:,1]
        for face in idx:
            tri=points[face];a,b,c=tri[:,:2]
            xmin=max(0,int(np.floor(tri[:,0].min())));xmax=min(width-1,int(np.ceil(tri[:,0].max())))
            ymin=max(0,int(np.floor(tri[:,1].min())));ymax=min(height-1,int(np.ceil(tri[:,1].max())))
            if xmin>xmax or ymin>ymax:continue
            denom=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
            if abs(denom)<1e-8:continue
            yy,xx=np.mgrid[ymin:ymax+1,xmin:xmax+1];xx=xx+.5;yy=yy+.5
            w0=((b[1]-c[1])*(xx-c[0])+(c[0]-b[0])*(yy-c[1]))/denom
            w1=((c[1]-a[1])*(xx-c[0])+(a[0]-c[0])*(yy-c[1]))/denom
            w2=1-w0-w1
            z=w0*tri[0,2]+w1*tri[1,2]+w2*tri[2,2]
            visible=(w0>=0)&(w1>=0)&(w2>=0)&(z>depth[ymin:ymax+1,xmin:xmax+1])
            if not visible.any():continue
            pixels=np.broadcast_to(color,(*xx.shape,4)).copy()
            if texture is not None and uv is not None:
                tex=w0[...,None]*uv[face[0]]+w1[...,None]*uv[face[1]]+w2[...,None]*uv[face[2]]
                tx=np.clip((tex[...,0]%1*texture.shape[1]).astype(int),0,texture.shape[1]-1)
                ty=np.clip((tex[...,1]%1*texture.shape[0]).astype(int),0,texture.shape[0]-1)
                pixels*=texture[ty,tx]
            if material.get('alphaMode')=='MASK':visible&=pixels[...,3]>=material.get('alphaCutoff',.5)
            normal=np.cross(world[face[1]]-world[face[0]],world[face[2]]-world[face[0]])
            normal/=max(np.linalg.norm(normal),1e-10)
            shade=.45+.55*abs(np.dot(normal,light))
            rgb[ymin:ymax+1,xmin:xmax+1][visible]=(pixels[...,:3]*shade)[visible]
            depth[ymin:ymax+1,xmin:xmax+1][visible]=z[visible]
    Image.fromarray(np.uint8(np.clip(rgb,0,1)*255)).save(output)


def import_model(source,reference,library,tool,profile=None):
    source,reference,library=Path(source),Path(reference),Path(library)
    digest=hashlib.sha256((sha(source)+sha(reference)+json.dumps(profile,sort_keys=True)+'library-v1').encode()).hexdigest()
    entries=library/'entries';entries.mkdir(parents=True,exist_ok=True)
    target=entries/digest
    if target.exists():
        require((target/'manifest.json').is_file() and (target/'preview.png').is_file(), 'Existing library entry is incomplete')
        validate_output(target/'character.glb',reference)
        return digest
    with tempfile.TemporaryDirectory(prefix='.import-',dir=library) as temp:
        staging=Path(temp)/'entry'
        # Already converted stock-rig GLBs are accepted as well as Mixamo exports.
        ready=False
        if source.suffix.lower()=='.glb':
            try:validate_output(source,reference);ready=True
            except (ValueError,KeyError):pass
        if ready:
            staging.mkdir();shutil.copy2(source,staging/'character.glb');shutil.copy2(source,staging/'source.glb')
        else:
            convert_file(source,reference,staging,tool,profile,keep_source=True)
        thumbnail(staging/'character.glb',staging/'preview.png')
        metadata={'version':1,'id':digest,'name':source.stem[:64],
                  'source_sha256':sha(source),'reference_sha256':sha(reference),
                  'created_utc':datetime.now(timezone.utc).isoformat()}
        (staging/'manifest.json').write_text(json.dumps(metadata,indent=2),encoding='utf-8')
        staging.rename(target)
    return digest


def run(args,app):
    result={'status':'cancelled'}
    try:
        require(args.result is not None and args.reference is not None,'Import needs result and reference paths')
        source=args.files[0] if args.files else None
        if source is None:
            import tkinter as tk
            from tkinter import filedialog
            root=tk.Tk();root.withdraw();root.attributes('-topmost',True)
            try:
                name=filedialog.askopenfilename(title='Import a Mixamo character',
                    filetypes=[('Character models','*.fbx *.glb')],parent=root)
                source=Path(name) if name else None
            finally:root.destroy()
        if source:
            profile_path=args.profile or app/'calibration.json'
            profile=json.loads(profile_path.read_text(encoding='utf-8')) if profile_path.is_file() else None
            if profile and profile['reference_sha256']!=sha(args.reference):profile=None
            identity=import_model(source,args.reference,args.library_import,
                                  args.fbx_tool or app/'tools/FBX2glTF.exe',profile)
            result={'status':'ready','id':identity}
    except Exception as error:
        result={'status':'error','message':str(error)}
    if args.result:
        args.result.parent.mkdir(parents=True,exist_ok=True)
        temporary=args.result.with_suffix('.tmp')
        temporary.write_text(json.dumps(result),encoding='utf-8');os.replace(temporary,args.result)
    return 1 if result['status']=='error' else 0
