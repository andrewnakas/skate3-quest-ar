"""Non-game protocol and dependency tests, with synthetic catalog data."""
import copy
import unittest
from tools.asset_pipeline.customisation_worker import resolve
from tools.asset_pipeline.retail_character import morph_weight


def fixture():
    def mat(tone):return {'flags':{'cas.SkinTone':tone,'cas.FacialHairStyle':'none'}}
    def model(id, gender='male', **flags):
        return {'id':id,'flags':{'cas.Gender':gender,**{'cas.'+k:v for k,v in flags.items()}},
                'lods':[{'index':0,'material_instances':[[{'id':'light'},{'id':'dark'}]]}]}
    catalog={'materials':{'light':mat('light'),'dark':mat('dark')},'components':[
        {'slot':'Rostral','models':[model('head'),model('female','female')]},
        {'slot':'OuterTorso','models':[model('top',ArmModelRequired='wrist')]},
        {'slot':'Arm','models':[model('long_arm',ArmModelType='shoulder'),model('wrist',ArmModelType='wrist')]}]}
    profile={'selections':{'Rostral':{'asset_id':'head','material_id':'light'},
                           'OuterTorso':{'asset_id':'top','material_id':'light'},
                           'Arm':{'asset_id':'long_arm','material_id':'light'}},'morphs':{}}
    return catalog,profile


class WorkerTests(unittest.TestCase):
    def test_dependency_replacement_does_not_mutate_request(self):
        c,p=fixture();before=copy.deepcopy(p);out=resolve(c,p)
        self.assertEqual(p,before)
        self.assertEqual(out['selections']['Arm']['asset_id'],'wrist')

    def test_tone_propagates_to_exposed_skin(self):
        c,p=fixture();p['skin']='dark';out=resolve(c,p)
        self.assertTrue(all(v['material_id']=='dark' for v in out['selections'].values()))

    def test_rejects_cross_model_material_and_unsupported_gender(self):
        c,p=fixture();p['selections']['Rostral']['material_id']='missing'
        with self.assertRaisesRegex(ValueError,'Invalid material'):resolve(c,p)
        p['selections']['Rostral']={'asset_id':'female','material_id':'light'}
        with self.assertRaisesRegex(ValueError,'Female'):resolve(c,p)

    def test_rejects_out_of_range_and_unmapped_morphs(self):
        c,p=fixture()
        for values in [{'fat':.6},{'thin':float('nan')},{'invented':.25}]:
            p['morphs']=values
            with self.assertRaises(ValueError):resolve(c,p)

    def test_per_target_face_values_preserve_other_defaults(self):
        recipe={'preset':{'body_mods':{'fatness':.4,'skinniness':.1,'face_fields':.25,
                                      'targets':{'local_nose_length':.5}}},
                'morph_assembly':{'face_targets':['local_nose_height','local_nose_length']}}
        self.assertEqual(morph_weight('local_nose_length',recipe),.5)
        self.assertEqual(morph_weight('local_nose_height',recipe),.25)
        self.assertEqual(morph_weight('fat_arms',recipe),.4)
        self.assertEqual(morph_weight('thin_arms',recipe),.1)


if __name__=='__main__':unittest.main()
