"""Validate University material provenance inside the generated Blender file."""

from __future__ import annotations

from collections import Counter
import json
import struct

import bpy
from mathutils import Vector
import numpy


EXPECTED_MODE_COUNTS = {0: 6389, 1: 2118, 2: 39}
EXPECTED_GRIND_RAILS = 4201
EXPECTED_GRIND_SEGMENTS = 27008
EXPECTED_CLOSED_GRIND_RAILS = 372
EXPECTED_COLLISION_SURFACES = 183
EXPECTED_COLLISION_TRIANGLES = 1_133_649
EXPECTED_NORMAL_MAPPED_OBJECTS = 2_987
EXPECTED_NORMAL_TEXTURES = 135
EXPECTED_SOURCE_LIGHTMAP_REFERENCES = 8_522
EXPECTED_LIGHTMAPPED_OBJECTS = 8_489
EXPECTED_LIGHTMAP_TEXTURES = 1_270
EXPECTED_LIGHTMAP_EXCLUSIONS = 33
EXPECTED_RETAIL_WORLD_FRAMES = 3_121
RETAIL_NORMAL_ATTRIBUTE = (
    "skate3_retail_normal",
    "FLOAT_VECTOR",
    "vector",
    3,
)
RETAIL_TANGENT_ATTRIBUTES = (
    "skate3_retail_tangent",
    "skate3_retail_binormal",
)
RETAIL_HANDEDNESS_ATTRIBUTE = (
    "skate3_retail_tangent_handedness",
    "FLOAT",
    "value",
    1,
)
REGRESSION_BINDINGS = {
    ("0xF6CC7BFCC2C45F8C", 40): (
        "0x861894DE4209CE82",
        "0x00800078",
        24,
        "0x2c70170a001d00aa",
        "0x893793c0de31aab1",
        0,
    ),
    ("0xF6CC7BFCC2C45F8C", 41): (
        "0xFA839F99082D42ED",
        "0x0080007B",
        38,
        "0x2c70170a0004000e",
        "0xba390198c38936f0",
        0,
    ),
    ("0xF6CC7BFCC2C45F8C", 42): (
        "0x96879C786774C31D",
        "0x0080007E",
        12,
        "0x00008d6a03e3870a",
        "0xba390198c38936f0",
        0,
    ),
    ("0x759E349006948F63", 2): (
        "0x861894DE4209CE82",
        "0x00800006",
        0,
        "0x2c70170a001d00aa",
        "0x893793c0de31aab1",
        0,
    ),
    ("0x759E349006948F63", 3): (
        "0xA0683D15728B6787",
        "0x00800009",
        3,
        "0x2c70170a00053a88",
        "0x893793c0de31aab1",
        1,
    ),
}


