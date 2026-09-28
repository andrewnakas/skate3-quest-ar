#!/usr/bin/env python3
"""Embed an exact RWCMSET1 collision archive in a SKATE v12-v14 package.

The resulting map remains portable because its ordinary collision triangles
are left untouched. The RWCM extension is an optional exact-retail payload
used first by compatible runtimes; older runtimes can ignore the extension.
Re-running this tool replaces any existing RWCM record without disturbing
other extensions such as SKYB, MOBJ, RCID, or BMAT.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct

from export_skate2_skybox import (
    COPY_BYTES,
    PackageError,
    _find_extension_offset,
    _read_extensions,
    _stored,
)


ARCHIVE_MAGIC = b"RWCMSET1"
EXTENSION_TAG = b"RWCM"
EXTENSION_SCHEMA = 1
MAXIMUM_ARCHIVE_BYTES = 512 * 1024 * 1024


def validate_archive(data: bytes) -> dict[str, int]:
    if len(data) < 12 or len(data) > MAXIMUM_ARCHIVE_BYTES:
        raise PackageError("retail collision archive size is invalid")
    if data[:8] != ARCHIVE_MAGIC:
        raise PackageError("retail collision archive magic is invalid")
    mesh_count = struct.unpack_from("<I", data, 8)[0]
    if mesh_count == 0 or mesh_count > 16 * 1024 * 1024:
        raise PackageError("retail collision archive mesh count is invalid")

    cursor = 12
    mesh_payload_bytes = 0
    for index in range(mesh_count):
        if cursor + 4 > len(data):
            raise PackageError(f"archive mesh {index} name is truncated")
        name_size = struct.unpack_from("<I", data, cursor)[0]
        cursor += 4
        if (
            name_size == 0
            or name_size > 4096
            or cursor + name_size + 4 > len(data)
        ):
            raise PackageError(f"archive mesh {index} name is invalid")
        try:
            data[cursor : cursor + name_size].decode("utf-8")
        except UnicodeDecodeError as error:
            raise PackageError(
                f"archive mesh {index} name is not valid UTF-8"
            ) from error
        cursor += name_size
        mesh_size = struct.unpack_from("<I", data, cursor)[0]
        cursor += 4
        if mesh_size < 96 or cursor + mesh_size > len(data):
            raise PackageError(f"archive mesh {index} payload is invalid")
        cursor += mesh_size
        mesh_payload_bytes += mesh_size
    if cursor != len(data):
        raise PackageError("retail collision archive has trailing bytes")
    return {
        "meshes": mesh_count,
        "mesh_payload_bytes": mesh_payload_bytes,
    }


def embed_archive(
    source: Path,
    archive: Path,
    destination: Path,
) -> dict[str, object]:
    source = source.resolve()
    archive = archive.resolve()
    destination = destination.resolve()
    if source == destination:
        raise ValueError("destination must differ from source")
    if archive == destination:
        raise ValueError("archive and destination must differ")

    archive_data = archive.read_bytes()
    archive_report = validate_archive(archive_data)
    extension_offset = _find_extension_offset(source)
    records = [
        record
        for tag, record in _read_extensions(source, extension_offset)
        if tag != EXTENSION_TAG
    ]
    encoded = (
        EXTENSION_TAG
        + struct.pack("<II", EXTENSION_SCHEMA, len(archive_data))
        + _stored(archive_data)
    )

    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    try:
        with source.open("rb") as input_stream, temporary.open("wb") as output:
            remaining = extension_offset
            while remaining:
                block = input_stream.read(min(remaining, COPY_BYTES))
                if not block:
                    raise PackageError(
                        "source ended before its extension table"
                    )
                output.write(block)
                remaining -= len(block)
            output.write(struct.pack("<I", len(records) + 1))
            for record in records:
                output.write(record)
            output.write(encoded)
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)

    return {
        "status": "SKATE_EMBEDDED_RETAIL_COLLISION_OK",
        "source": str(source),
        "archive": str(archive),
        "output": str(destination),
        "meshes": archive_report["meshes"],
        "archive_bytes": len(archive_data),
        "mesh_payload_bytes": archive_report["mesh_payload_bytes"],
        "archive_sha256": hashlib.sha256(archive_data).hexdigest(),
        "output_bytes": destination.stat().st_size,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("archive", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()
    print(
        json.dumps(
            embed_archive(
                arguments.source,
                arguments.archive,
                arguments.output,
            ),
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
