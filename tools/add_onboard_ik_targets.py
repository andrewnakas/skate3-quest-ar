"""Add Skate 3's four compact OnBoard SkeletonIK targets to the Bevy rig.

The RX2 mesh exporter intentionally creates only deform and board bones. The
retail compact OnBoard hierarchy continues with toe channels 32/33 and hand
channels 34/35, which are board-parented SkeletonIK targets. This post-process
decodes those channels from every selected retail action and bakes them as
non-deforming glTF bones.
"""

from __future__ import annotations

import math
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector
import numpy as np


TARGETS = {
    32: "RIGHTTOEBASE_REPARENTED",
    33: "LEFTTOEBASE_REPARENTED",
    34: "LEFTHAND_REPARENTED",
    35: "RIGHTHAND_REPARENTED",
}
BOARD = "SKATEBOARD_ROOT"
SOURCE_TO_BLENDER = Matrix(
    (
        (1.0, 0.0, 0.0, 0.0),
        (0.0, 0.0, -1.0, 0.0),
        (0.0, 1.0, 0.0, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    )
)
BLENDER_TO_SOURCE = SOURCE_TO_BLENDER.inverted()


def parse_args() -> tuple[Path, Path]:
    if "--" not in sys.argv:
        raise RuntimeError("Expected ABIN and importer-directory paths after --")
    tail = sys.argv[sys.argv.index("--") + 1 :]
    if len(tail) != 2:
        raise RuntimeError(
            "Usage: blender source.blend --python add_onboard_ik_targets.py "
            "-- OnBoard.abin sk8anims"
        )
    return Path(tail[0]).resolve(), Path(tail[1]).resolve()


def sqt_matrix(sqt) -> np.ndarray:
    x, y, z, w = (float(value) for value in sqt.quat)
    sx, sy, sz = (float(value) for value in sqt.scale)
    norm = math.sqrt(x * x + y * y + z * z + w * w)
    if norm < 1.0e-12:
        x = y = z = 0.0
        w = 1.0
    else:
        x, y, z, w = (x / norm, y / norm, z / norm, w / norm)
    xx, yy, zz = x * x, y * y, z * z
    xy, xz, yz = x * y, x * z, y * z
    wx, wy, wz = w * x, w * y, w * z
    result = np.identity(4, dtype=np.float64)
    result[0, :3] = (
        (1.0 - 2.0 * (yy + zz)) * sx,
        (2.0 * (xy + wz)) * sx,
        (2.0 * (xz - wy)) * sx,
    )
    result[1, :3] = (
        (2.0 * (xy - wz)) * sy,
        (1.0 - 2.0 * (xx + zz)) * sy,
        (2.0 * (yz + wx)) * sy,
    )
    result[2, :3] = (
        (2.0 * (xz + wy)) * sz,
        (2.0 * (yz - wx)) * sz,
        (1.0 - 2.0 * (xx + yy)) * sz,
    )
    result[3, :3] = tuple(float(value) for value in sqt.trans)
    return result


def source_row_to_blender(matrix: np.ndarray) -> Matrix:
    source_column = Matrix(np.asarray(matrix, dtype=np.float64).T.tolist())
    return SOURCE_TO_BLENDER @ source_column @ BLENDER_TO_SOURCE


def compose_world(frame, reference, parents) -> dict[int, np.ndarray]:
    world = {}
    for index, parent in enumerate(parents):
        delta = sqt_matrix(frame[index]) if index in frame else np.identity(4)
        rest = (
            sqt_matrix(reference[index])
            if index in reference
            else np.identity(4)
        )
        local = delta @ rest
        world[index] = local @ world[parent] if parent >= 0 else local
    return world


def add_target_bones(armature, reference, parents) -> None:
    rest_world = compose_world({}, reference, parents)
    bpy.ops.object.select_all(action="DESELECT")
    armature.select_set(True)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    board = armature.data.edit_bones[BOARD]
    for index, name in TARGETS.items():
        if name in armature.data.edit_bones:
            continue
        matrix = source_row_to_blender(rest_world[index])
        rotation = matrix.to_3x3().normalized()
        y_axis = (rotation @ Vector((0.0, 1.0, 0.0))).normalized()
        z_axis = (rotation @ Vector((0.0, 0.0, 1.0))).normalized()
        bone = armature.data.edit_bones.new(name)
        bone.head = matrix.translation
        bone.tail = bone.head + y_axis * 0.08
        bone.align_roll(z_axis)
        bone.parent = board
        bone.use_connect = False
        bone.use_deform = False
    bpy.ops.object.mode_set(mode="OBJECT")


def bake_targets(armature, data, abin, reference, parents) -> tuple[int, float]:
    clips = {clip.header.name: (index, clip) for index, clip in enumerate(abin.clips)}
    tracks = list(armature.animation_data.nla_tracks)
    track_mutes = [track.mute for track in tracks]
    for track in tracks:
        track.mute = True

    baked_keys = 0
    maximum_error = 0.0
    try:
        for action in bpy.data.actions:
            match = clips.get(action.name)
            if match is None:
                continue
            clip_index, clip = match
            armature.animation_data.action = action
            decoders = {}
            for frame_index in range(clip.num_frames):
                scene_frame = frame_index + 1
                bpy.context.scene.frame_set(scene_frame)
                decoded = __import__("abin_importer").decode_clip_frame(
                    data,
                    clip,
                    decoders,
                    frame_index,
                    abin.hierarchy,
                )
                source_world = compose_world(decoded, reference, parents)
                board_world = armature.pose.bones[BOARD].matrix.copy()
                board_rest = armature.data.bones[BOARD].matrix_local

                expected = {}
                for index, name in TARGETS.items():
                    target_world = source_row_to_blender(source_world[index])
                    target_rest = armature.data.bones[name].matrix_local
                    local_rest = board_rest.inverted() @ target_rest
                    pose_bone = armature.pose.bones[name]
                    pose_bone.rotation_mode = "QUATERNION"
                    pose_bone.matrix_basis = (
                        local_rest.inverted()
                        @ board_world.inverted()
                        @ target_world
                    )
                    expected[name] = target_world

                for name in TARGETS.values():
                    pose_bone = armature.pose.bones[name]
                    pose_bone.keyframe_insert(
                        data_path="location", frame=scene_frame, group=name
                    )
                    pose_bone.keyframe_insert(
                        data_path="rotation_quaternion",
                        frame=scene_frame,
                        group=name,
                    )
                    pose_bone.keyframe_insert(
                        data_path="scale", frame=scene_frame, group=name
                    )
                    baked_keys += 10

                bpy.context.view_layer.update()
                for name, target_world in expected.items():
                    actual = armature.pose.bones[name].matrix
                    maximum_error = max(
                        maximum_error,
                        max(
                            abs(actual[row][column] - target_world[row][column])
                            for row in range(4)
                            for column in range(4)
                        ),
                    )
    finally:
        armature.animation_data.action = None
        for track, mute in zip(tracks, track_mutes):
            track.mute = mute
    return baked_keys, maximum_error


def main() -> None:
    abin_path, importer_dir = parse_args()
    sys.path.insert(0, str(importer_dir))
    import abin_importer as abin_importer

    data = abin_path.read_bytes()
    abin = abin_importer.AbinFile(data)
    if abin.hierarchy is None:
        raise RuntimeError("OnBoard ABIN does not contain a hierarchy")
    poses = [pose for pose in abin.poses if pose.header.name == "RIG_TPOSE"]
    if not poses:
        raise RuntimeError("OnBoard ABIN does not contain RIG_TPOSE")
    # Match AnimSource._load_ref_pose, which supplies the reference used by
    # the already-baked deform/board actions in this same Blender file.
    reference = abin_importer.decode_pose_frame(data, poses[0], abin.hierarchy)
    parents = list(abin.hierarchy.parents)
    if len(parents) <= max(TARGETS):
        raise RuntimeError("OnBoard hierarchy does not contain target channels 32-35")

    armature = next(obj for obj in bpy.data.objects if obj.type == "ARMATURE")
    armature.animation_data_create()
    add_target_bones(armature, reference, parents)
    key_count, maximum_error = bake_targets(
        armature, data, abin, reference, parents
    )
    if maximum_error > 5.0e-5:
        raise RuntimeError(
            f"SkeletonIK target bake exceeded tolerance: {maximum_error:.9g}"
        )
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    print(
        "SKATE3_ONBOARD_IK_TARGETS "
        f"targets=4 keys={key_count} max_error={maximum_error:.9g}"
    )


main()
