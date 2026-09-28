#!/usr/bin/env python3
"""Package untouched retail RenderWare collision meshes beside a SKATE map."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct


RX2_COLLISION_TYPE = 0x00080006
MAGIC = b"RWCMSET1"


def _be_u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]


def collision_sections(rx2: bytes) -> list[bytes]:
    """Return untouched ClusteredMesh sections from one Xbox 360 RW4 RX2."""

    if len(rx2) < 0x34 or rx2[:7] != b"\x89RW4xb2":
        raise ValueError("input is not an Xbox 360 RW4 RX2 resource")
    count = _be_u32(rx2, 0x20)
    table = _be_u32(rx2, 0x30)
    if table + count * 24 > len(rx2):
        raise ValueError("RX2 section table extends beyond the resource")
    result: list[bytes] = []
    for index in range(count):
        record = table + index * 24
        offset = _be_u32(rx2, record)
        size = _be_u32(rx2, record + 8)
        section_type = _be_u32(rx2, record + 20)
        if section_type != RX2_COLLISION_TYPE:
            continue
        if offset + size > len(rx2):
            raise ValueError("RX2 collision section extends beyond the resource")
        result.append(rx2[offset : offset + size])
    return result


def build_archive(
    manifest_path: Path,
    output_path: Path,
    *,
    expected_asset_count: int = 0,
    expected_mesh_count: int = 0,
) -> dict[str, object]:
    manifest_path = manifest_path.resolve()
    output_path = output_path.resolve()
    root = manifest_path.parent
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets = sorted(
        (
            asset
            for asset in manifest["simulation_assets"]
            if asset.get("collision_meshes")
        ),
        key=lambda asset: (
            str(asset["stream_file"]).casefold(),
            str(asset["asset_id"]),
        ),
    )
    if expected_asset_count and len(assets) != expected_asset_count:
        raise RuntimeError(
            "retail collision asset count changed: "
            f"expected {expected_asset_count}, got {len(assets)}"
        )

    records: list[tuple[str, bytes]] = []
    total_triangles = 0
    total_clusters = 0
    for asset_index, asset in enumerate(assets, 1):
        rx2_path = root / Path(str(asset["rx2"]).replace("\\", "/"))
        sections = collision_sections(rx2_path.read_bytes())
        metadata = asset["collision_meshes"]
        if len(sections) != len(metadata):
            raise RuntimeError(
                f"{asset['asset_id']} collision section count changed: "
                f"manifest={len(metadata)} RX2={len(sections)}"
            )
        for mesh_index, (section, mesh) in enumerate(
            zip(sections, metadata, strict=True)
        ):
            records.append(
                (f"{asset['stream_file']}#{mesh_index}", section)
            )
            total_triangles += int(mesh["triangles"])
            total_clusters += int(mesh["clusters"])
        if asset_index % 100 == 0:
            print(
                f"Read {asset_index}/{len(assets)} retail collision assets",
                flush=True,
            )
    if expected_mesh_count and len(records) != expected_mesh_count:
        raise RuntimeError(
            "retail collision mesh count changed: "
            f"expected {expected_mesh_count}, got {len(records)}"
        )

    payload = bytearray(MAGIC)
    payload.extend(struct.pack("<I", len(records)))
    for name, mesh in records:
        encoded_name = name.encode("utf-8")
        payload.extend(struct.pack("<I", len(encoded_name)))
        payload.extend(encoded_name)
        payload.extend(struct.pack("<I", len(mesh)))
        payload.extend(mesh)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    temporary_path = output_path.with_name(output_path.name + ".tmp")
    temporary_path.write_bytes(payload)
    os.replace(temporary_path, output_path)
    result: dict[str, object] = {
        "status": "RETAIL_COLLISION_ARCHIVE_OK",
        "map_name": manifest.get("map_name", ""),
        "output": str(output_path),
        "assets": len(assets),
        "meshes": len(records),
        "triangles": total_triangles,
        "clusters": total_clusters,
        "bytes": len(payload),
        "sha256": hashlib.sha256(payload).hexdigest(),
    }
    print(json.dumps(result, indent=2), flush=True)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--expected-asset-count", type=int, default=0)
    parser.add_argument("--expected-mesh-count", type=int, default=0)
    args = parser.parse_args()
    if args.expected_asset_count < 0 or args.expected_mesh_count < 0:
        raise ValueError("expected counts must be non-negative")
    build_archive(
        args.manifest,
        args.output,
        expected_asset_count=args.expected_asset_count,
        expected_mesh_count=args.expected_mesh_count,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
