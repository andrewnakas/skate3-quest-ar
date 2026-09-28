"""Build any disc district using the supplied retail scene and map exporters."""
from pathlib import Path
import os,sys
import bpy

tools,manifest,blend,package=map(Path,sys.argv[sys.argv.index('--')+1:sys.argv.index('--')+5])
district=sys.argv[-1]
base=tools/'vendor/university/tools'
sys.path.insert(0,str(base/'vanilla_map_extraction/blender'))
from import_hawaiian_dream import build_scene
import prepare_university_owned as owned
summary=build_scene(manifest)
if summary['objects']!=summary['expected_objects']:
    raise RuntimeError('District import omitted objects')
if district!='DIST_University':
    # Other districts have different world-space bounds. Choose an actual
    # broad upward collision face, rather than spawning at University's XZ.
    candidates=[]
    for obj in bpy.data.objects:
        if obj.type!='MESH' or not obj.get('skate3_retail_collision',False): continue
        for face in obj.data.polygons:
            if face.normal.z>0.9 and face.area>2:
                p=obj.matrix_world@face.center
                candidates.append((p.x*p.x+p.y*p.y,p))
    if not candidates: raise RuntimeError(f'No walkable spawn surface in {district}')
    p=min(candidates,key=lambda item:item[0])[1]
    owned.SPAWN_RUNTIME_XZ=(p.x,-p.y)
    owned.SPAWN_TARGET_HEIGHT=p.z
sys.argv=['owned','--',str(blend)]
owned.main()
bpy.context.scene['ow_map_name']=district.removeprefix('DIST_')
os.environ['SKATE_EXPORT_COMPAT14']='1'
sys.path.insert(0,str(base/'blender_owned_map'))
from owned_world_material_addon.exporter import main
sys.argv=['export','--',str(package),'--force']
main()
