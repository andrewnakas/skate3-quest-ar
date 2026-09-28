"""Prepare a private Skate 3 RX2/ABIN Blender scene for Bevy glTF import.

This script is deliberately project-local and does not redistribute source
game data. It joins the eight compatible skinned RX2 parts, removes baked
horizontal travel in the board's frame from the complete rider/board pair,
and exports every selected retail action as a named glTF animation.
"""

from pathlib import Path
import json
import sys

import bpy
from mathutils import Vector


HIPS = "HIPS"
BOARD = "SKATEBOARD_ROOT"
TOE_TARGETS = (
    ("RIGHTTOEBASE", "RIGHTTOEBASE_REPARENTED"),
    ("LEFTTOEBASE", "LEFTTOEBASE_REPARENTED"),
)


def parse_args() -> tuple[Path, Path, Path]:
    if "--" not in sys.argv:
        raise RuntimeError(
            "Expected output GLB, manifest, and root-motion paths after --"
        )
    tail = sys.argv[sys.argv.index("--") + 1 :]
    if len(tail) != 3:
        raise RuntimeError(
            "Usage: blender source.blend --python export_bevy_glb.py "
            "-- output.glb manifest.txt root_motion.json"
        )
    return (
        Path(tail[0]).resolve(),
        Path(tail[1]).resolve(),
        Path(tail[2]).resolve(),
    )


def iter_fcurves(action):
    """Yield f-curves from Blender 4+/5 layered actions."""
    for layer in action.layers:
        for strip in layer.strips:
            for channelbag in strip.channelbags:
                yield from channelbag.fcurves


def location_curves(action, bone_name):
    path = f'pose.bones["{bone_name}"].location'
    return {
        fcurve.array_index: fcurve
        for fcurve in iter_fcurves(action)
        if fcurve.data_path == path
    }


def set_curve_value(fcurve, frame, value):
    for point in fcurve.keyframe_points:
        if abs(point.co.x - frame) <= 1.0e-4:
            difference = value - point.co.y
            point.co.y = value
            point.handle_left.y += difference
            point.handle_right.y += difference
            return
    fcurve.keyframe_points.insert(frame, value)


def bake_root_space_in_place(armature) -> tuple[int, int, float, float, dict]:
    """Remove shared travel without erasing body-to-board registration.

    HIPS and SKATEBOARD_ROOT use different local bases, so independently
    zeroing selected location channels is not a valid in-place conversion.
    For every source frame, translate both evaluated roots by one shared
    horizontal displacement, then write the resulting local-basis locations
    back to their own curves. OnBoard actions anchor to SKATEBOARD_ROOT;
    OffBoard BR/NB actions anchor to HIPS so the carried board retains its
    authored relationship to the skater.
    """

    armature.animation_data_create()
    tracks = list(armature.animation_data.nla_tracks)
    track_mutes = [track.mute for track in tracks]
    for track in tracks:
        track.mute = True

    changed_curves = 0
    changed_keys = 0
    maximum_anchor_travel = 0.0
    maximum_target_vector_error = 0.0
    root_motion = {}
    try:
        for action in bpy.data.actions:
            hips_curves = location_curves(action, HIPS)
            board_curves = location_curves(action, BOARD)
            if len(hips_curves) != 3 or len(board_curves) != 3:
                continue

            armature.animation_data.action = action
            start = int(action.frame_range[0])
            end = int(action.frame_range[1])
            offboard = action.name.startswith(("BR_", "NB_"))
            anchor_name = HIPS if offboard else BOARD
            samples = {}
            source_anchor_samples = []
            source_target_vectors = {}
            for frame in range(start, end + 1):
                bpy.context.scene.frame_set(frame)
                bpy.context.view_layer.update()
                hips = armature.pose.bones[HIPS]
                board = armature.pose.bones[BOARD]
                anchor = armature.pose.bones[anchor_name]
                source_anchor_samples.append(
                    {
                        "frame": frame,
                        "position_blender": [
                            float(anchor.matrix.translation.x),
                            float(anchor.matrix.translation.y),
                            float(anchor.matrix.translation.z),
                        ],
                    }
                )

                source_target_vectors[frame] = tuple(
                    (
                        armature.pose.bones[toe_name].matrix.translation
                        - armature.pose.bones[target_name].matrix.translation
                    ).copy()
                    for toe_name, target_name in TOE_TARGETS
                )

                horizontal = Vector(
                    (anchor.matrix.translation.x, anchor.matrix.translation.y, 0.0)
                )
                desired_board = board.matrix.copy()
                desired_board.translation -= horizontal
                desired_hips = hips.matrix.copy()
                desired_hips.translation -= horizontal

                # Assignment through pose matrices lets Blender perform each
                # root's distinct parent/rest-basis conversion.
                board.matrix = desired_board
                hips.matrix = desired_hips
                bpy.context.view_layer.update()
                samples[frame] = (
                    tuple(float(value) for value in hips.location),
                    tuple(float(value) for value in board.location),
                )

            origin = source_anchor_samples[0]["position_blender"]
            for sample in source_anchor_samples:
                position = sample["position_blender"]
                # Blender's source rig is Z-up. glTF/Bevy is Y-up and the
                # exporter maps source -Y to Bevy +Z.
                sample["delta_bevy"] = [
                    position[0] - origin[0],
                    position[2] - origin[2],
                    -(position[1] - origin[1]),
                ]
                del sample["position_blender"]
            root_motion[action.name] = {
                "anchor": anchor_name,
                "frame_start": start,
                "frame_end": end,
                "sample_rate_hz": 60.0,
                "samples": source_anchor_samples,
            }

            for frame, (hips_location, board_location) in samples.items():
                for axis, value in enumerate(hips_location):
                    set_curve_value(hips_curves[axis], frame, value)
                    changed_keys += 1
                for axis, value in enumerate(board_location):
                    set_curve_value(board_curves[axis], frame, value)
                    changed_keys += 1
            changed_curves += 6

            for frame in range(start, end + 1):
                bpy.context.scene.frame_set(frame)
                bpy.context.view_layer.update()
                anchor_position = armature.pose.bones[anchor_name].matrix.translation
                maximum_anchor_travel = max(
                    maximum_anchor_travel,
                    abs(anchor_position.x),
                    abs(anchor_position.y),
                )
                for source_vector, (toe_name, target_name) in zip(
                    source_target_vectors[frame], TOE_TARGETS
                ):
                    output_vector = (
                        armature.pose.bones[toe_name].matrix.translation
                        - armature.pose.bones[target_name].matrix.translation
                    )
                    maximum_target_vector_error = max(
                        maximum_target_vector_error,
                        (output_vector - source_vector).length,
                    )
    finally:
        armature.animation_data.action = None
        for track, mute in zip(tracks, track_mutes):
            track.mute = mute

    return (
        changed_curves,
        changed_keys,
        maximum_anchor_travel,
        maximum_target_vector_error,
        root_motion,
    )


