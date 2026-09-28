#!/usr/bin/env python3
"""Select the nearest native collision records from an RWCMSET1 archive."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import struct


MAGIC = b"RWCMSET1"


def _read_exact(stream, size: int) -> bytes:
    value = stream.read(size)
    if len(value) != size:
        raise ValueError("collision archive is truncated")
    return value


def _distance_squared(
    x: float,
    z: float,
    minimum_x: float,
    minimum_z: float,
    maximum_x: float,
    maximum_z: float,
) -> float:
    dx = minimum_x - x if x < minimum_x else x - maximum_x if x > maximum_x else 0.0
    dz = minimum_z - z if z < minimum_z else z - maximum_z if z > maximum_z else 0.0
    return dx * dx + dz * dz


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path, nargs="?")
    parser.add_argument("--count", type=int, default=32)
    parser.add_argument("--center-x", type=float, default=0.0)
    parser.add_argument("--center-z", type=float, default=0.0)
    parser.add_argument(
        "--describe",
        action="store_true",
        help="print record bounds and triangle counts without writing",
    )
    args = parser.parse_args()
    if args.count <= 0:
        raise ValueError("selection count must be positive")

    archive_size = args.input.stat().st_size
    records: list[dict[str, object]] = []
    with args.input.open("rb") as stream:
        if _read_exact(stream, len(MAGIC)) != MAGIC:
            raise ValueError("collision archive magic is invalid")
        record_count = struct.unpack("<I", _read_exact(stream, 4))[0]
        for index in range(record_count):
            name_size = struct.unpack("<I", _read_exact(stream, 4))[0]
            if name_size == 0 or name_size > 4096:
                raise ValueError(f"record {index} has an invalid name")
            name = _read_exact(stream, name_size)
            mesh_size = struct.unpack("<I", _read_exact(stream, 4))[0]
            if mesh_size < 96:
                raise ValueError(f"record {index} has an invalid mesh")
            mesh_offset = stream.tell()
            if mesh_offset + mesh_size > archive_size:
                raise ValueError(
                    f"record {index} extends beyond the collision archive"
                )
            header = _read_exact(stream, 48)
            minimum_x, _, minimum_z = struct.unpack_from(">fff", header, 0)
            maximum_x, _, maximum_z = struct.unpack_from(">fff", header, 16)
            triangle_count = struct.unpack_from(">I", header, 40)[0]
            records.append(
                {
                    "index": index,
                    "name": name,
                    "mesh_offset": mesh_offset,
                    "mesh_size": mesh_size,
                    "triangles": triangle_count,
                    "distance_squared": _distance_squared(
                        args.center_x,
                        args.center_z,
                        minimum_x,
                        minimum_z,
                        maximum_x,
                        maximum_z,
                    ),
                }
            )
            stream.seek(mesh_offset + mesh_size)
        if stream.read(1):
            raise ValueError("collision archive has trailing bytes")
    if not records:
        raise ValueError("collision archive contains no records")

    selected = sorted(
        records,
        key=lambda record: (
            record["distance_squared"],
            record["index"],
        ),
    )[: args.count]
    if args.describe:
        for record in sorted(
            records,
            key=lambda value: (
                -int(value["triangles"]),
                int(value["index"]),
            ),
        ):
            print(
                f"{record['name'].decode('utf-8', errors='replace')}\t"
                f"triangles={record['triangles']}\t"
                f"distance={float(record['distance_squared']) ** 0.5:.3f}"
            )
        return 0
    if args.output is None:
        parser.error("output is required unless --describe is used")
    temporary = args.output.with_name(args.output.name + ".tmp")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.input.open("rb") as source, temporary.open("wb") as target:
        target.write(MAGIC)
        target.write(struct.pack("<I", len(selected)))
        for record in selected:
            name = record["name"]
            mesh_size = int(record["mesh_size"])
            source.seek(int(record["mesh_offset"]))
            target.write(struct.pack("<I", len(name)))
            target.write(name)
            target.write(struct.pack("<I", mesh_size))
            remaining = mesh_size
            while remaining:
                block = _read_exact(source, min(8 * 1024 * 1024, remaining))
                target.write(block)
                remaining -= len(block)
    os.replace(temporary, args.output)

    print(
        "RW_COLLISION_ARCHIVE_SELECT_OK"
        f" input_records={len(records)}"
        f" selected_records={len(selected)}"
        f" triangles={sum(int(record['triangles']) for record in selected)}"
        f" bytes={args.output.stat().st_size}"
        f" center=({args.center_x},{args.center_z})"
        f" farthest_distance={selected[-1]['distance_squared'] ** 0.5:.3f}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
