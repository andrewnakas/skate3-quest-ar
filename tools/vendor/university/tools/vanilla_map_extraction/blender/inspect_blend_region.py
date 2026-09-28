"""Print export-relevant Blender object details near a runtime-space point."""

from __future__ import annotations

import argparse
import math
import sys

import bpy
from mathutils import Vector


def _arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime-center", nargs=3, type=float, required=True)
    parser.add_argument("--radius", type=float, default=50.0)
    return parser.parse_args(
        sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    )


def _runtime_position(blender_position: Vector) -> Vector:
    return Vector(
        (
            blender_position.x,
            blender_position.z,
            -blender_position.y,
        )
    )


def _object_runtime_bounds(
    obj: bpy.types.Object,
) -> tuple[Vector, Vector, Vector]:
    corners = [
        _runtime_position(obj.matrix_world @ Vector(corner))
        for corner in obj.bound_box
    ]
    minimum = Vector(
        tuple(min(corner[index] for corner in corners) for index in range(3))
    )
    maximum = Vector(
        tuple(max(corner[index] for corner in corners) for index in range(3))
    )
    return minimum, maximum, (minimum + maximum) * 0.5


def _public_properties(value: object) -> dict[str, object]:
    if not hasattr(value, "keys"):
        return {}
    omitted = {
        "_RNA_UI",
        "skate3_retail_grind_segment_payload",
        "skate3_retail_parameters",
        "skate3_retail_source",
        "skate3_retail_texture_ids",
    }
    return {str(key): value[key] for key in value.keys() if key not in omitted}


def main() -> None:
    args = _arguments()
    center = Vector(args.runtime_center)
    matches: list[tuple[float, bpy.types.Object, Vector, Vector, Vector]] = []
    for obj in bpy.context.scene.objects:
        if obj.type not in {"MESH", "CURVE"}:
            continue
        minimum, maximum, object_center = _object_runtime_bounds(obj)
        distance = (object_center - center).length
        if distance <= args.radius:
            matches.append((distance, obj, minimum, maximum, object_center))

    print(
        f"BLEND_REGION center={tuple(center)} radius={args.radius} "
        f"matches={len(matches)}"
    )
    for distance, obj, minimum, maximum, object_center in sorted(matches):
        print(
            f"OBJECT distance={distance:.3f} type={obj.type} "
            f"name={obj.name_full!r}"
        )
        print(
            f"  runtime_center={tuple(round(value, 3) for value in object_center)} "
            f"bounds_min={tuple(round(value, 3) for value in minimum)} "
            f"bounds_max={tuple(round(value, 3) for value in maximum)}"
        )
        print(f"  object_properties={_public_properties(obj)!r}")
        print(f"  data_properties={_public_properties(obj.data)!r}")
        if obj.type == "MESH":
            mesh = obj.data
            print(
                f"  mesh vertices={len(mesh.vertices)} loops={len(mesh.loops)} "
                f"polygons={len(mesh.polygons)} uv_layers="
                f"{[layer.name for layer in mesh.uv_layers]!r} "
                f"attributes={[(attribute.name, attribute.domain, attribute.data_type) for attribute in mesh.attributes]!r}"
            )
            for slot in obj.material_slots:
                material = slot.material
                if material is None:
                    continue
                print(
                    f"  material={material.name_full!r} "
                    f"properties={_public_properties(material)!r}"
                )
        else:
            print(
                f"  curve splines={len(obj.data.splines)} "
                f"bevel_depth={obj.data.bevel_depth}"
            )


if __name__ == "__main__":
    main()
