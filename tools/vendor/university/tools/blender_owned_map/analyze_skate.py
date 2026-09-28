#!/usr/bin/env python3
"""Report the byte layout and integrity-relevant counts of a SKATE package."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import zlib
from dataclasses import dataclass
from pathlib import Path
try:
    from compression import zstd
except ImportError:
    zstd = None


VERTEX_BYTES = 44
VERTEX_BYTES_V12 = 56
COLLISION_TRIANGLE_BYTES_V10 = 44
COLLISION_TRIANGLE_BYTES_V11 = 48
RETAIL_COLLISION_ARCHIVE_MAGIC = b"RWCMSET1"


def _inflate(stored: bytes, expected: int, label: str) -> bytes:
    try:
        decoded = zlib.decompress(stored)
    except zlib.error as error:
        raise PackageError(f"{label} DEFLATE payload is invalid") from error
    if len(decoded) != expected:
        raise PackageError(
            f"{label} decoded to {len(decoded)} bytes; expected {expected}"
        )
    return decoded


def _unzstd(stored: bytes, expected: int, label: str) -> bytes:
    if zstd is None:
        raise PackageError(
            f"{label} uses Zstandard; run this analyzer with Python 3.14+"
        )
    try:
        decoded = zstd.decompress(stored)
    except Exception as error:
        raise PackageError(f"{label} Zstandard payload is invalid") from error
    if len(decoded) != expected:
        raise PackageError(
            f"{label} decoded to {len(decoded)} bytes; expected {expected}"
        )
    return decoded


def _undo_rgba_filter(data: bytes, expected: int, label: str) -> bytes:
    source = Reader(data)
    width = source.u32(f"{label} filtered width")
    height = source.u32(f"{label} filtered height")
    row_size = width * 4
    if row_size * height != expected:
        raise PackageError(f"{label} filtered dimensions are invalid")
    result = bytearray(expected)
    for y in range(height):
        filter_type = source.take(1, f"{label} row filter")[0]
        if filter_type > 4:
            raise PackageError(f"{label} row filter is invalid")
        filtered = source.take(row_size, f"{label} filtered row")
        row_offset = y * row_size
        for x, value in enumerate(filtered):
            left = result[row_offset + x - 4] if x >= 4 else 0
            above = result[row_offset - row_size + x] if y else 0
            upper_left = (
                result[row_offset - row_size + x - 4]
                if y and x >= 4
                else 0
            )
            if filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = above
            elif filter_type == 3:
                predictor = (left + above) // 2
            elif filter_type == 4:
                prediction = left + above - upper_left
                distances = (
                    abs(prediction - left),
                    abs(prediction - above),
                    abs(prediction - upper_left),
                )
                predictor = (left, above, upper_left)[distances.index(
                    min(distances)
                )]
            else:
                predictor = 0
            result[row_offset + x] = (value + predictor) & 0xFF
    if source.offset != len(data):
        raise PackageError(f"{label} filtered payload has trailing bytes")
    return bytes(result)


def _undo_vertex_soa(data: bytes, expected: int, label: str) -> bytes:
    if expected % VERTEX_BYTES_V12 or len(data) != expected:
        raise PackageError(f"{label} vertex streams are invalid")
    result = bytearray(expected)
    fields = ((0, 12), (12, 12), (24, 8), (32, 8), (40, 4),
              (44, 8), (52, 4))
    source = 0
    count = expected // VERTEX_BYTES_V12
    for field_offset, field_size in fields:
        for vertex in range(count):
            destination = vertex * VERTEX_BYTES_V12 + field_offset
            result[destination : destination + field_size] = data[
                source : source + field_size
            ]
            source += field_size
    return bytes(result)


def _read_varuint(data: bytes, offset: int, label: str) -> tuple[int, int]:
    value = 0
    for shift in range(0, 64, 7):
        if offset >= len(data):
            raise PackageError(f"{label} varint is truncated")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, offset
    raise PackageError(f"{label} varint is invalid")


def _undo_index_delta(data: bytes, expected: int, label: str) -> bytes:
    if expected % 4:
        raise PackageError(f"{label} index size is invalid")
    result = bytearray(expected)
    previous = 0
    offset = 0
    for index in range(expected // 4):
        zigzag, offset = _read_varuint(data, offset, label)
        delta = (zigzag >> 1) ^ -(zigzag & 1)
        current = previous + delta
        if not 0 <= current <= 0xFFFFFFFF:
            raise PackageError(f"{label} index delta is invalid")
        struct.pack_into("<I", result, index * 4, current)
        previous = current
    if offset != len(data):
        raise PackageError(f"{label} has trailing index data")
    return bytes(result)


def _undo_indexed_collision(data: bytes, expected: int, label: str) -> bytes:
    if expected % COLLISION_TRIANGLE_BYTES_V11:
        raise PackageError(f"{label} collision size is invalid")
    source = Reader(data)
    vertex_count = source.u32(f"{label} collision vertex count")
    vertices = source.take(vertex_count * 12, f"{label} collision vertices")
    result = bytearray(expected)
    for triangle in range(expected // COLLISION_TRIANGLE_BYTES_V11):
        destination = triangle * COLLISION_TRIANGLE_BYTES_V11
        for corner in range(3):
            vertex = source.u32(f"{label} collision vertex index")
            if vertex >= vertex_count:
                raise PackageError(f"{label} collision vertex is invalid")
            result[
                destination + corner * 12 : destination + corner * 12 + 12
            ] = vertices[vertex * 12 : vertex * 12 + 12]
        result[destination + 36 : destination + 48] = source.take(
            12, f"{label} collision fields"
        )
    if source.offset != len(data):
        raise PackageError(f"{label} indexed collision has trailing bytes")
    return bytes(result)


class PackageError(ValueError):
    """Raised when a package cannot be parsed without guessing."""


@dataclass
class Reader:
    data: bytes
    offset: int = 0

    def take(self, size: int, label: str) -> bytes:
        if size < 0 or self.offset + size > len(self.data):
            raise PackageError(f"{label} extends past the end of the package")
        start = self.offset
        self.offset += size
        return self.data[start : self.offset]

    def u32(self, label: str) -> int:
        return struct.unpack("<I", self.take(4, label))[0]

    def u64(self, label: str) -> int:
        return struct.unpack("<Q", self.take(8, label))[0]

    def string(self, label: str) -> str:
        size = self.u32(f"{label} length")
        try:
            return self.take(size, label).decode("utf-8")
        except UnicodeDecodeError as error:
            raise PackageError(f"{label} is not valid UTF-8") from error

    def skip(self, size: int, label: str) -> None:
        self.take(size, label)

    def stored(
        self,
        expected_size: int,
        label: str,
        references: list[bytes] | None = None,
    ) -> bytes:
        method = self.u32(f"{label} storage method")
        stored_size = self.u32(f"{label} stored size")
        stored = self.take(stored_size, f"{label} payload")
        if method == 0:
            if stored_size != expected_size:
                raise PackageError(
                    f"{label} raw size is {stored_size}; "
                    f"expected {expected_size}"
                )
            return stored
        if method == 11:
            if references is None or len(stored) != 4:
                raise PackageError(f"{label} texture reference is invalid")
            source = struct.unpack("<I", stored)[0]
            if source >= len(references):
                raise PackageError(f"{label} texture reference is out of range")
            decoded = references[source]
            if len(decoded) != expected_size:
                raise PackageError(f"{label} texture reference size differs")
            return decoded
        if method in (1, 2):
            return (
                _inflate(stored, expected_size, label)
                if method == 1
                else _unzstd(stored, expected_size, label)
            )
        if method not in range(3, 11) or len(stored) < 4:
            raise PackageError(f"{label} uses unsupported method {method}")
        transformed_size = struct.unpack("<I", stored[:4])[0]
        transformed = (
            _unzstd(stored[4:], transformed_size, label)
            if method <= 6
            else _inflate(stored[4:], transformed_size, label)
        )
        if method in (3, 7):
            return _undo_rgba_filter(transformed, expected_size, label)
        if method in (4, 8):
            return _undo_vertex_soa(transformed, expected_size, label)
        if method in (5, 9):
            return _undo_index_delta(transformed, expected_size, label)
        return _undo_indexed_collision(transformed, expected_size, label)


def _section(
    sections: dict[str, int],
    reader: Reader,
    name: str,
    start: int,
) -> None:
    sections[name] = reader.offset - start


def _summarize_retail_collision_archive(
    payload: bytes,
) -> dict[str, object]:
    archive = Reader(payload)
    if archive.take(8, "RWCM archive magic") != RETAIL_COLLISION_ARCHIVE_MAGIC:
        raise PackageError("RWCM extension archive magic is invalid")
    mesh_count = archive.u32("RWCM mesh count")
    if mesh_count == 0:
        raise PackageError("RWCM extension contains no meshes")
    mesh_payload_bytes = 0
    for index in range(mesh_count):
        name_size = archive.u32(f"RWCM mesh {index} name length")
        if name_size == 0 or name_size > 4096:
            raise PackageError(f"RWCM mesh {index} name is invalid")
        try:
            archive.take(
                name_size, f"RWCM mesh {index} name"
            ).decode("utf-8")
        except UnicodeDecodeError as error:
            raise PackageError(
                f"RWCM mesh {index} name is not valid UTF-8"
            ) from error
        mesh_size = archive.u32(f"RWCM mesh {index} payload size")
        if mesh_size < 96:
            raise PackageError(f"RWCM mesh {index} payload is invalid")
        archive.skip(mesh_size, f"RWCM mesh {index} payload")
        mesh_payload_bytes += mesh_size
    if archive.offset != len(payload):
        raise PackageError("RWCM extension archive has trailing bytes")
    return {
        "meshes": mesh_count,
        "decoded_bytes": len(payload),
        "mesh_payload_bytes": mesh_payload_bytes,
        "sha256": hashlib.sha256(payload).hexdigest(),
    }


def analyze_package(
    path: Path,
    *,
    include_payloads: bool = False,
) -> dict[str, object]:
    reader = Reader(path.read_bytes())
    sections: dict[str, int] = {}

    start = reader.offset
    magic = reader.take(8, "magic")
    if magic not in (
        b"SKATE08\0",
        b"SKATE09\0",
        b"SKATE10\0",
        b"SKATE11\0",
        b"SKATE12\0",
        b"SKATE13\0",
        b"SKATE14\0",
        b"SKATE15\0",
    ):
        raise PackageError(
            f"unsupported magic {magic!r}; expected SKATE v8-v15"
        )
    version = int(magic[5:7])
    if reader.u32("endian marker") != 0x12345678:
        raise PackageError("invalid endian marker")
    semantic_metadata_start = reader.offset
    map_name = reader.string("map name")
    # Spawn/environment metadata between the map name and count table.
    reader.skip(49 * 4, "map metadata")
    counts = struct.unpack("<9I", reader.take(9 * 4, "count table"))
    (
        material_count,
        texture_count,
        vertex_count,
        index_count,
        collision_count,
        rail_count,
        door_count,
        light_count,
        route_count,
    ) = counts
    semantic_metadata = reader.data[semantic_metadata_start : reader.offset - 36]
    _section(sections, reader, "header_and_map_metadata", start)

    start = reader.offset
    package_reader = reader
    if version >= 15:
        decoded_material_size = reader.u32("material block decoded size")
        reader = Reader(
            reader.stored(decoded_material_size, "material block")
        )
    material_start = reader.offset
    material_alpha_modes = {0: 0, 1: 0, 2: 0}
    material_depth_layers = {0: 0, 1: 0, 2: 0, 3: 0}
    retail_family_counts: dict[int, int] = {}
    retail_binding_count = 0
    retail_parameter_count = 0
    material_records: list[dict[str, object]] = []
    for index in range(material_count):
        name = reader.string(f"material {index} name")
        fields = reader.take(76, f"material {index} fields")
        alpha_mode = struct.unpack_from("<I", fields, 56)[0]
        if alpha_mode not in material_alpha_modes:
            raise PackageError(
                f"material {index} uses invalid alpha mode {alpha_mode}"
            )
        material_alpha_modes[alpha_mode] += 1
        record = {
                "id": index + 1,
                "name": name,
                "display_color": struct.unpack_from("<3f", fields, 12),
                "roughness": struct.unpack_from("<f", fields, 24)[0],
                "emissive_intensity": struct.unpack_from(
                    "<f", fields, 28
                )[0],
                "albedo_texture": struct.unpack_from("<I", fields, 32)[0],
                "lightmap_texture": struct.unpack_from("<I", fields, 36)[0],
                "baked_indirect_strength": struct.unpack_from(
                    "<f", fields, 40
                )[0],
                "normal_texture": struct.unpack_from("<I", fields, 44)[0],
                "orm_texture": struct.unpack_from("<I", fields, 48)[0],
                "emissive_texture": struct.unpack_from("<I", fields, 52)[0],
                "alpha_mode": alpha_mode,
                "alpha_cutoff": struct.unpack_from("<f", fields, 60)[0],
                "audio_surface": struct.unpack_from("<I", fields, 64)[0],
                "physics_surface": struct.unpack_from("<I", fields, 68)[0],
                "surface_pattern": struct.unpack_from("<I", fields, 72)[0],
        }
        if version >= 13:
            depth_layer = reader.u32(
                f"material {index} presentation depth layer"
            )
        elif alpha_mode == 2:
            depth_layer = 3
        elif any(
            token in name.lower()
            for token in (
                "sign", "poster", "billboard", "adbord", "advert",
                "banner", "logo", "decal", "graffiti", "sticker",
                "plaque", "letter", "neon",
            )
        ):
            depth_layer = 2
        elif alpha_mode == 1:
            depth_layer = 1
        else:
            depth_layer = 0
        if depth_layer not in material_depth_layers:
            raise PackageError(
                f"material {index} uses invalid depth layer {depth_layer}"
            )
        material_depth_layers[depth_layer] += 1
        record["presentation_depth_layer"] = depth_layer
        if version >= 12:
            retail_enabled = reader.u32(
                f"material {index} retail enabled"
            ) != 0
            record["retail_enabled"] = retail_enabled
            if retail_enabled:
                record["retail_material_guid"] = reader.u64(
                    f"material {index} retail guid"
                )
                record["retail_material_handle"] = reader.u32(
                    f"material {index} retail handle"
                )
                record["retail_material_group_index"] = struct.unpack(
                    "<i",
                    reader.take(
                        4, f"material {index} retail group index"
                    ),
                )[0]
                record["retail_shader_name"] = reader.string(
                    f"material {index} retail shader"
                )
                family = reader.u32(
                    f"material {index} retail family"
                )
                record["retail_shader_family"] = family
                retail_family_counts[family] = (
                    retail_family_counts.get(family, 0) + 1
                )
                record["retail_render_flags"] = reader.u32(
                    f"material {index} retail flags"
                )
                binding_count = reader.u32(
                    f"material {index} retail binding count"
                )
                bindings = []
                for binding_index in range(binding_count):
                    bindings.append(
                        {
                            "semantic": reader.string(
                                f"material {index} binding "
                                f"{binding_index} semantic"
                            ),
                            "texture": reader.u32(
                                f"material {index} binding "
                                f"{binding_index} texture"
                            ),
                            "uv_set": reader.u32(
                                f"material {index} binding "
                                f"{binding_index} uv set"
                            ),
                            "address_u": reader.u32(
                                f"material {index} binding "
                                f"{binding_index} address u"
                            ),
                            "address_v": reader.u32(
                                f"material {index} binding "
                                f"{binding_index} address v"
                            ),
                        }
                    )
                record["retail_texture_bindings"] = bindings
                retail_binding_count += binding_count
                parameter_count = reader.u32(
                    f"material {index} retail parameter count"
                )
                parameters = {}
                for parameter_index in range(parameter_count):
                    parameter_name = reader.string(
                        f"material {index} parameter "
                        f"{parameter_index} name"
                    )
                    value_count = reader.u32(
                        f"material {index} parameter "
                        f"{parameter_index} value count"
                    )
                    parameters[parameter_name] = [
                        reader.string(
                            f"material {index} parameter "
                            f"{parameter_index} value {value_index}"
                        )
                        for value_index in range(value_count)
                    ]
                record["retail_parameters"] = parameters
                retail_parameter_count += parameter_count
                record["retail_source_metadata"] = reader.string(
                    f"material {index} retail source metadata"
                )
        material_records.append(record)
    material_bytes = reader.data[material_start : reader.offset]
    if version >= 15:
        if reader.offset != len(reader.data):
            raise PackageError("material block has trailing bytes")
        reader = package_reader
    _section(sections, reader, "materials", start)

    start = reader.offset
    texture_decoded_bytes = 0
    texture_payloads: list[bytes] = []
    texture_dimensions: list[dict[str, object]] = []
    texture_digest = hashlib.sha256()
    for index in range(texture_count):
        name = reader.string(f"texture {index} name")
        width = reader.u32(f"texture {index} width")
        height = reader.u32(f"texture {index} height")
        color_space = reader.u32(f"texture {index} color space")
        expected = width * height * 4
        if version >= 9:
            rgba8 = reader.stored(
                expected, f"texture {index}", texture_payloads
            )
        else:
            byte_count = reader.u32(f"texture {index} byte count")
            if byte_count != expected:
                raise PackageError(
                    f"texture {index} has {byte_count} bytes; expected {expected}"
                )
            rgba8 = reader.take(byte_count, f"texture {index} RGBA8")
        texture_payloads.append(rgba8)
        encoded_name = name.encode("utf-8")
        texture_digest.update(struct.pack("<I", len(encoded_name)))
        texture_digest.update(encoded_name)
        texture_digest.update(struct.pack("<3I", width, height, color_space))
        texture_digest.update(rgba8)
        texture_decoded_bytes += len(rgba8)
        alpha = rgba8[3::4]
        texture_dimensions.append(
            {
                "id": index + 1,
                "name": name,
                "width": width,
                "height": height,
                "color_space": color_space,
                "decoded_bytes": len(rgba8),
                "alpha_min": min(alpha) if alpha else None,
                "alpha_max": max(alpha) if alpha else None,
                "transparent_texels": alpha.count(0),
                "opaque_texels": alpha.count(255),
            }
        )
    _section(sections, reader, "textures", start)

    start = reader.offset
    vertex_stride = VERTEX_BYTES_V12 if version >= 12 else VERTEX_BYTES
    vertex_byte_count = vertex_count * vertex_stride
    if version >= 9:
        vertex_bytes = reader.stored(
            vertex_byte_count, "visual vertex block"
        )
    else:
        vertex_bytes = reader.take(vertex_byte_count, "visual vertices")
    _section(sections, reader, "visual_vertices", start)

    start = reader.offset
    index_byte_count = index_count * 4
    if version >= 9:
        index_bytes = reader.stored(index_byte_count, "visual index block")
    else:
        index_bytes = reader.take(index_byte_count, "visual indices")
    if index_count:
        indices = struct.unpack(f"<{index_count}I", index_bytes)
        maximum_index = max(indices)
        if maximum_index >= vertex_count:
            raise PackageError(
                f"visual index {maximum_index} exceeds vertex count {vertex_count}"
            )
    else:
        indices = ()
        maximum_index = None
    _section(sections, reader, "visual_indices", start)

    start = reader.offset
    collision_triangle_bytes = (
        COLLISION_TRIANGLE_BYTES_V11
        if version >= 11
        else COLLISION_TRIANGLE_BYTES_V10
    )
    collision_byte_count = collision_count * collision_triangle_bytes
    if version >= 9:
        collision_bytes = reader.stored(
            collision_byte_count, "collision block"
        )
    else:
        collision_bytes = reader.take(
            collision_byte_count, "collision triangles"
        )
    _section(sections, reader, "collision", start)

    authored_features_start = reader.offset
    start = reader.offset
    grind_section_start = reader.offset
    native_grind_segments = 0
    for index in range(rail_count):
        reader.string(f"rail {index} name")
        reader.u32(f"rail {index} closed")
        representation = (
            reader.u32(f"rail {index} representation")
            if version >= 10
            else 0
        )
        if representation == 0:
            point_count = reader.u32(f"rail {index} point count")
            reader.skip(point_count * 12, f"rail {index} points")
        elif representation == 1 and version >= 10:
            reader.u64(f"rail {index} retail spline id")
            reader.u64(f"rail {index} retail type signature")
            reader.u32(f"rail {index} retail flags")
            reader.u32(f"rail {index} retail trailing word")
            segment_count = reader.u32(f"rail {index} segment count")
            reader.skip(
                segment_count * 120,
                f"rail {index} native segments",
            )
            native_grind_segments += segment_count
        else:
            raise PackageError(
                f"rail {index} uses unsupported representation "
                f"{representation}"
            )
    _section(sections, reader, "grind_rails", start)
    grind_section_bytes = reader.data[grind_section_start : reader.offset]

    start = reader.offset
    for index in range(door_count):
        reader.string(f"door {index} name")
        reader.skip(28 * 4, f"door {index} float fields")
        reader.u32(f"door {index} surface")
        door_vertex_count = reader.u32(f"door {index} vertex count")
        door_index_count = reader.u32(f"door {index} index count")
        door_collision_count = reader.u32(f"door {index} collision count")
        reader.skip(
            door_vertex_count * vertex_stride,
            f"door {index} vertices",
        )
        door_indices = reader.take(
            door_index_count * 4,
            f"door {index} indices",
        )
        if door_index_count:
            maximum_door_index = max(
                struct.unpack(f"<{door_index_count}I", door_indices)
            )
            if maximum_door_index >= door_vertex_count:
                raise PackageError(
                    f"door {index} index {maximum_door_index} exceeds "
                    f"vertex count {door_vertex_count}"
                )
        reader.skip(
            door_collision_count * collision_triangle_bytes,
            f"door {index} collision",
        )
    _section(sections, reader, "hinged_doors", start)

    start = reader.offset
    for index in range(light_count):
        reader.string(f"light {index} name")
        reader.skip(4 + 14 * 4, f"light {index} fields")
    _section(sections, reader, "local_lights", start)

    start = reader.offset
    for index in range(route_count):
        reader.string(f"route {index} name")
        reader.skip(4 + 4 + 4 + 4, f"route {index} fields")
        point_count = reader.u32(f"route {index} point count")
        reader.skip(point_count * 12, f"route {index} points")
    _section(sections, reader, "npc_routes", start)

    extension_tags: list[str] = []
    map_objects: list[dict[str, object]] = []
    break_groups: dict[int, dict[str, object]] = {}
    blender_material_compatibility: list[dict[str, object]] = []
    embedded_retail_collision: dict[str, object] | None = None
    embedded_retail_collision_bytes = b""
    dynamic_lighting_enabled_by_default = True
    extension_bytes = b""
    if version >= 12:
        start = reader.offset
        extension_count = reader.u32("extension count")
        for extension_index in range(extension_count):
            tag = reader.take(
                4, f"extension {extension_index} tag"
            ).decode("ascii", errors="replace")
            schema = reader.u32(f"extension {extension_index} schema")
            decoded_size = reader.u32(
                f"extension {extension_index} decoded size"
            )
            payload = reader.stored(
                decoded_size, f"extension {extension_index}"
            )
            extension_tags.append(tag)
            if tag == "WCFG":
                if schema != 1 or len(payload) != 4:
                    raise PackageError("WCFG extension is invalid")
                dynamic_lighting_enabled_by_default = (
                    struct.unpack("<I", payload)[0] != 0
                )
            elif tag == "MOBJ":
                if schema not in (1, 2, 3) or (
                    schema == 3 and version < 14
                ):
                    raise PackageError(
                        f"MOBJ schema {schema} is incompatible with "
                        f"SKATE v{version}"
                    )
                object_reader = Reader(payload)
                object_count = object_reader.u32("MOBJ object count")
                for object_index in range(object_count):
                    object_id = object_reader.u32(
                        f"MOBJ object {object_index} id"
                    )
                    name = object_reader.string(
                        f"MOBJ object {object_index} name"
                    )
                    origin = struct.unpack(
                        "<3f",
                        object_reader.take(
                            12, f"MOBJ object {object_index} origin"
                        ),
                    )
                    object_first_index = object_reader.u32(
                        f"MOBJ object {object_index} first index"
                    )
                    object_index_count = object_reader.u32(
                        f"MOBJ object {object_index} index count"
                    )
                    object_first_collision = object_reader.u32(
                        f"MOBJ object {object_index} first collision"
                    )
                    object_collision_count = object_reader.u32(
                        f"MOBJ object {object_index} collision count"
                    )
                    grind_indices = []
                    if schema >= 2:
                        grind_count = object_reader.u32(
                            f"MOBJ object {object_index} grind count"
                        )
                        grind_indices = [
                            object_reader.u32(
                                f"MOBJ object {object_index} grind {grind}"
                            )
                            for grind in range(grind_count)
                        ]
                    physics = {
                        "type": 0,
                        "shape": 0,
                        "density": 100.0,
                        "friction": 0.55,
                        "restitution": 0.05,
                        "linear_damping": 0.05,
                        "angular_damping": 0.15,
                        "gravity_scale": 1.0,
                        "enable_sleep": True,
                        "initially_awake": True,
                        "break_group": 0,
                        "break_speed_threshold": 2.5,
                        "break_impulse_scale": 0.45,
                        "break_angular_impulse": 0.08,
                        "break_gravity_scale": 1.0,
                    }
                    if schema >= 3:
                        values = struct.unpack(
                            "<II6fII",
                            object_reader.take(
                                40,
                                f"MOBJ object {object_index} physics",
                            ),
                        )
                        if (
                            values[0] > 2
                            or values[1] > 2
                            or values[8] > 1
                            or values[9] > 1
                            or not 0.0 < values[2] <= 100000.0
                            or not 0.0 <= values[3] <= 2.0
                            or not 0.0 <= values[4] <= 1.0
                            or not 0.0 <= values[5] <= 100.0
                            or not 0.0 <= values[6] <= 100.0
                            or not -10.0 <= values[7] <= 10.0
                        ):
                            raise PackageError(
                                f"MOBJ object {object_index} has invalid "
                                "physics metadata"
                            )
                        physics = {
                            "type": values[0],
                            "shape": values[1],
                            "density": values[2],
                            "friction": values[3],
                            "restitution": values[4],
                            "linear_damping": values[5],
                            "angular_damping": values[6],
                            "gravity_scale": values[7],
                            "enable_sleep": bool(values[8]),
                            "initially_awake": bool(values[9]),
                            "break_group": 0,
                            "break_speed_threshold": 2.5,
                            "break_impulse_scale": 0.45,
                            "break_angular_impulse": 0.08,
                            "break_gravity_scale": 1.0,
                        }
                    map_objects.append(
                        {
                            "id": object_id,
                            "name": name,
                            "origin": origin,
                            "first_index": object_first_index,
                            "index_count": object_index_count,
                            "first_collision": object_first_collision,
                            "collision_count": object_collision_count,
                            "grind_indices": grind_indices,
                            "physics": physics,
                        }
                    )
                if object_reader.offset != len(payload):
                    raise PackageError("MOBJ has trailing bytes")
            elif tag == "BGRP":
                if schema != 1:
                    raise PackageError(
                        f"BGRP schema {schema} is unsupported"
                    )
                break_reader = Reader(payload)
                break_count = break_reader.u32("BGRP object count")
                for break_index in range(break_count):
                    object_id, group, speed, impulse, spin, gravity = (
                        struct.unpack(
                            "<II4f",
                            break_reader.take(
                                24, f"BGRP object {break_index}"
                            ),
                        )
                    )
                    if (
                        object_id in break_groups
                        or group == 0
                        or not 0.1 <= speed <= 30.0
                        or not 0.0 <= impulse <= 10.0
                        or not 0.0 <= spin <= 10.0
                        or not 0.0 <= gravity <= 4.0
                    ):
                        raise PackageError(
                            f"BGRP object {break_index} is invalid"
                        )
                    break_groups[object_id] = {
                        "break_group": group,
                        "break_speed_threshold": speed,
                        "break_impulse_scale": impulse,
                        "break_angular_impulse": spin,
                        "break_gravity_scale": gravity,
                    }
                if break_reader.offset != len(payload):
                    raise PackageError("BGRP has trailing bytes")
            elif tag == "BMAT" and schema == 1:
                material_reader = Reader(payload)
                count = material_reader.u32(
                    "Blender material compatibility count"
                )
                if count != material_count:
                    raise PackageError(
                        "Blender material compatibility count "
                        f"{count} does not match {material_count} materials"
                    )
                for material_index in range(count):
                    values = struct.unpack(
                        "<IIIfIIIII",
                        material_reader.take(
                            36,
                            f"Blender material compatibility "
                            f"{material_index}",
                        ),
                    )
                    material_id = values[0]
                    if not 1 <= material_id <= material_count:
                        raise PackageError(
                            "Blender material compatibility uses invalid "
                            f"material ID {material_id}"
                        )
                    compatibility = {
                        "material_id": material_id,
                        "secondary_albedo_texture": values[1],
                        "blend_mask_texture": values[2],
                        "blend_factor": values[3],
                        "blend_mask_channel": values[4],
                        "albedo_address_mode": values[5],
                        "secondary_address_mode": values[6],
                        "blend_mask_address_mode": values[7],
                        "cull_mode": values[8],
                    }
                    blender_material_compatibility.append(compatibility)
                    material_records[material_id - 1].update(compatibility)
                if material_reader.offset != len(payload):
                    raise PackageError(
                        "Blender material compatibility has trailing bytes"
                    )
            elif tag == "RWCM":
                if schema != 1:
                    raise PackageError(
                        f"RWCM schema {schema} is unsupported"
                    )
                if embedded_retail_collision is not None:
                    raise PackageError(
                        "package contains duplicate RWCM extensions"
                    )
                embedded_retail_collision = (
                    _summarize_retail_collision_archive(payload)
                )
                embedded_retail_collision["schema"] = schema
                embedded_retail_collision_bytes = payload
        objects_by_id = {record["id"]: record for record in map_objects}
        for object_id, breakable in break_groups.items():
            record = objects_by_id.get(object_id)
            if record is None or record["physics"]["type"] != 2:
                raise PackageError(
                    "BGRP references a missing or non-dynamic object"
                )
            record["physics"].update(breakable)
        _section(sections, reader, "extensions", start)
        extension_bytes = reader.data[start : reader.offset]

    if reader.offset != len(reader.data):
        raise PackageError(
            f"package has {len(reader.data) - reader.offset} trailing bytes"
        )

    triangle_digest = hashlib.sha256()
    quantized_triangle_digest = hashlib.sha256()
    for index in indices:
        offset = index * vertex_stride
        record = vertex_bytes[offset : offset + vertex_stride]
        triangle_digest.update(record)
        if version >= 12:
            base_values = struct.unpack("<10fI2f4b", record)
            floats = (
                *base_values[:10],
                *base_values[11:13],
                *(value / 127.0 for value in base_values[13:17]),
            )
            material_id = base_values[10]
        else:
            base_values = struct.unpack("<10fI", record)
            floats = base_values[:10]
            material_id = base_values[10]
        quantized = [
            round(value * 1_000_000.0)
            for value in floats
        ]
        try:
            quantized_triangle_digest.update(
                struct.pack(
                    f"<{len(floats)}qI",
                    *quantized,
                    material_id,
                )
            )
        except struct.error as error:
            raise PackageError(
                f"vertex {index} contains values outside the analyzer's "
                f"quantized range: {floats!r}"
            ) from error
    positions = [
        struct.unpack_from("<3f", vertex_bytes, offset)
        for offset in range(0, len(vertex_bytes), vertex_stride)
    ]
    bounds_min = [
        min(position[axis] for position in positions) for axis in range(3)
    ]
    bounds_max = [
        max(position[axis] for position in positions) for axis in range(3)
    ]

    result = {
        "path": str(path.resolve()),
        "version": version,
        "map_name": map_name,
        "file_bytes": len(reader.data),
        "counts": {
            "materials": material_count,
            "textures": texture_count,
            "vertices": vertex_count,
            "indices": index_count,
            "collision_triangles": collision_count,
            "grind_rails": rail_count,
            "native_grind_segments": native_grind_segments,
            "hinged_doors": door_count,
            "local_lights": light_count,
            "npc_routes": route_count,
            "retail_texture_bindings": retail_binding_count,
            "retail_material_parameters": retail_parameter_count,
        },
        "sections": sections,
        "material_alpha_modes": {
            "opaque": material_alpha_modes[0],
            "mask": material_alpha_modes[1],
            "blend": material_alpha_modes[2],
        },
        "material_depth_layers": material_depth_layers,
        "retail_shader_families": retail_family_counts,
        "extension_tags": extension_tags,
        "dynamic_lighting_enabled_by_default":
            dynamic_lighting_enabled_by_default,
        "embedded_retail_collision": embedded_retail_collision,
        "map_objects": map_objects,
        "blender_material_compatibility": (
            blender_material_compatibility
        ),
        "texture_decoded_bytes": texture_decoded_bytes,
        "texture_dimensions": texture_dimensions,
        "maximum_visual_index": maximum_index,
        "integrity": {
            "semantic_metadata_sha256": hashlib.sha256(
                semantic_metadata
            ).hexdigest(),
            "materials_sha256": hashlib.sha256(material_bytes).hexdigest(),
            "decoded_textures_sha256": texture_digest.hexdigest(),
            "expanded_visual_triangles_sha256": (
                triangle_digest.hexdigest()
            ),
            "expanded_visual_triangles_1e6_sha256": (
                quantized_triangle_digest.hexdigest()
            ),
            "collision_sha256": hashlib.sha256(
                collision_bytes
            ).hexdigest(),
            "grind_rails_sha256": hashlib.sha256(
                grind_section_bytes
            ).hexdigest(),
            "extensions_sha256": hashlib.sha256(
                extension_bytes
            ).hexdigest(),
            "authored_features_sha256": hashlib.sha256(
                reader.data[authored_features_start : reader.offset]
            ).hexdigest(),
            "bounds_min": bounds_min,
            "bounds_max": bounds_max,
        },
    }
    if include_payloads:
        result["_materials"] = material_records
        result["_vertex_bytes"] = vertex_bytes
        result["_indices"] = indices
        result["_collision_bytes"] = collision_bytes
        result["_embedded_retail_collision_bytes"] = (
            embedded_retail_collision_bytes
        )
    return result


def _format_bytes(value: int) -> str:
    units = ("B", "KiB", "MiB", "GiB")
    amount = float(value)
    for unit in units:
        if amount < 1024.0 or unit == units[-1]:
            return f"{amount:.2f} {unit}"
        amount /= 1024.0
    raise AssertionError("unreachable")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument(
        "--json",
        action="store_true",
        help="write machine-readable JSON instead of a table",
    )
    args = parser.parse_args()
    result = analyze_package(args.package)
    if args.json:
        print(json.dumps(result, indent=2))
        return 0

    total = int(result["file_bytes"])
    print(f"{result['map_name']} — SKATE v{result['version']}")
    print(f"File: {_format_bytes(total)} ({total:,} bytes)")
    print()
    print(f"{'Section':<26} {'Bytes':>14} {'Share':>9}")
    for name, size_value in result["sections"].items():
        size = int(size_value)
        share = 100.0 * size / max(1, total)
        print(f"{name:<26} {size:>14,} {share:>8.2f}%")
    print()
    print("Counts:", json.dumps(result["counts"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
