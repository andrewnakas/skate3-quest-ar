"""Prepare the extracted Skate 2 city scene for the owned-world exporter."""

from __future__ import annotations

from array import array
import json
import math
from pathlib import Path
import sys

import bpy


sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_university_owned import (
    GROUP_1,
    GROUP_2,
    GROUP_3,
    GROUP_4,
    GROUP_5,
    NO_COLLISION_MARKERS,
    SPAWN,
    _configure_material,
    _default_material,
    _ensure_export_uvs,
    _new_group,
    _presentation_only,
    _surface_hits,
)


# A broad, nearly level sidewalk confirmed against both presentation geometry
# and the exact retail collision mesh.
SPAWN_RUNTIME_XZ = (0.0, 50.0)
SPAWN_TARGET_HEIGHT = -26.917


def _configure_skate2_material(material: bpy.types.Material) -> int:
    """Configure one material and omit retail bindings absent from BAM."""

    _configure_material(material)
    raw_texture_ids = json.loads(
        str(material.get("skate3_retail_texture_ids", "{}"))
    )
    if not isinstance(raw_texture_ids, dict):
        raise RuntimeError(
            f"{material.name!r} has invalid retail texture metadata"
        )
    available: dict[str, str] = {}
    excluded: dict[str, str] = {}
    for semantic, source_id in raw_texture_ids.items():
        normalized_id = str(source_id).lower()
        if bpy.data.images.get(normalized_id) is None:
            excluded[str(semantic)] = normalized_id
        else:
            available[str(semantic)] = normalized_id
    material["skate3_retail_texture_ids"] = json.dumps(
        available,
        sort_keys=True,
        separators=(",", ":"),
    )
    material["skate2_excluded_retail_texture_ids"] = json.dumps(
        excluded,
        sort_keys=True,
        separators=(",", ":"),
    )
    return len(excluded)


def _sanitize_export_uvs(mesh: bpy.types.Mesh) -> int:
    """Replace Skate 2's FLT_MAX unused-UV sentinel in the owned copy."""

    replaced = 0
    for layer in mesh.uv_layers:
        values = array("f", [0.0]) * (len(mesh.loops) * 2)
        layer.data.foreach_get("uv", values)
        changed = False
        for index, value in enumerate(values):
            if not math.isfinite(value) or abs(value) >= 1.0e30:
                values[index] = 0.0
                replaced += 1
                changed = True
        if changed:
            layer.data.foreach_set("uv", values)
    if replaced:
        mesh["skate2_sanitized_uv_sentinel_values"] = replaced
    return replaced


