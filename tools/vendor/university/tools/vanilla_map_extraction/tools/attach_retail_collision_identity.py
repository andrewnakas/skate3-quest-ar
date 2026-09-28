#!/usr/bin/env python3
"""Attach exact-retail collision provenance to an existing SKATE v12 map.

RCID v1 maps each portable collision triangle back to every untouched
ClusteredMesh resource/cluster that supplied it. This allows the runtime to
stream exact retail collision until an editor object moves, then replace only
the affected resource without leaving the object's original hitbox behind.
"""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import struct
import sys
import zlib


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import analyze_package  # noqa: E402
from retail_collision_mesh import decode_clustered_mesh  # noqa: E402


ARCHIVE_MAGIC = b"RWCMSET1"
EXTENSION_TAG = b"RCID"
EXTENSION_SCHEMA = 1
COLLISION_RECORD = struct.Struct("<9fII4B")
ASSOCIATION_RECORD = struct.Struct("<IHHIB3x")


def _canonical_key(
    points: tuple[
        tuple[float, float, float],
        tuple[float, float, float],
        tuple[float, float, float],
    ],
    surface: int,
    edge_codes: tuple[int, int, int],
) -> tuple[int, ...]:
    rotations = tuple(
        (
            points[offset:] + points[:offset],
            edge_codes[offset:] + edge_codes[:offset],
        )
        for offset in range(3)
    )
    rotated_points, rotated_codes = min(
        rotations,
        key=lambda item: tuple(
            round(component * 1_000_000.0)
            for point in item[0]
            for component in point
        ),
    )
    return (
        surface,
        *(
            round(component * 1_000_000.0)
            for point in rotated_points
            for component in point
        ),
        *rotated_codes,
    )


def _read_archive(
    path: Path,
) -> list[tuple[str, bytes]]:
    data = path.read_bytes()
    if len(data) < 12 or data[:8] != ARCHIVE_MAGIC:
        raise ValueError("retail collision archive magic is invalid")
    count = struct.unpack_from("<I", data, 8)[0]
    cursor = 12
    records: list[tuple[str, bytes]] = []
    for index in range(count):
        if cursor + 4 > len(data):
            raise ValueError(f"archive record {index} name is truncated")
        name_size = struct.unpack_from("<I", data, cursor)[0]
        cursor += 4
        if cursor + name_size + 4 > len(data):
            raise ValueError(f"archive record {index} name is invalid")
        name = data[cursor : cursor + name_size].decode("utf-8")
        cursor += name_size
        mesh_size = struct.unpack_from("<I", data, cursor)[0]
        cursor += 4
        if cursor + mesh_size > len(data):
            raise ValueError(f"archive record {index} mesh is truncated")
        records.append((name, data[cursor : cursor + mesh_size]))
        cursor += mesh_size
    if cursor != len(data):
        raise ValueError("retail collision archive has trailing bytes")
    return records


def _stored(payload: bytes) -> bytes:
    compressed = zlib.compress(payload, level=9)
    if len(compressed) < len(payload):
        return struct.pack("<II", 1, len(compressed)) + compressed
    return struct.pack("<II", 0, len(payload)) + payload


