"""Original Blender -> SKATE v15 exporter.

This module intentionally targets the narrow project-owned scene contract
documented beside it. It has no ArenaBuilder imports or runtime dependency.
"""

from __future__ import annotations

from array import array
from dataclasses import dataclass
from pathlib import Path
import hashlib
import io
import json
import math
import os
import struct
import sys
import time
import zlib
from typing import BinaryIO, Callable

import bpy
from mathutils import Vector
try:
    import numpy
except ImportError:
    numpy = None
try:
    from compression import zstd
except ImportError:
    zstd = None


# The Rust runtime requests the existing v14 storage contract. This changes
# compression only; material, vertex, collision and native spline data remain.
COMPAT14 = os.environ.get("SKATE_EXPORT_COMPAT14") == "1"
MAGIC = b"SKATE14\0" if COMPAT14 else b"SKATE15\0"
ENDIAN_MARKER = 0x12345678
STORAGE_RAW = 0
STORAGE_DEFLATE = 1
STORAGE_ZSTD = 2
STORAGE_ZSTD_RGBA_FILTER = 3
STORAGE_ZSTD_VERTEX_SOA = 4
STORAGE_ZSTD_INDEX_DELTA = 5
STORAGE_ZSTD_COLLISION_INDEXED = 6
STORAGE_DEFLATE_RGBA_FILTER = 7
STORAGE_DEFLATE_VERTEX_SOA = 8
STORAGE_DEFLATE_INDEX_DELTA = 9
STORAGE_DEFLATE_COLLISION_INDEXED = 10
STORAGE_TEXTURE_REFERENCE = 11
PRESENTATION_COLLISION_COLLECTION = "OW_GROUP_1_PRESENTATION_COLLISION"
NO_PRESENTATION_COLLECTION = "OW_GROUP_2_NO_PRESENTATION"
NO_COLLISION_COLLECTION = "OW_GROUP_3_NO_COLLISION"
GRIND_COLLECTION = "OW_GROUP_4_GRINDS"
NPC_PATH_COLLECTION = "OW_GROUP_5_PATHING"
# Public aliases retained for scripts written against add-on 1.x. New scenes
# use the five exclusive collections above.
VISUAL_COLLECTION = PRESENTATION_COLLISION_COLLECTION
COLLISION_COLLECTION = NO_PRESENTATION_COLLECTION
LEGACY_VISUAL_COLLECTION = "OW_VISUAL"
LEGACY_COLLISION_COLLECTION = "OW_COLLISION"
LEGACY_GRIND_COLLECTION = "OW_GRIND"
LEGACY_NPC_PATH_COLLECTION = "OW_NPC_PATHS"
GROUP_COLLECTIONS = (
    PRESENTATION_COLLISION_COLLECTION,
    NO_PRESENTATION_COLLECTION,
    NO_COLLISION_COLLECTION,
    GRIND_COLLECTION,
    NPC_PATH_COLLECTION,
)
SPAWN_OBJECT = "OW_SPAWN"
_HELPER_OBJECT_MARKERS = (
    "character_size",
    "character size",
    "player_size",
    "player size",
    "scale_reference",
    "scale reference",
)
CACHE_SCHEMA = 22
METADATA_FLOAT_COUNT = 49
METADATA_BYTE_COUNT = METADATA_FLOAT_COUNT * 4
RETAIL_NORMAL_ATTRIBUTE = "skate3_retail_normal"
RETAIL_TANGENT_ATTRIBUTE = "skate3_retail_tangent"
# University files created before the usage-6 semantic was verified contain
# the retail tangent under this incorrect name. Read it as a tangent so those
# large working files remain exportable without a destructive migration.
LEGACY_RETAIL_TANGENT_ATTRIBUTE = "skate3_retail_binormal"
RETAIL_HANDEDNESS_ATTRIBUTE = "skate3_retail_tangent_handedness"
RETAIL_EDGE_CODE_ATTRIBUTES = tuple(
    f"skate3_retail_edge_code_{corner}" for corner in range(3)
)
EDITOR_COLLISION_OWNER_ATTRIBUTE = "ow_editor_collision_owner"


@dataclass
class ExportMaterial:
    blender_material: bpy.types.Material
    material_id: int
    export_name: str
    display_color: tuple[float, float, float]
    albedo_texture: int
    lightmap_texture: int
    normal_texture: int
    orm_texture: int
    emissive_texture: int
    secondary_albedo_texture: int
    blend_mask_texture: int
    retail_shader_name: str
    retail_shader_family: int
    retail_render_flags: int
    retail_material_guid: int
    retail_material_handle: int
    retail_material_group_index: int
    retail_texture_bindings: list[tuple[str, int, int, int, int]]
    retail_parameters: list[tuple[str, list[str]]]
    retail_source_metadata: str


@dataclass
class ExportHingedDoor:
    name: str
    hinge_position: tuple[float, float, float]
    hinge_axis: tuple[float, float, float]
    closed_width_axis: tuple[float, float, float]
    closed_depth_axis: tuple[float, float, float]
    local_min: tuple[float, float, float]
    local_max: tuple[float, float, float]
    minimum_angle_radians: float
    maximum_angle_radians: float
    initial_angle_radians: float
    mass: float
    angular_damping: float
    return_spring_strength: float
    maximum_angular_speed: float
    contact_impulse_scale: float
    friction: float
    restitution: float
    surface_id: int
    vertices: list[tuple]
    indices: list[int]
    collision: list[tuple]


@dataclass
class PackedVisualGeometry:
    vertex_chunks: list[bytes]
    index_chunks: list[bytes]
    vertex_count: int
    index_count: int
    objects: list["ExportMapObject"]


@dataclass
class ExportMapObject:
    source_identity: int
    object_id: int
    name: str
    origin: tuple[float, float, float]
    first_index: int
    index_count: int
    editor_editable: bool
    physics_type: int
    collision_shape: int
    density: float
    friction: float
    restitution: float
    linear_damping: float
    angular_damping: float
    gravity_scale: float
    enable_sleep: bool
    initially_awake: bool
    break_group: int
    break_speed_threshold: float
    break_impulse_scale: float
    break_angular_impulse: float
    break_gravity_scale: float


@dataclass
class ExportLocalLight:
    name: str
    light_type: int
    position: tuple[float, float, float]
    direction: tuple[float, float, float]
    color: tuple[float, float, float]
    intensity: float
    influence_radius: float
    source_radius: float
    spot_inner_cosine: float
    spot_outer_cosine: float


@dataclass
class ExportGrindRail:
    name: str
    closed: bool
    points: list[tuple[float, float, float]]
    retail_spline_id: int = 0
    retail_type_signature: int = 0
    retail_flags: int = 0
    retail_trailing_word: int = 0
    native_segment_payload: bytes = b""
    parent_source_identity: int = 0


@dataclass
class SceneContentFingerprint:
    digest: str
    visual_vertices: int
    visual_indices: int
    collision_triangles: int
    grind_rails: int
    npc_routes: int
    hinged_doors: int
    local_lights: int


@dataclass
class CollisionGeometryAudit:
    issues: list[str]
    warnings: list[str]
    source_triangles: int
    exported_triangles: int
    skipped_degenerate: int
    skipped_duplicates: int


class CollisionGeometryError(ValueError):
    def __init__(self, issues: list[str], warnings: list[str]) -> None:
        self.issues = list(issues)
        self.warnings = list(warnings)
        summary = "Collision validation failed:\n" + "\n".join(
            f"- {issue}" for issue in self.issues
        )
        super().__init__(summary)


LAST_COLLISION_AUDIT: CollisionGeometryAudit | None = None
ProgressCallback = Callable[[float, str], None]


def _report_progress(
    callback: ProgressCallback | None,
    fraction: float,
    stage: str,
) -> None:
    if callback is not None:
        callback(max(0.0, min(1.0, float(fraction))), stage)


def _write_u32(stream: BinaryIO, value: int) -> None:
    stream.write(struct.pack("<I", value))


def _write_u64(stream: BinaryIO, value: int) -> None:
    stream.write(struct.pack("<Q", value))


def _write_f32(stream: BinaryIO, value: float) -> None:
    if not math.isfinite(value):
        raise ValueError("SKATE does not permit non-finite floats")
    stream.write(struct.pack("<f", value))


def _write_vec(stream: BinaryIO, value) -> None:
    for component in value:
        _write_f32(stream, float(component))


def _write_string(stream: BinaryIO, value: str) -> None:
    encoded = value.encode("utf-8")
    _write_u32(stream, len(encoded))
    stream.write(encoded)


def _write_stored_bytes(
    stream: BinaryIO,
    decoded: bytes,
) -> tuple[int, int]:
    candidates = [
        (STORAGE_DEFLATE, zlib.compress(decoded, level=9)),
    ]
    if zstd is not None:
        candidates.append((STORAGE_ZSTD, zstd.compress(decoded, level=10)))
    method, payload = min(candidates, key=lambda item: len(item[1]))
    if len(payload) >= len(decoded):
        method, payload = STORAGE_RAW, decoded
    _write_u32(stream, method)
    _write_u32(stream, len(payload))
    stream.write(payload)
    return method, len(payload)


def _write_transformed_bytes(
    stream: BinaryIO,
    decoded: bytes,
    transformed: bytes,
    *,
    zstd_method: int,
    deflate_method: int,
) -> tuple[int, int]:
    if COMPAT14:
        return _write_stored_bytes(stream, decoded)
    prefix = struct.pack("<I", len(transformed))
    candidates = [
        (
            deflate_method,
            prefix + zlib.compress(transformed, level=9),
        ),
        (STORAGE_DEFLATE, zlib.compress(decoded, level=9)),
    ]
    if zstd is not None:
        candidates.extend(
            (
                (
                    zstd_method,
                    prefix + zstd.compress(transformed, level=10),
                ),
                (STORAGE_ZSTD, zstd.compress(decoded, level=10)),
            )
        )
    method, payload = min(candidates, key=lambda item: len(item[1]))
    if len(payload) >= len(decoded):
        method, payload = STORAGE_RAW, decoded
    _write_u32(stream, method)
    _write_u32(stream, len(payload))
    stream.write(payload)
    return method, len(payload)


