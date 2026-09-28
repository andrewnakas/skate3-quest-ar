"""Validate the repaired Skate 2 Blender scene after a fresh file reopen."""

from __future__ import annotations

import json
import math
from pathlib import Path
import sys

import bpy


SOURCE_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(SOURCE_ROOT / "tools" / "blender_owned_map"))

from owned_world_material_addon.exporter import _export_retail_grind  # noqa: E402


EXPECTED_RAIL_COUNT = 36_058
EXPECTED_CELL_LOCAL_COUNT = 10_789
EXPECTED_AMBIGUOUS_COUNT = 97
SIGN_35_ASSET_ID = "0xd4538d2e1da5434f"


def main() -> int:
    rails = [
        obj
        for obj in bpy.data.objects
        if bool(obj.get("skate3_retail_grind", False))
    ]
    if len(rails) != EXPECTED_RAIL_COUNT:
        raise RuntimeError(
            f"expected {EXPECTED_RAIL_COUNT} rails, found {len(rails)}"
        )

    coordinate_counts = {
        "cell_local": 0,
        "world_space": 0,
        "ambiguous": 0,
    }
    minimum = [math.inf, math.inf, math.inf]
    maximum = [-math.inf, -math.inf, -math.inf]
    for rail_index, obj in enumerate(rails, 1):
        source_frame = str(
            obj["skate3_retail_grind_source_coordinate_frame"]
        )
        coordinate_counts[source_frame] += 1
        # This is the addon's real exact-native validation path. It compares
        # every world-space Bezier control against the export payload.
        _export_retail_grind(obj)
        for point in obj.data.splines[0].bezier_points:
            world = obj.matrix_world @ point.co
            for axis in range(3):
                minimum[axis] = min(minimum[axis], world[axis])
                maximum[axis] = max(maximum[axis], world[axis])
        if rail_index % 5_000 == 0:
            print(
                f"Validated {rail_index}/{len(rails)} exportable rails",
                flush=True,
            )

    if coordinate_counts["cell_local"] != EXPECTED_CELL_LOCAL_COUNT:
        raise RuntimeError(
            f"unexpected cell-local count: {coordinate_counts['cell_local']}"
        )
    if coordinate_counts["ambiguous"] != EXPECTED_AMBIGUOUS_COUNT:
        raise RuntimeError(
            f"unexpected ambiguous count: {coordinate_counts['ambiguous']}"
        )
    signs = [
        obj.name
        for obj in bpy.data.objects
        if str(obj.get("skate3_asset_id", "")).lower() == SIGN_35_ASSET_ID
        and not bool(obj.get("skate3_retail_grind", False))
    ]
    if signs:
        raise RuntimeError(f"unplaced Sign_35 objects remain: {signs}")

    result = {
        "objects": len(bpy.data.objects),
        "meshes": len(bpy.data.meshes),
        "images": len(bpy.data.images),
        "materials": len(bpy.data.materials),
        "grind_rails": len(rails),
        "coordinate_frames": coordinate_counts,
        "grind_point_bounds": {
            "minimum": minimum,
            "maximum": maximum,
        },
        "sign_35_objects": signs,
        "exporter_validation": "passed",
    }
    print(json.dumps(result, indent=2), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