def attach_identity(
    package_path: Path,
    archive_path: Path,
    output_path: Path,
) -> dict[str, object]:
    analysis = analyze_package(package_path, include_payloads=True)
    if int(analysis["version"]) != 12:
        raise ValueError("RCID requires a SKATE v12 package")
    if EXTENSION_TAG.decode("ascii") in analysis["extension_tags"]:
        raise ValueError("SKATE package already contains RCID")

    materials = {
        int(material["id"]): material
        for material in analysis["_materials"]
    }
    collision = analysis["_collision_bytes"]
    if len(collision) % COLLISION_RECORD.size:
        raise ValueError("SKATE collision block has an invalid size")
    triangle_count = len(collision) // COLLISION_RECORD.size

    package_lookup: dict[tuple[int, ...], int] = {}
    for triangle_index in range(triangle_count):
        values = COLLISION_RECORD.unpack_from(
            collision, triangle_index * COLLISION_RECORD.size
        )
        material = materials.get(int(values[10]))
        if material is None:
            raise ValueError(
                f"collision triangle {triangle_index} has no material"
            )
        surface = (
            int(material["audio_surface"])
            | (int(material["physics_surface"]) << 7)
            | (int(material["surface_pattern"]) << 12)
        )
        if values[14] != 1:
            raise ValueError(
                f"collision triangle {triangle_index} omits edge codes"
            )
        key = _canonical_key(
            (
                tuple(values[0:3]),
                tuple(values[3:6]),
                tuple(values[6:9]),
            ),
            surface,
            tuple(values[11:14]),
        )
        previous = package_lookup.setdefault(key, triangle_index)
        if previous != triangle_index:
            raise ValueError(
                "SKATE collision contains an ambiguous duplicate at "
                f"triangles {previous} and {triangle_index}"
            )

    archive_records = _read_archive(archive_path)
    if len(archive_records) > 0xFFFF:
        raise ValueError("retail collision archive has too many resources")
    associations: list[tuple[int, int, int, int, int]] = []
    coverage = bytearray(triangle_count)
    unmatched_exact = 0
    for resource_index, (name, mesh_bytes) in enumerate(archive_records):
        mesh = decode_clustered_mesh(mesh_bytes)
        if mesh.cluster_count > 0xFFFF:
            raise ValueError(f"{name} has too many collision clusters")
        for source_index, triangle in enumerate(mesh.triangles):
            if triangle.edge_codes is None:
                raise ValueError(f"{name} omits retail edge codes")
            key = _canonical_key(
                (triangle.a, triangle.b, triangle.c),
                triangle.surface,
                triangle.edge_codes,
            )
            triangle_index = package_lookup.get(key)
            if triangle_index is None:
                unmatched_exact += 1
                continue
            cluster_index = mesh.triangle_cluster_indices[source_index]
            associations.append(
                (
                    triangle_index,
                    resource_index,
                    cluster_index,
                    (
                        triangle.group_id
                        if triangle.group_id is not None
                        else 0xFFFFFFFF
                    ),
                    triangle.unit_flags,
                )
            )
            coverage[triangle_index] = 1

    missing = coverage.count(0)
    if missing:
        first = coverage.index(0)
        raise ValueError(
            "retail collision archive does not cover every package triangle: "
            f"missing={missing} first={first}"
        )
    associations = sorted(set(associations))

    payload = bytearray()
    payload.extend(struct.pack("<I", len(archive_records)))
    for name, _mesh in archive_records:
        encoded = name.encode("utf-8")
        payload.extend(struct.pack("<I", len(encoded)))
        payload.extend(encoded)
    payload.extend(struct.pack("<II", triangle_count, len(associations)))
    for association in associations:
        payload.extend(ASSOCIATION_RECORD.pack(*association))

    package_bytes = bytearray(package_path.read_bytes())
    extension_bytes = int(analysis["sections"]["extensions"])
    extension_offset = len(package_bytes) - extension_bytes
    if extension_offset < 0 or extension_offset + 4 > len(package_bytes):
        raise ValueError("SKATE extension table location is invalid")
    extension_count = struct.unpack_from(
        "<I", package_bytes, extension_offset
    )[0]
    if extension_count != len(analysis["extension_tags"]):
        raise ValueError("SKATE extension count changed during analysis")
    struct.pack_into(
        "<I", package_bytes, extension_offset, extension_count + 1
    )
    package_bytes.extend(EXTENSION_TAG)
    package_bytes.extend(struct.pack("<II", EXTENSION_SCHEMA, len(payload)))
    package_bytes.extend(_stored(bytes(payload)))

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(package_bytes)
    return {
        "status": "SKATE_RETAIL_COLLISION_IDENTITY_OK",
        "input": str(package_path),
        "archive": str(archive_path),
        "output": str(output_path),
        "resources": len(archive_records),
        "triangles": triangle_count,
        "associations": len(associations),
        "unmatched_exact_triangles": unmatched_exact,
        "identity_decoded_bytes": len(payload),
        "output_bytes": len(package_bytes),
        "output_sha256": hashlib.sha256(package_bytes).hexdigest(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("archive", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()
    result = attach_identity(
        arguments.package.resolve(),
        arguments.archive.resolve(),
        arguments.output.resolve(),
    )
    import json

    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