def _create_spawn() -> tuple[float, float, float]:
    old = bpy.data.objects.get(SPAWN)
    if old is not None:
        bpy.data.objects.remove(old, do_unlink=True)
    x, runtime_z = SPAWN_RUNTIME_XZ
    candidates = [
        hit
        for hit in _surface_hits(x, runtime_z)
        if hit[1] >= 0.65
        and not any(marker in hit[2].lower() for marker in NO_COLLISION_MARKERS)
    ]
    if not candidates:
        raise RuntimeError(
            f"no upward Skate 2 surface found at runtime XZ {(x, runtime_z)}"
        )
    surface_height, _normal_z, owner = min(
        candidates,
        key=lambda hit: abs(hit[0] - SPAWN_TARGET_HEIGHT),
    )
    vertices = [
        (-2.0, -2.0, 0.0),
        (2.0, -2.0, 0.0),
        (2.0, 2.0, 0.0),
        (-2.0, 2.0, 0.0),
        (0.0, -3.0, 0.0),
    ]
    mesh = bpy.data.meshes.new(f"{SPAWN}_MESH")
    mesh.from_pydata(vertices, [], [(0, 1, 2, 3), (0, 4, 1)])
    spawn = bpy.data.objects.new(SPAWN, mesh)
    bpy.context.scene.collection.objects.link(spawn)
    spawn.location = (x, -runtime_z, surface_height + 1.0)
    spawn.rotation_euler.z = math.radians(-90.0)
    spawn.hide_render = True
    spawn["skate2_surface_object"] = owner
    return (x, surface_height + 1.0, runtime_z)


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 1:
        raise SystemExit(
            "usage: blender --background Skate_2_New_San_Vanelona.blend "
            "--python prepare_skate2_owned.py -- OUTPUT_BLEND"
        )
    output = Path(arguments[0]).resolve()
    group_1 = _new_group(GROUP_1)
    group_2 = _new_group(GROUP_2)
    group_3 = _new_group(GROUP_3)
    group_4 = _new_group(GROUP_4)
    _new_group(GROUP_5)
    fallback = _default_material()
    _configure_skate2_material(fallback)

    mesh_objects = sorted(
        (obj for obj in bpy.data.objects if obj.type == "MESH"),
        key=lambda obj: obj.name_full,
    )
    material_ids: set[int] = set()
    collidable = 0
    presentation_only = 0
    triangle_count = 0
    retail_collision_objects = 0
    retail_collision_triangles = 0
    excluded_retail_texture_bindings = 0
    sanitized_uv_sentinel_values = 0
    for object_index, obj in enumerate(mesh_objects, 1):
        if obj.name == SPAWN:
            continue
        if bool(obj.get("skate3_retail_collision", False)):
            if not obj.data.materials:
                raise RuntimeError(
                    f"{obj.name!r} has no retail collision material"
                )
            material = obj.data.materials[0]
            obj["ow_material"] = material.name
            group_2.objects.link(obj)
            retail_collision_objects += 1
            retail_collision_triangles += len(obj.data.polygons)
            continue
        if not obj.data.materials:
            obj.data.materials.append(fallback)
        require_retail_lightmap = any(
            material is not None
            and bool(material.get("skate3_lightmap_texture_id", ""))
            for material in obj.data.materials
        )
        _ensure_export_uvs(
            obj.data,
            require_retail_lightmap=require_retail_lightmap,
        )
        sanitized_uv_sentinel_values += _sanitize_export_uvs(obj.data)
        for material in obj.data.materials:
            if (
                material is not None
                and material.as_pointer() not in material_ids
            ):
                material_ids.add(material.as_pointer())
                excluded_retail_texture_bindings += (
                    _configure_skate2_material(material)
                )
        obj["ow_material"] = obj.data.materials[0].name
        triangle_count += len(obj.data.polygons)
        if _presentation_only(obj):
            group_3.objects.link(obj)
            presentation_only += 1
        else:
            group_1.objects.link(obj)
            collidable += 1
        if object_index % 5_000 == 0:
            print(
                f"Prepared {object_index}/{len(mesh_objects)} mesh objects",
                flush=True,
            )

    grind_objects = sorted(
        (
            obj
            for obj in bpy.data.objects
            if obj.type == "CURVE"
            and bool(obj.get("skate3_retail_grind", False))
        ),
        key=lambda obj: obj.name_full,
    )
    grind_segments = 0
    for obj in grind_objects:
        group_4.objects.link(obj)
        grind_segments += int(obj["skate3_retail_grind_segment_count"])

    spawn_runtime = _create_spawn()
    scene = bpy.context.scene
    scene["ow_map_name"] = "Skate 2 — New San Vanelona"
    scene["ow_cycle_seconds"] = 0.0
    scene["ow_start_hour"] = 11.0
    scene["ow_end_hour"] = 18.0
    scene["ow_cycle_ping_pong"] = False
    scene["ow_sky_zenith"] = (0.10, 0.36, 0.75)
    scene["ow_sky_horizon"] = (0.64, 0.82, 1.0)
    scene["ow_sky_nadir"] = (0.18, 0.24, 0.30)
    scene["ow_day_ambient"] = 0.34
    scene["ow_night_ambient"] = 0.10
    scene["skate2_collision_source"] = (
        "exact retail RenderWare ClusteredMesh triangles and packed surfaces"
    )
    scene["skate2_grind_source"] = (
        "exact corrected retail Pegasus tSplineData cubic segment payloads"
    )
    scene["skate2_spawn_runtime"] = spawn_runtime
    output.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(output), check_existing=False)
    print(
        json.dumps(
            {
                "output": str(output),
                "mesh_objects": len(mesh_objects),
                "materials": len(material_ids),
                "triangles": triangle_count,
                "collidable_objects": collidable,
                "presentation_only_objects": presentation_only,
                "retail_collision_objects": retail_collision_objects,
                "retail_collision_triangles": retail_collision_triangles,
                "grind_rails": len(grind_objects),
                "grind_segments": grind_segments,
                "excluded_absent_retail_texture_bindings": (
                    excluded_retail_texture_bindings
                ),
                "sanitized_uv_sentinel_values": (
                    sanitized_uv_sentinel_values
                ),
                "spawn_runtime": spawn_runtime,
                "collision_source": scene["skate2_collision_source"],
            },
            indent=2,
        ),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
