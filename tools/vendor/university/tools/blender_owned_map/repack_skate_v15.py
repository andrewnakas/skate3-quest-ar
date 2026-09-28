#!/usr/bin/env python3
"""Losslessly repack an existing SKATE12-14 package as SKATE15.

The decoded material, texture, geometry, collision, authored-feature, and
extension bytes are preserved exactly. Only reversible storage transforms and
the optional per-map dynamic-lighting startup preference are changed.
"""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import struct
import sys
import zlib

import numpy
try:
    from compression import zstd
except ImportError:
    zstd = None

from analyze_skate import Reader, PackageError


MAGICS = {b"SKATE12\0": 12, b"SKATE13\0": 13, b"SKATE14\0": 14}
MAGIC_V15 = b"SKATE15\0"
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
VERTEX_BYTES = 56
COLLISION_BYTES = 48


def write_u32(stream, value: int) -> None:
    stream.write(struct.pack("<I", value))


def write_storage_record(stream, method: int, payload: bytes) -> None:
    write_u32(stream, method)
    write_u32(stream, len(payload))
    stream.write(payload)


def best_plain_storage(decoded: bytes) -> tuple[int, bytes]:
    candidates = [
        (STORAGE_DEFLATE, zlib.compress(decoded, level=9)),
    ]
    if zstd is not None:
        candidates.append(
            (STORAGE_ZSTD, zstd.compress(decoded, level=10))
        )
    method, payload = min(candidates, key=lambda item: len(item[1]))
    return (
        (STORAGE_RAW, decoded)
        if len(payload) >= len(decoded)
        else (method, payload)
    )


def best_transformed_storage(
    decoded: bytes,
    transformed: bytes,
    *,
    zstd_method: int,
    deflate_method: int,
) -> tuple[int, bytes]:
    prefix = struct.pack("<I", len(transformed))
    candidates = [
        (
            deflate_method,
            prefix + zlib.compress(transformed, level=9),
        ),
        (STORAGE_DEFLATE, zlib.compress(decoded, level=9)),
    ]
    if zstd is not None:
        candidates.extend((
            (
                zstd_method,
                prefix + zstd.compress(transformed, level=10),
            ),
            (STORAGE_ZSTD, zstd.compress(decoded, level=10)),
        ))
    selected = min(candidates, key=lambda item: len(item[1]))
    return (
        (STORAGE_RAW, decoded)
        if len(selected[1]) >= len(decoded)
        else selected
    )


