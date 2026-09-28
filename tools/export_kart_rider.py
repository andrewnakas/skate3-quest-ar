"""Export fitted Blender poses to native vehicle clips, including steering reach."""
import bpy,json,sys,math,argparse
from pathlib import Path
from mathutils import Matrix,Vector,Quaternion
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools/mixamo_to_skate'))
from glb import Document

def main():
 p=argparse.ArgumentParser();p.add_argument('--enter',type=Path,required=True);p.add_argument('--exit',type=Path,required=True);p.add_argument('--bone-names',type=Path,required=True);p.add_argument('--reference',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:])
 bone_names=json.loads(a.bone_names.read_text())
 reference=Document(a.reference)
 world_rest={n['name']:Matrix(reference.worlds[i].tolist()) for i,n in enumerate(reference.doc['nodes']) if 'name' in n}
 basis=Matrix(((1,0,0,0),(0,0,-1,0),(0,1,0,0),(0,0,0,1)))
 seat=Vector((0,.2,.73));clips={}
 def export(arm):
  frame=[]
  for name in bone_names:
   if name in arm.pose.bones and name in world_rest:
    bone=arm.pose.bones[name]
    # Preserve glTF's skin bind basis rather than assuming Blender bone roll.
    g=basis.inverted()@bone.matrix@bone.bone.matrix_local.inverted()@basis@world_rest[name]
    g.translation-=basis.inverted().to_3x3()@seat
    native=g@basis
   else:native=Matrix.Identity(4)
   frame.append([round(native[r][c],6) for c in range(4) for r in range(4)])
  return frame
 for name,path in [('enter',a.enter),('exit',a.exit)]:
  bpy.ops.wm.open_mainfile(filepath=str(path.resolve()));s=bpy.context.scene;arm=bpy.data.objects['Skate character — kart fitted']
  frames=[]
  for f in range(s.frame_start,s.frame_end+1):s.frame_set(f);bpy.context.view_layer.update();frames.append(export(arm))
  clips[name]={'fps':s.render.fps,'frames':frames}
 # Seated centre and two steering endpoints; host blends between them continuously.
 bpy.ops.wm.open_mainfile(filepath=str(a.enter.resolve()));s=bpy.context.scene;s.frame_set(s.frame_end);bpy.context.view_layer.update();arm=bpy.data.objects['Skate character — kart fitted']
 base={b.name:b.matrix.copy() for b in arm.pose.bones}
 arm.animation_data_clear()
 clips['drive']={'fps':30,'frames':[export(arm)]}
 for name,angle in [('steer_left',.55),('steer_right',-.55)]:
  for b in arm.pose.bones:b.matrix=base[b.name];bpy.context.view_layer.update()
  rotation=Quaternion(Vector((0,-.8,.6)),angle);center=Vector((0,-.36,.9))
  for side in ['LEFT','RIGHT']:
   upper,lower,hand=[arm.pose.bones[side+n] for n in ['ARM','FOREARM','HAND']]
   r,m,t=upper.head.copy(),lower.head.copy(),hand.head.copy();goal=center+rotation@(t-center)
   l1,l2=(m-r).length,(t-m).length;direction=(goal-r).normalized();distance=min((goal-r).length,l1+l2-.00001)
   bend=(m-r)-direction*(m-r).dot(direction);bend.normalize();x=(l1*l1-l2*l2+distance*distance)/(2*distance)
   elbow=r+direction*x+bend*math.sqrt(max(0,l1*l1-x*x));reach=r+direction*distance
   qu=(m-r).normalized().rotation_difference((elbow-r).normalized())@base[upper.name].to_quaternion()
   ql=(t-m).normalized().rotation_difference((reach-elbow).normalized())@base[lower.name].to_quaternion()
   upper.matrix=Matrix.LocRotScale(r,qu,Vector((1,1,1)));bpy.context.view_layer.update()
   lower.matrix=Matrix.LocRotScale(elbow,ql,Vector((1,1,1)));bpy.context.view_layer.update()
   hand.matrix=Matrix.LocRotScale(reach,rotation@base[hand.name].to_quaternion(),Vector((1,1,1)));bpy.context.view_layer.update()
  clips[name]={'fps':30,'frames':[export(arm)]}
  s.render.filepath=str((a.output.parent/(name+'.png')).resolve());bpy.ops.render.render(write_still=True)
 data={'version':1,'bone_names':bone_names,'clips':clips}
 a.output.write_text(json.dumps(data,separators=(',',':'),allow_nan=False),encoding='utf-8')
 print('RIDER_EXPORT',a.output,a.output.stat().st_size,len(bone_names))

if __name__=='__main__':main()