def main() -> int:
    objects = [
        obj
        for obj in bpy.data.objects
        if obj.type == "MESH" and "skate3_asset_id" in obj
    ]
    if len(objects) != 8546:
        raise RuntimeError(
            f"University has {len(objects)} imported mesh parts, expected 8546"
        )

    modes: Counter[int] = Counter()
    unresolved: Counter[int] = Counter()
    normal_mapped_objects = 0
    normal_texture_ids: set[str] = set()
    normal_materials: set[int] = set()
    source_lightmap_references = 0
    lightmapped_objects = 0
    lightmap_texture_ids: set[str] = set()
    lightmap_materials: set[int] = set()
    lightmap_exclusions = 0
    retail_world_frames = 0
    lookup: dict[tuple[str, int], bpy.types.Object] = {}
    for obj in objects:
        alpha_mode = int(obj.get("skate3_alpha_mode", 0))
        texture_id = str(obj.get("skate3_texture_id", ""))
        modes[alpha_mode] += 1
        lookup[
            (
                str(obj["skate3_asset_id"]),
                int(obj["skate3_mesh_index"]),
            )
        ] = obj
        source_lightmap_texture_id = str(
            obj.get("skate3_source_lightmap_texture_id", "")
        )
        lightmap_texture_id = str(
            obj.get("skate3_lightmap_texture_id", "")
        )
        if source_lightmap_texture_id:
            source_lightmap_references += 1
        if not obj.data.materials:
            raise RuntimeError(
                f"{obj.name!r} lost its retail material definition"
            )
        material = obj.data.materials[0]
        has_retail_world_frame = bool(
            obj.get("skate3_retail_world_frame", False)
        )
        tangent_attributes = [
            obj.data.attributes.get(name)
            for name in RETAIL_TANGENT_ATTRIBUTES
        ]
        if sum(attribute is not None for attribute in tangent_attributes) > 1:
            raise RuntimeError(
                f"{obj.name!r} has both current and legacy retail tangent "
                "attributes"
            )
        selected_tangent_name = (
            RETAIL_TANGENT_ATTRIBUTES[
                next(
                    (
                        index
                        for index, attribute in enumerate(tangent_attributes)
                        if attribute is not None
                    ),
                    0,
                )
            ]
        )
        frame_schema = (
            RETAIL_NORMAL_ATTRIBUTE,
            (selected_tangent_name, "FLOAT_VECTOR", "vector", 3),
            RETAIL_HANDEDNESS_ATTRIBUTE,
        )
        frame_attributes = []
        for (
            attribute_name,
            data_type,
            _property_name,
            _components,
        ) in frame_schema:
            attribute = obj.data.attributes.get(attribute_name)
            if has_retail_world_frame:
                if (
                    attribute is None
                    or attribute.domain != "POINT"
                    or attribute.data_type != data_type
                    or len(attribute.data) != len(obj.data.vertices)
                ):
                    raise RuntimeError(
                        f"{obj.name!r} has invalid retail world-frame "
                        f"attribute {attribute_name!r}"
                    )
                frame_attributes.append(attribute)
            elif attribute is not None:
                raise RuntimeError(
                    f"{obj.name!r} has unexpected retail world-frame "
                    f"attribute {attribute_name!r}"
                )
        if has_retail_world_frame:
            decoded = []
            for attribute, (
                _attribute_name,
                _data_type,
                property_name,
                components,
            ) in zip(
                frame_attributes,
                frame_schema,
                strict=True,
            ):
                values = numpy.empty(
                    len(attribute.data) * components,
                    dtype=numpy.float32,
                )
                attribute.data.foreach_get(property_name, values)
                if not numpy.isfinite(values).all():
                    raise RuntimeError(
                        f"{obj.name!r} retail world frame is non-finite"
                    )
                decoded.append(values.reshape((-1, components)))
            normal_lengths = numpy.linalg.norm(decoded[0], axis=1)
            if (
                normal_lengths.size
                and (
                    float(normal_lengths.min()) < 0.999
                    or float(normal_lengths.max()) > 1.001
                )
            ):
                raise RuntimeError(
                    f"{obj.name!r} retail normals are not unit length"
                )
            if not numpy.all(numpy.abs(decoded[2][:, 0]) > 0.999):
                raise RuntimeError(
                    f"{obj.name!r} retail tangent handedness is invalid"
                )
            retail_world_frames += 1
        expected_material_texture_id = texture_id or "NO_DIFFUSE"
        if (
            str(material.get("skate3_texture_id", ""))
            != expected_material_texture_id
            or int(material.get("skate3_alpha_mode", -1)) != alpha_mode
        ):
            raise RuntimeError(
                f"{obj.name!r} material provenance does not match the mesh"
            )
        if "skate3_fallback_reason" in material:
            unresolved[alpha_mode] += 1
        if lightmap_texture_id:
            if lightmap_texture_id != source_lightmap_texture_id:
                raise RuntimeError(
                    f"{obj.name!r} substituted retail lightmap "
                    f"{source_lightmap_texture_id!r} with "
                    f"{lightmap_texture_id!r}"
                )
            lightmap_layer = obj.data.uv_layers.get("Lightmap")
            if (
                lightmap_layer is None
                or len(lightmap_layer.data) != len(obj.data.loops)
            ):
                raise RuntimeError(
                    f"{obj.name!r} lost its retail lightmap UVs"
                )
            lightmap_image = bpy.data.images.get(lightmap_texture_id)
            if lightmap_image is None:
                raise RuntimeError(
                    f"{obj.name!r} is missing lightmap image "
                    f"{lightmap_texture_id!r}"
                )
            if lightmap_image.colorspace_settings.name != "Non-Color":
                raise RuntimeError(
                    f"{lightmap_texture_id!r} is not marked Non-Color"
                )
            if (
                str(material.get("skate3_lightmap_texture_id", ""))
                != lightmap_texture_id
                or str(material.get("ow_lightmap_image", ""))
                != lightmap_image.name
                or str(material.get("ow_lightmap_encoding", ""))
                != "skate3_retail_sqrt_linear_over_4"
                or float(material.get("ow_baked_strength", 0.0)) != 1.0
            ):
                raise RuntimeError(
                    f"{material.name!r} does not export its retail lightmap"
                )
            lightmapped_objects += 1
            lightmap_texture_ids.add(lightmap_texture_id)
            lightmap_materials.add(material.as_pointer())
        elif source_lightmap_texture_id:
            reason = str(obj.get("skate3_lightmap_exclusion", ""))
            if not reason:
                raise RuntimeError(
                    f"{obj.name!r} silently discarded retail lightmap "
                    f"{source_lightmap_texture_id!r}"
                )
            if (
                str(material.get("skate3_lightmap_texture_id", ""))
                or str(material.get("ow_lightmap_image", ""))
            ):
                raise RuntimeError(
                    f"{obj.name!r} inherited a lightmap despite exclusion"
                )
            lightmap_exclusions += 1
        normal_texture_id = str(
            obj.get("skate3_normal_texture_id", "")
        )
        source_normal_texture_id = str(
            obj.get("skate3_source_normal_texture_id", "")
        )
        if normal_texture_id:
            if normal_texture_id != source_normal_texture_id:
                raise RuntimeError(
                    f"{obj.name!r} substituted retail normal "
                    f"{source_normal_texture_id!r} with "
                    f"{normal_texture_id!r}"
                )
            if (
                str(material.get("skate3_normal_texture_id", ""))
                != normal_texture_id
            ):
                raise RuntimeError(
                    f"{obj.name!r} material lost normal provenance"
                )
            normal_image = bpy.data.images.get(normal_texture_id)
            if normal_image is None:
                raise RuntimeError(
                    f"{obj.name!r} is missing normal image "
                    f"{normal_texture_id!r}"
                )
            if normal_image.colorspace_settings.name != "Non-Color":
                raise RuntimeError(
                    f"{normal_texture_id!r} is not marked Non-Color"
                )
            if (
                str(material.get("ow_normal_image", ""))
                != normal_image.name
            ):
                raise RuntimeError(
                    f"{material.name!r} does not export its retail normal"
                )
            normal_mapped_objects += 1
            normal_texture_ids.add(normal_texture_id)
            normal_materials.add(material.as_pointer())
        elif (
            str(material.get("skate3_normal_texture_id", ""))
            or str(material.get("ow_normal_image", ""))
        ):
            raise RuntimeError(
                f"{obj.name!r} unexpectedly inherited another mesh's normal"
            )

    if dict(modes) != EXPECTED_MODE_COUNTS:
        raise RuntimeError(
            f"University alpha modes changed: {dict(modes)}"
        )
    if normal_mapped_objects != EXPECTED_NORMAL_MAPPED_OBJECTS:
        raise RuntimeError(
            f"University has {normal_mapped_objects} normal-mapped mesh "
            f"parts, expected {EXPECTED_NORMAL_MAPPED_OBJECTS}"
        )
    if len(normal_texture_ids) != EXPECTED_NORMAL_TEXTURES:
        raise RuntimeError(
            f"University has {len(normal_texture_ids)} conventional retail "
            f"normal textures, expected {EXPECTED_NORMAL_TEXTURES}"
        )
    if source_lightmap_references != EXPECTED_SOURCE_LIGHTMAP_REFERENCES:
        raise RuntimeError(
            f"University has {source_lightmap_references} source lightmap "
            f"references, expected {EXPECTED_SOURCE_LIGHTMAP_REFERENCES}"
        )
    if lightmapped_objects != EXPECTED_LIGHTMAPPED_OBJECTS:
        raise RuntimeError(
            f"University has {lightmapped_objects} lightmapped mesh parts, "
            f"expected {EXPECTED_LIGHTMAPPED_OBJECTS}"
        )
    if len(lightmap_texture_ids) != EXPECTED_LIGHTMAP_TEXTURES:
        raise RuntimeError(
            f"University has {len(lightmap_texture_ids)} exported retail "
            f"lightmaps, expected {EXPECTED_LIGHTMAP_TEXTURES}"
        )
    if lightmap_exclusions != EXPECTED_LIGHTMAP_EXCLUSIONS:
        raise RuntimeError(
            f"University has {lightmap_exclusions} explicit lightmap "
            f"exclusions, expected {EXPECTED_LIGHTMAP_EXCLUSIONS}"
        )
    if retail_world_frames != EXPECTED_RETAIL_WORLD_FRAMES:
        raise RuntimeError(
            f"University has {retail_world_frames} exact retail world "
            f"frames, expected {EXPECTED_RETAIL_WORLD_FRAMES}"
        )

    for key, expected in REGRESSION_BINDINGS.items():
        obj = lookup.get(key)
        if obj is None:
            raise RuntimeError(f"regression mesh {key!r} is missing")
        actual = (
            str(obj.get("skate3_retail_material_guid", "")),
            str(obj.get("skate3_retail_material_handle", "")),
            int(obj.get("skate3_retail_material_group_index", -1)),
            str(obj.get("skate3_texture_id", "")),
            str(obj.get("skate3_lightmap_texture_id", "")),
            int(obj.get("skate3_alpha_mode", 0)),
        )
        if actual != expected:
            raise RuntimeError(
                f"regression mesh {key!r}: {actual!r} != {expected!r}"
            )

    grind_objects = [
        obj
        for obj in bpy.data.objects
        if obj.type == "CURVE"
        and bool(obj.get("skate3_retail_grind", False))
    ]
    if len(grind_objects) != EXPECTED_GRIND_RAILS:
        raise RuntimeError(
            f"University has {len(grind_objects)} retail grind rails, "
            f"expected {EXPECTED_GRIND_RAILS}"
        )
    grind_segments = 0
    closed_grinds = 0
    for obj in grind_objects:
        if len(obj.data.splines) != 1:
            raise RuntimeError(
                f"{obj.name!r} does not contain exactly one retail spline"
            )
        spline = obj.data.splines[0]
        if spline.type != "BEZIER":
            raise RuntimeError(f"{obj.name!r} is not a Bezier spline")
        segment_count = int(
            obj["skate3_retail_grind_segment_count"]
        )
        actual_segment_count = (
            len(spline.bezier_points)
            if spline.use_cyclic_u
            else len(spline.bezier_points) - 1
        )
        if actual_segment_count != segment_count:
            raise RuntimeError(
                f"{obj.name!r} has {actual_segment_count} Blender segments "
                f"but {segment_count} retail segments"
            )
        payload = bytes.fromhex(
            str(obj["skate3_retail_grind_segment_payload"])
        )
        if len(payload) != segment_count * 120:
            raise RuntimeError(
                f"{obj.name!r} has an invalid native segment payload"
            )
        if int(
            str(obj["skate3_retail_grind_spline_id"]),
            16,
        ) == 0 or int(
            str(obj["skate3_retail_grind_type_signature"]),
            16,
        ) == 0:
            raise RuntimeError(
                f"{obj.name!r} lost its retail spline identity"
            )

        points = spline.bezier_points
        for segment_index in range(segment_count):
            values = struct.unpack_from(
                ">30f",
                payload,
                segment_index * 120,
            )
            coefficient_a = values[0:3]
            coefficient_b = values[4:7]
            coefficient_c = values[8:11]
            coefficient_d = values[12:15]
            runtime_controls = (
                coefficient_d,
                tuple(
                    coefficient_d[axis] + coefficient_c[axis] / 3.0
                    for axis in range(3)
                ),
                tuple(
                    coefficient_d[axis]
                    + (
                        2.0 * coefficient_c[axis]
                        + coefficient_b[axis]
                    )
                    / 3.0
                    for axis in range(3)
                ),
                tuple(
                    coefficient_d[axis]
                    + coefficient_c[axis]
                    + coefficient_b[axis]
                    + coefficient_a[axis]
                    for axis in range(3)
                ),
            )
            expected_controls = tuple(
                Vector((point[0], -point[2], point[1]))
                for point in runtime_controls
            )
            current = points[segment_index]
            following = points[
                (segment_index + 1) % len(points)
            ]
            actual_controls = (
                obj.matrix_world @ current.co,
                obj.matrix_world @ current.handle_right,
                obj.matrix_world @ following.handle_left,
                obj.matrix_world @ following.co,
            )
            control_errors = [
                (actual - expected).length
                for actual, expected in zip(
                    actual_controls,
                    expected_controls,
                )
            ]
            # Blender stores curve points as float32. Retail cubic endpoints
            # are independently evaluated coefficients, so cancellation at
            # University-scale coordinates can add sub-millimetre rounding.
            if any(error > 2.0e-3 for error in control_errors):
                raise RuntimeError(
                    f"{obj.name!r} segment {segment_index} no longer "
                    "matches its exact retail cubic: "
                    f"control errors={control_errors}"
                )
        grind_segments += segment_count
        closed_grinds += bool(spline.use_cyclic_u)

    if (
        grind_segments != EXPECTED_GRIND_SEGMENTS
        or closed_grinds != EXPECTED_CLOSED_GRIND_RAILS
    ):
        raise RuntimeError(
            "University retail grind totals changed: "
            f"segments={grind_segments}, closed={closed_grinds}"
        )

    collision_objects = [
        obj
        for obj in bpy.data.objects
        if obj.type == "MESH"
        and bool(obj.get("skate3_retail_collision", False))
    ]
    if len(collision_objects) != EXPECTED_COLLISION_SURFACES:
        raise RuntimeError(
            f"University has {len(collision_objects)} retail collision "
            f"surfaces, expected {EXPECTED_COLLISION_SURFACES}"
        )
    collision_triangles = 0
    for obj in collision_objects:
        if len(obj.data.materials) != 1 or obj.data.materials[0] is None:
            raise RuntimeError(
                f"{obj.name!r} does not have one retail collision material"
            )
        surface = int(str(obj["skate3_retail_surface_id"]), 16)
        if not bool(
            obj.get("ow_preserve_opposite_wound_collision", False)
        ):
            raise RuntimeError(
                f"{obj.name!r} would discard reverse-wound retail collision"
            )
        if not bool(obj.get("ow_preserve_retail_edge_codes", False)):
            raise RuntimeError(
                f"{obj.name!r} would discard native retail edge codes"
            )
        for corner in range(3):
            attribute_name = f"skate3_retail_edge_code_{corner}"
            attribute = obj.data.attributes.get(attribute_name)
            if (
                attribute is None
                or attribute.domain != "FACE"
                or attribute.data_type != "INT"
                or len(attribute.data) != len(obj.data.polygons)
            ):
                raise RuntimeError(
                    f"{obj.name!r} has invalid {attribute_name!r} metadata"
                )
        material = obj.data.materials[0]
        encoded = (
            int(material["ow_audio_surface"])
            | (int(material["ow_physics_surface"]) << 7)
            | (int(material["ow_surface_pattern"]) << 12)
        )
        if encoded != surface:
            raise RuntimeError(
                f"{obj.name!r} packed surface changed: "
                f"0x{encoded:04X} != 0x{surface:04X}"
            )
        triangle_count = len(obj.data.polygons)
        if triangle_count != int(obj["skate3_retail_triangle_count"]):
            raise RuntimeError(
                f"{obj.name!r} retail triangle count changed"
            )
        collision_triangles += triangle_count
    if collision_triangles != EXPECTED_COLLISION_TRIANGLES:
        raise RuntimeError(
            f"University has {collision_triangles} retail collision triangles, "
            f"expected {EXPECTED_COLLISION_TRIANGLES}"
        )

    print(
        json.dumps(
            {
                "status": "UNIVERSITY_BLEND_MATERIALS_OK",
                "mesh_parts": len(objects),
                "alpha_modes": dict(sorted(modes.items())),
                "unresolved_by_alpha_mode": dict(sorted(unresolved.items())),
                "regression_bindings": len(REGRESSION_BINDINGS),
                "normal_mapped_objects": normal_mapped_objects,
                "normal_textures": len(normal_texture_ids),
                "normal_materials": len(normal_materials),
                "source_lightmap_references": source_lightmap_references,
                "lightmapped_objects": lightmapped_objects,
                "lightmap_textures": len(lightmap_texture_ids),
                "lightmap_materials": len(lightmap_materials),
                "lightmap_exclusions": lightmap_exclusions,
                "retail_world_frames": retail_world_frames,
                "grind_rails": len(grind_objects),
                "grind_segments": grind_segments,
                "closed_grind_rails": closed_grinds,
                "collision_surfaces": len(collision_objects),
                "collision_triangles": collision_triangles,
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