def _filter_rgba8(decoded: bytes, width: int, height: int) -> bytes:
    row_size = width * 4
    if len(decoded) != row_size * height:
        raise ValueError("RGBA8 texture dimensions do not match its bytes")
    output = bytearray(struct.pack("<II", width, height))
    previous = bytes(row_size)
    for row_index in range(height):
        row = decoded[row_index * row_size : (row_index + 1) * row_size]
        if numpy is not None and row_size:
            current_u8 = numpy.frombuffer(row, dtype=numpy.uint8)
            current = current_u8.astype(numpy.int16)
            above = numpy.frombuffer(previous, dtype=numpy.uint8).astype(
                numpy.int16
            )
            left = numpy.zeros(row_size, dtype=numpy.int16)
            left[4:] = current[:-4]
            candidates = (
                current_u8,
                ((current - left) & 0xFF).astype(numpy.uint8),
                ((current - above) & 0xFF).astype(numpy.uint8),
                (
                    (
                        current
                        - ((left.astype(numpy.int32) + above) // 2)
                    )
                    & 0xFF
                ).astype(numpy.uint8),
            )
            scores = [
                int(
                    numpy.minimum(
                        candidate.astype(numpy.uint16),
                        256 - candidate.astype(numpy.uint16),
                    ).sum()
                )
                for candidate in candidates
            ]
            filter_type = min(range(len(scores)), key=scores.__getitem__)
            filtered = candidates[filter_type].tobytes()
        else:
            sub = bytes(
                (
                    value - (row[index - 4] if index >= 4 else 0)
                )
                & 0xFF
                for index, value in enumerate(row)
            )
            up = bytes(
                (value - previous[index]) & 0xFF
                for index, value in enumerate(row)
            )
            options = (row, sub, up)
            scores = [
                sum(min(value, 256 - value) for value in option)
                for option in options
            ]
            filter_type = min(range(len(scores)), key=scores.__getitem__)
            filtered = options[filter_type]
        output.append(filter_type)
        output.extend(filtered)
        previous = row
    return bytes(output)


def _vertex_soa(decoded: bytes) -> bytes:
    vertex_size = 56
    if len(decoded) % vertex_size:
        raise ValueError("visual vertex block is not SKATE15-compatible")
    fields = ((0, 12), (12, 12), (24, 8), (32, 8), (40, 4),
              (44, 8), (52, 4))
    output = bytearray(len(decoded))
    offset = 0
    for field_offset, field_size in fields:
        for vertex in range(0, len(decoded), vertex_size):
            output[offset : offset + field_size] = decoded[
                vertex + field_offset : vertex + field_offset + field_size
            ]
            offset += field_size
    return bytes(output)


def _index_delta_varints(decoded: bytes) -> bytes:
    if len(decoded) % 4:
        raise ValueError("visual index block is not u32-aligned")
    output = bytearray()
    previous = 0
    for (current,) in struct.iter_unpack("<I", decoded):
        delta = current - previous
        zigzag = (delta << 1) if delta >= 0 else ((-delta << 1) - 1)
        while zigzag >= 0x80:
            output.append((zigzag & 0x7F) | 0x80)
            zigzag >>= 7
        output.append(zigzag)
        previous = current
    return bytes(output)


def _indexed_collision(decoded: bytes) -> bytes:
    triangle = struct.Struct("<9fII4B")
    if len(decoded) % triangle.size:
        raise ValueError("collision block is not SKATE15-compatible")
    vertices: list[bytes] = []
    vertex_indices: dict[bytes, int] = {}
    records = bytearray()
    for values in triangle.iter_unpack(decoded):
        packed_vertices = struct.pack("<9f", *values[:9])
        for corner in range(3):
            packed = packed_vertices[corner * 12 : corner * 12 + 12]
            index = vertex_indices.get(packed)
            if index is None:
                index = len(vertices)
                vertex_indices[packed] = index
                vertices.append(packed)
            records.extend(struct.pack("<I", index))
        records.extend(struct.pack("<II4B", *values[9:]))
    return (
        struct.pack("<I", len(vertices))
        + b"".join(vertices)
        + bytes(records)
    )


def _write_stored_chunks(
    stream: BinaryIO,
    chunks: list[bytes],
) -> tuple[int, int]:
    return _write_stored_bytes(stream, b"".join(chunks))


def _map_object_extension(
    geometry: PackedVisualGeometry,
    collision_ranges: dict[int, tuple[int, int]],
    rails: list[ExportGrindRail],
    export_editable_objects: bool,
) -> bytes:
    payload = io.BytesIO()
    editable_objects = (
        [
            record
            for record in geometry.objects
            if record.editor_editable
        ]
        if export_editable_objects
        else []
    )
    _write_u32(payload, len(editable_objects))
    for record in editable_objects:
        first_collision, collision_count = collision_ranges.get(
            record.source_identity, (0, 0)
        )
        _write_u32(payload, record.object_id)
        _write_string(payload, record.name)
        _write_vec(payload, record.origin)
        _write_u32(payload, record.first_index)
        _write_u32(payload, record.index_count)
        _write_u32(payload, first_collision)
        _write_u32(payload, collision_count)
        grind_indices = [
            index
            for index, rail in enumerate(rails)
            if rail.parent_source_identity == record.source_identity
        ]
        _write_u32(payload, len(grind_indices))
        for grind_index in grind_indices:
            _write_u32(payload, grind_index)
        _write_u32(payload, record.physics_type)
        _write_u32(payload, record.collision_shape)
        _write_f32(payload, record.density)
        _write_f32(payload, record.friction)
        _write_f32(payload, record.restitution)
        _write_f32(payload, record.linear_damping)
        _write_f32(payload, record.angular_damping)
        _write_f32(payload, record.gravity_scale)
        _write_u32(payload, 1 if record.enable_sleep else 0)
        _write_u32(payload, 1 if record.initially_awake else 0)
    return payload.getvalue()

def _break_group_extension(geometry: PackedVisualGeometry) -> bytes:
    payload = io.BytesIO()
    records = [
        record for record in geometry.objects
        if record.editor_editable and record.break_group != 0
    ]
    _write_u32(payload, len(records))
    for record in records:
        _write_u32(payload, record.object_id)
        _write_u32(payload, record.break_group)
        _write_f32(payload, record.break_speed_threshold)
        _write_f32(payload, record.break_impulse_scale)
        _write_f32(payload, record.break_angular_impulse)
        _write_f32(payload, record.break_gravity_scale)
    return payload.getvalue()


def _cache_manifest_path(output: Path) -> Path:
    return output.with_name(output.name + ".export-cache.json")


def _hash_text(digest, value: object) -> None:
    encoded = str(value).encode("utf-8")
    digest.update(struct.pack("<I", len(encoded)))
    digest.update(encoded)


def _hash_floats(digest, values) -> None:
    for value in values:
        digest.update(struct.pack("<f", float(value)))


def _hash_foreach(
    digest,
    collection,
    property_name: str,
    component_count: int,
    typecode: str,
) -> None:
    count = len(collection) * component_count
    if typecode == "f":
        values = array("f", [0.0]) * count
    else:
        values = array(typecode, [0]) * count
    if count:
        collection.foreach_get(property_name, values)
    if sys.byteorder != "little":
        values.byteswap()
    digest.update(values.tobytes())


def _hash_matrix(digest, matrix) -> None:
    for row in matrix:
        _hash_floats(digest, row)


def _hash_image_source(digest, image: bpy.types.Image) -> None:
    _hash_text(digest, image.name)
    _hash_text(digest, image.source)
    _hash_text(digest, image.colorspace_settings.name)
    digest.update(struct.pack("<II", int(image.size[0]), int(image.size[1])))
    raw_path = image.filepath_raw or image.filepath
    resolved = Path(bpy.path.abspath(raw_path)).resolve() if raw_path else None
    if resolved is not None and resolved.is_file() and image.packed_file is None:
        stat = resolved.stat()
        _hash_text(digest, os.path.normcase(str(resolved)))
        digest.update(struct.pack("<QQ", stat.st_size, stat.st_mtime_ns))
        return

    packed = image.packed_file
    if packed is not None and not image.is_dirty:
        # Clean packed images decode deterministically from these source
        # bytes. Hashing the packed payload avoids expanding large texture
        # sets to float RGBA solely for an incremental-cache check.
        _hash_text(digest, "PACKED_SOURCE")
        digest.update(struct.pack("<Q", int(packed.size)))
        digest.update(packed.data)
        return

    # Generated, baked, and dirty packed images have no trustworthy immutable
    # source fingerprint. Hash their actual pixels so cache reuse is correct.
    expected = int(image.size[0]) * int(image.size[1]) * 4
    values = array("f", [0.0]) * expected
    if expected:
        image.pixels.foreach_get(values)
    if sys.byteorder != "little":
        values.byteswap()
    digest.update(values.tobytes())


def _mesh_for_export(
    source_object: bpy.types.Object,
    depsgraph,
    *,
    preserve_all_data_layers: bool = False,
) -> tuple[bpy.types.Mesh, bpy.types.Object | None]:
    # Blender 5.1 can keep newly-created UV layers off an already-built
    # evaluated mesh even after the source datablock is tagged and the view
    # layer is refreshed. An unmodified mesh does not need evaluation, so use
    # its authoritative source data directly. Modified or shape-keyed meshes
    # still use Blender's evaluated result.
    source_mesh = source_object.data
    if not source_object.modifiers and source_mesh.shape_keys is None:
        return source_mesh, None

    evaluated = source_object.evaluated_get(depsgraph)
    mesh = (
        evaluated.to_mesh(
            preserve_all_data_layers=True, depsgraph=depsgraph
        )
        if preserve_all_data_layers
        else evaluated.to_mesh()
    )
    if mesh is None:
        raise ValueError(
            f"Blender could not evaluate mesh {source_object.name!r}"
        )
    return mesh, evaluated


def _visual_uv_layers(
    mesh: bpy.types.Mesh, source_name: str
) -> tuple[bpy.types.MeshUVLoopLayer, bpy.types.MeshUVLoopLayer]:
    uv0 = mesh.uv_layers.get("UVMap")
    if uv0 is None:
        raise ValueError(
            f"visual mesh {source_name!r} requires a UVMap UV layer"
        )
    # The package always stores two UV streams, but unlit maps do not need
    # authors to manufacture a duplicate Blender lightmap layer.
    return uv0, mesh.uv_layers.get("Lightmap") or uv0


def _retail_world_frame_attributes(mesh):
    normal = mesh.attributes.get(RETAIL_NORMAL_ATTRIBUTE)
    tangent = mesh.attributes.get(RETAIL_TANGENT_ATTRIBUTE)
    legacy_tangent = mesh.attributes.get(LEGACY_RETAIL_TANGENT_ATTRIBUTE)
    handedness = mesh.attributes.get(RETAIL_HANDEDNESS_ATTRIBUTE)
    if all(
        attribute is None
        for attribute in (normal, tangent, legacy_tangent, handedness)
    ):
        return None
    if tangent is not None and legacy_tangent is not None:
        raise ValueError(
            f"mesh {mesh.name!r} has both "
            f"{RETAIL_TANGENT_ATTRIBUTE!r} and the legacy "
            f"{LEGACY_RETAIL_TANGENT_ATTRIBUTE!r}; remove the legacy "
            "attribute"
        )
    tangent = tangent if tangent is not None else legacy_tangent
    names = (
        RETAIL_NORMAL_ATTRIBUTE,
        RETAIL_TANGENT_ATTRIBUTE,
        RETAIL_HANDEDNESS_ATTRIBUTE,
    )
    attributes = (normal, tangent, handedness)
    if any(attribute is None for attribute in attributes):
        missing = [
            name
            for name, attribute in zip(names, attributes, strict=True)
            if attribute is None
        ]
        raise ValueError(
            f"mesh {mesh.name!r} has an incomplete retail world frame; "
            f"missing {', '.join(missing)}"
        )
    expected = (
        (normal, "POINT", "FLOAT_VECTOR"),
        (tangent, "POINT", "FLOAT_VECTOR"),
        (handedness, "POINT", "FLOAT"),
    )
    for attribute, domain, data_type in expected:
        if (
            attribute.domain != domain
            or attribute.data_type != data_type
            or len(attribute.data) != len(mesh.vertices)
        ):
            raise ValueError(
                f"mesh {mesh.name!r} retail attribute {attribute.name!r} "
                f"must be {domain}/{data_type} with one value per vertex"
            )
    return normal, tangent, handedness


def _hash_mesh(
    digest,
    source_object: bpy.types.Object,
    *,
    visual: bool,
    depsgraph,
) -> tuple[int, int]:
    mesh, evaluated = _mesh_for_export(
        source_object,
        depsgraph,
        preserve_all_data_layers=visual,
    )
    try:
        mesh.calc_loop_triangles()
        _hash_text(digest, source_object.name_full)
        _hash_matrix(digest, source_object.matrix_world)
        digest.update(
            struct.pack(
                "<4f",
                *(float(value) for value in source_object.color),
            )
        )
        digest.update(
            struct.pack(
                "<IIII",
                len(mesh.vertices),
                len(mesh.loops),
                len(mesh.polygons),
                len(mesh.loop_triangles),
            )
        )
        _hash_foreach(digest, mesh.vertices, "co", 3, "f")
        _hash_foreach(digest, mesh.loops, "vertex_index", 1, "I")
        _hash_foreach(digest, mesh.polygons, "loop_start", 1, "I")
        _hash_foreach(digest, mesh.polygons, "loop_total", 1, "I")
        _hash_foreach(digest, mesh.polygons, "material_index", 1, "I")
        for material in mesh.materials:
            _hash_text(digest, material.name if material else "")

        if visual:
            for property_name in (
                "ow_export_visual",
                "ow_editor_editable",
                "ow_physics_type",
                "ow_box3d_collision_shape",
                "ow_box3d_density",
                "ow_box3d_friction",
                "ow_box3d_restitution",
                "ow_box3d_linear_damping",
                "ow_box3d_angular_damping",
                "ow_box3d_gravity_scale",
                "ow_box3d_enable_sleep",
                "ow_box3d_initially_awake",
                "ow_box3d_break_group",
                "ow_box3d_break_speed_threshold",
                "ow_box3d_break_impulse_scale",
                "ow_box3d_break_angular_impulse",
                "ow_box3d_break_gravity_scale",
                "ow_hinge_position",
                "ow_hinge_axis",
                "ow_door_min_angle_degrees",
                "ow_door_max_angle_degrees",
                "ow_door_initial_angle_degrees",
                "ow_door_mass",
                "ow_door_angular_damping",
                "ow_door_return_spring_strength",
                "ow_door_maximum_angular_speed",
                "ow_door_contact_impulse_scale",
                "ow_door_friction",
                "ow_door_restitution",
                "ow_door_collision_material",
            ):
                _hash_text(
                    digest,
                    repr(source_object.get(property_name, None)),
                )
            uv0, uv1 = _visual_uv_layers(mesh, source_object.name)
            _hash_foreach(digest, mesh.loops, "normal", 3, "f")
            for uv_layer in sorted(
                mesh.uv_layers, key=lambda layer: layer.name.casefold()
            ):
                _hash_text(digest, uv_layer.name)
                _hash_foreach(digest, uv_layer.data, "uv", 2, "f")
            retail_frame = _retail_world_frame_attributes(mesh)
            if retail_frame is not None:
                normal, tangent, handedness = retail_frame
                _hash_text(digest, "RETAIL_WORLD_FRAME")
                _hash_foreach(digest, normal.data, "vector", 3, "f")
                _hash_foreach(digest, tangent.data, "vector", 3, "f")
                _hash_foreach(digest, handedness.data, "value", 1, "f")
        else:
            _hash_text(digest, source_object.get("ow_material", ""))
            _hash_text(
                digest,
                int(bool(source_object.get("ow_upward_surface", False))),
            )
        triangle_count = len(mesh.loop_triangles)
        return triangle_count * 3, triangle_count
    finally:
        if evaluated is not None:
            evaluated.to_mesh_clear()


def _scene_content_fingerprint(
    visual_objects: list[bpy.types.Object],
    collision_objects: list[bpy.types.Object],
    grind_objects: list[bpy.types.Object],
    npc_path_objects: list[bpy.types.Object],
    materials: list[bpy.types.Material],
    images: list[bpy.types.Image],
    collision_triangle_count: int,
    export_editable_objects: bool,
    progress: ProgressCallback | None = None,
) -> SceneContentFingerprint:
    started = time.perf_counter()
    print(
        "SKATE cache: fingerprinting scene content",
        f"visual_objects={len(visual_objects)}",
        f"collision_objects={len(collision_objects)}",
        flush=True,
    )
    digest = hashlib.sha256()
    digest.update(f"SKATE_EXPORT_CACHE_{CACHE_SCHEMA}".encode("ascii"))
    _hash_text(
        digest,
        f"EXPORT_EDITABLE_OBJECTS={int(export_editable_objects)}",
    )
    _hash_text(
        digest,
        "DYNAMIC_LIGHTING_DEFAULT="
        + str(
            int(
                bool(
                    bpy.context.scene.get(
                        "ow_dynamic_lighting_enabled_by_default", True
                    )
                )
            )
        ),
    )

    material_properties = (
        "ow_flags",
        "ow_friction",
        "ow_restitution",
        "ow_display_color",
        "ow_roughness",
        "ow_emissive",
        "ow_albedo_image",
        "ow_secondary_albedo_image",
        "ow_blend_mask_image",
        "ow_lightmap_image",
        "ow_lightmap_encoding",
        "ow_baked_strength",
        "ow_normal_image",
        "ow_orm_image",
        "ow_emissive_image",
        "ow_alpha_mode",
        "ow_alpha_cutoff",
        "ow_blend_factor",
        "ow_blend_mask_channel",
        "ow_albedo_address_mode",
        "ow_secondary_address_mode",
        "ow_blend_mask_address_mode",
        "ow_cull_mode",
        "ow_uv_map",
        "ow_secondary_uv_map",
        "ow_blend_mask_uv_map",
        "ow_uv_transform",
        "ow_secondary_uv_transform",
        "ow_blend_mask_uv_transform",
        "ow_audio_surface",
        "ow_physics_surface",
        "ow_surface_pattern",
        "ow_depth_layer",
        "ow_collision_enabled",
        "skate3_shader_name",
        "skate3_retail_material_guid",
        "skate3_retail_material_handle",
        "skate3_retail_material_group_index",
        "skate3_retail_texture_ids",
        "skate3_retail_parameters",
        "skate3_retail_source",
    )
    for material in materials:
        _hash_text(digest, material.name)
        _hash_floats(digest, material.diffuse_color)
        for property_name in material_properties:
            _hash_text(digest, property_name)
            _hash_text(digest, repr(material.get(property_name, None)))
    retail_manifest = bpy.data.texts.get("SKATE3_RETAIL_MANIFEST")
    if retail_manifest is not None:
        _hash_text(digest, retail_manifest.as_string())
    for image_index, image in enumerate(images, start=1):
        _hash_image_source(digest, image)
        _report_progress(
            progress,
            0.15 * image_index / max(1, len(images)),
            f"Hashing textures ({image_index}/{len(images)}): "
            f"{image.name}",
        )

    depsgraph = bpy.context.evaluated_depsgraph_get()
    total_objects = max(
        1,
        len(visual_objects)
        + len(collision_objects)
        + len(grind_objects)
        + len(npc_path_objects)
        + len(_visible_local_light_objects()),
    )
    processed_objects = 0

    def object_complete(label: str) -> None:
        nonlocal processed_objects
        processed_objects += 1
        _report_progress(
            progress,
            0.15 + 0.85 * processed_objects / total_objects,
            label,
        )

    visual_vertices = 0
    visual_indices = 0
    hinged_doors = 0
    for obj in visual_objects:
        if obj.type != "MESH":
            continue
        object_vertices, _ = _hash_mesh(
            digest, obj, visual=True, depsgraph=depsgraph
        )
        if str(obj.get("ow_physics_type", "STATIC")) == "HINGED_DOOR":
            hinged_doors += 1
        else:
            visual_vertices += object_vertices
            visual_indices += object_vertices
        object_complete(f"Hashing visuals: {obj.name}")

    for obj in collision_objects:
        if obj.type != "MESH":
            continue
        material = bpy.data.materials.get(str(obj.get("ow_material", "")))
        if material is not None and not bool(
            material.get("ow_collision_enabled", True)
        ):
            continue
        _hash_text(
            digest,
            int(
                bool(
                    obj.get(
                        "ow_preserve_opposite_wound_collision",
                        False,
                    )
                )
            ),
        )
        preserve_retail_codes = bool(
            obj.get("ow_preserve_retail_edge_codes", False)
        )
        _hash_text(digest, int(preserve_retail_codes))
        editor_owner = obj.data.attributes.get(
            EDITOR_COLLISION_OWNER_ATTRIBUTE
        )
        _hash_text(digest, EDITOR_COLLISION_OWNER_ATTRIBUTE)
        if editor_owner is None:
            _hash_text(digest, "MISSING")
        else:
            _hash_text(digest, editor_owner.domain)
            _hash_text(digest, editor_owner.data_type)
            _hash_foreach(
                digest,
                editor_owner.data,
                "value",
                1,
                "i",
            )
        if preserve_retail_codes:
            for attribute_name in RETAIL_EDGE_CODE_ATTRIBUTES:
                attribute = obj.data.attributes.get(attribute_name)
                _hash_text(digest, attribute_name)
                if attribute is None:
                    _hash_text(digest, "MISSING")
                else:
                    _hash_text(digest, attribute.domain)
                    _hash_text(digest, attribute.data_type)
                    _hash_foreach(
                        digest,
                        attribute.data,
                        "value",
                        1,
                        "i",
                    )
        _hash_mesh(
            digest, obj, visual=False, depsgraph=depsgraph
        )
        object_complete(f"Hashing collision: {obj.name}")

    grind_rails = 0
    for obj in grind_objects:
        if obj.type != "CURVE":
            continue
        _hash_text(digest, obj.name_full)
        _hash_matrix(digest, obj.matrix_world)
        for spline in obj.data.splines:
            _hash_text(digest, spline.type)
            _hash_text(digest, int(bool(spline.use_cyclic_u)))
            if spline.type == "POLY":
                _hash_foreach(digest, spline.points, "co", 4, "f")
                if len(spline.points) >= 2:
                    grind_rails += 1
            elif spline.type == "BEZIER":
                _hash_foreach(digest, spline.bezier_points, "co", 3, "f")
                _hash_foreach(
                    digest, spline.bezier_points, "handle_left", 3, "f"
                )
                _hash_foreach(
                    digest, spline.bezier_points, "handle_right", 3, "f"
                )
                if len(spline.bezier_points) >= 2:
                    grind_rails += 1
        for property_name in (
            "skate3_retail_grind",
            "skate3_retail_grind_spline_id",
            "skate3_retail_grind_type_signature",
            "skate3_retail_grind_flags",
            "skate3_retail_grind_trailing_word",
            "skate3_retail_grind_segment_count",
            "skate3_retail_grind_segment_payload",
        ):
            _hash_text(digest, repr(obj.get(property_name, None)))
        _hash_text(
            digest,
            obj.parent.name_full
            if obj.parent is not None and obj.parent.type == "MESH"
            else "",
        )
        object_complete(f"Hashing grind paths: {obj.name}")

    npc_routes = 0
    for obj in npc_path_objects:
        if obj.type != "CURVE":
            continue
        _hash_text(digest, obj.name_full)
        _hash_matrix(digest, obj.matrix_world)
        for property_name, default in (
            ("ow_npc_skater_count", 1),
            ("ow_npc_speed", 5.5),
            ("ow_npc_spawn_spacing", 3.0),
        ):
            _hash_text(digest, repr(obj.get(property_name, default)))
        for spline in obj.data.splines:
            _hash_text(digest, spline.type)
            _hash_text(digest, int(bool(spline.use_cyclic_u)))
            if spline.type == "POLY":
                _hash_foreach(digest, spline.points, "co", 4, "f")
                if len(spline.points) >= 2:
                    npc_routes += 1
            elif spline.type == "BEZIER":
                _hash_foreach(digest, spline.bezier_points, "co", 3, "f")
                if len(spline.bezier_points) >= 2:
                    npc_routes += 1
        object_complete(f"Hashing NPC paths: {obj.name}")

    local_lights = 0
    scene_eevee = getattr(bpy.context.scene, "eevee", None)
    _hash_text(
        digest,
        repr(float(getattr(scene_eevee, "light_threshold", 0.01))),
    )
    for obj in _visible_local_light_objects():
        light = obj.data
        local_lights += 1
        _hash_text(digest, obj.name_full)
        _hash_matrix(digest, obj.matrix_world)
        for value in (
            light.type,
            tuple(light.color),
            light.energy,
            bool(getattr(light, "use_custom_distance", False)),
            light.cutoff_distance,
            light.shadow_soft_size,
            getattr(light, "diffuse_factor", 1.0),
            getattr(light, "specular_factor", 1.0),
            getattr(light, "volume_factor", 1.0),
            getattr(light, "size", 0.0),
            getattr(light, "size_y", 0.0),
            getattr(light, "shape", ""),
            getattr(light, "spot_size", 0.0),
            getattr(light, "spot_blend", 0.0),
        ):
            _hash_text(digest, repr(value))
        object_complete(f"Hashing lights: {obj.name}")

    result = SceneContentFingerprint(
        digest=digest.hexdigest(),
        visual_vertices=visual_vertices,
        visual_indices=visual_indices,
        collision_triangles=collision_triangle_count,
        grind_rails=grind_rails,
        npc_routes=npc_routes,
        hinged_doors=hinged_doors,
        local_lights=local_lights,
    )
    print(
        "SKATE cache: fingerprint complete",
        f"seconds={time.perf_counter() - started:.3f}",
        f"digest={result.digest[:16]}",
        flush=True,
    )
    return result


def _visible_light_objects() -> list[bpy.types.Object]:
    return sorted(
        (
            obj
            for obj in bpy.data.objects
            if obj.type == "LIGHT"
            and not obj.hide_render
            and math.isfinite(float(obj.data.energy))
            and float(obj.data.energy) > 0.0
        ),
        key=lambda item: item.name_full,
    )


def _visible_local_light_objects() -> list[bpy.types.Object]:
    return [
        obj
        for obj in _visible_light_objects()
        if obj.data.type in {"POINT", "SPOT", "AREA"}
    ]


def _sun_metadata() -> tuple[tuple[float, float, float], float, float]:
    scene = bpy.context.scene
    sun_color = tuple(scene.get("ow_sun_color", (1.0, 0.92, 0.78)))
    sun_intensity = float(scene.get("ow_sun_intensity", 1.25))
    orbit_azimuth = float(scene.get("ow_orbit_azimuth", 0.62))
    suns = [
        obj for obj in _visible_light_objects() if obj.data.type == "SUN"
    ]
    if not suns:
        return sun_color, sun_intensity, orbit_azimuth

    sun = suns[0]
    emitted = sun.matrix_world.to_quaternion() @ Vector((0.0, 0.0, -1.0))
    runtime_emitted = _to_runtime(emitted)
    direction_to_light = (
        -runtime_emitted[0],
        -runtime_emitted[1],
        -runtime_emitted[2],
    )
    horizontal_length = math.hypot(
        direction_to_light[0], direction_to_light[2]
    )
    if horizontal_length > 1.0e-5:
        orbit_azimuth = math.atan2(
            direction_to_light[2], direction_to_light[0]
        )
    return (
        tuple(float(value) for value in sun.data.color),
        max(0.0, float(sun.data.energy) * 0.42),
        orbit_azimuth,
    )


def _read_package_header(output: Path) -> tuple[str, int, tuple[int, ...]]:
    with output.open("rb") as stream:
        if stream.read(len(MAGIC)) != MAGIC:
            raise ValueError(f"{output} is not an SKATE v15 package")
        marker = struct.unpack("<I", stream.read(4))[0]
        if marker != ENDIAN_MARKER:
            raise ValueError(f"{output} has an invalid endian marker")
        name_length = struct.unpack("<I", stream.read(4))[0]
        package_name = stream.read(name_length).decode("utf-8")
        metadata_offset = stream.tell()
        stream.seek(METADATA_BYTE_COUNT, os.SEEK_CUR)
        counts = struct.unpack("<9I", stream.read(36))
    return package_name, metadata_offset, counts


def _package_static_digest(
    output: Path,
    metadata_offset: int,
    world_config_value_offset: int | None = None,
) -> str:
    digest = hashlib.sha256()
    excluded_ranges = [
        (metadata_offset, metadata_offset + METADATA_BYTE_COUNT),
    ]
    if world_config_value_offset is not None:
        excluded_ranges.append(
            (world_config_value_offset, world_config_value_offset + 4)
        )
    excluded_ranges.sort()
    with output.open("rb") as stream:
        cursor = 0
        for excluded_start, excluded_end in excluded_ranges:
            remaining = excluded_start - cursor
            if remaining < 0:
                raise ValueError("SKATE cache exclusions overlap")
            while remaining:
                data = stream.read(min(8 * 1024 * 1024, remaining))
                if not data:
                    raise ValueError(
                        f"{output} ended before its cached metadata"
                    )
                digest.update(data)
                remaining -= len(data)
            stream.seek(excluded_end - excluded_start, os.SEEK_CUR)
            cursor = excluded_end
        while True:
            data = stream.read(8 * 1024 * 1024)
            if not data:
                break
            digest.update(data)
    return digest.hexdigest()


def _scene_metadata(output: Path) -> tuple[str, bytes]:
    spawn = bpy.data.objects.get(SPAWN_OBJECT)
    if spawn is None:
        raise ValueError(f"required spawn object is missing: {SPAWN_OBJECT}")
    scene = bpy.context.scene
    map_name = str(scene.get("ow_map_name", output.stem))
    sun_color, sun_intensity, orbit_azimuth = _sun_metadata()
    values = (
        *_to_runtime(spawn.matrix_world.translation),
        _spawn_heading(spawn),
        *tuple(scene.get("ow_sky_zenith", (0.09, 0.34, 0.72))),
        *tuple(scene.get("ow_sky_horizon", (0.58, 0.78, 0.98))),
        *tuple(scene.get("ow_sky_nadir", (0.18, 0.25, 0.34))),
        float(scene.get("ow_cycle_seconds", 96.0)),
        float(scene.get("ow_start_hour", 9.0)),
        orbit_azimuth,
        float(scene.get("ow_end_hour", 17.0)),
        1.0 if bool(scene.get("ow_cycle_ping_pong", False)) else 0.0,
        *tuple(scene.get("ow_twilight_zenith", (0.045, 0.10, 0.26))),
        *tuple(scene.get("ow_twilight_horizon", (1.0, 0.32, 0.10))),
        *tuple(scene.get("ow_twilight_nadir", (0.05, 0.035, 0.06))),
        *tuple(scene.get("ow_night_zenith", (0.007, 0.015, 0.045))),
        *tuple(scene.get("ow_night_horizon", (0.045, 0.085, 0.17))),
        *tuple(scene.get("ow_night_nadir", (0.008, 0.014, 0.032))),
        *sun_color,
        *tuple(scene.get("ow_moon_color", (0.42, 0.56, 0.92))),
        sun_intensity,
        float(scene.get("ow_moon_intensity", 0.18)),
        float(scene.get("ow_day_ambient", 0.32)),
        float(scene.get("ow_night_ambient", 0.11)),
        *tuple(scene.get("ow_sky_tint", (1.0, 1.0, 1.0))),
    )
    if len(values) != METADATA_FLOAT_COUNT or not all(
        math.isfinite(float(value)) for value in values
    ):
        raise ValueError(
            f"SKATE metadata must contain {METADATA_FLOAT_COUNT} finite floats"
        )
    return map_name, struct.pack("<49f", *(float(value) for value in values))


def _patch_package_metadata(output: Path, manifest: dict | None = None) -> None:
    map_name, metadata = _scene_metadata(output)
    package_name, metadata_offset, _ = _read_package_header(output)
    if package_name != map_name:
        raise ValueError(
            "metadata-only export cannot change the map name; use a full "
            f"export ({package_name!r} != {map_name!r})"
        )
    with output.open("r+b") as stream:
        stream.seek(metadata_offset)
        stream.write(metadata)
        if manifest is not None:
            world_config_value_offset = manifest.get(
                "world_config_value_offset"
            )
            if not isinstance(world_config_value_offset, int):
                raise ValueError(
                    "metadata-only export requires a fresh SKATE v15 cache; "
                    "run one normal export"
                )
            stream.seek(world_config_value_offset - 20)
            if stream.read(20) != struct.pack(
                "<4sIIII", b"WCFG", 1, 4, STORAGE_RAW, 4
            ):
                raise ValueError(
                    "cached dynamic-lighting metadata is invalid; "
                    "run one normal export"
                )
            _write_u32(
                stream,
                1
                if bool(
                    bpy.context.scene.get(
                        "ow_dynamic_lighting_enabled_by_default", True
                    )
                )
                else 0,
            )
        stream.flush()
        os.fsync(stream.fileno())


def _load_cache_manifest(output: Path) -> dict | None:
    path = _cache_manifest_path(output)
    if not path.is_file():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def _write_cache_manifest(
    output: Path,
    fingerprint: SceneContentFingerprint,
    material_count: int,
    texture_count: int,
    world_config_value_offset: int | None = None,
) -> None:
    package_name, metadata_offset, counts = _read_package_header(output)
    output_stat = output.stat()
    manifest = {
        "schema": CACHE_SCHEMA,
        "package_name": package_name,
        "output_length": output_stat.st_size,
        "output_mtime_ns": output_stat.st_mtime_ns,
        "static_sha256": _package_static_digest(
            output, metadata_offset, world_config_value_offset
        ),
        "content_sha256": fingerprint.digest,
        "material_count": material_count,
        "texture_count": texture_count,
        "visual_vertex_count": counts[2],
        "visual_index_count": counts[3],
        "collision_triangle_count": fingerprint.collision_triangles,
        "grind_rail_count": fingerprint.grind_rails,
        "npc_route_count": fingerprint.npc_routes,
        "hinged_door_count": fingerprint.hinged_doors,
        "local_light_count": fingerprint.local_lights,
        "package_counts": list(counts),
    }
    if world_config_value_offset is not None:
        manifest["world_config_value_offset"] = world_config_value_offset
    path = _cache_manifest_path(output)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def _refresh_manifest_file_state(output: Path, manifest: dict) -> None:
    output_stat = output.stat()
    manifest["output_length"] = output_stat.st_size
    manifest["output_mtime_ns"] = output_stat.st_mtime_ns
    path = _cache_manifest_path(output)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def _manifest_matches_package(output: Path, manifest: dict) -> bool:
    if (
        manifest.get("schema") != CACHE_SCHEMA
        or not output.is_file()
        or output.stat().st_size != manifest.get("output_length")
    ):
        return False
    try:
        package_name, metadata_offset, counts = _read_package_header(output)
    except (OSError, UnicodeError, ValueError, struct.error):
        return False
    if (
        package_name != manifest.get("package_name")
        or list(counts) != manifest.get("package_counts")
    ):
        return False
    # The exporter records file identity after every metadata patch. An
    # unchanged size/mtime pair means the already-verified static payload has
    # not been touched, so rereading a 1.2 GiB package would add no value.
    if output.stat().st_mtime_ns == manifest.get("output_mtime_ns"):
        return True
    return (
        _package_static_digest(
            output,
            metadata_offset,
            manifest.get("world_config_value_offset"),
        )
        == manifest.get("static_sha256")
    )


def _to_runtime(value) -> tuple[float, float, float]:
    return float(value.x), float(value.z), float(-value.y)


def _stable_object_id(name: str) -> int:
    """Return a deterministic non-zero FNV-1a ID for one Blender object."""
    value = 2166136261
    for byte in name.encode("utf-8"):
        value ^= byte
        value = (value * 16777619) & 0xFFFFFFFF
    return value or 1


def _box3d_properties(
    source_object: bpy.types.Object,
) -> tuple:
    physics_name = str(source_object.get("ow_physics_type", "STATIC"))
    physics_type = {
        "STATIC": 0,
        "PRESENTATION_ONLY": 0,
        "BOX3D_STATIC": 1,
        "BOX3D_DYNAMIC": 2,
    }.get(physics_name)
    if physics_type is None:
        # Hinged doors are exported through their existing dedicated runtime
        # path and never become MOBJ records.
        if physics_name == "HINGED_DOOR":
            return (
                0, 0, 100.0, 0.55, 0.05, 0.05, 0.15, 1.0, True, True,
                0, 2.5, 0.45, 0.08, 1.0,
            )
        raise ValueError(
            f"object {source_object.name!r} has unknown physics mode "
            f"{physics_name!r}"
        )

    shape_name = str(
        source_object.get("ow_box3d_collision_shape", "BOX")
    )
    collision_shape = {
        "BOX": 0,
        "SPHERE": 1,
        "CONVEX_HULL": 2,
    }.get(shape_name)
    if collision_shape is None:
        raise ValueError(
            f"object {source_object.name!r} has unknown Box3D collision "
            f"shape {shape_name!r}"
        )

    values = (
        float(source_object.get("ow_box3d_density", 100.0)),
        float(source_object.get("ow_box3d_friction", 0.55)),
        float(source_object.get("ow_box3d_restitution", 0.05)),
        float(source_object.get("ow_box3d_linear_damping", 0.05)),
        float(source_object.get("ow_box3d_angular_damping", 0.15)),
        float(source_object.get("ow_box3d_gravity_scale", 1.0)),
    )
    labels_and_ranges = (
        ("density", values[0], 0.001, 100000.0),
        ("friction", values[1], 0.0, 2.0),
        ("restitution", values[2], 0.0, 1.0),
        ("linear damping", values[3], 0.0, 100.0),
        ("angular damping", values[4], 0.0, 100.0),
        ("gravity scale", values[5], -10.0, 10.0),
    )
    for label, value, minimum, maximum in labels_and_ranges:
        if not math.isfinite(value) or not minimum <= value <= maximum:
            raise ValueError(
                f"object {source_object.name!r} Box3D {label} must be "
                f"between {minimum:g} and {maximum:g}"
            )
    break_values = (
        int(source_object.get("ow_box3d_break_group", 0)),
        float(source_object.get("ow_box3d_break_speed_threshold", 2.5)),
        float(source_object.get("ow_box3d_break_impulse_scale", 0.45)),
        float(source_object.get("ow_box3d_break_angular_impulse", 0.08)),
        float(source_object.get("ow_box3d_break_gravity_scale", 1.0)),
    )
    if break_values[0] < 0 or break_values[0] > 0xFFFFFFFF:
        raise ValueError(
            f"object {source_object.name!r} break group is out of range"
        )
    break_ranges = (
        ("break speed", break_values[1], 0.1, 30.0),
        ("break impulse", break_values[2], 0.0, 10.0),
        ("break spin", break_values[3], 0.0, 10.0),
        ("released gravity", break_values[4], 0.0, 4.0),
    )
    for label, value, minimum, maximum in break_ranges:
        if not math.isfinite(value) or not minimum <= value <= maximum:
            raise ValueError(
                f"object {source_object.name!r} Box3D {label} must be "
                f"between {minimum:g} and {maximum:g}"
            )
    if break_values[0] != 0 and physics_type != 2:
        raise ValueError(
            f"object {source_object.name!r} must be Box3D Dynamic to break"
        )
    return (
        physics_type,
        collision_shape,
        *values,
        bool(source_object.get("ow_box3d_enable_sleep", True)),
        bool(source_object.get("ow_box3d_initially_awake", True)),
        *break_values,
    )


def _pack_snorm8(value: float) -> int:
    return max(-127, min(127, round(max(-1.0, min(1.0, value)) * 127.0)))


def _pack_tangent_frame(
    binormal: tuple[float, float, float] | Vector,
    handedness: float,
) -> tuple[int, int, int, int]:
    return (
        _pack_snorm8(float(binormal[0])),
        _pack_snorm8(float(binormal[1])),
        _pack_snorm8(float(binormal[2])),
        _pack_snorm8(handedness),
    )


def _export_local_lights() -> list[ExportLocalLight]:
    result: list[ExportLocalLight] = []
    type_ids = {"POINT": 0, "SPOT": 1, "AREA": 2}
    scene_eevee = getattr(bpy.context.scene, "eevee", None)
    light_threshold = max(
        1.0e-16,
        float(getattr(scene_eevee, "light_threshold", 0.01)),
    )
    for obj in _visible_local_light_objects():
        light = obj.data
        forward = obj.matrix_world.to_quaternion() @ Vector((0.0, 0.0, -1.0))
        runtime_forward = Vector(_to_runtime(forward))
        if runtime_forward.length <= 1.0e-6:
            raise ValueError(f"light {obj.name!r} has an invalid direction")
        runtime_forward.normalize()

        if light.type == "AREA":
            source_radius = 0.5 * max(
                float(getattr(light, "size", 0.0)),
                float(getattr(light, "size_y", 0.0)),
                0.02,
            )
        else:
            source_radius = max(float(light.shadow_soft_size), 0.01)
        if bool(getattr(light, "use_custom_distance", False)):
            influence_radius = float(light.cutoff_distance)
        else:
            # Match Eevee's automatic influence radius. Blender's
            # cutoff_distance remains at an inactive 40 m default when
            # Custom Distance is disabled; exporting that value made every
            # indoor light overlap neighbouring rooms.
            maximum_power = (
                max(float(value) for value in light.color)
                * abs(float(light.energy) / 100.0)
                * max(
                    float(getattr(light, "diffuse_factor", 1.0)),
                    float(getattr(light, "specular_factor", 1.0)),
                    float(getattr(light, "volume_factor", 1.0)),
                )
            )
            influence_radius = math.sqrt(
                maximum_power / light_threshold
            )
        influence_radius = max(
            influence_radius, source_radius + 0.01
        )

        inner_cosine = 1.0
        outer_cosine = 1.0
        if light.type == "SPOT":
            outer_half_angle = max(
                0.001, min(math.pi * 0.5, float(light.spot_size) * 0.5)
            )
            inner_half_angle = outer_half_angle * (
                1.0 - max(0.0, min(1.0, float(light.spot_blend)))
            )
            inner_cosine = math.cos(inner_half_angle)
            outer_cosine = math.cos(outer_half_angle)

        result.append(
            ExportLocalLight(
                name=obj.name_full,
                light_type=type_ids[light.type],
                position=_to_runtime(obj.matrix_world.translation),
                direction=tuple(float(value) for value in runtime_forward),
                color=tuple(float(value) for value in light.color),
                # Eevee derives local-light influence from power / 100 and
                # evaluates inverse-square falloff. Keep the same power scale
                # in the package; the shader applies Blender's attenuation.
                intensity=max(0.001, float(light.energy) * 0.01),
                influence_radius=influence_radius,
                source_radius=source_radius,
                spot_inner_cosine=inner_cosine,
                spot_outer_cosine=outer_cosine,
            )
        )
    return result


def _image_rgba8(
    image: bpy.types.Image,
    *,
    lightmap: bool = False,
    lightmap_encoding: str = "",
) -> bytes:
    retail_encoded = (
        lightmap_encoding == "skate3_retail_sqrt_linear_over_4"
    )
    if lightmap_encoding and not retail_encoded:
        raise ValueError(
            f"image {image.name!r} uses unsupported lightmap encoding "
            f"{lightmap_encoding!r}"
        )
    expected = image.size[0] * image.size[1] * 4
    if numpy is not None:
        # Blender bundles NumPy and foreach_get writes directly into its
        # contiguous storage. Vectorizing clamp/transfer/quantization avoids
        # billions of Python scalar operations on large imported maps.
        values = numpy.empty(expected, dtype=numpy.float32)
        image.pixels.foreach_get(values)
        values = numpy.nan_to_num(
            values, nan=0.0, posinf=1.0, neginf=0.0, copy=False
        )
        pixels = values.reshape((-1, 4))
        if lightmap and not retail_encoded:
            pixels[:, :3] = numpy.sqrt(
                numpy.clip(pixels[:, :3], 0.0, 4.0) * 0.25
            )
            pixels[:, 3] = numpy.clip(pixels[:, 3], 0.0, 1.0)
        else:
            numpy.clip(pixels, 0.0, 1.0, out=pixels)
        # Python round() and numpy.rint() both use ties-to-even, preserving
        # the byte-exact exporter contract.
        return numpy.rint(pixels * 255.0).astype(numpy.uint8).tobytes()
    values = array("f", [0.0]) * expected
    image.pixels.foreach_get(values)
    result = bytearray(expected)
    for index, value in enumerate(values):
        channel = index & 3
        linear = max(0.0, float(value))
        if lightmap and not retail_encoded and channel != 3:
            # SKATE v1 lightmaps use sqrt(linear / 4). This preserves dark
            # indirect energy that direct linear UNORM8 quantization erased,
            # while retaining headroom up to 4.0 for bright colour bounce.
            encoded = math.sqrt(min(linear, 4.0) * 0.25)
        else:
            encoded = min(linear, 1.0)
        result[index] = max(0, min(255, round(encoded * 255.0)))
    return bytes(result)


def _has_nonblank_rgb(rgba8: bytes) -> bool:
    return any(
        rgba8[index] or rgba8[index + 1] or rgba8[index + 2]
        for index in range(0, len(rgba8), 4)
    )


def _collection(name: str) -> bpy.types.Collection:
    collection = bpy.data.collections.get(name)
    if collection is None:
        raise ValueError(f"required Blender collection is missing: {name}")
    return collection


def _objects_from_collections(*names: str) -> list[bpy.types.Object]:
    result: list[bpy.types.Object] = []
    seen: set[int] = set()
    for name in names:
        collection = bpy.data.collections.get(name)
        if collection is None:
            continue
        for obj in collection.all_objects:
            identity = obj.as_pointer()
            if identity not in seen:
                seen.add(identity)
                result.append(obj)
    return result


def _spawn_heading(spawn: bpy.types.Object) -> float:
    if spawn.type == "MESH":
        return float(spawn.matrix_world.to_euler("XYZ").z)
    return float(spawn.get("ow_heading_radians", 0.0))


def _is_helper_object(obj: bpy.types.Object) -> bool:
    """Reject authoring references even when an old add-on linked them."""
    identity = f"{obj.name} {getattr(obj.data, 'name', '')}".lower()
    return any(marker in identity for marker in _HELPER_OBJECT_MARKERS)


def _used_export_materials(
    visual_objects: list[bpy.types.Object],
    collision_objects: list[bpy.types.Object],
) -> list[bpy.types.Material]:
    result: list[bpy.types.Material] = []
    seen: set[int] = set()
    for obj in [*visual_objects, *collision_objects]:
        if obj.type != "MESH":
            continue
        for slot in obj.material_slots:
            material = slot.material
            if material is None:
                continue
            identity = material.as_pointer()
            if identity not in seen:
                result.append(material)
                seen.add(identity)
    if not result:
        raise ValueError("export groups do not reference any materials")
    return result


def _object_material_tint(
    obj: bpy.types.Object,
    material: bpy.types.Material,
) -> tuple[float, float, float]:
    if not bool(material.get("ow_uses_object_color", False)):
        return 1.0, 1.0, 1.0
    return tuple(
        round(max(0.0, min(1.0, float(obj.color[index]))), 6)
        for index in range(3)
    )


def _used_export_material_variants(
    visual_objects: list[bpy.types.Object],
    collision_objects: list[bpy.types.Object],
) -> list[
    tuple[bpy.types.Material, tuple[float, float, float]]
]:
    result: list[
        tuple[bpy.types.Material, tuple[float, float, float]]
    ] = []
    seen: set[tuple[int, tuple[float, float, float]]] = set()
    for obj in visual_objects:
        if obj.type != "MESH":
            continue
        for slot in obj.material_slots:
            material = slot.material
            if material is None:
                continue
            tint = _object_material_tint(obj, material)
            key = material.as_pointer(), tint
            if key not in seen:
                seen.add(key)
                result.append((material, tint))
    for obj in collision_objects:
        if obj.type != "MESH":
            continue
        for slot in obj.material_slots:
            material = slot.material
            if material is None:
                continue
            existing = next(
                (
                    variant
                    for variant in result
                    if variant[0] == material
                ),
                None,
            )
            if existing is not None:
                continue
            tint = (1.0, 1.0, 1.0)
            seen.add((material.as_pointer(), tint))
            result.append((material, tint))
    if not result:
        raise ValueError("export groups do not reference any materials")
    return result


def _json_object_property(
    owner,
    property_name: str,
) -> dict[str, object]:
    value = owner.get(property_name, "")
    if not value:
        return {}
    if isinstance(value, str):
        parsed = json.loads(value)
    else:
        parsed = dict(value)
    if not isinstance(parsed, dict):
        raise ValueError(
            f"{owner.name!r} property {property_name!r} must contain a JSON "
            "object"
        )
    return {str(key): item for key, item in parsed.items()}


def _parse_retail_integer(value: object, bits: int) -> int:
    if value in (None, ""):
        return 0
    parsed = int(str(value), 0)
    maximum = (1 << bits) - 1
    if parsed < 0 or parsed > maximum:
        raise ValueError(f"retail integer {value!r} exceeds u{bits}")
    return parsed


def _retail_shader_family(shader_name: str) -> int:
    shader = shader_name.lower()
    if shader.startswith("environment.reflective_simple"):
        return 6
    if shader.startswith("environment.reflective_trans"):
        return 13
    if shader.startswith("environment.reflective"):
        return 5
    if shader.startswith("environment.decal_tileable"):
        return 4
    if shader.startswith("environment.decal"):
        return 3
    if shader.startswith("environment.default"):
        return 1
    if shader.startswith("environmentsimple.alphatest"):
        return 7
    if shader.startswith("environmentsimple.diffuse"):
        return 8
    if shader.startswith("environmentsimple.default"):
        return 2
    if shader.startswith("tree.default"):
        return 9
    if shader.startswith("animated.tree"):
        return 10
    if shader.startswith("proxyworld."):
        return 11
    if shader.startswith("incandescent.backlituvscroll"):
        return 14
    if shader.startswith("incandescent.default"):
        return 12
    if shader.startswith("water.flowing"):
        return 30
    if shader.startswith("ocean.default"):
        return 31
    if shader.startswith("ocean.reflection"):
        return 32
    if shader.startswith("sky."):
        return 40
    return 0


def _retail_render_flags(shader_name: str, alpha_mode: int) -> int:
    shader = shader_name.lower()
    flags = 0
    if alpha_mode == 1:
        flags |= 1
    elif alpha_mode == 2:
        flags |= 2
    if (
        shader.startswith(("tree.", "animated.tree"))
        or "alphatest" in shader
    ):
        flags |= 1 | 4
    if shader.startswith("sky."):
        flags |= 8
    if shader.startswith("environment.decal"):
        flags |= 16
    if shader.startswith("environment.decal_tileable"):
        flags |= 32
    if shader.startswith(("water.", "ocean.")):
        flags |= 64
    return flags


def _effective_alpha_mode(material: bpy.types.Material) -> int:
    authored_mode = _bounded_int(material, "ow_alpha_mode", 0, 2)
    if authored_mode != 0 or bool(material.get("ow_force_opaque", False)):
        return authored_mode

    material_name = material.name.casefold()
    image_name = str(material.get("ow_albedo_image", ""))
    image_stem = image_name.casefold().rsplit(".", 1)[0]
    alpha_named = (
        material_name.endswith("_a")
        or image_stem.endswith("_a")
        or "alpha" in material_name
    )
    image = bpy.data.images.get(image_name)
    if alpha_named and image is not None and image.channels >= 4:
        return 1
    return authored_mode


def _retail_material_data(
    material: bpy.types.Material,
    image_ids: dict[int, int],
) -> tuple[
    str,
    int,
    int,
    int,
    int,
    int,
    list[tuple[str, int, int, int, int]],
    list[tuple[str, list[str]]],
    str,
]:
    shader_name = str(
        material.get(
            "skate3_shader_name",
            material.get("ow_retail_shader_name", ""),
        )
    )
    if not shader_name:
        return "", 0, 0, 0, 0, -1, [], [], ""

    texture_ids = _json_object_property(
        material, "skate3_retail_texture_ids"
    )
    bindings: list[tuple[str, int, int, int, int]] = []
    tileable_decal = shader_name.lower().startswith(
        "environment.decal_tileable"
    )
    for semantic, source_id in sorted(texture_ids.items()):
        image = bpy.data.images.get(str(source_id).lower())
        if image is None:
            raise ValueError(
                f"retail material {material.name!r} references missing "
                f"{semantic} image {source_id!r}"
            )
        texture_id = image_ids.get(image.as_pointer(), 0)
        if texture_id == 0:
            raise ValueError(
                f"retail image {image.name!r} was not collected for export"
            )
        uv_set = 1 if semantic in {
            "lightmap", "chromaticity", "alpha"
        } else (
            2 if semantic == "decal" else 0
        )
        clamp = semantic in {"lightmap", "chromaticity"} or (
            semantic == "decal" and not tileable_decal
        )
        address = 1 if clamp else 0
        bindings.append(
            (semantic, texture_id, uv_set, address, address)
        )

    raw_parameters = _json_object_property(
        material, "skate3_retail_parameters"
    )
    parameters: list[tuple[str, list[str]]] = []
    for name, values in raw_parameters.items():
        if isinstance(values, (list, tuple)):
            encoded_values = [str(value) for value in values]
        else:
            encoded_values = [str(values)]
        if encoded_values:
            parameters.append((name, encoded_values))
    parameters.sort(key=lambda item: item[0])

    alpha_mode = _effective_alpha_mode(material)
    source = _json_object_property(
        material, "skate3_retail_source"
    )
    return (
        shader_name,
        _retail_shader_family(shader_name),
        _retail_render_flags(shader_name, alpha_mode),
        _parse_retail_integer(
            material.get("skate3_retail_material_guid", ""), 64
        ),
        _parse_retail_integer(
            material.get("skate3_retail_material_handle", ""), 32
        ),
        int(material.get("skate3_retail_material_group_index", -1)),
        bindings,
        parameters,
        json.dumps(
            source, sort_keys=True, separators=(",", ":")
        ) if source else "",
    )


def _is_placeholder_image_name(name: str) -> bool:
    return name.strip().casefold().rsplit(".", 1)[0] in {
        "none",
        "null",
    }


def _referenced_images(
    materials: list[bpy.types.Material],
) -> tuple[list[bpy.types.Image], dict[int, int]]:
    images: list[bpy.types.Image] = []
    ids: dict[int, int] = {}
    for material in materials:
        for property_name in (
            "ow_albedo_image",
            "ow_secondary_albedo_image",
            "ow_blend_mask_image",
            "ow_lightmap_image",
            "ow_normal_image",
            "ow_orm_image",
            "ow_emissive_image",
        ):
            image_name = str(material.get(property_name, ""))
            if not image_name or _is_placeholder_image_name(image_name):
                continue
            image = bpy.data.images.get(image_name)
            if image is None:
                raise ValueError(
                    f"material {material.name!r} references missing image "
                    f"{image_name!r}"
                )
            identity = image.as_pointer()
            if identity not in ids:
                ids[identity] = len(images) + 1
                images.append(image)
        for source_id in _json_object_property(
            material, "skate3_retail_texture_ids"
        ).values():
            source_name = str(source_id).lower()
            if _is_placeholder_image_name(source_name):
                continue
            image = bpy.data.images.get(source_name)
            if image is None:
                raise ValueError(
                    f"material {material.name!r} references missing retail "
                    f"image {source_id!r}"
                )
            identity = image.as_pointer()
            if identity not in ids:
                ids[identity] = len(images) + 1
                images.append(image)
    return images, ids


def _texture_id(
    material: bpy.types.Material,
    property_name: str,
    image_ids: dict[int, int],
) -> int:
    image = bpy.data.images.get(str(material.get(property_name, "")))
    return (
        image_ids.get(image.as_pointer(), 0)
        if image is not None
        else 0
    )


def _bounded_int(
    material: bpy.types.Material,
    property_name: str,
    default: int,
    maximum: int,
) -> int:
    value = int(material.get(property_name, default))
    if value < 0 or value > maximum:
        raise ValueError(
            f"material {material.name!r} has invalid {property_name}={value}"
        )
    return value


def _material_color(material: bpy.types.Material) -> tuple[float, float, float]:
    authored = material.get("ow_display_color")
    if authored is not None and len(authored) >= 3:
        return tuple(float(authored[index]) for index in range(3))
    return tuple(float(material.diffuse_color[index]) for index in range(3))


def _presentation_depth_layer(material: bpy.types.Material) -> int:
    authored = material.get("ow_depth_layer")
    if authored is not None:
        value = int(authored)
        if value < 0 or value > 3:
            raise ValueError(
                f"material {material.name!r} has invalid ow_depth_layer={value}"
            )
        return value
    alpha_mode = _effective_alpha_mode(material)
    if alpha_mode == 2:
        return 3
    lower_name = material.name.casefold()
    overlay_tokens = (
        "sign",
        "_sgn",
        "sgn_",
        "sgns",
        "poster",
        "billboard",
        "adbord",
        "advert",
        "banner",
        "logo",
        "decal",
        "graffiti",
        "sticker",
        "plaque",
        "letter",
        "neon",
        "marking",
        "videowall",
        "vwall",
        "branding",
    )
    if any(token in lower_name for token in overlay_tokens):
        return 2
    return 1 if alpha_mode == 1 else 0


def _duplicate_visual_surface_keep_mask(
    mesh: bpy.types.Mesh,
    triangle_vertices,
    triangle_polygons,
    source_positions,
):
    """Discard redundant dense copies of the same authored visual surface.

    Some retail rips contain both a conventional static billboard quad and a
    densely tessellated video-wall copy in separate material slots that point
    at the same material. Rendering both produces persistent triangle-shaped
    depth fighting. This detects only geometrically coincident connected
    surfaces with the same material name, retaining the simpler copy.
    """
    triangle_count = len(triangle_polygons)
    keep = numpy.ones(triangle_count, dtype=numpy.bool_)
    if triangle_count < 2:
        return keep

    material_names = [
        material.name if material is not None else None
        for material in mesh.materials
    ]
    slots_by_name: dict[str, list[int]] = {}
    for slot, name in enumerate(material_names):
        if name is not None:
            slots_by_name.setdefault(name, []).append(slot)
    duplicated_slots = {
        slot
        for slots in slots_by_name.values()
        if len(slots) > 1
        for slot in slots
    }
    if not duplicated_slots:
        return keep

    polygon_materials = numpy.empty(
        len(mesh.polygons), dtype=numpy.int32
    )
    mesh.polygons.foreach_get("material_index", polygon_materials)
    triangle_materials = polygon_materials[triangle_polygons]
    candidate_indices = numpy.flatnonzero(
        numpy.isin(
            triangle_materials,
            numpy.fromiter(duplicated_slots, dtype=numpy.int32),
        )
    )
    if len(candidate_indices) < 2:
        return keep

    parent = {int(index): int(index) for index in candidate_indices}

    def find(index: int) -> int:
        root = index
        while parent[root] != root:
            root = parent[root]
        while parent[index] != index:
            next_index = parent[index]
            parent[index] = root
            index = next_index
        return root

    def union(left: int, right: int) -> None:
        left_root = find(left)
        right_root = find(right)
        if left_root != right_root:
            parent[right_root] = left_root

    first_triangle_by_slot_vertex: dict[tuple[int, int], int] = {}
    for index_value in candidate_indices:
        index = int(index_value)
        slot = int(triangle_materials[index])
        for vertex_value in triangle_vertices[index]:
            key = (slot, int(vertex_value))
            previous = first_triangle_by_slot_vertex.get(key)
            if previous is None:
                first_triangle_by_slot_vertex[key] = index
            else:
                union(index, previous)

    component_indices: dict[int, list[int]] = {}
    for index_value in candidate_indices:
        index = int(index_value)
        component_indices.setdefault(find(index), []).append(index)

    components_by_name: dict[str, list[dict]] = {}
    for indices in component_indices.values():
        slot = int(triangle_materials[indices[0]])
        points = source_positions[triangle_vertices[indices]].astype(
            numpy.float64, copy=False
        )
        edge_a = points[:, 1] - points[:, 0]
        edge_b = points[:, 2] - points[:, 0]
        crosses = numpy.cross(edge_a, edge_b)
        double_areas = numpy.linalg.norm(crosses, axis=1)
        area = float(double_areas.sum() * 0.5)
        if area <= 1.0e-10:
            continue
        centroids = points.mean(axis=1)
        centroid = (
            centroids * double_areas[:, None]
        ).sum(axis=0) / double_areas.sum()
        normal_sum = crosses.sum(axis=0)
        normal_length = float(numpy.linalg.norm(normal_sum))
        if normal_length <= 1.0e-10:
            continue
        flattened = points.reshape((-1, 3))
        minimum = flattened.min(axis=0)
        maximum = flattened.max(axis=0)
        components_by_name.setdefault(material_names[slot], []).append(
            {
                "indices": indices,
                "slot": slot,
                "area": area,
                "centroid": centroid,
                "normal": normal_sum / normal_length,
                "minimum": minimum,
                "maximum": maximum,
                "extent": float((maximum - minimum).max()),
            }
        )

    removed = set()
    for components in components_by_name.values():
        for left_index, left in enumerate(components):
            if left_index in removed:
                continue
            for right_index in range(left_index + 1, len(components)):
                if right_index in removed:
                    continue
                right = components[right_index]
                if left["slot"] == right["slot"]:
                    continue
                scale = max(left["extent"], right["extent"], 1.0)
                position_tolerance = max(0.030, scale * 2.0e-4)
                if (
                    numpy.max(
                        numpy.abs(left["minimum"] - right["minimum"])
                    )
                    > position_tolerance
                    or numpy.max(
                        numpy.abs(left["maximum"] - right["maximum"])
                    )
                    > position_tolerance
                    or abs(left["area"] - right["area"])
                    > max(0.005, max(left["area"], right["area"]) * 5.0e-4)
                    or numpy.dot(left["normal"], right["normal"]) < 0.999
                ):
                    continue
                left_key = (len(left["indices"]), left["slot"])
                right_key = (len(right["indices"]), right["slot"])
                discard_index = (
                    right_index if left_key <= right_key else left_index
                )
                discard = components[discard_index]
                keep[discard["indices"]] = False
                removed.add(discard_index)
                if discard_index == left_index:
                    break
    return keep


def _blender_material_extension(
    materials: list[ExportMaterial],
) -> bytes:
    payload = bytearray(struct.pack("<I", len(materials)))
    record = struct.Struct("<IIIfIIIII")
    for exported in materials:
        material = exported.blender_material
        blend_factor = float(material.get("ow_blend_factor", 0.0))
        if not math.isfinite(blend_factor) or not 0.0 <= blend_factor <= 1.0:
            raise ValueError(
                f"material {material.name!r} has invalid blend factor"
            )
        payload.extend(
            record.pack(
                exported.material_id,
                exported.secondary_albedo_texture,
                exported.blend_mask_texture,
                blend_factor,
                _bounded_int(
                    material, "ow_blend_mask_channel", 0, 4
                ),
                _bounded_int(
                    material, "ow_albedo_address_mode", 0, 3
                ),
                _bounded_int(
                    material, "ow_secondary_address_mode", 0, 3
                ),
                _bounded_int(
                    material, "ow_blend_mask_address_mode", 0, 3
                ),
                _bounded_int(material, "ow_cull_mode", 1, 2),
            )
        )
    return bytes(payload)


def _material_uv_layer(
    mesh: bpy.types.Mesh,
    material: bpy.types.Material,
    *,
    secondary: bool = False,
) -> bpy.types.MeshUVLoopLayer:
    if secondary and str(material.get("skate3_shader_name", "")):
        return mesh.uv_layers.get("Decal") or mesh.uv_layers["UVMap"]
    if secondary:
        uv_name = str(
            material.get("ow_blend_mask_uv_map", "")
            or material.get("ow_secondary_uv_map", "")
            or material.get("ow_uv_map", "")
        )
    else:
        uv_name = str(material.get("ow_uv_map", ""))
    return (
        mesh.uv_layers.get(uv_name)
        if uv_name
        else None
    ) or mesh.uv_layers["UVMap"]


def _material_uv_transform(
    material: bpy.types.Material,
    *,
    secondary: bool = False,
) -> tuple[float, float, float, float, float]:
    if secondary:
        value = (
            material.get("ow_blend_mask_uv_transform")
            if str(material.get("ow_blend_mask_image", ""))
            else None
        ) or material.get("ow_secondary_uv_transform")
    else:
        value = material.get("ow_uv_transform")
    if value is None or len(value) != 5:
        return 1.0, 1.0, 0.0, 0.0, 0.0
    result = tuple(float(component) for component in value)
    if not all(math.isfinite(component) for component in result):
        raise ValueError(
            f"material {material.name!r} has a non-finite UV transform"
        )
    return result


def _transform_uv(uv, transform):
    scale_u, scale_v, rotation, translate_u, translate_v = transform
    source_u = float(uv[0]) * scale_u
    source_v = float(uv[1]) * scale_v
    cosine = math.cos(rotation)
    sine = math.sin(rotation)
    return (
        source_u * cosine - source_v * sine + translate_u,
        source_u * sine + source_v * cosine + translate_v,
    )


def _export_visual_geometry(
    visual_objects: list[bpy.types.Object],
    material_variant_ids: dict[
        tuple[str, tuple[float, float, float]], int
    ],
    progress: ProgressCallback | None = None,
) -> PackedVisualGeometry:
    depsgraph = bpy.context.evaluated_depsgraph_get()
    vertex_chunks: list[bytes] = []
    index_chunks: list[bytes] = []
    vertex_count = 0
    index_count = 0
    objects: list[ExportMapObject] = []
    mesh_objects = [
        obj for obj in visual_objects if obj.type == "MESH"
    ]
    weights = [
        max(1, len(obj.data.polygons)) for obj in mesh_objects
    ]
    total_weight = max(1, sum(weights))
    completed_weight = 0
    packed_vertex = struct.Struct("<3f3f2f2fI2f4b")

    for object_index, (source_object, object_weight) in enumerate(
        zip(mesh_objects, weights, strict=True),
        start=1,
    ):
        if source_object.type != "MESH":
            continue
        object_first_index = index_count
        mesh, evaluated = _mesh_for_export(
            source_object,
            depsgraph,
            preserve_all_data_layers=True,
        )
        try:
            mesh.calc_loop_triangles()
            uv0, uv1 = _visual_uv_layers(mesh, source_object.name)
            loop_count = len(mesh.loops)
            if len(uv0.data) != loop_count or len(uv1.data) != loop_count:
                raise ValueError(
                    f"visual mesh {source_object.name!r} has malformed UV "
                    "layer data"
                )
            decal_uv = mesh.uv_layers.get("Decal") or uv0
            if len(decal_uv.data) != loop_count:
                raise ValueError(
                    f"visual mesh {source_object.name!r} has malformed "
                    "decal UV layer data"
                )
            retail_frame_attributes = _retail_world_frame_attributes(mesh)
            tangents_available = retail_frame_attributes is None
            if tangents_available:
                try:
                    mesh.calc_tangents(uvmap=uv0.name)
                except RuntimeError:
                    tangents_available = False
            # Blender 5 can replace implicitly shared UV storage while
            # calculating tangents. Reacquire every layer so the references
            # below cannot alias the active UVMap after that mutation.
            uv0, uv1 = _visual_uv_layers(mesh, source_object.name)
            decal_uv = mesh.uv_layers.get("Decal") or uv0
            if decal_uv is None:
                raise ValueError(
                    f"visual mesh {source_object.name!r} lost an export UV "
                    "layer while calculating tangents"
                )

            if numpy is not None:
                triangle_count = len(mesh.loop_triangles)
                corner_count = triangle_count * 3
                triangle_loops = numpy.empty(
                    corner_count, dtype=numpy.int32
                )
                triangle_polygons = numpy.empty(
                    triangle_count, dtype=numpy.int32
                )
                mesh.loop_triangles.foreach_get(
                    "loops", triangle_loops
                )
                mesh.loop_triangles.foreach_get(
                    "polygon_index", triangle_polygons
                )

                loop_vertices = numpy.empty(
                    loop_count, dtype=numpy.int32
                )
                mesh.loops.foreach_get(
                    "vertex_index", loop_vertices
                )
                if retail_frame_attributes is None:
                    loop_normals = numpy.empty(
                        loop_count * 3, dtype=numpy.float32
                    )
                    mesh.loops.foreach_get("normal", loop_normals)
                    loop_normals = loop_normals.reshape((-1, 3))
                else:
                    retail_normal_attribute = retail_frame_attributes[0]
                    retail_point_normals = numpy.empty(
                        len(mesh.vertices) * 3,
                        dtype=numpy.float32,
                    )
                    retail_normal_attribute.data.foreach_get(
                        "vector", retail_point_normals
                    )
                    retail_point_normals = retail_point_normals.reshape(
                        (-1, 3)
                    )

                source_positions = numpy.empty(
                    len(mesh.vertices) * 3, dtype=numpy.float32
                )
                mesh.vertices.foreach_get("co", source_positions)
                source_positions = source_positions.reshape((-1, 3))

                base_uvs = numpy.empty(
                    loop_count * 2, dtype=numpy.float32
                )
                light_uvs = numpy.empty(
                    loop_count * 2, dtype=numpy.float32
                )
                decal_uvs = numpy.empty(
                    loop_count * 2, dtype=numpy.float32
                )
                uv0.data.foreach_get("uv", base_uvs)
                uv1.data.foreach_get("uv", light_uvs)
                decal_uv.data.foreach_get("uv", decal_uvs)
                base_uvs = base_uvs.reshape((-1, 2))
                light_uvs = light_uvs.reshape((-1, 2))
                decal_uvs = decal_uvs.reshape((-1, 2))

                polygon_materials = numpy.empty(
                    len(mesh.polygons), dtype=numpy.int32
                )
                mesh.polygons.foreach_get(
                    "material_index", polygon_materials
                )
                used_material_indices = numpy.unique(
                    polygon_materials[triangle_polygons]
                )
                if (
                    used_material_indices.size
                    and (
                        int(used_material_indices.min()) < 0
                        or int(used_material_indices.max())
                        >= len(mesh.materials)
                    )
                ):
                    raise ValueError(
                        f"mesh {source_object.name!r} has an invalid "
                        "material slot"
                    )
                mesh_material_ids = numpy.zeros(
                    len(mesh.materials), dtype=numpy.uint32
                )
                for material_index in used_material_indices:
                    material_index = int(material_index)
                    material = mesh.materials[material_index]
                    if material is None:
                        raise ValueError(
                            f"mesh {source_object.name!r} has an "
                            "unexported material"
                        )
                    variant_key = (
                        material.name,
                        _object_material_tint(source_object, material),
                    )
                    if variant_key not in material_variant_ids:
                        raise ValueError(
                            f"mesh {source_object.name!r} has an "
                            "unexported material colour variant"
                        )
                    mesh_material_ids[material_index] = (
                        material_variant_ids[variant_key]
                    )

                triangle_loops = triangle_loops.reshape((-1, 3))
                triangle_vertices = loop_vertices[triangle_loops]
                if not bool(
                    source_object.get(
                        "ow_preserve_duplicate_visual_surfaces", False
                    )
                ):
                    keep_triangles = _duplicate_visual_surface_keep_mask(
                        mesh,
                        triangle_vertices,
                        triangle_polygons,
                        source_positions,
                    )
                    removed_triangles = int(
                        len(keep_triangles) - keep_triangles.sum()
                    )
                    if removed_triangles:
                        print(
                            "SKATE visual cleanup:"
                            f" {source_object.name!r} omitted"
                            f" {removed_triangles} redundant coincident"
                            " triangle(s)",
                            flush=True,
                        )
                        triangle_loops = triangle_loops[keep_triangles]
                        triangle_polygons = triangle_polygons[keep_triangles]
                        triangle_vertices = triangle_vertices[keep_triangles]
                        triangle_count = len(triangle_polygons)
                        corner_count = triangle_count * 3
                triangle_loops = triangle_loops.reshape(-1)
                corner_vertices = triangle_vertices.reshape(-1)
                positions = source_positions[corner_vertices].astype(
                    numpy.float64
                )
                world_matrix = numpy.asarray(
                    source_object.matrix_world, dtype=numpy.float64
                )
                positions = (
                    positions @ world_matrix[:3, :3].T
                    + world_matrix[:3, 3]
                )

                normal_matrix = numpy.asarray(
                    source_object.matrix_world
                    .to_3x3()
                    .inverted()
                    .transposed(),
                    dtype=numpy.float64,
                )
                if retail_frame_attributes is None:
                    normals = (
                        loop_normals[triangle_loops].astype(numpy.float64)
                        @ normal_matrix.T
                    )
                else:
                    normals = (
                        retail_point_normals[corner_vertices].astype(
                            numpy.float64
                        )
                        @ normal_matrix.T
                    )
                lengths = numpy.linalg.norm(normals, axis=1)
                nonzero_normals = lengths > 1.0e-12
                if retail_frame_attributes is None:
                    normals[nonzero_normals] /= lengths[
                        nonzero_normals, None
                    ]

                runtime_positions = positions[:, (0, 2, 1)].copy()
                runtime_positions[:, 2] *= -1.0
                runtime_normals = normals[:, (0, 2, 1)].copy()
                runtime_normals[:, 2] *= -1.0
                runtime_binormals = numpy.zeros_like(runtime_normals)
                runtime_handedness = numpy.zeros(
                    corner_count, dtype=numpy.float32
                )
                runtime_linear = numpy.asarray(
                    (
                        (1.0, 0.0, 0.0),
                        (0.0, 0.0, 1.0),
                        (0.0, -1.0, 0.0),
                    ),
                    dtype=numpy.float64,
                ) @ world_matrix[:3, :3]
                orientation = (
                    -1.0
                    if numpy.linalg.det(runtime_linear) < 0.0
                    else 1.0
                )
                if retail_frame_attributes is not None:
                    (
                        _retail_normal_attribute,
                        retail_tangent_attribute,
                        retail_handedness_attribute,
                    ) = retail_frame_attributes
                    retail_point_tangents = numpy.empty(
                        len(mesh.vertices) * 3,
                        dtype=numpy.float32,
                    )
                    retail_point_handedness = numpy.empty(
                        len(mesh.vertices),
                        dtype=numpy.float32,
                    )
                    retail_tangent_attribute.data.foreach_get(
                        "vector", retail_point_tangents
                    )
                    retail_handedness_attribute.data.foreach_get(
                        "value", retail_point_handedness
                    )
                    retail_point_tangents = (
                        retail_point_tangents.reshape((-1, 3))
                    )
                    tangents = (
                        retail_point_tangents[corner_vertices].astype(
                            numpy.float64
                        )
                        @ world_matrix[:3, :3].T
                    )
                    runtime_tangents = tangents[:, (0, 2, 1)].copy()
                    runtime_tangents[:, 2] *= -1.0
                    runtime_handedness = (
                        retail_point_handedness[corner_vertices] * orientation
                    ).astype(numpy.float32)
                    source_tangent_lengths = numpy.linalg.norm(
                        runtime_tangents, axis=1
                    )
                    runtime_tangents -= runtime_normals * numpy.sum(
                        runtime_tangents * runtime_normals,
                        axis=1,
                    )[:, None]
                    tangent_lengths = numpy.linalg.norm(
                        runtime_tangents, axis=1
                    )
                    valid_tangents = tangent_lengths > numpy.maximum(
                        1.0e-12,
                        source_tangent_lengths * 1.0e-6,
                    )
                    runtime_tangents[valid_tangents] /= tangent_lengths[
                        valid_tangents, None
                    ]
                    runtime_handedness[~valid_tangents] = 0.0
                    runtime_binormals = numpy.cross(
                        runtime_normals, runtime_tangents
                    ) * runtime_handedness[:, None]
                elif tangents_available:
                    loop_tangents = numpy.empty(
                        loop_count * 3, dtype=numpy.float32
                    )
                    loop_signs = numpy.empty(
                        loop_count, dtype=numpy.float32
                    )
                    mesh.loops.foreach_get("tangent", loop_tangents)
                    mesh.loops.foreach_get(
                        "bitangent_sign", loop_signs
                    )
                    loop_tangents = loop_tangents.reshape((-1, 3))
                    tangents = (
                        loop_tangents[triangle_loops].astype(numpy.float64)
                        @ world_matrix[:3, :3].T
                    )
                    runtime_tangents = tangents[:, (0, 2, 1)].copy()
                    runtime_tangents[:, 2] *= -1.0
                    source_tangent_lengths = numpy.linalg.norm(
                        runtime_tangents, axis=1
                    )
                    runtime_tangents -= runtime_normals * numpy.sum(
                        runtime_tangents * runtime_normals,
                        axis=1,
                    )[:, None]
                    tangent_lengths = numpy.linalg.norm(
                        runtime_tangents, axis=1
                    )
                    valid_tangents = tangent_lengths > numpy.maximum(
                        1.0e-12,
                        source_tangent_lengths * 1.0e-6,
                    )
                    runtime_tangents[valid_tangents] /= tangent_lengths[
                        valid_tangents, None
                    ]
                    runtime_handedness = (
                        loop_signs[triangle_loops] * orientation
                    ).astype(numpy.float32)
                    runtime_handedness[~valid_tangents] = 0.0
                    runtime_binormals = numpy.cross(
                        runtime_normals, runtime_tangents
                    ) * runtime_handedness[:, None]
                triangle_material_indices = polygon_materials[
                    triangle_polygons
                ]
                corner_material_indices = numpy.repeat(
                    triangle_material_indices, 3
                )
                corner_materials = mesh_material_ids[
                    corner_material_indices
                ]
                corner_base_uvs = base_uvs[triangle_loops].copy()
                corner_secondary_uvs = decal_uvs[
                    triangle_loops
                ].copy()
                uv_array_cache: dict[str, numpy.ndarray] = {
                    uv0.name: base_uvs,
                    decal_uv.name: decal_uvs,
                }

                def uv_values(layer):
                    values = uv_array_cache.get(layer.name)
                    if values is None:
                        values = numpy.empty(
                            loop_count * 2, dtype=numpy.float32
                        )
                        layer.data.foreach_get("uv", values)
                        values = values.reshape((-1, 2))
                        uv_array_cache[layer.name] = values
                    return values

                for material_index in used_material_indices:
                    material_index = int(material_index)
                    material = mesh.materials[material_index]
                    corner_mask = (
                        corner_material_indices == material_index
                    )
                    primary_layer = _material_uv_layer(
                        mesh, material
                    )
                    secondary_layer = _material_uv_layer(
                        mesh, material, secondary=True
                    )
                    selected_base = uv_values(primary_layer)[
                        triangle_loops[corner_mask]
                    ].copy()
                    selected_secondary = uv_values(secondary_layer)[
                        triangle_loops[corner_mask]
                    ].copy()
                    for selected, transform in (
                        (
                            selected_base,
                            _material_uv_transform(material),
                        ),
                        (
                            selected_secondary,
                            _material_uv_transform(
                                material, secondary=True
                            ),
                        ),
                    ):
                        scale_u, scale_v, rotation, move_u, move_v = (
                            transform
                        )
                        source_u = selected[:, 0] * scale_u
                        source_v = selected[:, 1] * scale_v
                        cosine = math.cos(rotation)
                        sine = math.sin(rotation)
                        selected[:, 0] = (
                            source_u * cosine
                            - source_v * sine
                            + move_u
                        )
                        selected[:, 1] = (
                            source_u * sine
                            + source_v * cosine
                            + move_v
                        )
                    corner_base_uvs[corner_mask] = selected_base
                    corner_secondary_uvs[corner_mask] = (
                        selected_secondary
                    )

                if not all(
                    numpy.isfinite(values).all()
                    for values in (
                        runtime_positions,
                        runtime_normals,
                        corner_base_uvs,
                        light_uvs[triangle_loops],
                        corner_secondary_uvs,
                        runtime_binormals,
                        runtime_handedness,
                    )
                ):
                    raise ValueError(
                        f"visual mesh {source_object.name!r} contains "
                        "non-finite geometry or UV values"
                    )

                record_dtype = numpy.dtype(
                    [
                        ("position", "<f4", (3,)),
                        ("normal", "<f4", (3,)),
                        ("uv0", "<f4", (2,)),
                        ("uv1", "<f4", (2,)),
                        ("material", "<u4"),
                        ("uv2", "<f4", (2,)),
                        ("tangent_frame", "i1", (4,)),
                    ],
                    align=False,
                )
                records = numpy.empty(corner_count, dtype=record_dtype)
                records["position"] = runtime_positions
                records["normal"] = runtime_normals
                records["uv0"] = corner_base_uvs
                records["uv1"] = light_uvs[triangle_loops]
                records["material"] = corner_materials
                records["uv2"] = corner_secondary_uvs
                records["tangent_frame"][:, :3] = numpy.rint(
                    numpy.clip(runtime_binormals, -1.0, 1.0) * 127.0
                ).astype(numpy.int8)
                records["tangent_frame"][:, 3] = numpy.rint(
                    numpy.clip(runtime_handedness, -1.0, 1.0) * 127.0
                ).astype(numpy.int8)
                unique_records, inverse = numpy.unique(
                    records,
                    return_inverse=True,
                )
                if vertex_count + len(unique_records) > 0xFFFFFFFF:
                    raise ValueError(
                        "visual geometry exceeds the SKATE u32 index limit"
                    )
                indices = inverse.astype(
                    numpy.dtype("<u4"), copy=False
                )
                if vertex_count:
                    indices += vertex_count
                vertex_chunks.append(unique_records.tobytes())
                index_chunks.append(indices.tobytes())
                vertex_count += len(unique_records)
                index_count += corner_count
            else:
                normal_matrix = (
                    source_object.matrix_world
                    .to_3x3()
                    .inverted()
                    .transposed()
                )
                unique_vertices: dict[bytes, int] = {}
                vertex_chunk = bytearray()
                index_values = array("I")
                for triangle in mesh.loop_triangles:
                    polygon = mesh.polygons[triangle.polygon_index]
                    if polygon.material_index >= len(mesh.materials):
                        raise ValueError(
                            f"mesh {source_object.name!r} has an invalid "
                            "material slot"
                        )
                    material = mesh.materials[polygon.material_index]
                    if material is None:
                        raise ValueError(
                            f"mesh {source_object.name!r} has an "
                            "unexported material"
                        )
                    variant_key = (
                        material.name,
                        _object_material_tint(source_object, material),
                    )
                    if variant_key not in material_variant_ids:
                        raise ValueError(
                            f"mesh {source_object.name!r} has an "
                            "unexported material colour variant"
                        )
                    material_id = material_variant_ids[variant_key]
                    material_uv = _material_uv_layer(
                        mesh, material
                    )
                    material_secondary_uv = _material_uv_layer(
                        mesh, material, secondary=True
                    )
                    for loop_index in triangle.loops:
                        loop = mesh.loops[loop_index]
                        point = (
                            source_object.matrix_world
                            @ mesh.vertices[loop.vertex_index].co
                        )
                        if retail_frame_attributes is None:
                            normal = (
                                normal_matrix @ loop.normal
                            ).normalized()
                        else:
                            normal = (
                                normal_matrix
                                @ Vector(
                                    retail_frame_attributes[0]
                                    .data[loop.vertex_index]
                                    .vector
                                )
                            )
                        base_uv = _transform_uv(
                            material_uv.data[loop_index].uv,
                            _material_uv_transform(material),
                        )
                        light_uv = uv1.data[loop_index].uv
                        decal = _transform_uv(
                            material_secondary_uv.data[loop_index].uv,
                            _material_uv_transform(
                                material, secondary=True
                            ),
                        )
                        handedness = 0.0
                        binormal = Vector((0.0, 0.0, 0.0))
                        orientation = (
                            1.0
                            if source_object.matrix_world.to_3x3()
                            .determinant() > 0.0
                            else -1.0
                        )
                        if retail_frame_attributes is not None:
                            local_binormal = Vector(
                                retail_frame_attributes[1]
                                .data[loop.vertex_index]
                                .vector
                            )
                            binormal = Vector(
                                _to_runtime(
                                    source_object.matrix_world.to_3x3()
                                    @ local_binormal
                                )
                            )
                            handedness = (
                                float(
                                    retail_frame_attributes[2]
                                    .data[loop.vertex_index]
                                    .value
                                )
                                * orientation
                            )
                        elif tangents_available:
                            tangent = (
                                source_object.matrix_world.to_3x3()
                                @ loop.tangent
                            )
                            source_tangent_length = tangent.length
                            tangent -= normal * normal.dot(tangent)
                            if tangent.length > max(
                                1.0e-12,
                                source_tangent_length * 1.0e-6,
                            ):
                                tangent.normalize()
                                runtime_normal = Vector(
                                    _to_runtime(normal)
                                )
                                runtime_tangent = Vector(
                                    _to_runtime(tangent)
                                )
                                handedness = (
                                    float(loop.bitangent_sign)
                                    * orientation
                                )
                                binormal = (
                                    runtime_normal.cross(runtime_tangent)
                                    * handedness
                                )
                        values = (
                            *_to_runtime(point),
                            *_to_runtime(normal),
                            float(base_uv[0]),
                            float(base_uv[1]),
                            float(light_uv.x),
                            float(light_uv.y),
                        )
                        retail_values = (
                            float(decal[0]),
                            float(decal[1]),
                        )
                        if not all(
                            math.isfinite(value)
                            for value in (*values, *retail_values)
                        ):
                            raise ValueError(
                                f"visual mesh {source_object.name!r} "
                                "contains non-finite geometry or UV values"
                            )
                        record = packed_vertex.pack(
                            *values,
                            material_id,
                            *retail_values,
                            *_pack_tangent_frame(binormal, handedness),
                        )
                        local_index = unique_vertices.get(record)
                        if local_index is None:
                            local_index = len(unique_vertices)
                            unique_vertices[record] = local_index
                            vertex_chunk.extend(record)
                        index_values.append(vertex_count + local_index)
                        index_count += 1
                if sys.byteorder != "little":
                    index_values.byteswap()
                vertex_chunks.append(bytes(vertex_chunk))
                index_chunks.append(index_values.tobytes())
                vertex_count += len(unique_vertices)
        finally:
            if evaluated is not None:
                evaluated.to_mesh_clear()

        object_index_count = index_count - object_first_index
        if object_index_count:
            (
                physics_type,
                collision_shape,
                density,
                friction,
                restitution,
                linear_damping,
                angular_damping,
                gravity_scale,
                enable_sleep,
                initially_awake,
                break_group,
                break_speed_threshold,
                break_impulse_scale,
                break_angular_impulse,
                break_gravity_scale,
            ) = _box3d_properties(source_object)
            objects.append(
                ExportMapObject(
                    source_identity=source_object.as_pointer(),
                    object_id=_stable_object_id(source_object.name_full),
                    name=source_object.name_full,
                    origin=_to_runtime(
                        source_object.matrix_world.translation
                    ),
                    first_index=object_first_index,
                    index_count=object_index_count,
                    editor_editable=bool(
                        source_object.get("ow_editor_editable", True)
                    ),
                    physics_type=physics_type,
                    collision_shape=collision_shape,
                    density=density,
                    friction=friction,
                    restitution=restitution,
                    linear_damping=linear_damping,
                    angular_damping=angular_damping,
                    gravity_scale=gravity_scale,
                    enable_sleep=enable_sleep,
                    initially_awake=initially_awake,
                    break_group=break_group,
                    break_speed_threshold=break_speed_threshold,
                    break_impulse_scale=break_impulse_scale,
                    break_angular_impulse=break_angular_impulse,
                    break_gravity_scale=break_gravity_scale,
                )
            )
        completed_weight += object_weight
        _report_progress(
            progress,
            completed_weight / total_weight,
            f"Packing visuals ({object_index}/{len(mesh_objects)}): "
            f"{source_object.name}",
        )

    if not vertex_count:
        raise ValueError("presentation groups contain no exportable triangles")
    object_ids = [record.object_id for record in objects]
    if len(set(object_ids)) != len(object_ids):
        raise ValueError(
            "Blender object names produced a stable-ID collision; rename "
            "one of the colliding objects"
        )
    return PackedVisualGeometry(
        vertex_chunks,
        index_chunks,
        vertex_count,
        index_count,
        objects,
    )


def audit_collision_geometry(
    collision_objects: list[bpy.types.Object],
    material_name_ids: dict[str, int] | None = None,
    progress: ProgressCallback | None = None,
    object_ranges: dict[int, tuple[int, int]] | None = None,
    include_editor_ownership: bool = True,
) -> tuple[list[tuple], CollisionGeometryAudit]:
    depsgraph = bpy.context.evaluated_depsgraph_get()
    triangles: list[tuple] = []
    partitioned_triangles: dict[int, list[tuple]] = {}
    partitioned_owners: dict[int, bpy.types.Object] = {}
    editable_owner_ids = (
        {
            _stable_object_id(obj.name_full): obj
            for obj in bpy.data.objects
            if obj.type == "MESH"
            and bool(obj.get("ow_editor_editable", True))
        }
        if include_editor_ownership
        else {}
    )
    triangle_owners: dict[tuple, str] = {}
    issues: list[str] = []
    warnings: list[str] = []
    source_triangle_count = 0
    skipped_degenerate = 0
    skipped_duplicates = 0
    surface_id = 1
    mesh_objects = [
        obj for obj in collision_objects if obj.type == "MESH"
    ]
    for object_index, source_object in enumerate(
        mesh_objects, start=1
    ):
        object_first_triangle = len(triangles)
        _report_progress(
            progress,
            (object_index - 1) / max(1, len(mesh_objects)),
            f"Auditing collision ({object_index}/"
            f"{len(mesh_objects)}): {source_object.name}",
        )
        material_name = str(source_object.get("ow_material", ""))
        blender_material = bpy.data.materials.get(material_name)
        if (
            blender_material is not None
            and not bool(
                blender_material.get("ow_collision_enabled", True)
            )
        ):
            continue
        if (
            material_name_ids is not None
            and material_name not in material_name_ids
        ):
            issues.append(
                f"{source_object.name}: collision material "
                f"{material_name!r} is not exported by a presentation group."
            )
            continue
        material_id = (
            material_name_ids[material_name]
            if material_name_ids is not None
            else 0
        )
        preserve_retail_codes = bool(
            source_object.get("ow_preserve_retail_edge_codes", False)
        )
        mesh, evaluated = _mesh_for_export(
            source_object,
            depsgraph,
            preserve_all_data_layers=preserve_retail_codes,
        )
        edge_attributes = []
        editor_owner_attribute = (
            mesh.attributes.get(EDITOR_COLLISION_OWNER_ATTRIBUTE)
            if include_editor_ownership
            else None
        )
        if editor_owner_attribute is not None and (
            editor_owner_attribute.domain != "FACE"
            or editor_owner_attribute.data_type != "INT"
        ):
            issues.append(
                f"{source_object.name}: editor collision owner attribute "
                f"{EDITOR_COLLISION_OWNER_ATTRIBUTE!r} must be a FACE INT."
            )
            editor_owner_attribute = None
        if preserve_retail_codes:
            for attribute_name in RETAIL_EDGE_CODE_ATTRIBUTES:
                attribute = mesh.attributes.get(attribute_name)
                if (
                    attribute is None
                    or attribute.domain != "FACE"
                    or attribute.data_type != "INT"
                ):
                    issues.append(
                        f"{source_object.name}: native retail collision "
                        f"attribute {attribute_name!r} is missing or invalid."
                    )
                    edge_attributes = []
                    break
                edge_attributes.append(attribute)
        object_degenerate = 0
        object_duplicates = 0
        object_non_finite = 0
        object_wrong_facing = 0
        try:
            mesh.calc_loop_triangles()
            for triangle in mesh.loop_triangles:
                source_triangle_count += 1
                blender_points = [
                    source_object.matrix_world @ mesh.vertices[index].co
                    for index in triangle.vertices
                ]
                if not all(
                    math.isfinite(float(component))
                    for point in blender_points
                    for component in point
                ):
                    object_non_finite += 1
                    continue
                try:
                    points = [
                        tuple(
                            struct.unpack(
                                "<f", struct.pack("<f", component)
                            )[0]
                            for component in _to_runtime(point)
                        )
                        for point in blender_points
                    ]
                except (OverflowError, struct.error):
                    object_non_finite += 1
                    continue
                runtime_cross = (Vector(points[1]) - Vector(points[0])).cross(
                    Vector(points[2]) - Vector(points[0])
                )
                # Keep this exactly aligned with the C++ SKATE loader. Very
                # small source triangles can survive Blender's mesh cleanup
                # but collapse after float32 package serialization.
                if runtime_cross.length_squared <= 1.0e-10:
                    object_degenerate += 1
                    skipped_degenerate += 1
                    continue
                if (
                    bool(source_object.get("ow_upward_surface", False))
                    and runtime_cross.y <= 1.0e-6
                ):
                    object_wrong_facing += 1
                    continue
                rounded_points = tuple(
                    tuple(
                        round(float(component), 6)
                        for component in point
                    )
                    for point in points
                )
                if bool(
                    source_object.get(
                        "ow_preserve_opposite_wound_collision",
                        False,
                    )
                ):
                    # Cyclic rotation does not change triangle orientation,
                    # but reversing two vertices does. Retail ClusteredMesh
                    # uses reverse-wound partners to provide intentional
                    # two-sided collision, so only same-wound copies are
                    # duplicates for these objects.
                    key = min(
                        rounded_points,
                        (
                            rounded_points[1],
                            rounded_points[2],
                            rounded_points[0],
                        ),
                        (
                            rounded_points[2],
                            rounded_points[0],
                            rounded_points[1],
                        ),
                    )
                else:
                    # Generic authored maps retain the stricter cleanup:
                    # opposite-wound copies can otherwise create
                    # contradictory contacts and broken adjacency.
                    key = tuple(sorted(rounded_points))
                if key in triangle_owners:
                    object_duplicates += 1
                    skipped_duplicates += 1
                    continue
                triangle_owners[key] = source_object.name
                native_edge_codes = None
                if preserve_retail_codes and len(edge_attributes) == 3:
                    polygon_vertices = tuple(
                        mesh.polygons[triangle.polygon_index].vertices
                    )
                    triangle_vertices = tuple(triangle.vertices)
                    if len(polygon_vertices) != 3:
                        issues.append(
                            f"{source_object.name}: native retail collision "
                            "metadata is attached to a non-triangle face."
                        )
                        continue
                    rotation = next(
                        (
                            offset
                            for offset in range(3)
                            if triangle_vertices
                            == polygon_vertices[offset:]
                            + polygon_vertices[:offset]
                        ),
                        None,
                    )
                    if rotation is None:
                        issues.append(
                            f"{source_object.name}: Blender reversed a face "
                            "with native retail collision edge codes."
                        )
                        continue
                    polygon_codes = tuple(
                        int(attribute.data[triangle.polygon_index].value)
                        for attribute in edge_attributes
                    )
                    if any(code < 0 or code > 255 for code in polygon_codes):
                        issues.append(
                            f"{source_object.name}: native retail collision "
                            "edge code is outside the byte range."
                        )
                        continue
                    native_edge_codes = tuple(
                        polygon_codes[(rotation + corner) % 3]
                        for corner in range(3)
                    )
                exported_triangle = (
                    points[0],
                    points[1],
                    points[2],
                    surface_id,
                    material_id,
                    native_edge_codes,
                )
                editor_owner = None
                if editor_owner_attribute is not None:
                    owner_id = (
                        int(
                            editor_owner_attribute.data[
                                triangle.polygon_index
                            ].value
                        )
                        & 0xFFFFFFFF
                    )
                    if owner_id != 0:
                        editor_owner = editable_owner_ids.get(owner_id)
                        if editor_owner is None:
                            issues.append(
                                f"{source_object.name}: collision face "
                                f"references missing editable object ID "
                                f"0x{owner_id:08X}."
                            )
                if editor_owner is None:
                    triangles.append(exported_triangle)
                else:
                    owner_identity = editor_owner.as_pointer()
                    partitioned_owners[owner_identity] = editor_owner
                    partitioned_triangles.setdefault(
                        owner_identity, []
                    ).append(exported_triangle)
        finally:
            if evaluated is not None:
                evaluated.to_mesh_clear()
        if object_degenerate:
            warnings.append(
                f"{source_object.name}: skipped {object_degenerate} "
                "zero-area collision triangle(s). No mesh dissolve or UV "
                "changes are required."
            )
        if object_duplicates:
            warnings.append(
                f"{source_object.name}: skipped {object_duplicates} "
                + (
                    "same-wound duplicate collision triangle(s); retained "
                    "reverse-wound retail partners."
                    if bool(
                        source_object.get(
                            "ow_preserve_opposite_wound_collision",
                            False,
                        )
                    )
                    else "exact or opposite-wound duplicate collision "
                    "triangle(s)."
                )
            )
        if object_non_finite:
            issues.append(
                f"{source_object.name}: {object_non_finite} collision "
                "triangle(s) contain non-finite or float32-out-of-range "
                "coordinates."
            )
        if object_wrong_facing:
            issues.append(
                f"{source_object.name}: {object_wrong_facing} triangle(s) "
                "face downward or vertically, but this object is marked "
                "Rideable Top Surface."
            )
        if (
            include_editor_ownership
            and object_ranges is not None
            and editor_owner_attribute is None
        ):
            owner_identity = source_object.as_pointer()
            owner_name = str(
                source_object.get("ow_map_object_owner", "")
            ).strip()
            if owner_name:
                owner = bpy.data.objects.get(owner_name)
                if owner is None or owner.type != "MESH":
                    issues.append(
                        f"{source_object.name}: editable collision owner "
                        f"{owner_name!r} is missing or is not a mesh."
                    )
                else:
                    owner_identity = owner.as_pointer()
            object_triangle_count = len(triangles) - object_first_triangle
            previous_range = object_ranges.get(owner_identity)
            if previous_range is None:
                object_ranges[owner_identity] = (
                    object_first_triangle,
                    object_triangle_count,
                )
            elif (
                previous_range[0] + previous_range[1]
                == object_first_triangle
            ):
                object_ranges[owner_identity] = (
                    previous_range[0],
                    previous_range[1] + object_triangle_count,
                )
            else:
                issues.append(
                    f"{source_object.name}: collision proxies for editable "
                    f"owner {owner_name or source_object.name!r} are not "
                    "contiguous in OW_COLLISION."
                )
        surface_id += 1
    for owner_identity in sorted(
        partitioned_triangles,
        key=lambda identity: partitioned_owners[identity].name_full,
    ):
        owned = partitioned_triangles[owner_identity]
        if not owned:
            continue
        if object_ranges is not None and owner_identity in object_ranges:
            issues.append(
                f"{partitioned_owners[owner_identity].name}: collision is "
                "owned by both a proxy object and face-level partitions."
            )
            continue
        first_triangle = len(triangles)
        triangles.extend(owned)
        if object_ranges is not None:
            object_ranges[owner_identity] = (
                first_triangle,
                len(owned),
            )
    if not triangles:
        issues.append("collision groups contain no usable collision triangles.")
    audit = CollisionGeometryAudit(
        issues=issues,
        warnings=warnings,
        source_triangles=source_triangle_count,
        exported_triangles=len(triangles),
        skipped_degenerate=skipped_degenerate,
        skipped_duplicates=skipped_duplicates,
    )
    _report_progress(progress, 1.0, "Collision audit complete")
    return triangles, audit


def _export_collision(
    collision_objects: list[bpy.types.Object],
    material_name_ids: dict[str, int],
    export_editable_objects: bool,
    progress: ProgressCallback | None = None,
) -> tuple[
    list[tuple],
    CollisionGeometryAudit,
    dict[int, tuple[int, int]],
]:
    global LAST_COLLISION_AUDIT
    object_ranges: dict[int, tuple[int, int]] = {}
    triangles, audit = audit_collision_geometry(
        collision_objects,
        material_name_ids,
        progress,
        object_ranges,
        export_editable_objects,
    )
    LAST_COLLISION_AUDIT = audit
    if audit.issues:
        raise CollisionGeometryError(audit.issues, audit.warnings)
    for warning in audit.warnings:
        print(f"SKATE collision cleanup: {warning}", flush=True)
    return triangles, audit, object_ranges


def _retail_grind_controls(
    payload: bytes,
) -> tuple[tuple[float, float, float], ...]:
    values = struct.unpack(">30f", payload)
    coefficient_a = values[0:3]
    coefficient_b = values[4:7]
    coefficient_c = values[8:11]
    coefficient_d = values[12:15]
    return (
        tuple(coefficient_d),
        tuple(
            coefficient_d[axis] + coefficient_c[axis] / 3.0
            for axis in range(3)
        ),
        tuple(
            coefficient_d[axis]
            + (2.0 * coefficient_c[axis] + coefficient_b[axis]) / 3.0
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


def _close_enough(
    left: tuple[float, float, float],
    right: tuple[float, float, float],
    tolerance: float = 2.0e-3,
) -> bool:
    return all(
        abs(left[axis] - right[axis]) <= tolerance
        for axis in range(3)
    )


def _export_retail_grind(
    obj: bpy.types.Object,
) -> ExportGrindRail:
    if len(obj.data.splines) != 1:
        raise ValueError(
            f"retail grind {obj.name!r} must contain exactly one spline"
        )
    spline = obj.data.splines[0]
    if spline.type != "BEZIER":
        raise ValueError(
            f"retail grind {obj.name!r} must remain a Bezier spline"
        )
    segment_count = int(obj["skate3_retail_grind_segment_count"])
    actual_segments = (
        len(spline.bezier_points)
        if spline.use_cyclic_u
        else len(spline.bezier_points) - 1
    )
    if segment_count <= 0 or actual_segments != segment_count:
        raise ValueError(
            f"retail grind {obj.name!r} segment count changed: "
            f"{actual_segments} versus {segment_count}"
        )
    payload_hex = str(obj["skate3_retail_grind_segment_payload"])
    try:
        payload = bytes.fromhex(payload_hex)
    except ValueError as error:
        raise ValueError(
            f"retail grind {obj.name!r} has invalid native payload"
        ) from error
    if len(payload) != segment_count * 120:
        raise ValueError(
            f"retail grind {obj.name!r} native payload size changed"
        )

    points = spline.bezier_points
    for segment_index in range(segment_count):
        current = points[segment_index]
        following = points[(segment_index + 1) % len(points)]
        actual_controls = (
            _to_runtime(obj.matrix_world @ current.co),
            _to_runtime(obj.matrix_world @ current.handle_right),
            _to_runtime(obj.matrix_world @ following.handle_left),
            _to_runtime(obj.matrix_world @ following.co),
        )
        expected_controls = _retail_grind_controls(
            payload[segment_index * 120 : (segment_index + 1) * 120]
        )
        if not all(
            _close_enough(actual, expected)
            for actual, expected in zip(
                actual_controls,
                expected_controls,
            )
        ):
            raise ValueError(
                f"retail grind {obj.name!r} was edited; exact native "
                f"segment {segment_index} no longer matches its Blender curve"
            )

    return ExportGrindRail(
        name=obj.name,
        closed=bool(spline.use_cyclic_u),
        points=[],
        retail_spline_id=int(
            str(obj["skate3_retail_grind_spline_id"]),
            16,
        ),
        retail_type_signature=int(
            str(obj["skate3_retail_grind_type_signature"]),
            16,
        ),
        retail_flags=int(obj["skate3_retail_grind_flags"]),
        retail_trailing_word=int(
            obj["skate3_retail_grind_trailing_word"]
        ),
        native_segment_payload=payload,
        parent_source_identity=(
            obj.parent.as_pointer()
            if obj.parent is not None and obj.parent.type == "MESH"
            else 0
        ),
    )


def _export_grinds(
    grind_objects: list[bpy.types.Object],
) -> list[ExportGrindRail]:
    rails: list[ExportGrindRail] = []
    for obj in grind_objects:
        if obj.type != "CURVE":
            continue
        if bool(obj.get("skate3_retail_grind", False)):
            rails.append(_export_retail_grind(obj))
            continue
        for spline_index, spline in enumerate(obj.data.splines):
            points: list[tuple[float, float, float]] = []
            if spline.type == "POLY":
                points = [
                    _to_runtime(obj.matrix_world @ Vector(point.co[:3]))
                    for point in spline.points
                ]
            elif spline.type == "BEZIER":
                points = [
                    _to_runtime(obj.matrix_world @ point.co)
                    for point in spline.bezier_points
                ]
            if len(points) < 2:
                continue
            name = obj.name if len(obj.data.splines) == 1 else (
                f"{obj.name}_{spline_index}"
            )
            rails.append(
                ExportGrindRail(
                    name=name,
                    closed=bool(spline.use_cyclic_u),
                    points=points,
                    parent_source_identity=(
                        obj.parent.as_pointer()
                        if obj.parent is not None
                        and obj.parent.type == "MESH"
                        else 0
                    ),
                )
            )
    return rails


def _export_npc_routes(
    npc_path_objects: list[bpy.types.Object],
) -> list[tuple]:
    routes: list[tuple] = []
    for obj in npc_path_objects:
        if obj.type != "CURVE":
            continue
        skater_count = int(obj.get("ow_npc_skater_count", 1))
        speed = float(obj.get("ow_npc_speed", 5.5))
        spawn_spacing = float(obj.get("ow_npc_spawn_spacing", 3.0))
        if not 1 <= skater_count <= 32:
            raise ValueError(
                f"NPC path {obj.name!r} skater count must be 1-32"
            )
        if not math.isfinite(speed) or not 0.0 < speed <= 30.0:
            raise ValueError(
                f"NPC path {obj.name!r} speed must be above 0 and at most 30"
            )
        if (
            not math.isfinite(spawn_spacing)
            or not 0.0 <= spawn_spacing <= 100.0
        ):
            raise ValueError(
                f"NPC path {obj.name!r} spacing must be 0-100 metres"
            )
        for spline_index, spline in enumerate(obj.data.splines):
            points: list[tuple[float, float, float]] = []
            if spline.type == "POLY":
                points = [
                    _to_runtime(obj.matrix_world @ Vector(point.co[:3]))
                    for point in spline.points
                ]
            elif spline.type == "BEZIER":
                points = [
                    _to_runtime(obj.matrix_world @ point.co)
                    for point in spline.bezier_points
                ]
            if len(points) < 2:
                continue
            name = obj.name if len(obj.data.splines) == 1 else (
                f"{obj.name}_{spline_index}"
            )
            routes.append(
                (
                    name,
                    bool(spline.use_cyclic_u),
                    skater_count,
                    speed,
                    spawn_spacing,
                    points,
                )
            )
    return routes


def _door_objects(
    visual_objects: list[bpy.types.Object],
) -> list[bpy.types.Object]:
    return [
        obj
        for obj in visual_objects
        if obj.type == "MESH"
        and str(obj.get("ow_physics_type", "STATIC")) == "HINGED_DOOR"
    ]


def _static_visual_objects(
    visual_objects: list[bpy.types.Object],
) -> list[bpy.types.Object]:
    door_ids = {obj.as_pointer() for obj in _door_objects(visual_objects)}
    return [
        obj
        for obj in visual_objects
        if obj.as_pointer() not in door_ids
    ]


def _normalized_vector(value, label: str) -> Vector:
    vector = Vector(tuple(float(component) for component in value))
    if vector.length <= 1.0e-6:
        raise ValueError(f"{label} must be a non-zero vector")
    return vector.normalized()


def _door_box_collision(
    minimum: Vector,
    maximum: Vector,
    surface_id: int,
    material_id: int,
) -> list[tuple]:
    corners = [
        (
            maximum.x if index & 1 else minimum.x,
            maximum.y if index & 2 else minimum.y,
            maximum.z if index & 4 else minimum.z,
        )
        for index in range(8)
    ]
    faces = (
        (0, 2, 3), (0, 3, 1),
        (4, 5, 7), (4, 7, 6),
        (0, 4, 6), (0, 6, 2),
        (1, 3, 7), (1, 7, 5),
        (0, 1, 5), (0, 5, 4),
        (2, 6, 7), (2, 7, 3),
    )
    return [
        (
            corners[a],
            corners[b],
            corners[c],
            surface_id,
            material_id,
        )
        for a, b, c in faces
    ]


def _export_hinged_doors(
    visual_objects: list[bpy.types.Object],
    material_name_ids: dict[str, int],
    material_variant_ids: dict[
        tuple[str, tuple[float, float, float]], int
    ],
) -> list[ExportHingedDoor]:
    depsgraph = bpy.context.evaluated_depsgraph_get()
    result: list[ExportHingedDoor] = []
    for source_object in _door_objects(visual_objects):
        mesh, evaluated = _mesh_for_export(
            source_object,
            depsgraph,
            preserve_all_data_layers=True,
        )
        try:
            mesh.calc_loop_triangles()
            uv0 = mesh.uv_layers.get("UVMap")
            uv1 = mesh.uv_layers.get("Lightmap")
            if uv0 is None or uv1 is None:
                raise ValueError(
                    f"hinged door {source_object.name!r} requires UVMap "
                    "and Lightmap UV layers"
                )
            if not mesh.loop_triangles:
                raise ValueError(
                    f"hinged door {source_object.name!r} has no triangles"
                )

            hinge_source = source_object.get("ow_hinge_position")
            hinge = (
                Vector(tuple(float(value) for value in hinge_source))
                if hinge_source is not None
                else source_object.matrix_world.translation.copy()
            )
            axis = _normalized_vector(
                source_object.get("ow_hinge_axis", (0.0, 0.0, 1.0)),
                f"hinged door {source_object.name!r} axis",
            )
            world_corners = [
                source_object.matrix_world @ Vector(corner)
                for corner in source_object.bound_box
            ]
            center = sum(world_corners, Vector()) / len(world_corners)
            width = center - hinge
            width -= axis * width.dot(axis)
            if width.length <= 0.05:
                raise ValueError(
                    f"hinged door {source_object.name!r} origin/hinge must "
                    "lie on one side of the door leaf"
                )
            width.normalize()
            depth = width.cross(axis).normalized()
            # Re-orthogonalize width after the cross product so slightly
            # imprecise authored axes cannot introduce render/collision drift.
            width = axis.cross(depth).normalized()

            def local_point(point: Vector) -> Vector:
                delta = point - hinge
                return Vector(
                    (delta.dot(width), delta.dot(axis), delta.dot(depth))
                )

            def local_direction(direction: Vector) -> Vector:
                return Vector(
                    (
                        direction.dot(width),
                        direction.dot(axis),
                        direction.dot(depth),
                    )
                )

            normal_matrix = (
                source_object.matrix_world.to_3x3().inverted().transposed()
            )
            vertices: list[tuple] = []
            indices: list[int] = []
            local_points: list[Vector] = []
            for triangle in mesh.loop_triangles:
                polygon = mesh.polygons[triangle.polygon_index]
                if polygon.material_index >= len(mesh.materials):
                    raise ValueError(
                        f"hinged door {source_object.name!r} has an invalid "
                        "material slot"
                    )
                material = mesh.materials[polygon.material_index]
                if material is None or material.name not in material_name_ids:
                    raise ValueError(
                        f"hinged door {source_object.name!r} has an "
                        "unexported material"
                    )
                variant_key = (
                    material.name,
                    _object_material_tint(source_object, material),
                )
                material_id = material_variant_ids.get(
                    variant_key, material_name_ids[material.name]
                )
                for loop_index in triangle.loops:
                    loop = mesh.loops[loop_index]
                    world_point = (
                        source_object.matrix_world
                        @ mesh.vertices[loop.vertex_index].co
                    )
                    point = local_point(world_point)
                    world_normal = (
                        normal_matrix @ loop.normal
                    ).normalized()
                    normal = local_direction(world_normal).normalized()
                    base_uv = uv0.data[loop_index].uv
                    light_uv = uv1.data[loop_index].uv
                    local_points.append(point)
                    vertices.append(
                        (
                            tuple(float(value) for value in point),
                            tuple(float(value) for value in normal),
                            (float(base_uv.x), float(base_uv.y)),
                            (float(light_uv.x), float(light_uv.y)),
                            material_id,
                        )
                    )
                    indices.append(len(vertices) - 1)

            minimum = Vector(
                tuple(min(point[axis_index] for point in local_points)
                      for axis_index in range(3))
            )
            maximum = Vector(
                tuple(max(point[axis_index] for point in local_points)
                      for axis_index in range(3))
            )
            # Very thin visual planes still need a stable physical leaf.
            if maximum.z - minimum.z < 0.08:
                middle = (minimum.z + maximum.z) * 0.5
                minimum.z = middle - 0.04
                maximum.z = middle + 0.04

            material_name = str(
                source_object.get("ow_door_collision_material", "")
            )
            if not material_name and len(mesh.materials):
                material = mesh.materials[0]
                material_name = material.name if material else ""
            if material_name not in material_name_ids:
                raise ValueError(
                    f"hinged door {source_object.name!r} collision material "
                    f"{material_name!r} is not exported"
                )
            material_id = material_name_ids[material_name]
            surface_id = len(result) + 1

            minimum_angle = math.radians(
                float(source_object.get(
                    "ow_door_min_angle_degrees", -105.0
                ))
            )
            maximum_angle = math.radians(
                float(source_object.get(
                    "ow_door_max_angle_degrees", 105.0
                ))
            )
            initial_angle = math.radians(
                float(source_object.get(
                    "ow_door_initial_angle_degrees", 0.0
                ))
            )
            if not minimum_angle < maximum_angle:
                raise ValueError(
                    f"hinged door {source_object.name!r} has invalid limits"
                )
            if not minimum_angle <= initial_angle <= maximum_angle:
                raise ValueError(
                    f"hinged door {source_object.name!r} initial angle lies "
                    "outside its limits"
                )
            result.append(
                ExportHingedDoor(
                    name=str(source_object.get(
                        "ow_door_name", source_object.name
                    )),
                    hinge_position=_to_runtime(hinge),
                    hinge_axis=_to_runtime(axis),
                    closed_width_axis=_to_runtime(width),
                    closed_depth_axis=_to_runtime(depth),
                    local_min=tuple(float(value) for value in minimum),
                    local_max=tuple(float(value) for value in maximum),
                    minimum_angle_radians=minimum_angle,
                    maximum_angle_radians=maximum_angle,
                    initial_angle_radians=initial_angle,
                    mass=float(source_object.get("ow_door_mass", 32.0)),
                    angular_damping=float(source_object.get(
                        "ow_door_angular_damping", 2.2
                    )),
                    return_spring_strength=float(source_object.get(
                        "ow_door_return_spring_strength", 0.0
                    )),
                    maximum_angular_speed=float(source_object.get(
                        "ow_door_maximum_angular_speed", 8.0
                    )),
                    contact_impulse_scale=float(source_object.get(
                        "ow_door_contact_impulse_scale", 1.0
                    )),
                    friction=float(source_object.get(
                        "ow_door_friction", 0.55
                    )),
                    restitution=float(source_object.get(
                        "ow_door_restitution", 0.02
                    )),
                    surface_id=surface_id,
                    vertices=vertices,
                    indices=indices,
                    collision=_door_box_collision(
                        minimum, maximum, surface_id, material_id
                    ),
                )
            )
        finally:
            if evaluated is not None:
                evaluated.to_mesh_clear()
    return result


def export_scene(
    output_path: str | Path,
    *,
    force_rebuild: bool = False,
    metadata_only: bool = False,
    adopt_existing_cache: bool = False,
    export_editable_objects: bool = True,
    progress: ProgressCallback | None = None,
) -> Path:
    started = time.perf_counter()
    output = Path(output_path).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    _report_progress(progress, 0.0, "Preparing export")

    if metadata_only:
        manifest = _load_cache_manifest(output)
        if manifest is None or not _manifest_matches_package(output, manifest):
            raise ValueError(
                "metadata-only export requires a valid incremental cache; "
                "run a normal export once or use --adopt-existing-cache"
            )
        _patch_package_metadata(output, manifest)
        _refresh_manifest_file_state(output, manifest)
        print(
            "SKATE incremental export:",
            output,
            "mode=metadata_only",
            f"seconds={time.perf_counter() - started:.3f}",
            flush=True,
        )
        _report_progress(progress, 1.0, "Metadata export complete")
        return output

    visual_objects = [
        obj
        for obj in _objects_from_collections(
            PRESENTATION_COLLISION_COLLECTION,
            NO_COLLISION_COLLECTION,
            LEGACY_VISUAL_COLLECTION,
        )
        if bool(obj.get("ow_export_visual", True))
        and not _is_helper_object(obj)
    ]
    static_visual_objects = _static_visual_objects(visual_objects)
    collision_objects = [
        obj
        for obj in _objects_from_collections(
            PRESENTATION_COLLISION_COLLECTION,
            NO_PRESENTATION_COLLECTION,
            LEGACY_COLLISION_COLLECTION,
        )
        if not _is_helper_object(obj)
    ]
    grind_objects = _objects_from_collections(
        GRIND_COLLECTION, LEGACY_GRIND_COLLECTION
    )
    npc_path_objects = _objects_from_collections(
        NPC_PATH_COLLECTION, LEGACY_NPC_PATH_COLLECTION
    )
    material_variants = _used_export_material_variants(
        visual_objects, collision_objects
    )
    materials: list[bpy.types.Material] = []
    seen_materials: set[int] = set()
    for material, _tint in material_variants:
        identity = material.as_pointer()
        if identity not in seen_materials:
            seen_materials.add(identity)
            materials.append(material)
    images, image_ids = _referenced_images(materials)
    material_variant_ids = {
        (material.name, tint): index + 1
        for index, (material, tint) in enumerate(material_variants)
    }
    material_name_ids: dict[str, int] = {}
    for material, tint in material_variants:
        material_name_ids.setdefault(
            material.name,
            material_variant_ids[(material.name, tint)],
        )

    export_materials: list[ExportMaterial] = []
    for material, tint in material_variants:
        material_id = material_variant_ids[
            (material.name, tint)
        ]
        base_color = _material_color(material)
        display_color = tuple(
            base_color[index] * tint[index] for index in range(3)
        )
        export_name = material.name
        if tint != (1.0, 1.0, 1.0):
            export_name += " [Object Color " + ",".join(
                f"{value:.3f}" for value in tint
            ) + "]"
        (
            retail_shader_name,
            retail_shader_family,
            retail_render_flags,
            retail_material_guid,
            retail_material_handle,
            retail_material_group_index,
            retail_texture_bindings,
            retail_parameters,
            retail_source_metadata,
        ) = _retail_material_data(material, image_ids)
        export_materials.append(
            ExportMaterial(
                blender_material=material,
                material_id=material_id,
                export_name=export_name,
                display_color=display_color,
                albedo_texture=_texture_id(
                    material, "ow_albedo_image", image_ids
                ),
                lightmap_texture=_texture_id(
                    material, "ow_lightmap_image", image_ids
                ),
                normal_texture=_texture_id(
                    material, "ow_normal_image", image_ids
                ),
                orm_texture=_texture_id(
                    material, "ow_orm_image", image_ids
                ),
                emissive_texture=_texture_id(
                    material, "ow_emissive_image", image_ids
                ),
                secondary_albedo_texture=_texture_id(
                    material, "ow_secondary_albedo_image", image_ids
                ),
                blend_mask_texture=_texture_id(
                    material, "ow_blend_mask_image", image_ids
                ),
                retail_shader_name=retail_shader_name,
                retail_shader_family=retail_shader_family,
                retail_render_flags=retail_render_flags,
                retail_material_guid=retail_material_guid,
                retail_material_handle=retail_material_handle,
                retail_material_group_index=retail_material_group_index,
                retail_texture_bindings=retail_texture_bindings,
                retail_parameters=retail_parameters,
                retail_source_metadata=retail_source_metadata,
            )
        )

    # Collision is audited before cache checks so a previously cached package
    # cannot conceal newly-invalid source geometry. Harmless zero-area and
    # duplicate triangles are omitted from the package without modifying the
    # Blender mesh (or its visual UVs).
    _report_progress(progress, 0.05, "Auditing collision geometry")
    collision, _collision_audit, collision_object_ranges = _export_collision(
        collision_objects,
        material_name_ids,
        export_editable_objects,
        progress=lambda fraction, stage: _report_progress(
            progress,
            0.05 + fraction * 0.20,
            stage,
        ),
    )
    _report_progress(progress, 0.25, "Collision geometry ready")

    fingerprint: SceneContentFingerprint | None = None
    manifest = _load_cache_manifest(output)
    if adopt_existing_cache:
        if not output.is_file():
            raise ValueError(
                "--adopt-existing-cache requires an existing SKATE package"
            )
        fingerprint = _scene_content_fingerprint(
            visual_objects,
            collision_objects,
            grind_objects,
            npc_path_objects,
            materials,
            images,
            len(collision),
            export_editable_objects,
            progress=lambda fraction, stage: _report_progress(
                progress,
                0.25 + fraction * 0.05,
                stage,
            ),
        )
        package_name, _, package_counts = _read_package_header(output)
        expected_name, _ = _scene_metadata(output)
        expected_counts_except_vertices = (
            len(export_materials),
            len(images),
            fingerprint.visual_indices,
            fingerprint.collision_triangles,
            fingerprint.grind_rails,
            fingerprint.hinged_doors,
            fingerprint.local_lights,
            fingerprint.npc_routes,
        )
        actual_counts_except_vertices = (
            package_counts[0],
            package_counts[1],
            *package_counts[3:],
        )
        if (
            package_name != expected_name
            or actual_counts_except_vertices
            != expected_counts_except_vertices
            or package_counts[2] == 0
            or package_counts[2] > fingerprint.visual_vertices
        ):
            raise ValueError(
                "existing package counts do not match the Blender scene; "
                "a full export is required"
            )
        _patch_package_metadata(output)
        _write_cache_manifest(
            output, fingerprint, len(export_materials), len(images)
        )
        print(
            "SKATE incremental cache adopted:",
            output,
            f"seconds={time.perf_counter() - started:.3f}",
            flush=True,
        )
        _report_progress(progress, 1.0, "Cache adoption complete")
        return output

    if not force_rebuild and manifest is not None:
        fingerprint = _scene_content_fingerprint(
            visual_objects,
            collision_objects,
            grind_objects,
            npc_path_objects,
            materials,
            images,
            len(collision),
            export_editable_objects,
            progress=lambda fraction, stage: _report_progress(
                progress,
                0.25 + fraction * 0.05,
                stage,
            ),
        )
        if (
            fingerprint.digest == manifest.get("content_sha256")
            and _manifest_matches_package(output, manifest)
        ):
            _patch_package_metadata(output, manifest)
            _refresh_manifest_file_state(output, manifest)
            print(
                "SKATE incremental export:",
                output,
                "mode=content_cache_hit",
                f"seconds={time.perf_counter() - started:.3f}",
                flush=True,
            )
            _report_progress(progress, 1.0, "Fast export complete")
            return output
        print(
            "SKATE cache: content changed; rebuilding package",
            flush=True,
        )

    geometry = _export_visual_geometry(
        static_visual_objects,
        material_variant_ids,
        progress=lambda fraction, stage: _report_progress(
            progress,
            0.30 + fraction * 0.40,
            stage,
        ),
    )
    _report_progress(progress, 0.71, "Packing map metadata")
    rails = _export_grinds(grind_objects)
    npc_routes = _export_npc_routes(npc_path_objects)
    doors = _export_hinged_doors(
        visual_objects,
        material_name_ids,
        material_variant_ids,
    )
    lights = _export_local_lights()

    spawn = bpy.data.objects.get(SPAWN_OBJECT)
    if spawn is None:
        raise ValueError(f"required spawn object is missing: {SPAWN_OBJECT}")
    spawn_position = _to_runtime(spawn.matrix_world.translation)
    heading = _spawn_heading(spawn)
    scene = bpy.context.scene
    sun_color, sun_intensity, orbit_azimuth = _sun_metadata()

    world_config_value_offset: int | None = None
    with output.open("wb") as stream:
        stream.write(MAGIC)
        _write_u32(stream, ENDIAN_MARKER)
        _write_string(stream, str(scene.get("ow_map_name", output.stem)))
        _write_vec(stream, spawn_position)
        _write_f32(stream, heading)
        _write_vec(stream, scene.get("ow_sky_zenith", (0.09, 0.34, 0.72)))
        _write_vec(stream, scene.get("ow_sky_horizon", (0.58, 0.78, 0.98)))
        _write_vec(stream, scene.get("ow_sky_nadir", (0.18, 0.25, 0.34)))
        _write_f32(stream, float(scene.get("ow_cycle_seconds", 96.0)))
        _write_f32(stream, float(scene.get("ow_start_hour", 9.0)))
        _write_f32(stream, orbit_azimuth)
        _write_f32(stream, float(scene.get("ow_end_hour", 17.0)))
        _write_f32(
            stream,
            1.0 if bool(scene.get("ow_cycle_ping_pong", False)) else 0.0,
        )
        _write_vec(
            stream,
            scene.get("ow_twilight_zenith", (0.045, 0.10, 0.26)),
        )
        _write_vec(
            stream,
            scene.get("ow_twilight_horizon", (1.0, 0.32, 0.10)),
        )
        _write_vec(
            stream,
            scene.get("ow_twilight_nadir", (0.05, 0.035, 0.06)),
        )
        _write_vec(
            stream,
            scene.get("ow_night_zenith", (0.007, 0.015, 0.045)),
        )
        _write_vec(
            stream,
            scene.get("ow_night_horizon", (0.045, 0.085, 0.17)),
        )
        _write_vec(
            stream,
            scene.get("ow_night_nadir", (0.008, 0.014, 0.032)),
        )
        _write_vec(stream, sun_color)
        _write_vec(
            stream, scene.get("ow_moon_color", (0.42, 0.56, 0.92))
        )
        _write_f32(stream, sun_intensity)
        _write_f32(stream, float(scene.get("ow_moon_intensity", 0.18)))
        _write_f32(stream, float(scene.get("ow_day_ambient", 0.32)))
        _write_f32(stream, float(scene.get("ow_night_ambient", 0.11)))
        _write_vec(stream, scene.get("ow_sky_tint", (1.0, 1.0, 1.0)))
        for count in (
            len(export_materials),
            len(images),
            geometry.vertex_count,
            geometry.index_count,
            len(collision),
            len(rails),
            len(doors),
            len(lights),
            len(npc_routes),
        ):
            _write_u32(stream, count)

        package_stream = stream
        material_stream = io.BytesIO()
        stream = material_stream
        for exported in export_materials:
            material = exported.blender_material
            lightmap_encoding = str(
                material.get("ow_lightmap_encoding", "")
            )
            baked_strength = float(
                material.get("ow_baked_strength", 1.0)
            )
            if (
                exported.lightmap_texture
                and lightmap_encoding
                == "skate3_retail_sqrt_linear_over_4"
            ):
                # The owned shader decodes authored Blender bakes from
                # sqrt(linear / 4) with encoded^2 * 4. Retail pages already
                # contain the console light value consumed as encoded^2,
                # so compensate the common decode without altering a byte
                # of the source page or the artist-facing strength control.
                baked_strength *= 0.25
            _write_string(stream, exported.export_name)
            _write_u32(stream, int(material.get("ow_flags", 1)))
            _write_f32(stream, float(material.get("ow_friction", 0.82)))
            _write_f32(stream, float(material.get("ow_restitution", 0.0)))
            _write_vec(stream, exported.display_color)
            _write_f32(stream, float(material.get("ow_roughness", 0.78)))
            _write_f32(stream, float(material.get("ow_emissive", 0.0)))
            _write_u32(stream, exported.albedo_texture)
            _write_u32(stream, exported.lightmap_texture)
            _write_f32(stream, baked_strength)
            _write_u32(stream, exported.normal_texture)
            _write_u32(stream, exported.orm_texture)
            _write_u32(stream, exported.emissive_texture)
            _write_u32(
                stream,
                _effective_alpha_mode(material),
            )
            alpha_cutoff = float(material.get("ow_alpha_cutoff", 0.5))
            if not 0.0 <= alpha_cutoff <= 1.0:
                raise ValueError(
                    f"material {material.name!r} has invalid alpha cutoff"
                )
            _write_f32(stream, alpha_cutoff)
            _write_u32(
                stream,
                _bounded_int(material, "ow_audio_surface", 3, 93),
            )
            _write_u32(
                stream,
                _bounded_int(material, "ow_physics_surface", 1, 13),
            )
            _write_u32(
                stream,
                _bounded_int(material, "ow_surface_pattern", 0, 15),
            )
            _write_u32(stream, _presentation_depth_layer(material))
            retail_enabled = bool(exported.retail_shader_name)
            _write_u32(stream, 1 if retail_enabled else 0)
            if retail_enabled:
                _write_u64(stream, exported.retail_material_guid)
                _write_u32(stream, exported.retail_material_handle)
                stream.write(
                    struct.pack(
                        "<i", exported.retail_material_group_index
                    )
                )
                _write_string(stream, exported.retail_shader_name)
                _write_u32(stream, exported.retail_shader_family)
                _write_u32(stream, exported.retail_render_flags)
                _write_u32(
                    stream, len(exported.retail_texture_bindings)
                )
                for (
                    semantic,
                    texture_id,
                    uv_set,
                    address_u,
                    address_v,
                ) in exported.retail_texture_bindings:
                    _write_string(stream, semantic)
                    _write_u32(stream, texture_id)
                    _write_u32(stream, uv_set)
                    _write_u32(stream, address_u)
                    _write_u32(stream, address_v)
                _write_u32(stream, len(exported.retail_parameters))
                for name, values in exported.retail_parameters:
                    _write_string(stream, name)
                    _write_u32(stream, len(values))
                    for value in values:
                        _write_string(stream, value)
                _write_string(stream, exported.retail_source_metadata)
        stream = package_stream
        material_bytes = material_stream.getvalue()
        if COMPAT14:
            stream.write(material_bytes)
        else:
            _write_u32(stream, len(material_bytes))
            _write_stored_bytes(stream, material_bytes)

        lightmap_encodings: dict[str, str] = {}
        for material in materials:
            lightmap_name = str(material.get("ow_lightmap_image", ""))
            if not lightmap_name:
                continue
            encoding = str(material.get("ow_lightmap_encoding", ""))
            previous = lightmap_encodings.setdefault(lightmap_name, encoding)
            if previous != encoding:
                raise ValueError(
                    f"lightmap image {lightmap_name!r} is referenced with "
                    "conflicting encodings"
                )
        checked_rgb_images = 0
        any_checked_rgb = False
        texture_payloads: dict[tuple[int, int, bytes], int] = {}
        for image_index, image in enumerate(images, start=1):
            lightmap_encoding = lightmap_encodings.get(image.name, "")
            rgba8 = _image_rgba8(
                image,
                lightmap=image.name in lightmap_encodings,
                lightmap_encoding=lightmap_encoding,
            )
            check_rgb = (
                not bool(image.get("skate3_allow_blank_rgb", False))
                and not str(
                    image.get("skate3_retail_texture_id", "")
                )
            )
            if check_rgb:
                checked_rgb_images += 1
                any_checked_rgb = (
                    any_checked_rgb or _has_nonblank_rgb(rgba8)
                )
            _write_string(stream, image.name)
            _write_u32(stream, int(image.size[0]))
            _write_u32(stream, int(image.size[1]))
            # Authored bakes arrive scene-linear and use sqrt(linear / 4).
            # Retail lightmaps already contain that exact encoded quantity,
            # so their Non-Color bytes pass through without a second transfer.
            # Compression is lossless in both cases.
            _write_u32(stream, 0)
            width = int(image.size[0])
            height = int(image.size[1])
            identity = (width, height, hashlib.sha256(rgba8).digest())
            source_index = None if COMPAT14 else texture_payloads.get(identity)
            if source_index is not None:
                _write_u32(stream, STORAGE_TEXTURE_REFERENCE)
                _write_u32(stream, 4)
                _write_u32(stream, source_index)
            else:
                texture_payloads[identity] = image_index - 1
                _write_transformed_bytes(
                    stream,
                    rgba8,
                    b"" if COMPAT14 else _filter_rgba8(rgba8, width, height),
                    zstd_method=STORAGE_ZSTD_RGBA_FILTER,
                    deflate_method=STORAGE_DEFLATE_RGBA_FILTER,
                )
            _report_progress(
                progress,
                0.72
                + 0.06 * image_index / max(1, len(images)),
                f"Packing textures ({image_index}/{len(images)}): "
                f"{image.name}",
            )
        if checked_rgb_images and not any_checked_rgb:
            raise ValueError(
                "every referenced non-retail texture contains no RGB data; "
                "refusing to export a black SKATE package"
            )

        vertex_bytes = b"".join(geometry.vertex_chunks)
        _write_transformed_bytes(
            stream,
            vertex_bytes,
            b"" if COMPAT14 else _vertex_soa(vertex_bytes),
            zstd_method=STORAGE_ZSTD_VERTEX_SOA,
            deflate_method=STORAGE_DEFLATE_VERTEX_SOA,
        )
        _report_progress(progress, 0.86, "Compressed visual vertices")
        index_bytes = b"".join(geometry.index_chunks)
        _write_transformed_bytes(
            stream,
            index_bytes,
            b"" if COMPAT14 else _index_delta_varints(index_bytes),
            zstd_method=STORAGE_ZSTD_INDEX_DELTA,
            deflate_method=STORAGE_DEFLATE_INDEX_DELTA,
        )
        _report_progress(progress, 0.89, "Compressed visual indices")
        packed_collision = struct.Struct("<9fII4B")
        collision_chunks: list[bytes] = []
        collision_buffer = bytearray()
        collision_flush_size = 16_384
        for collision_index, (
            a,
            b,
            c,
            surface_id,
            material_id,
            native_edge_codes,
        ) in enumerate(collision, start=1):
            edge_codes = native_edge_codes or (0, 0, 0)
            collision_buffer.extend(
                packed_collision.pack(
                    *a,
                    *b,
                    *c,
                    surface_id,
                    material_id,
                    *edge_codes,
                    1 if native_edge_codes is not None else 0,
                )
            )
            if (
                collision_index % collision_flush_size == 0
                or collision_index == len(collision)
            ):
                collision_chunks.append(bytes(collision_buffer))
                collision_buffer.clear()
                _report_progress(
                    progress,
                    0.89
                    + 0.05
                    * collision_index
                    / max(1, len(collision)),
                    f"Writing collision ({collision_index}/"
                    f"{len(collision)})",
                )
        collision_bytes = b"".join(collision_chunks)
        _write_transformed_bytes(
            stream,
            collision_bytes,
            b"" if COMPAT14 else _indexed_collision(collision_bytes),
            zstd_method=STORAGE_ZSTD_COLLISION_INDEXED,
            deflate_method=STORAGE_DEFLATE_COLLISION_INDEXED,
        )
        for rail in rails:
            _write_string(stream, rail.name)
            _write_u32(stream, 1 if rail.closed else 0)
            if rail.native_segment_payload:
                _write_u32(stream, 1)
                _write_u64(stream, rail.retail_spline_id)
                _write_u64(stream, rail.retail_type_signature)
                _write_u32(stream, rail.retail_flags)
                _write_u32(stream, rail.retail_trailing_word)
                segment_count = len(rail.native_segment_payload) // 120
                _write_u32(stream, segment_count)
                for offset in range(
                    0,
                    len(rail.native_segment_payload),
                    4,
                ):
                    _write_u32(
                        stream,
                        int.from_bytes(
                            rail.native_segment_payload[offset : offset + 4],
                            "big",
                        ),
                    )
            else:
                _write_u32(stream, 0)
                _write_u32(stream, len(rail.points))
                for point in rail.points:
                    _write_vec(stream, point)
        for door in doors:
            _write_string(stream, door.name)
            _write_vec(stream, door.hinge_position)
            _write_vec(stream, door.hinge_axis)
            _write_vec(stream, door.closed_width_axis)
            _write_vec(stream, door.closed_depth_axis)
            _write_vec(stream, door.local_min)
            _write_vec(stream, door.local_max)
            _write_f32(stream, door.minimum_angle_radians)
            _write_f32(stream, door.maximum_angle_radians)
            _write_f32(stream, door.initial_angle_radians)
            _write_f32(stream, door.mass)
            _write_f32(stream, door.angular_damping)
            _write_f32(stream, door.return_spring_strength)
            _write_f32(stream, door.maximum_angular_speed)
            _write_f32(stream, door.contact_impulse_scale)
            _write_f32(stream, door.friction)
            _write_f32(stream, door.restitution)
            _write_u32(stream, door.surface_id)
            _write_u32(stream, len(door.vertices))
            _write_u32(stream, len(door.indices))
            _write_u32(stream, len(door.collision))
            for position, normal, uv0, uv1, material_id in door.vertices:
                _write_vec(stream, position)
                _write_vec(stream, normal)
                _write_vec(stream, uv0)
                _write_vec(stream, uv1)
                _write_u32(stream, material_id)
                _write_vec(stream, uv0)
                stream.write(b"\0\0\0\0")
            for index in door.indices:
                _write_u32(stream, index)
            for a, b, c, surface_id, material_id in door.collision:
                _write_vec(stream, a)
                _write_vec(stream, b)
                _write_vec(stream, c)
                _write_u32(stream, surface_id)
                _write_u32(stream, material_id)
                stream.write(b"\0\0\0\0")
        for light in lights:
            _write_string(stream, light.name)
            _write_u32(stream, light.light_type)
            _write_vec(stream, light.position)
            _write_vec(stream, light.direction)
            _write_vec(stream, light.color)
            _write_f32(stream, light.intensity)
            _write_f32(stream, light.influence_radius)
            _write_f32(stream, light.source_radius)
            _write_f32(stream, light.spot_inner_cosine)
            _write_f32(stream, light.spot_outer_cosine)
        for (
            name,
            closed,
            skater_count,
            speed,
            spawn_spacing,
            points,
        ) in npc_routes:
            _write_string(stream, name)
            _write_u32(stream, 1 if closed else 0)
            _write_u32(stream, skater_count)
            _write_f32(stream, speed)
            _write_f32(stream, spawn_spacing)
            _write_u32(stream, len(points))
            for point in points:
                _write_vec(stream, point)
        retail_manifest = bpy.data.texts.get("SKATE3_RETAIL_MANIFEST")
        map_objects = _map_object_extension(
            geometry,
            collision_object_ranges,
            rails,
            export_editable_objects,
        )
        break_groups = _break_group_extension(geometry)
        has_break_groups = any(
            record.editor_editable and record.break_group != 0
            for record in geometry.objects
        )
        _write_u32(
            stream,
            3 + (1 if has_break_groups else 0) +
            (1 if retail_manifest is not None else 0),
        )
        world_config = struct.pack(
            "<I",
            1
            if bool(
                scene.get("ow_dynamic_lighting_enabled_by_default", True)
            )
            else 0,
        )
        stream.write(b"WCFG")
        _write_u32(stream, 1)
        _write_u32(stream, len(world_config))
        _write_u32(stream, STORAGE_RAW)
        _write_u32(stream, len(world_config))
        world_config_value_offset = stream.tell()
        stream.write(world_config)
        stream.write(b"MOBJ")
        _write_u32(stream, 3)
        _write_u32(stream, len(map_objects))
        _write_stored_bytes(stream, map_objects)
        if has_break_groups:
            stream.write(b"BGRP")
            _write_u32(stream, 1)
            _write_u32(stream, len(break_groups))
            _write_stored_bytes(stream, break_groups)
        blender_material_metadata = _blender_material_extension(
            export_materials
        )
        stream.write(b"BMAT")
        _write_u32(stream, 1)
        _write_u32(stream, len(blender_material_metadata))
        _write_stored_bytes(stream, blender_material_metadata)
        if retail_manifest is not None:
            metadata = retail_manifest.as_string().encode("utf-8")
            stream.write(b"WMET")
            _write_u32(stream, 1)
            _write_u32(stream, len(metadata))
            _write_stored_bytes(stream, metadata)

    print(
        "SKATE export:",
        output,
        f"materials={len(export_materials)}",
        f"textures={len(images)}",
        f"triangles={geometry.index_count // 3}",
        f"collision={len(collision)}",
        f"rails={len(rails)}",
        f"doors={len(doors)}",
        f"lights={len(lights)}",
        f"npc_routes={len(npc_routes)}",
        f"seconds={time.perf_counter() - started:.3f}",
        flush=True,
    )
    if fingerprint is None:
        fingerprint = _scene_content_fingerprint(
            visual_objects,
            collision_objects,
            grind_objects,
            npc_path_objects,
            materials,
            images,
            len(collision),
            export_editable_objects,
            progress=lambda fraction, stage: _report_progress(
                progress,
                0.94 + fraction * 0.05,
                stage,
            ),
        )
    _write_cache_manifest(
        output,
        fingerprint,
        len(export_materials),
        len(images),
        world_config_value_offset,
    )
    print(
        "SKATE cache: manifest updated",
        _cache_manifest_path(output),
        flush=True,
    )
    _report_progress(progress, 1.0, "Export complete")
    return output


def main(arguments: list[str] | None = None) -> Path:
    if arguments is None:
        arguments = (
            sys.argv[sys.argv.index("--") + 1 :]
            if "--" in sys.argv
            else []
        )
    if not arguments:
        raise SystemExit("usage: blender --background file.blend --python "
                         "export_skate.py -- output.skate "
                         "[--force|--metadata-only|--adopt-existing-cache] "
                         "[--no-editable-objects]")
    flags = set(arguments[1:])
    allowed_flags = {
        "--force",
        "--metadata-only",
        "--adopt-existing-cache",
        "--no-editable-objects",
    }
    unknown_flags = flags - allowed_flags
    if unknown_flags:
        raise SystemExit(
            "unknown SKATE export arguments: "
            + ", ".join(sorted(unknown_flags))
        )
    selected_modes = sum(
        flag in flags
        for flag in ("--force", "--metadata-only", "--adopt-existing-cache")
    )
    if selected_modes > 1:
        raise SystemExit(
            "--force, --metadata-only, and --adopt-existing-cache are "
            "mutually exclusive"
        )
    return export_scene(
        arguments[0],
        force_rebuild="--force" in flags,
        metadata_only="--metadata-only" in flags,
        adopt_existing_cache="--adopt-existing-cache" in flags,
        export_editable_objects="--no-editable-objects" not in flags,
    )


if __name__ == "__main__":
    main()
