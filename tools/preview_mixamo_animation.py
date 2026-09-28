"""Blender-only raw Mixamo -> stock animation review. No vehicle or runtime edits.

blender -b --python tools/preview_mixamo_animation.py -- --source clip.fbx
    --reference skater.glb --output new-preview-directory
"""
import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Quaternion, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent / 'mixamo_to_skate'))
from converter import MAP, sha


def aim(obj, point):
    obj.rotation_euler = (Vector(point) - obj.location).to_track_quat('-Z', 'Y').to_euler()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--reference', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--profile', type=Path, help='Paired-stock calibration JSON')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    args.output.mkdir(parents=True, exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.unit_settings.system = 'METRIC'
    scene.render.fps = 30
    bpy.ops.import_scene.fbx(filepath=str(args.source.resolve()))
    source_objects = set(scene.objects)
    source = next(o for o in source_objects if o.type == 'ARMATURE')
    source_names = {b.name.rsplit(':', 1)[-1].lower(): b.name for b in source.data.bones}
    source_action = source.animation_data.action
    start, end = map(int, source_action.frame_range)
    source_action.name = 'Original Mixamo — unchanged'
    source_action.use_fake_user = True
    bpy.ops.import_scene.gltf(filepath=str(args.reference.resolve()))
    target_objects = set(scene.objects) - source_objects
    target = next(o for o in target_objects if o.type == 'ARMATURE')
    target.name = 'Skate character — raw animation transfer'
    meshes = [o for o in target_objects if o.type == 'MESH' and
              any(m.type == 'ARMATURE' and m.object == target for m in o.modifiers)]
    # Remove only the board from the review copy of the combined character mesh.
    for obj in meshes:
        body = {g.index for g in obj.vertex_groups if g.name in MAP.values()}
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        deform = bm.verts.layers.deform.active
        if deform:
            bmesh.ops.delete(bm, geom=[v for v in bm.verts if
                sum(w for g, w in v[deform].items() if g in body) < .01], context='VERTS')
            bm.to_mesh(obj.data)
        bm.free()
    for obj in target_objects:
        if obj.type == 'MESH' and obj not in meshes:
            obj.hide_render = True
            obj.hide_set(True)
    rests = {b.name: b.matrix_local.copy() for b in target.data.bones}
    source_rests = {key: source.matrix_world @ source.data.bones[name].matrix_local
                    for key, name in source_names.items()}
    def position(key):
        return source_rests[key].translation
    def length(bones, positions):
        return sum((positions(b)-positions(a)).length for a, b in zip(bones, bones[1:]))
    leg = ['leftupleg', 'leftleg', 'leftfoot']
    scale = length(leg, lambda k: rests[MAP[k]].translation) / length(leg, position)
    child = {'hips': 'spine', 'spine': 'spine1', 'spine1': 'spine2',
             'spine2': 'neck', 'neck': 'head'}
    for side in ['left', 'right']:
        for a, b in [('shoulder', 'arm'), ('arm', 'forearm'), ('forearm', 'hand'),
                     ('upleg', 'leg'), ('leg', 'foot')]:
            child[side+a] = side+b
    corrections, alignments = {}, {}
    for key, name in MAP.items():
        if key not in source_rests:
            continue
        align = Quaternion()
        if key in child:
            next_key = child[key]
            target_direction = rests[MAP[next_key]].translation - rests[name].translation
            source_direction = position(next_key) - position(key)
            align = target_direction.normalized().rotation_difference(source_direction.normalized())
        if key.endswith('hand'):
            align = alignments[key.replace('hand', 'forearm')]
        alignments[key] = align
        corrections[name] = source_rests[key].to_quaternion().inverted() @ align @ rests[name].to_quaternion()
    # Feet have different anatomical landmark offsets. Calibrate their rigid
    # orientation from the source's standing endpoint, without changing poses.
    neutral = start if 'enter' in args.source.stem.lower() else end
    scene.frame_set(neutral)
    bpy.context.view_layer.update()
    hips_q = (source.matrix_world @ source.pose.bones[source_names['hips']].matrix).to_quaternion()
    facing = hips_q @ corrections['HIPS'] @ rests['HIPS'].to_quaternion().inverted() @ Vector((0, -1, 0))
    yaw = Quaternion(Vector((0, 0, 1)), math.atan2(facing.x, -facing.y))
    for key, name in MAP.items():
        if key in source_names and (key.endswith('foot') or key.endswith('toebase')):
            world = source.matrix_world @ source.pose.bones[source_names[key]].matrix
            corrections[name] = world.to_quaternion().inverted() @ yaw @ rests[name].to_quaternion()
    reverse = {v: k for k, v in MAP.items() if k in source_names}
    calibrated = {}
    profile_errors = {}
    if args.profile:
        profile = json.loads(args.profile.read_text(encoding='utf-8'))
        if profile['reference_sha256'] != sha(args.reference):
            raise ValueError('Calibration does not match the reference character')
        # FBX and glTF importer world axes agree after glTF Y-up -> Blender Z-up.
        basis = Matrix(((1, 0, 0, 0), (0, 0, -1, 0), (0, 1, 0, 0), (0, 0, 0, 1)))
        paired = {k: basis @ Matrix(v) for k, v in profile['bones'].items()}
        scale = profile['normalization_scale']
        offset = paired['hips'].translation - position('hips') * scale
        for name, key in reverse.items():
            reference = paired[key]
            angle = source_rests[key].to_quaternion().rotation_difference(reference.to_quaternion()).angle
            error = (position(key)*scale+offset-reference.translation).length
            profile_errors[key] = {'angle_radians': angle, 'position_metres': error}
            if angle > .02 or error > .02:
                raise ValueError('Source bind rig differs from paired calibration: '+key)
            rigid = Matrix.LocRotScale(reference.translation, reference.to_quaternion(), Vector((1, 1, 1)))
            calibrated[name] = rigid.inverted() @ rests[name]
    target.animation_data_clear()
    for bone in target.pose.bones:
        bone.rotation_mode = 'QUATERNION'
    body_names = set(MAP.values()) | {'SPINE2', 'NECK1'}
    ordered = []
    def visit(bone):
        if bone.name in body_names:
            ordered.append(bone.name)
        for c in bone.children:
            visit(c)
    visit(target.data.bones['HIPS'])
    for frame in range(start, end + 1):
        scene.frame_set(frame)
        bpy.context.view_layer.update()
        for name in ordered:
            bone = target.pose.bones[name]
            if bone.parent:
                local_rest = rests[bone.parent.name].inverted() @ rests[name]
                inherited = bone.parent.matrix @ local_rest
            else:
                inherited = rests[name].copy()
            if name in reverse:
                key = reverse[name]
                world = source.matrix_world @ source.pose.bones[source_names[key]].matrix
                rotation = world.to_quaternion() @ corrections[name]
                location = inherited.translation
                if name == 'HIPS':
                    location = rests['HIPS'].translation + (world.translation-position('hips')) * scale
                if calibrated:
                    # Transfer the measured source deformation to the matching
                    # native bind frame; no anatomical aim or standing-pose overrides.
                    rigid = Matrix.LocRotScale(world.translation*scale+offset,
                                               world.to_quaternion(), Vector((1, 1, 1)))
                    bone.matrix = rigid @ calibrated[name]
                else:
                    bone.matrix = Matrix.LocRotScale(location, rotation, Vector((1, 1, 1)))
            else:
                # Extra stock spine/neck joints keep their actual local rest transform.
                bone.matrix = inherited
            bpy.context.view_layer.update()
            bone.keyframe_insert(data_path='location', frame=frame)
            bone.keyframe_insert(data_path='rotation_quaternion', frame=frame)
            bone.keyframe_insert(data_path='scale', frame=frame)
    target.animation_data.action.name = args.source.stem + ' — raw Skate animation'
    # Authoring invariants: no altered bind bones, IK, contact targets or kart.
    assert all(rests[b.name] == b.matrix_local for b in target.data.bones)
    assert not any(b.constraints for b in target.pose.bones)
    report = {'source': str(args.source), 'reference': str(args.reference),
              'frames': [start, end], 'fps': scene.render.fps, 'motion_scale': scale,
              'mapping': MAP, 'rest_bones_unchanged': True, 'constraints': 0,
              'vehicle_adjustments': False, 'profile': str(args.profile) if args.profile else None,
              'profile_errors': profile_errors, 'samples': {}}
    samples = sorted({start, start+(end-start)//3, start+2*(end-start)//3, end})
    positions = []
    for frame in range(start, end+1):
        scene.frame_set(frame)
        bpy.context.view_layer.update()
        for name in ordered:
            matrix = target.pose.bones[name].matrix
            assert all(math.isfinite(x) for row in matrix for x in row)
        positions.append(target.pose.bones['HIPS'].head.copy())
        if frame in samples:
            report['samples'][frame] = {name: list(target.pose.bones[name].head)
                                        for name in ['HIPS', 'HEAD', 'LEFTHAND', 'RIGHTHAND', 'LEFTFOOT', 'RIGHTFOOT']}
    for obj in source_objects:
        obj.hide_render = True
        obj.hide_set(True)
    center = sum(positions, Vector()) / len(positions)
    center.z = .9
    bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, -.01))
    floor = bpy.context.object
    floor.name = 'Preview floor — visual reference only'
    mat = bpy.data.materials.new('Studio gray')
    mat.diffuse_color = (.12, .14, .18, 1)
    floor.data.materials.append(mat)
    scene.world = bpy.data.worlds.new('Studio')
    scene.world.color = (.3, .3, .3)
    for offset, power in [((3, -4, 6), 1200), ((-4, 0, 4), 1000)]:
        bpy.ops.object.light_add(type='AREA', location=center+Vector(offset))
        light = bpy.context.object
        light.data.energy = power
        light.data.size = 5
        aim(light, center)
    bpy.ops.object.camera_add(location=center+Vector((4, -5, 2.5)))
    camera = bpy.context.object
    aim(camera, center)
    camera.data.type = 'ORTHO'
    camera.data.ortho_scale = max(4, max((p-center).length for p in positions)*2+2)
    scene.camera = camera
    scene.frame_start, scene.frame_end = start, end
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = 12
    scene.render.resolution_x, scene.render.resolution_y = 900, 700
    scene.render.resolution_percentage = 100
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == 'VIEW_3D':
                space = area.spaces.active
                space.region_3d.view_location = center
                space.region_3d.view_distance = camera.data.ortho_scale*1.3
                space.region_3d.view_rotation = camera.rotation_euler.to_quaternion()
                space.shading.type = 'MATERIAL'
                space.overlay.show_overlays = False
    notes = bpy.data.texts.new('READ ME — raw animation review')
    notes.write('Raw animation transfer onto the native Skate character.\n'
                'No kart, IK, seat alignment, grip edits or animation cleanup.\n'
                'Original source action and character retained hidden.\n'
                'Native rest bones unchanged. Space plays/pauses.\n'
                + ('Uses matching paired-stock calibration; no anatomical aim overrides.\n' if calibrated else 'Root travel is scaled by leg-length ratio; no ground snapping.\n'))
    bpy.ops.object.select_all(action='DESELECT')
    target.select_set(True)
    bpy.context.view_layer.objects.active = target
    scene.frame_set(start)
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str((args.output/'Raw-Skate-Animation.blend').resolve()))
    for frame in samples:
        scene.frame_set(frame)
        scene.render.filepath = str((args.output/f'frame-{frame}.png').resolve())
        bpy.ops.render.render(write_still=True)
    (args.output/'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
