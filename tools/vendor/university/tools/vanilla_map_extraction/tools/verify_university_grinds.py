#!/usr/bin/env python3
"""Verify that University native grind data round-tripped byte-for-byte."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import analyze_package  # noqa: E402


def _write_string(output: bytearray, value: str) -> None:
    encoded = value.encode("utf-8")
    output.extend(struct.pack("<I", len(encoded)))
    output.extend(encoded)


def expected_grind_section(manifest: dict[str, object]) -> bytes:
    records: list[tuple[str, dict[str, object]]] = []
    for rail in manifest.get("grind_splines", []):
        name = (
            f"GRIND_{str(rail['asset_id'])[2:]}_"
            f"{int(rail['rail_index']):04d}"
        )
        records.append((name, rail))
    records.sort(key=lambda record: record[0])
    names = [record[0] for record in records]
    if len(names) != len(set(names)):
        raise ValueError("University retail grind object names are not unique")

    output = bytearray()
    for name, rail in records:
        payloads = rail["native_segment_payloads"]
        segment_count = int(rail["segment_count"])
        if len(payloads) != segment_count:
            raise ValueError(f"{name} has an inconsistent segment count")
        _write_string(output, name)
        output.extend(
            struct.pack(
                "<IIQQIII",
                int(bool(rail["closed"])),
                1,
                int(str(rail["spline_id"]), 16),
                int(str(rail["type_signature"]), 16),
                int(rail["flags"]),
                int(rail["trailing_word"]),
                segment_count,
            )
        )
        for payload_hex in payloads:
            payload = bytes.fromhex(str(payload_hex))
            if len(payload) != 120:
                raise ValueError(f"{name} has an invalid native payload")
            words = struct.unpack(">30I", payload)
            output.extend(struct.pack("<30I", *words))
    return bytes(output)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    expected = expected_grind_section(manifest)
    analysis = analyze_package(args.package)
    actual_hash = analysis["integrity"]["grind_rails_sha256"]
    expected_hash = hashlib.sha256(expected).hexdigest()
    expected_rails = len(manifest.get("grind_splines", []))
    expected_segments = sum(
        int(rail["segment_count"])
        for rail in manifest.get("grind_splines", [])
    )
    if analysis["counts"]["grind_rails"] != expected_rails:
        raise RuntimeError("SKATE grind rail count differs from the manifest")
    if analysis["counts"]["native_grind_segments"] != expected_segments:
        raise RuntimeError(
            "SKATE native grind segment count differs from the manifest"
        )
    if actual_hash != expected_hash:
        raise RuntimeError(
            "SKATE native grind bytes differ from the retail manifest: "
            f"actual={actual_hash} expected={expected_hash}"
        )
    print(
        json.dumps(
            {
                "status": "UNIVERSITY_GRINDS_EXACT",
                "rails": expected_rails,
                "segments": expected_segments,
                "bytes": len(expected),
                "sha256": expected_hash,
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
