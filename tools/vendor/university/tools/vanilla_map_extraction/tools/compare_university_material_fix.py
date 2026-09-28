#!/usr/bin/env python3
"""Verify that a University material fix did not alter visual geometry."""

from __future__ import annotations

import argparse
import hashlib
import struct
from pathlib import Path

import sys

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import PackageError, analyze_package  # noqa: E402


def _triangle_geometry_digests(package: dict[str, object]) -> list[bytes]:
    vertex_bytes = package["_vertex_bytes"]
    indices = package["_indices"]
    digests: list[bytes] = []
    for first_corner in range(0, len(indices), 3):
        digest = hashlib.blake2b(digest_size=16)
        for index in indices[first_corner : first_corner + 3]:
            offset = index * 44
            digest.update(vertex_bytes[offset : offset + 40])
        digests.append(digest.digest())
    digests.sort()
    return digests


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()

    before = analyze_package(args.before, include_payloads=True)
    after = analyze_package(args.after, include_payloads=True)
    failures: list[str] = []

    for count_name in ("textures", "indices"):
        if before["counts"][count_name] != after["counts"][count_name]:
            failures.append(
                f"{count_name} changed: {before['counts'][count_name]} -> "
                f"{after['counts'][count_name]}"
            )
    if (
        before["integrity"]["decoded_textures_sha256"]
        != after["integrity"]["decoded_textures_sha256"]
    ):
        failures.append("decoded texture payloads changed")
    for bound_name in ("bounds_min", "bounds_max"):
        if before["integrity"][bound_name] != after["integrity"][bound_name]:
            failures.append(f"{bound_name} changed")

    ordered_stream_matches = True
    maximum_ordered_float_delta = 0.0
    if before["counts"]["indices"] == after["counts"]["indices"]:
        before_vertices = before["_vertex_bytes"]
        after_vertices = after["_vertex_bytes"]
        for corner, (before_index, after_index) in enumerate(
            zip(before["_indices"], after["_indices"], strict=True)
        ):
            before_record = struct.unpack_from(
                "<10fI", before_vertices, before_index * 44
            )
            after_record = struct.unpack_from(
                "<10fI", after_vertices, after_index * 44
            )
            delta = max(
                abs(left - right)
                for left, right in zip(
                    before_record[:10], after_record[:10], strict=True
                )
            )
            maximum_ordered_float_delta = max(
                maximum_ordered_float_delta, delta
            )
            if delta > 1.0e-6:
                ordered_stream_matches = False
                break

    geometry_multiset_matches = (
        _triangle_geometry_digests(before)
        == _triangle_geometry_digests(after)
    )
    if not geometry_multiset_matches:
        failures.append(
            "position/normal/UV triangle multiset changed after "
            "normalizing export order and material IDs"
        )

    if failures:
        print("UNIVERSITY_MATERIAL_FIX_MISMATCH")
        for failure in failures:
            print(f"- {failure}")
        return 1

    print(
        "UNIVERSITY_MATERIAL_FIX_OK",
        f"triangles={after['counts']['indices'] // 3}",
        f"ordered_stream_matches={str(ordered_stream_matches).lower()}",
        f"maximum_ordered_float_delta={maximum_ordered_float_delta:.9g}",
        "geometry_multiset=unchanged",
        "material_assignments=intentionally_changed",
        f"textures={after['counts']['textures']}",
        "texture_payloads=unchanged",
        f"collision_triangles_before="
        f"{before['counts']['collision_triangles']}",
        f"collision_triangles_after="
        f"{after['counts']['collision_triangles']}",
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except PackageError as error:
        raise SystemExit(f"SKATE_PACKAGE_ERROR {error}") from error