def filtered_rgba8(decoded: bytes, width: int, height: int) -> bytes:
    row_size = width * 4
    if len(decoded) != row_size * height:
        raise PackageError("texture dimensions do not match RGBA8 bytes")
    output = bytearray(struct.pack("<II", width, height))
    previous = numpy.zeros(row_size, dtype=numpy.int16)
    for row_index in range(height):
        row_u8 = numpy.frombuffer(
            decoded,
            dtype=numpy.uint8,
            count=row_size,
            offset=row_index * row_size,
        )
        row = row_u8.astype(numpy.int16)
        left = numpy.zeros(row_size, dtype=numpy.int16)
        left[4:] = row[:-4]
        candidates = (
            row_u8,
            ((row - left) & 0xFF).astype(numpy.uint8),
            ((row - previous) & 0xFF).astype(numpy.uint8),
            (
                (
                    row
                    - (
                        (
                            left.astype(numpy.int32)
                            + previous.astype(numpy.int32)
                        )
                        // 2
                    )
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
        selected = min(range(len(scores)), key=scores.__getitem__)
        output.append(selected)
        output.extend(candidates[selected].tobytes())
        previous = row
    return bytes(output)


def vertex_soa(decoded: bytes) -> bytes:
    if len(decoded) % VERTEX_BYTES:
        raise PackageError("visual vertex block is not 56-byte aligned")
    vertices = numpy.frombuffer(decoded, dtype=numpy.uint8).reshape(
        -1, VERTEX_BYTES
    )
    fields = ((0, 12), (12, 24), (24, 32), (32, 40),
              (40, 44), (44, 52), (52, 56))
    return b"".join(
        numpy.ascontiguousarray(vertices[:, first:last]).tobytes()
        for first, last in fields
    )


def index_delta_varints(decoded: bytes) -> bytes:
    if len(decoded) % 4:
        raise PackageError("visual index block is not u32 aligned")
    output = bytearray()
    previous = 0
    for current in numpy.frombuffer(decoded, dtype="<u4"):
        value = int(current)
        delta = value - previous
        zigzag = (delta << 1) if delta >= 0 else ((-delta << 1) - 1)
        while zigzag >= 0x80:
            output.append((zigzag & 0x7F) | 0x80)
            zigzag >>= 7
        output.append(zigzag)
        previous = value
    return bytes(output)


def indexed_collision(decoded: bytes) -> bytes:
    if len(decoded) % COLLISION_BYTES:
        raise PackageError("collision block is not 48-byte aligned")
    records = numpy.frombuffer(decoded, dtype=numpy.uint8).reshape(
        -1, COLLISION_BYTES
    )
    positions = (
        records[:, :36]
        .reshape(-1, 12)
        .view("<u4")
        .reshape(-1, 3)
    )
    unique, inverse = numpy.unique(
        positions, axis=0, return_inverse=True
    )
    triangle_indices = inverse.astype("<u4").reshape(-1, 3)
    compact_records = numpy.empty(
        (len(records), 24), dtype=numpy.uint8
    )
    compact_records[:, :12] = triangle_indices.view(numpy.uint8).reshape(
        -1, 12
    )
    compact_records[:, 12:] = records[:, 36:48]
    return (
        struct.pack("<I", len(unique))
        + numpy.ascontiguousarray(unique).tobytes()
        + compact_records.tobytes()
    )


def material_block_v15(
    data: bytes,
    reader: Reader,
    count: int,
    version: int,
) -> bytes:
    output = bytearray()
    overlay_tokens = (
        "sign", "poster", "billboard", "adbord", "advert",
        "banner", "logo", "decal", "graffiti", "sticker",
        "plaque", "letter", "neon",
    )
    for material in range(count):
        name_start = reader.offset
        name = reader.string(f"material {material} name")
        output.extend(data[name_start : reader.offset])
        fields = reader.take(76, f"material {material} fields")
        output.extend(fields)
        if version >= 13:
            output.extend(
                reader.take(4, f"material {material} depth layer")
            )
        else:
            alpha_mode = struct.unpack_from("<I", fields, 56)[0]
            lower_name = name.lower()
            if alpha_mode == 2:
                depth_layer = 3
            elif any(token in lower_name for token in overlay_tokens):
                depth_layer = 2
            elif alpha_mode == 1:
                depth_layer = 1
            else:
                depth_layer = 0
            output.extend(struct.pack("<I", depth_layer))
        if version < 12:
            continue
        retail_start = reader.offset
        retail_enabled = reader.u32(f"material {material} retail enabled")
        if not retail_enabled:
            output.extend(data[retail_start : reader.offset])
            continue
        reader.skip(16, f"material {material} retail identity")
        reader.string(f"material {material} retail shader")
        reader.skip(8, f"material {material} retail shader fields")
        binding_count = reader.u32(f"material {material} binding count")
        for binding in range(binding_count):
            reader.string(f"material {material} binding {binding}")
            reader.skip(16, f"material {material} binding fields")
        parameter_count = reader.u32(f"material {material} parameter count")
        for parameter in range(parameter_count):
            reader.string(f"material {material} parameter {parameter}")
            value_count = reader.u32(
                f"material {material} parameter {parameter} value count"
            )
            for value in range(value_count):
                reader.string(
                    f"material {material} parameter {parameter} value {value}"
                )
        reader.string(f"material {material} retail source metadata")
        output.extend(data[retail_start : reader.offset])
    return bytes(output)


def skip_authored_features(
    reader: Reader,
    counts: tuple[int, ...],
    version: int,
) -> None:
    _, _, _, _, _, rails, doors, lights, routes = counts
    for rail in range(rails):
        reader.string(f"rail {rail} name")
        reader.skip(4, f"rail {rail} closed")
        representation = reader.u32(f"rail {rail} representation")
        if representation == 0:
            points = reader.u32(f"rail {rail} point count")
            reader.skip(points * 12, f"rail {rail} points")
        elif representation == 1:
            reader.skip(24, f"rail {rail} retail header")
            segments = reader.u32(f"rail {rail} segment count")
            reader.skip(segments * 120, f"rail {rail} segments")
        else:
            raise PackageError(f"rail {rail} representation is invalid")
    for door in range(doors):
        reader.string(f"door {door} name")
        reader.skip(116, f"door {door} header")
        vertex_count = reader.u32(f"door {door} vertex count")
        index_count = reader.u32(f"door {door} index count")
        collision_count = reader.u32(f"door {door} collision count")
        reader.skip(vertex_count * VERTEX_BYTES, f"door {door} vertices")
        reader.skip(index_count * 4, f"door {door} indices")
        reader.skip(collision_count * COLLISION_BYTES, f"door {door} collision")
    for light in range(lights):
        reader.string(f"light {light} name")
        reader.skip(60, f"light {light} fields")
    for route in range(routes):
        reader.string(f"route {route} name")
        reader.skip(16, f"route {route} fields")
        points = reader.u32(f"route {route} point count")
        reader.skip(points * 12, f"route {route} points")


def repack(
    source: Path,
    destination: Path,
    *,
    dynamic_lighting: bool,
) -> dict[str, int]:
    data = source.read_bytes()
    reader = Reader(data)
    magic = reader.take(8, "magic")
    version = MAGICS.get(magic)
    if version is None:
        raise PackageError("input must be an SKATE12, SKATE13, or SKATE14 map")
    header_start = reader.offset
    marker = reader.u32("endian marker")
    if marker != 0x12345678:
        raise PackageError("input endian marker is invalid")
    reader.string("map name")
    reader.skip(49 * 4, "map metadata")
    counts = struct.unpack("<9I", reader.take(36, "count table"))
    header_end = reader.offset

    material_bytes = material_block_v15(
        data, reader, counts[0], version
    )

    temporary = destination.with_name(destination.name + ".tmp")
    temporary.parent.mkdir(parents=True, exist_ok=True)
    temporary.unlink(missing_ok=True)
    stats: dict[str, int] = {}
    with temporary.open("wb") as output:
        output.write(MAGIC_V15)
        output.write(data[header_start:header_end])
        write_u32(output, len(material_bytes))
        method, payload = best_plain_storage(material_bytes)
        write_storage_record(output, method, payload)
        stats["materials"] = len(payload) + 8

        texture_payloads: dict[tuple[int, int, bytes], int] = {}
        decoded_textures: list[bytes] = []
        texture_start = output.tell()
        for texture in range(counts[1]):
            record_start = reader.offset
            reader.string(f"texture {texture} name")
            width = reader.u32(f"texture {texture} width")
            height = reader.u32(f"texture {texture} height")
            reader.u32(f"texture {texture} color space")
            metadata = data[record_start : reader.offset]
            rgba8 = reader.stored(
                width * height * 4,
                f"texture {texture}",
                decoded_textures,
            )
            decoded_textures.append(rgba8)
            output.write(metadata)
            identity = (width, height, hashlib.sha256(rgba8).digest())
            reference = texture_payloads.get(identity)
            if reference is not None:
                write_storage_record(
                    output,
                    STORAGE_TEXTURE_REFERENCE,
                    struct.pack("<I", reference),
                )
            else:
                texture_payloads[identity] = texture
                method, payload = best_transformed_storage(
                    rgba8,
                    filtered_rgba8(rgba8, width, height),
                    zstd_method=STORAGE_ZSTD_RGBA_FILTER,
                    deflate_method=STORAGE_DEFLATE_RGBA_FILTER,
                )
                write_storage_record(output, method, payload)
        stats["textures"] = output.tell() - texture_start

        vertex_bytes = reader.stored(
            counts[2] * VERTEX_BYTES, "visual vertex block"
        )
        method, payload = best_transformed_storage(
            vertex_bytes,
            vertex_soa(vertex_bytes),
            zstd_method=STORAGE_ZSTD_VERTEX_SOA,
            deflate_method=STORAGE_DEFLATE_VERTEX_SOA,
        )
        write_storage_record(output, method, payload)
        stats["vertices"] = len(payload) + 8

        index_bytes = reader.stored(
            counts[3] * 4, "visual index block"
        )
        method, payload = best_transformed_storage(
            index_bytes,
            index_delta_varints(index_bytes),
            zstd_method=STORAGE_ZSTD_INDEX_DELTA,
            deflate_method=STORAGE_DEFLATE_INDEX_DELTA,
        )
        write_storage_record(output, method, payload)
        stats["indices"] = len(payload) + 8

        collision_bytes = reader.stored(
            counts[4] * COLLISION_BYTES, "collision block"
        )
        method, payload = best_transformed_storage(
            collision_bytes,
            indexed_collision(collision_bytes),
            zstd_method=STORAGE_ZSTD_COLLISION_INDEXED,
            deflate_method=STORAGE_DEFLATE_COLLISION_INDEXED,
        )
        write_storage_record(output, method, payload)
        stats["collision"] = len(payload) + 8

        authored_start = reader.offset
        skip_authored_features(reader, counts, version)
        output.write(data[authored_start : reader.offset])

        extension_count = reader.u32("extension count")
        extensions: list[tuple[bytes, int, bytes]] = []
        for extension in range(extension_count):
            tag = reader.take(4, f"extension {extension} tag")
            schema = reader.u32(f"extension {extension} schema")
            decoded_size = reader.u32(
                f"extension {extension} decoded size"
            )
            payload = reader.stored(
                decoded_size, f"extension {extension}"
            )
            if tag != b"WCFG":
                extensions.append((tag, schema, payload))
        if reader.offset != len(data):
            raise PackageError("input package has trailing bytes")
        extensions.append(
            (b"WCFG", 1, struct.pack("<I", int(dynamic_lighting)))
        )
        write_u32(output, len(extensions))
        extension_start = output.tell()
        for tag, schema, decoded in extensions:
            output.write(tag)
            write_u32(output, schema)
            write_u32(output, len(decoded))
            method, payload = best_plain_storage(decoded)
            write_storage_record(output, method, payload)
        stats["extensions"] = output.tell() - extension_start
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, destination)
    stats["input"] = len(data)
    stats["output"] = destination.stat().st_size
    return stats


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    lighting = parser.add_mutually_exclusive_group()
    lighting.add_argument(
        "--dynamic-lighting-on", action="store_true", default=True
    )
    lighting.add_argument(
        "--dynamic-lighting-off",
        action="store_false",
        dest="dynamic_lighting_on",
    )
    args = parser.parse_args()
    stats = repack(
        args.source.resolve(),
        args.destination.resolve(),
        dynamic_lighting=args.dynamic_lighting_on,
    )
    saved = stats["input"] - stats["output"]
    print(
        f"SKATE15 repack: {args.destination.resolve()}\n"
        f"input={stats['input']} output={stats['output']} "
        f"saved={saved} ({saved / stats['input']:.2%})"
    )
    for name in (
        "materials", "textures", "vertices", "indices",
        "collision", "extensions",
    ):
        print(f"{name}={stats[name]}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, PackageError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
