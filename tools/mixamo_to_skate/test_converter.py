"""Procedural fixtures only: no stock or other copyrighted meshes."""
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np

from converter import MAP, convert_file, convert_glb, create_profile, reference_rig
from glb import ConversionError, Document, Writer


def fixture(path, mixamo, scale=1, bad_weights=False, morph=False):
    positions = {'hips':(0,1,0),'spine':(0,1.15,0),'spine1':(0,1.3,0),
                 'spine2':(0,1.45,0),'neck':(0,1.6,0),'head':(0,1.8,0)}
    for s,sign in [('left',1),('right',-1)]:
        for part,p in {'shoulder':(.1,1.5,0),'arm':(.2,1.5,0),'forearm':(.45,1.5,0),
                       'hand':(.7,1.5,0),'upleg':(.1,.95,0),'leg':(.1,.5,0),
                       'foot':(.1,.08,0),'toebase':(.1,.03,.15)}.items():
            positions[s+part]=(p[0]*sign,p[1],p[2])
    keys = list(MAP)
    w=Writer()
    nodes=[]
    inverses=[]
    verts=[]
    joint_weights=[]
    joint_indices=[]
    for i,k in enumerate(keys):
        p=np.asarray(positions[k])*scale
        nodes.append({'name':('mixamorig:'+k if mixamo else MAP[k]),'translation':p.tolist()})
        if i:
            nodes[-1]['translation']=(p-np.asarray(positions['hips'])*scale).tolist()
        m=np.eye(4);m[:3,3]=-p
        inverses.append(m.T.ravel())
        for v in [(0,0,0),(.015,0,0),(0,.015,0)]:
            verts.append(p+np.asarray(v)*scale)
            joint_indices.append([i,0,0,0])
            joint_weights.append([1,0,0,0])
    nodes[0]['children']=list(range(1,len(nodes)))
    if mixamo:
        # Extra weighted finger is not present in the destination rig.
        hand=keys.index('lefthand');finger=len(nodes)
        nodes.append({'name':'mixamorig:LeftHandIndex1','translation':[.02*scale,0,0]})
        nodes[hand]['children']=[finger]
        m=np.eye(4);m[:3,3]=-(np.asarray(positions['lefthand'])+np.array([.02,0,0]))*scale
        inverses.append(m.T.ravel())
        for i in range(hand*3,hand*3+3):
            joint_indices[i]=[finger,hand,0,0];joint_weights[i]=[.7,.3,0,0]
    if bad_weights:
        joint_indices[0][0]=999
    attrs={'POSITION':w.accessor(verts,'VEC3'),
           'NORMAL':w.accessor([[0,0,1]]*len(verts),'VEC3'),
           'JOINTS_0':w.accessor(joint_indices,'VEC4',5123),
           'WEIGHTS_0':w.accessor(joint_weights,'VEC4')}
    primitive={'attributes':attrs,'indices':w.accessor(np.arange(len(verts)),'SCALAR',5125)}
    if morph:
        primitive['targets']=[{'POSITION':attrs['POSITION']}]
    skin={'joints':list(range(len(nodes))),'inverseBindMatrices':w.accessor(inverses,'MAT4')}
    nodes.append({'name':'body','mesh':0,'skin':0})
    w.doc.update(nodes=nodes,skins=[skin],meshes=[{'primitives':[primitive]}],
                 scenes=[{'nodes':[0,len(nodes)-1]}],scene=0)
    w.save(path)


class ConversionTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root=Path(self.tmp.name)
        self.ref=self.root/'stock.glb';self.src=self.root/'mixamo.glb';self.out=self.root/'out.glb'
        fixture(self.ref,False)
        fixture(self.src,True,.01)

    def test_calibrated_roundtrip_scale_weights_and_runtime_pose(self):
        profile=create_profile(self.src,self.ref)
        report=convert_glb(self.src,self.ref,self.out,profile,False)
        src,ref=Document(self.out),Document(self.ref)
        p=src.doc['meshes'][0]['primitives'][0]
        r=ref.doc['meshes'][0]['primitives'][0]
        got=src.accessor(p['attributes']['POSITION'])
        expected=ref.accessor(r['attributes']['POSITION'])
        np.testing.assert_allclose(got,expected,atol=2e-6)
        _,names,inverse=reference_rig(src)
        hand=names.index('LEFTHAND')
        js=src.accessor(p['attributes']['JOINTS_0']).astype(int)
        ws=src.accessor(p['attributes']['WEIGHTS_0'])
        np.testing.assert_allclose(ws.sum(1),1,atol=1e-6)
        for i in range(hand*3,hand*3+3):
            self.assertEqual(js[i,0],hand)
            self.assertAlmostEqual(ws[i,0],1)
        # Simulate a translated stock hand: its vertices move exactly 20 cm,
        # while an unrelated head vertex stays put. No engine/game is run.
        globals=np.linalg.inv(inverse)
        globals[hand,0,3]+=.2
        skin_mats=globals@inverse
        hv=np.column_stack([got,np.ones(len(got))])
        deformed=np.einsum('vi,vijk,vk->vj',ws,skin_mats[js],hv)[:,:3]
        expected_pose=got.copy();expected_pose[hand*3:hand*3+3,0]+=.2
        np.testing.assert_allclose(deformed,expected_pose,atol=2e-6)
        self.assertIn('mixamorig:LeftHandIndex1',report['collapsed_bones'])

    def test_generic_fit_and_no_imported_bones(self):
        report=convert_glb(self.src,self.ref,self.out,None,False)
        self.assertFalse(report['calibrated'])
        names=[n.get('name','') for n in Document(self.out).doc['nodes']]
        self.assertFalse(any('mixamorig' in n for n in names))
        got,ref=Document(self.out),Document(self.ref)
        p=got.doc['meshes'][0]['primitives'][0]['attributes']['POSITION']
        r=ref.doc['meshes'][0]['primitives'][0]['attributes']['POSITION']
        np.testing.assert_allclose(got.accessor(p),ref.accessor(r),atol=2e-6)

    def test_rotated_translated_rescaled_scene_uses_same_calibration(self):
        profile=create_profile(self.src,self.ref)
        d=Document(self.src);w=Writer();w.doc=d.doc;w.data=bytearray(d.data)
        root=len(w.doc['nodes'])
        w.doc['nodes'].append({'name':'export_axes','children':[0,root-1],
                              'rotation':[0,2**-.5,0,2**-.5],
                              'scale':[3,3,3],'translation':[8,2,-3]})
        w.doc['scenes'][0]['nodes']=[root];w.save(self.src)
        convert_glb(self.src,self.ref,self.out,profile,False)
        got,ref=Document(self.out),Document(self.ref)
        p=got.doc['meshes'][0]['primitives'][0]['attributes']['POSITION']
        r=ref.doc['meshes'][0]['primitives'][0]['attributes']['POSITION']
        np.testing.assert_allclose(got.accessor(p),ref.accessor(r),atol=2e-6)

    def test_bad_joint_fails_transaction_without_mutating_source(self):
        fixture(self.src,True,.01,bad_weights=True)
        before=self.src.read_bytes();ref_before=self.ref.read_bytes()
        with self.assertRaisesRegex(ConversionError,'joint index'):
            convert_file(self.src,self.ref,self.root/'result','unused')
        self.assertFalse((self.root/'result').exists())
        self.assertEqual(self.src.read_bytes(),before)
        self.assertEqual(self.ref.read_bytes(),ref_before)
        self.assertFalse(list(self.root.glob('.mixamo-*')))

    def test_existing_output_never_overwritten(self):
        destination=self.root/'result';destination.mkdir()
        (destination/'keep').write_text('keep')
        with self.assertRaisesRegex(ConversionError,'already exists'):
            convert_file(self.src,self.ref,destination,'unused')
        self.assertEqual((destination/'keep').read_text(),'keep')

    def test_profile_identity_mismatch(self):
        profile=create_profile(self.src,self.ref)
        profile['reference_sha256']='wrong'
        with self.assertRaisesRegex(ConversionError,'another stock reference'):
            convert_glb(self.src,self.ref,self.out,profile,False)

    def test_morph_rejected_explicitly(self):
        fixture(self.src,True,.01,morph=True)
        with self.assertRaisesRegex(ConversionError,'Blendshapes'):
            convert_glb(self.src,self.ref,self.out,None,False)

    def test_truncated_glb_rejected(self):
        self.src.write_bytes(self.src.read_bytes()[:-20])
        with self.assertRaisesRegex(ConversionError,'header'):
            Document(self.src)

    def test_missing_bone_rejected(self):
        d=Document(self.src);w=Writer();w.doc=d.doc;w.data=bytearray(d.data)
        w.doc['nodes'][1]['name']='wrong';w.save(self.src)
        with self.assertRaisesRegex(ConversionError,'missing'):
            convert_glb(self.src,self.ref,self.out,None,False)


if __name__=='__main__':
    unittest.main()
