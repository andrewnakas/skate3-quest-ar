#!/usr/bin/env python3
"""Verify University retail collision and packed surfaces through SKATE."""

from __future__ import annotations

from collections import defaultdict
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import analyze_package  # noqa: E402
from retail_collision_mesh import decode_rx2_clustered_meshes  # noqa: E402


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _cross_length_squared(
    a: tuple[float, float, float],
    b: tuple[float, float, float],
    c: tuple[float, float, float],
) -> float:
    left = tuple(b[axis] - a[axis] for axis in range(3))
    right = tuple(c[axis] - a[axis] for axis in range(3))
    cross = (
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    )
    return sum(component * component for component in cross)


def _canonical_oriented_record(
    points: tuple[
        tuple[float, float, float],
        tuple[float, float, float],
        tuple[float, float, float],
    ],
) -> bytes:
    rotations = (
        points,
        (points[1], points[2], points[0]),
        (points[2], points[0], points[1]),
    )
    canonical = min(rotations)
    return struct.pack(
        "<9q",
        *(
            round(component * 1_000_000.0)
            for point in canonical
            for component in point
        ),
    )


def _canonical_oriented_edge_record(
    points: tuple[
        tuple[float, float, float],
        tuple[float, float, float],
        tuple[float, float, float],
    ],
    edge_codes: tuple[int, int, int],
) -> bytes:
    rotations = tuple(
        (
            points[offset:] + points[:offset],
            edge_codes[offset:] + edge_codes[:offset],
        )
        for offset in range(3)
    )
    rotated_points, rotated_codes = min(rotations, key=lambda item: item[0])
    return _canonical_oriented_record(rotated_points) + bytes(rotated_codes)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("package", type=Path)
    parser.add_argument("--expected", type=Path)
    args = parser.parse_args()

    manifest_path = args.manifest.resolve()
    cache_root = manifest_path.parent
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    by_surface: dict[int, list[object]] = defaultdict(list)
    source_assets = 0
    source_meshes = 0
    source_clusters = 0
    source_triangles = 0
    for asset in manifest["simulation_assets"]:
        if not asset.get("collision_meshes"):
            continue
        path = cache_root / Path(str(asset["rx2"]).replace("\\", "/"))
        meshes = decode_rx2_clustered_meshes(path.read_bytes())
        source_assets += 1
        source_meshes += len(meshes)
        for mesh in meshes:
            source_clusters += mesh.cluster_count
            source_triangles += len(mesh.triangles)
            for triangle in mesh.triangles:
                by_surface[triangle.surface].append(triangle)

    analysis = analyze_package(args.package.resolve(), include_payloads=True)
    material_by_name = {
        str(material["name"]): material
        for material in analysis["_materials"]
    }
    material_by_id = {
        int(material["id"]): material
        for material in analysis["_materials"]
    }
    expected_xor: dict[int, int] = defaultdict(int)
    expected_sum: dict[int, int] = defaultdict(int)
    expected_counts: dict[int, int] = defaultdict(int)
    expected_edge_xor = 0
    expected_edge_sum = 0
    seen_positions: set[tuple[tuple[float, float, float], ...]] = set()
    seen_oriented: set[tuple[tuple[float, float, float], ...]] = set()
    degenerate = 0
    same_wound_duplicates = 0
    retained_reverse_wound = 0
    surfaces = sorted(by_surface)
    for surface in surfaces:
        name = f"MAT_RETAIL_COLLISION_{surface:04X}"
        material = material_by_name.get(name)
        if material is None:
            raise RuntimeError(f"SKATE package omits retail material {name}")
        encoded = (
            int(material["audio_surface"])
            | (int(material["physics_surface"]) << 7)
            | (int(material["surface_pattern"]) << 12)
        )
        if encoded != surface:
            raise RuntimeError(
                f"{name} changed packed surface: "
                f"0x{encoded:04X} != 0x{surface:04X}"
            )
        for triangle in by_surface[surface]:
            points = tuple(
                tuple(_f32(component) for component in point)
                for point in (triangle.a, triangle.b, triangle.c)
            )
            if _cross_length_squared(*points) <= 1.0e-10:
                degenerate += 1
                continue
            rounded = tuple(
                tuple(round(component, 6) for component in point)
                for point in points
            )
            position_key = tuple(
                sorted(
                    rounded
                )
            )
            oriented_key = min(
                rounded,
                (rounded[1], rounded[2], rounded[0]),
                (rounded[2], rounded[0], rounded[1]),
            )
            if oriented_key in seen_oriented:
                same_wound_duplicates += 1
                continue
            if position_key in seen_positions:
                retained_reverse_wound += 1
            seen_positions.add(position_key)
            seen_oriented.add(oriented_key)
            if triangle.edge_codes is None:
                raise RuntimeError(
                    "retail collision triangle omits native edge codes"
                )
            record_hash = int.from_bytes(
                hashlib.sha256(
                    _canonical_oriented_record(points)
                ).digest(),
                "big",
            )
            expected_xor[surface] ^= record_hash
            expected_sum[surface] = (
                expected_sum[surface] + record_hash
            ) & ((1 << 256) - 1)
            expected_counts[surface] += 1
            edge_hash = int.from_bytes(
                hashlib.sha256(
                    _canonical_oriented_edge_record(
                        points,
                        triangle.edge_codes,
                    )
                ).digest(),
                "big",
            )
            expected_edge_xor ^= edge_hash
            expected_edge_sum = (
                expected_edge_sum + edge_hash
            ) & ((1 << 256) - 1)

    actual_xor: dict[int, int] = defaultdict(int)
    actual_sum: dict[int, int] = defaultdict(int)
    actual_counts: dict[int, int] = defaultdict(int)
    actual_edge_xor = 0
    actual_edge_sum = 0
    actual = analysis["_collision_bytes"]
    record_size = 48 if int(analysis["version"]) >= 11 else 44
    if record_size != 48:
        raise RuntimeError(
            "University package format does not preserve native collision "
            "edge codes"
        )
    for offset in range(0, len(actual), record_size):
        values = struct.unpack_from("<9fII4B", actual, offset)
        material_id = values[10]
        material = material_by_id.get(material_id)
        if material is None:
            raise RuntimeError(
                f"collision triangle references missing material {material_id}"
            )
        surface = (
            int(material["audio_surface"])
            | (int(material["physics_surface"]) << 7)
            | (int(material["surface_pattern"]) << 12)
        )
        if surface not in by_surface:
            raise RuntimeError(
                f"collision triangle uses non-retail surface 0x{surface:04X}"
            )
        points = (
            tuple(values[0:3]),
            tuple(values[3:6]),
            tuple(values[6:9]),
        )
        record_hash = int.from_bytes(
            hashlib.sha256(_canonical_oriented_record(points)).digest(),
            "big",
        )
        actual_xor[surface] ^= record_hash
        actual_sum[surface] = (
            actual_sum[surface] + record_hash
        ) & ((1 << 256) - 1)
        actual_counts[surface] += 1
        edge_codes = tuple(values[11:14])
        if values[14] != 1:
            raise RuntimeError(
                "University package collision triangle omits its native "
                "edge-code marker"
            )
        edge_hash = int.from_bytes(
            hashlib.sha256(
                _canonical_oriented_edge_record(points, edge_codes)
            ).digest(),
            "big",
        )
        actual_edge_xor ^= edge_hash
        actual_edge_sum = (
            actual_edge_sum + edge_hash
        ) & ((1 << 256) - 1)

    mismatches = [
        surface
        for surface in surfaces
        if actual_counts[surface] != expected_counts[surface]
        or actual_xor[surface] != expected_xor[surface]
        or actual_sum[surface] != expected_sum[surface]
    ]
    if mismatches:
        surface = mismatches[0]
        raise RuntimeError(
            "SKATE collision differs from decoded retail surface "
            f"0x{surface:04X}: actual_count={actual_counts[surface]} "
            f"expected_count={expected_counts[surface]} "
            f"actual_xor={actual_xor[surface]:064x} "
            f"expected_xor={expected_xor[surface]:064x}"
        )
    if (
        actual_edge_xor != expected_edge_xor
        or actual_edge_sum != expected_edge_sum
    ):
        raise RuntimeError(
            "SKATE collision native edge codes differ from decoded retail "
            f"metadata: actual_xor={actual_edge_xor:064x} "
            f"expected_xor={expected_edge_xor:064x}"
        )
    canonical_digest = hashlib.sha256()
    for surface in surfaces:
        canonical_digest.update(struct.pack("<H", surface))
        canonical_digest.update(expected_counts[surface].to_bytes(8, "little"))
        canonical_digest.update(expected_xor[surface].to_bytes(32, "big"))
        canonical_digest.update(expected_sum[surface].to_bytes(32, "big"))
    result = {
        "status": "UNIVERSITY_COLLISION_ROUND_TRIP_VERIFIED",
        "position_tolerance": 1.0e-6,
        "source_assets": source_assets,
        "source_meshes": source_meshes,
        "source_clusters": source_clusters,
        "source_surfaces": len(surfaces),
        "source_triangles": source_triangles,
        "removed_degenerate": degenerate,
        "removed_same_wound_duplicates": same_wound_duplicates,
        "retained_reverse_wound": retained_reverse_wound,
        "package_triangles": sum(expected_counts.values()),
        "canonical_sha256": canonical_digest.hexdigest(),
        "native_edge_codes_xor": f"{expected_edge_xor:064x}",
        "native_edge_codes_sum": f"{expected_edge_sum:064x}",
    }
    if args.expected is not None:
        contract = json.loads(
            args.expected.read_text(encoding="utf-8")
        )["retail_collision"]
        comparisons = {
            "assets": result["source_assets"],
            "meshes": result["source_meshes"],
            "clusters": result["source_clusters"],
            "surfaces": result["source_surfaces"],
            "source_triangles": result["source_triangles"],
            "removed_degenerate": result["removed_degenerate"],
            "removed_same_wound_duplicates": result[
                "removed_same_wound_duplicates"
            ],
            "retained_reverse_wound": result["retained_reverse_wound"],
            "package_triangles": result["package_triangles"],
            "position_tolerance": result["position_tolerance"],
            "canonical_sha256": result["canonical_sha256"],
            "native_edge_codes_xor": result["native_edge_codes_xor"],
            "native_edge_codes_sum": result["native_edge_codes_sum"],
        }
        for name, actual_value in comparisons.items():
            if contract[name] != actual_value:
                raise RuntimeError(
                    f"retail collision contract {name} changed: "
                    f"{actual_value!r} != {contract[name]!r}"
                )
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
