"""Fit the calibrated raw Blender previews to the kart without changing bind bones."""
import argparse, json, math, sys
from pathlib import Path
import bpy
from mathutils import Matrix, Quaternion, Vector

def smooth(t):
    t=max(0.,min(1.,t));return t*t*(3-2*t)

def aim(o,p):o.rotation_euler=(Vector(p)-o.location).to_track_quat('-Z','Y').to_euler()

def fit(args):
    args.output.mkdir(parents=True,exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(args.raw.resolve()))
    scene=bpy.context.scene
    arm=bpy.data.objects['Skate character — raw animation transfer']
    arm.name='Skate character — kart fitted'
    first,last=scene.frame_start,scene.frame_end
    seated=last if args.mode=='enter' else first
    rests={b.name:b.matrix_local.copy() for b in arm.data.bones}
    body=[]
    def visit(b):
        body.append(b.name)
        for child in b.children:visit(child)
    visit(arm.data.bones['HIPS'])
    scene.frame_set(seated);bpy.context.view_layer.update()
    rotation=Matrix.Rotation(-math.pi/2 if args.mode=='enter' else 0,4,'Z')
    seat=Vector((0,.20,.73))
    alignment=Matrix.Translation(seat-rotation@arm.pose.bones['HIPS'].head)@rotation
    originals={}
    for f in range(first,last+1):
        scene.frame_set(f);bpy.context.view_layer.update()
        originals[f]={name:alignment@arm.pose.bones[name].matrix for name in body}
    raw=arm.animation_data.action;raw.use_fake_user=True
    arm.animation_data.action=raw.copy();arm.animation_data.action.name='Kart '+args.mode+' — fitted from calibrated animation'
    bpy.ops.import_scene.gltf(filepath=str(args.kart.resolve()))
    bpy.data.objects['kart'].location.z=.55
    controls=bpy.data.collections.new('Kart fit targets');scene.collection.children.link(controls)
    markers={}
    for name,pos in {'Seat':seat,'Left grip':(.19,-.36,.90),'Right grip':(-.19,-.36,.90),
                     'Left footwell':(.17,-.55,.33),'Right footwell':(-.17,-.55,.33)}.items():
        o=bpy.data.objects.new(name,None);controls.objects.link(o);o.location=pos
        o.empty_display_type='SPHERE';o.empty_display_size=.04;markers[name]=o
    errors=[]
    def put(name,matrix):
        arm.pose.bones[name].matrix=matrix
        bpy.context.view_layer.update()
    def solve(upper,lower,end,goal):
        a,b,c=[arm.pose.bones[n].matrix.copy() for n in (upper,lower,end)]
        root,mid,tip=a.translation,b.translation,c.translation
        l1,l2=(mid-root).length,(tip-mid).length
        delta=goal-root;distance=delta.length
        direction=delta.normalized()
        distance=max(abs(l1-l2)+.00001,min(l1+l2-.00001,distance))
        reach=root+direction*distance
        bend=(mid-root)-direction*(mid-root).dot(direction)
        if bend.length<1e-5:bend=Vector((1 if upper.startswith('LEFT') else -1,0,0))
        bend.normalize()
        along=(l1*l1-l2*l2+distance*distance)/(2*distance)
        knee=root+direction*along+bend*math.sqrt(max(0,l1*l1-along*along))
        qa=(mid-root).normalized().rotation_difference((knee-root).normalized())@a.to_quaternion()
        qb=(tip-mid).normalized().rotation_difference((reach-knee).normalized())@b.to_quaternion()
        put(upper,Matrix.LocRotScale(root,qa,Vector((1,1,1))))
        put(lower,Matrix.LocRotScale(knee,qb,Vector((1,1,1))))
        c.translation=reach;put(end,c)
        return (reach-goal).length
    for f in range(first,last+1):
        scene.frame_set(f);bpy.context.view_layer.update()
        t=(f-first)/(last-first)
        weight=smooth((t-.46)/.30) if args.mode=='enter' else 1-smooth((t-.08)/.30)
        # Keep the standing ground height while blending down into the lower kart seat.
        lift=Vector((0,0,-alignment.translation.z*(1-weight)))
        hips=originals[f]['HIPS'].translation+lift
        lean=Matrix.Translation(hips)@Matrix.Rotation(math.radians(27)*weight,4,'X')@Matrix.Translation(-hips)
        for name in body:
            m=originals[f][name].copy();m.translation+=lift
            upper=name!='HIPS' and not any(k in name for k in ['LEG','FOOT','TOE'])
            if upper:
                m=lean@m
                if name=='HEAD':
                    q=originals[f][name].to_quaternion()
                    m=Matrix.LocRotScale(m.translation,q,Vector((1,1,1)))
            put(name,m)
        frame_errors=[]
        for side,label in [('LEFT','Left'),('RIGHT','Right')]:
            hand=arm.pose.bones[side+'HAND'].head.copy()
            if weight>0:
                goal=hand.lerp(markers[label+' grip'].location,weight)
                frame_errors.append(solve(side+'ARM',side+'FOREARM',side+'HAND',goal))
            foot=arm.pose.bones[side+'FOOT'].head.copy()
            toe=arm.pose.bones[side+'TOEBASE'].matrix.copy()
            goal=foot.lerp(markers[label+' footwell'].location,weight)
            if .30<abs(goal.x)<.90 and -.35<goal.y<.9:
                goal.z+=.65*math.sin(math.pi*(abs(goal.x)-.30)/.60)*(1-weight)
            if (goal-foot).length>1e-6:
                solve(side+'UPLEG',side+'LEG',side+'FOOT',goal)
                toe.translation+=arm.pose.bones[side+'FOOT'].head-foot
                put(side+'TOEBASE',toe)
        if weight>.99:errors.extend(frame_errors)
        for name in body:
            b=arm.pose.bones[name]
            assert all(math.isfinite(v) for row in b.matrix for v in row)
            for path in ['location','rotation_quaternion','scale']:b.keyframe_insert(data_path=path,frame=f)
    assert all(b.matrix_local==rests[b.name] for b in arm.data.bones)
    assert not any(b.constraints for b in arm.pose.bones)
    camera=scene.camera;camera.location=(4,-5,3);aim(camera,(.55,0,.95));camera.data.ortho_scale=5
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type=='VIEW_3D':
                area.spaces.active.region_3d.view_location=Vector((.5,0,.9))
                area.spaces.active.region_3d.view_distance=6
                area.spaces.active.region_3d.view_rotation=camera.rotation_euler.to_quaternion()
    for o in scene.objects:
        if o.type=='LIGHT':o.location+=Vector((.7,1,0))
    text=bpy.data.texts.new('READ ME — calibrated kart fit')
    text.write('Fitted from the calibrated Skate-character animation.\nRaw action retained. Bind bones unchanged.\nSeat alignment, torso lean and analytical limb contact baked into a separate action.\nNo game files changed. Space plays/pauses.\nTarget markers document grip/footwell positions; rerun this tool to rebake changes.\n')
    scene.timeline_markers.new('Seated / contact',frame=seated)
    scene.frame_set(first)
    bpy.ops.object.select_all(action='DESELECT');arm.select_set(True);bpy.context.view_layer.objects.active=arm
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str((args.output/('Kart-'+args.mode+'.blend')).resolve()))
    for f in sorted({first,first+int((last-first)*(.5 if args.mode=='enter' else .4)),seated,last}):
        scene.frame_set(f);scene.render.filepath=str((args.output/f'frame-{f}.png').resolve());bpy.ops.render.render(write_still=True)
    (args.output/'fit-report.json').write_text(json.dumps({'mode':args.mode,'source':str(args.raw),'rest_bones_unchanged':True,'max_seated_wrist_target_error_m':max(errors,default=0),'seat':list(seat)},indent=2))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--raw',type=Path,required=True);p.add_argument('--kart',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--mode',choices=['enter','exit'],required=True)
    fit(p.parse_args(sys.argv[sys.argv.index('--')+1:]))