def join_skinned_meshes() -> tuple[list[str], list[str]]:
    meshes = sorted(
        (obj for obj in bpy.data.objects if obj.type == "MESH"),
        key=lambda obj: obj.name,
    )
    source_names = [obj.name for obj in meshes]
    if len(meshes) > 1:
        bpy.ops.object.select_all(action="DESELECT")
        for mesh in meshes:
            mesh.select_set(True)
        bpy.context.view_layer.objects.active = meshes[0]
        bpy.ops.object.join()
        meshes[0].name = "Skate3_SkaterAndBoard"
    return source_names, sorted(
        obj.name for obj in bpy.data.objects if obj.type == "MESH"
    )


def main() -> None:
    output, manifest, root_motion_path = parse_args()
    output.parent.mkdir(parents=True, exist_ok=True)
    manifest.parent.mkdir(parents=True, exist_ok=True)
    root_motion_path.parent.mkdir(parents=True, exist_ok=True)

    armature_objects = [obj for obj in bpy.data.objects if obj.type == "ARMATURE"]
    if len(armature_objects) != 1:
        raise RuntimeError(
            f"Expected exactly one armature, found {len(armature_objects)}"
        )
    (
        changed_curves,
        changed_keys,
        maximum_anchor_travel,
        maximum_target_vector_error,
        root_motion,
    ) = bake_root_space_in_place(armature_objects[0])
    if maximum_anchor_travel > 5.0e-5:
        raise RuntimeError(
            "Root-space in-place bake left horizontal anchor travel: "
            f"{maximum_anchor_travel:.9g}"
        )
    if maximum_target_vector_error > 5.0e-5:
        raise RuntimeError(
            "Board-space in-place bake changed toe-target registration: "
            f"{maximum_target_vector_error:.9g}"
        )
    source_meshes, meshes = join_skinned_meshes()
    actions = sorted(action.name for action in bpy.data.actions)
    armatures = sorted(
        obj.name for obj in bpy.data.objects if obj.type == "ARMATURE"
    )

    bpy.ops.export_scene.gltf(
        filepath=str(output),
        export_format="GLB",
        export_animations=True,
        export_skins=True,
        export_morph=True,
        export_apply=False,
        export_yup=True,
    )

    root_motion_path.write_text(
        json.dumps(
            {
                "schema": 1,
                "coordinate_system": "bevy_y_up_forward_positive_z",
                "actions": root_motion,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    manifest.write_text(
        "\n".join(
            [
                f"output={output.name}",
                f"root_motion={root_motion_path.name}",
                f"actions={len(actions)}",
                f"armatures={len(armatures)}",
                f"meshes={len(meshes)}",
                f"source_meshes={len(source_meshes)}",
                f"root_motion_curves_locked={changed_curves}",
                f"root_motion_keys_locked={changed_keys}",
                f"maximum_anchor_horizontal_travel={maximum_anchor_travel:.9g}",
                (
                    "maximum_toe_target_vector_error="
                    f"{maximum_target_vector_error:.9g}"
                ),
                *[f"action={name}" for name in actions],
                *[f"armature={name}" for name in armatures],
                *[f"source_mesh={name}" for name in source_meshes],
                *[f"mesh={name}" for name in meshes],
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    print(
        "SKATE3_BEVY_PRIVATE_EXPORT "
        f"output={output} actions={len(actions)} meshes={len(meshes)} "
        f"locked_curves={changed_curves} locked_keys={changed_keys} "
        f"anchor_travel={maximum_anchor_travel:.9g} "
        f"toe_target_error={maximum_target_vector_error:.9g}"
    )


main()
