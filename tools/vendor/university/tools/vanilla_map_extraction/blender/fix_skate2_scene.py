"""Repair Skate 2's mixed-coordinate grind rails in an existing .blend."""

from __future__ import annotations

import json
from pathlib import Path
import sys

import bpy


BLENDER_DIRECTORY = Path(__file__).resolve().parent
TOOLS_DIRECTORY = BLENDER_DIRECTORY.parent / "tools"
sys.path.insert(0, str(BLENDER_DIRECTORY))
sys.path.insert(0, str(TOOLS_DIRECTORY))

from import_hawaiian_dream import _retail_grind_controls  # noqa: E402
from retail_grind_splines import (  # noqa: E402
    classify_grind_coordinate_frame,
    grind_cell_translation,
    translate_native_segment_payload,
)


SIGN_35_ASSET_ID = "0xd4538d2e1da5434f"
EXPECTED_RAIL_COUNT = 36_058


def _payloads(obj: bpy.types.Object) -> list[str]:
    segment_count = int(obj["skate3_retail_grind_segment_count"])
    payload_hex = str(obj["skate3_retail_grind_segment_payload"])
    segment_hex_size = 120 * 2
    if len(payload_hex) != segment_count * segment_hex_size:
        raise ValueError(f"{obj.name} has an invalid native payload size")
    return [
        payload_hex[offset : offset + segment_hex_size]
        for offset in range(0, len(payload_hex), segment_hex_size)
    ]


def _set_curve_from_payloads(
    obj: bpy.types.Object,
    payloads: list[str],
) -> None:
    if obj.type != "CURVE" or len(obj.data.splines) != 1:
        raise ValueError(f"{obj.name} is not a single-spline curve")
    spline = obj.data.splines[0]
    if spline.type != "BEZIER":
        raise ValueError(f"{obj.name} is not a Bezier curve")
    expected_segments = len(payloads)
    actual_segments = (
        len(spline.bezier_points)
        if spline.use_cyclic_u
        else len(spline.bezier_points) - 1
    )
    if actual_segments != expected_segments:
        raise ValueError(
            f"{obj.name} has {actual_segments} curve segments but "
            f"{expected_segments} native segments"
        )

    point_count = len(spline.bezier_points)
    for point in spline.bezier_points:
        point.handle_left_type = "FREE"
        point.handle_right_type = "FREE"
    for segment_index, payload in enumerate(payloads):
        point_0, point_1, point_2, point_3 = _retail_grind_controls(payload)
        current = spline.bezier_points[segment_index]
        following_index = (segment_index + 1) % point_count
        following = spline.bezier_points[following_index]
        current.co = point_0
        current.handle_right = point_1
        following.handle_left = point_2
        if not spline.use_cyclic_u or following_index != 0:
            following.co = point_3
    if not spline.use_cyclic_u:
        spline.bezier_points[0].handle_left = spline.bezier_points[0].co
        spline.bezier_points[-1].handle_right = (
            spline.bezier_points[-1].co
        )


def _repair_grinds() -> dict[str, int]:
    rails = [
        obj
        for obj in bpy.data.objects
        if bool(obj.get("skate3_retail_grind", False))
    ]
    if len(rails) != EXPECTED_RAIL_COUNT:
        raise RuntimeError(
            f"expected {EXPECTED_RAIL_COUNT} Skate 2 rails, found {len(rails)}"
        )

    counts = {"cell_local": 0, "world_space": 0, "ambiguous": 0}
    for rail_index, obj in enumerate(rails, 1):
        source_frame = obj.get(
            "skate3_retail_grind_source_coordinate_frame"
        )
        if source_frame is not None:
            raise RuntimeError(
                f"{obj.name} is already marked as coordinate-repaired"
            )
        source_payloads = _payloads(obj)
        source_frame = classify_grind_coordinate_frame(
            str(obj["skate3_stream_file"]),
            source_payloads,
            margin=40.0,
        )
        counts[source_frame] += 1
        output_payloads = source_payloads
        translation = None
        if source_frame == "cell_local":
            translation = grind_cell_translation(
                str(obj["skate3_stream_file"])
            )
            if translation is None:
                raise RuntimeError(
                    f"{obj.name} is cell-local without a cell translation"
                )
            output_payloads = [
                translate_native_segment_payload(payload, translation)
                for payload in source_payloads
            ]

        obj.location = (0.0, 0.0, 0.0)
        obj.rotation_euler = (0.0, 0.0, 0.0)
        obj.scale = (1.0, 1.0, 1.0)
        _set_curve_from_payloads(obj, output_payloads)
        obj["skate3_retail_grind_segment_payload"] = "".join(
            output_payloads
        )
        obj["skate3_retail_grind_source_coordinate_frame"] = source_frame
        if translation is not None:
            obj["skate3_retail_grind_cell_translation"] = translation
        if rail_index % 2_000 == 0:
            print(
                f"Repaired {rail_index}/{len(rails)} grind rails",
                flush=True,
            )
    return counts


def _remove_unplaced_sign() -> int:
    objects = [
        obj
        for obj in bpy.data.objects
        if str(obj.get("skate3_asset_id", "")).lower() == SIGN_35_ASSET_ID
        and not bool(obj.get("skate3_retail_grind", False))
    ]
    for obj in objects:
        bpy.data.objects.remove(obj, do_unlink=True)
    for collection in list(bpy.data.collections):
        if collection.name.upper() == "MODEL_D4538D2E1DA5434F":
            bpy.data.collections.remove(collection)
    return len(objects)


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 1:
        raise SystemExit(
            "usage: blender INPUT.blend --background --python "
            "fix_skate2_scene.py -- OUTPUT.blend"
        )
    output = Path(arguments[0]).resolve()
    scene = bpy.context.scene
    if "Skate 2" not in str(scene.get("skate3_map_name", scene.name)):
        raise RuntimeError("the open scene is not the Skate 2 BAM import")

    counts = _repair_grinds()
    removed_sign_objects = _remove_unplaced_sign()
    if removed_sign_objects != 1:
        raise RuntimeError(
            f"expected one Sign_35 object, removed {removed_sign_objects}"
        )

    scene["skate3_grind_status"] = (
        f"{EXPECTED_RAIL_COUNT} exact retail grind rails; "
        f"{counts['cell_local']} cell-local rails baked into world space; "
        f"{counts['ambiguous']} central boundary rails preserved unchanged"
    )
    scene["skate3_excluded_static_model_status"] = (
        "1 unplaced Sign_35 prop mesh excluded by Skate 2 import policy"
    )
    scene["skate3_skate2_coordinate_repair"] = json.dumps(
        counts,
        sort_keys=True,
    )
    bpy.ops.wm.save_as_mainfile(filepath=str(output), check_existing=False)
    print(
        json.dumps(
            {
                "output": str(output),
                "grind_coordinate_frames": counts,
                "removed_sign_objects": removed_sign_objects,
            },
            indent=2,
        ),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
