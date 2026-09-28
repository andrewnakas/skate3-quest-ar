#!/usr/bin/env python3
"""Compare two SKATE packages after decoding storage optimizations."""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path

from analyze_skate import PackageError, analyze_package


def compare_packages(
    before_path: Path,
    after_path: Path,
) -> tuple[list[str], dict[str, object], dict[str, object], float]:
    before = analyze_package(before_path, include_payloads=True)
    after = analyze_package(after_path, include_payloads=True)
    failures: list[str] = []
    for name in (
        "materials",
        "textures",
        "indices",
        "collision_triangles",
        "grind_rails",
        "hinged_doors",
        "local_lights",
        "npc_routes",
    ):
        if before["counts"][name] != after["counts"][name]:
            failures.append(
                f"{name}: {before['counts'][name]} != "
                f"{after['counts'][name]}"
            )
    for name, value in before["integrity"].items():
        if (
            name.startswith("expanded_visual_triangles")
            or name.startswith("bounds_")
        ):
            continue
        if value != after["integrity"][name]:
            failures.append(
                f"{name}: {json.dumps(value)} != "
                f"{json.dumps(after['integrity'][name])}"
            )

    maximum_float_delta = 0.0
    before_vertices = before["_vertex_bytes"]
    after_vertices = after["_vertex_bytes"]
    before_v12 = int(before["version"]) >= 12
    after_v12 = int(after["version"]) >= 12
    before_stride = 56 if before_v12 else 44
    after_stride = 56 if after_v12 else 44
    for corner, (before_index, after_index) in enumerate(
        zip(before["_indices"], after["_indices"], strict=True)
    ):
        before_record = struct.unpack_from(
            "<10fI2f4b" if before_v12 else "<10fI",
            before_vertices,
            before_index * before_stride,
        )
        after_record = struct.unpack_from(
            "<10fI2f4b" if after_v12 else "<10fI",
            after_vertices,
            after_index * after_stride,
        )
        if before_record[10] != after_record[10]:
            failures.append(
                f"visual corner {corner} material: "
                f"{before_record[10]} != {after_record[10]}"
            )
            break
        before_floats = (
            (
                *before_record[:10],
                *before_record[11:13],
                *(value / 127.0 for value in before_record[13:17]),
            )
            if before_v12
            else before_record[:10]
        )
        after_floats = (
            (
                *after_record[:10],
                *after_record[11:13],
                *(value / 127.0 for value in after_record[13:17]),
            )
            if after_v12
            else after_record[:10]
        )
        deltas = tuple(
            abs(left - right)
            for left, right in zip(
                before_floats,
                after_floats,
                strict=True,
            )
        )
        delta = max(deltas)
        maximum_float_delta = max(maximum_float_delta, delta)
        if delta > 1.0e-6:
            field_names = (
                "position.x",
                "position.y",
                "position.z",
                "normal.x",
                "normal.y",
                "normal.z",
                "uv0.x",
                "uv0.y",
                "uv1.x",
                "uv1.y",
                "uv2.x",
                "uv2.y",
                "binormal.x",
                "binormal.y",
                "binormal.z",
                "handedness",
            )
            field_index = deltas.index(delta)
            failures.append(
                f"visual corner {corner} maximum float delta "
                f"{delta:.9g} exceeds 1e-6 in {field_names[field_index]}: "
                f"{before_floats[field_index]:.9g} != "
                f"{after_floats[field_index]:.9g}"
            )
            break

    return failures, before, after, maximum_float_delta


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()
    failures, before, after, maximum_float_delta = compare_packages(
        args.before, args.after
    )
    before_size = int(before["file_bytes"])
    after_size = int(after["file_bytes"])
    reduction = 100.0 * (before_size - after_size) / before_size
    if failures:
        print("SKATE_SEMANTIC_MISMATCH")
        for failure in failures:
            print(f"- {failure}")
        return 1
    print(
        "SKATE_SEMANTIC_MATCH",
        f"before_bytes={before_size}",
        f"after_bytes={after_size}",
        f"reduction_percent={reduction:.2f}",
        f"before_vertices={before['counts']['vertices']}",
        f"after_vertices={after['counts']['vertices']}",
        f"triangles={after['counts']['indices'] // 3}",
        f"maximum_float_delta={maximum_float_delta:.9g}",
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except PackageError as error:
        raise SystemExit(f"SKATE_PACKAGE_ERROR {error}") from error
