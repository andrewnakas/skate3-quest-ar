#!/usr/bin/env python3
"""Build an exact-retail collision archive for the University district."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import struct


RX2_COLLISION_TYPE = 0x00080006
MAGIC = b"RWCMSET1"
CELL_NAME = re.compile(
    r"^cSim_(-?\d+)_(-?\d+)_high\.xsf$", re.IGNORECASE
)
SPAWN_XZ = (330.0, -710.0)
DEFAULT_ASSET_COUNT = 0
EXPECTED_FULL_ASSET_COUNT = 299
EXPECTED_FULL_MESH_COUNT = 301
REQUIRED_DIAGNOSTIC_ASSET = "0xDE463D6F91D30023"


def _be_u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]


def _collision_sections(rx2: bytes) -> list[bytes]:
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--asset-count", type=int, default=DEFAULT_ASSET_COUNT)
    args = parser.parse_args()
    if args.asset_count < 0:
        raise ValueError("asset count must be zero (all) or positive")

    manifest_path = args.manifest.resolve()
    root = manifest_path.parent
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    candidates: list[tuple[float, str, dict[str, object]]] = []
    for asset in manifest["simulation_assets"]:
        if not asset.get("collision_meshes"):
            continue
        match = CELL_NAME.match(str(asset["stream_file"]))
        if match is None:
            continue
        center_x, center_z = (float(value) for value in match.groups())
        distance_squared = (
            (center_x - SPAWN_XZ[0]) ** 2
            + (center_z - SPAWN_XZ[1]) ** 2
        )
        candidates.append(
            (distance_squared, str(asset["asset_id"]), asset)
        )
    candidates.sort(key=lambda item: (item[0], item[1]))
    selected = (
        candidates
        if args.asset_count == 0
        else candidates[: args.asset_count]
    )
    selected_ids = {asset_id for _distance, asset_id, _asset in selected}
    if REQUIRED_DIAGNOSTIC_ASSET not in selected_ids:
        raise RuntimeError(
            "spawn collision selection omitted the known Mega Park "
            f"diagnostic asset {REQUIRED_DIAGNOSTIC_ASSET}"
        )

    records: list[tuple[str, bytes]] = []
    selected_assets: list[dict[str, object]] = []
    for distance_squared, asset_id, asset in selected:
        rx2_path = root / Path(str(asset["rx2"]).replace("\\", "/"))
        sections = _collision_sections(rx2_path.read_bytes())
        if len(sections) != len(asset["collision_meshes"]):
            raise RuntimeError(
                f"{asset_id} collision section count changed"
            )
        for mesh_index, section in enumerate(sections):
            records.append(
                (
                    f"{asset['stream_file']}#{mesh_index}",
                    section,
                )
            )
        selected_assets.append(
            {
                "asset_id": asset_id,
                "stream_file": asset["stream_file"],
                "distance": distance_squared**0.5,
                "meshes": len(sections),
            }
        )
    if args.asset_count == 0:
        if len(selected) != EXPECTED_FULL_ASSET_COUNT:
            raise RuntimeError(
                "full University collision asset count changed: "
                f"expected {EXPECTED_FULL_ASSET_COUNT}, got {len(selected)}"
            )
        if len(records) != EXPECTED_FULL_MESH_COUNT:
            raise RuntimeError(
                "full University collision mesh count changed: "
                f"expected {EXPECTED_FULL_MESH_COUNT}, got {len(records)}"
            )

    payload = bytearray(MAGIC)
    payload.extend(struct.pack("<I", len(records)))
    for name, mesh in records:
        encoded_name = name.encode("utf-8")
        payload.extend(struct.pack("<I", len(encoded_name)))
        payload.extend(encoded_name)
        payload.extend(struct.pack("<I", len(mesh)))
        payload.extend(mesh)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(payload)
    print(
        json.dumps(
            {
                "status": "UNIVERSITY_RETAIL_COLLISION_ARCHIVE_OK",
                "output": str(args.output.resolve()),
                "spawn_xz": SPAWN_XZ,
                "asset_count": len(selected_assets),
                "nearest_assets": selected_assets[:12],
                "farthest_asset_distance": (
                    selected_assets[-1]["distance"]
                    if selected_assets
                    else 0.0
                ),
                "meshes": len(records),
                "bytes": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
