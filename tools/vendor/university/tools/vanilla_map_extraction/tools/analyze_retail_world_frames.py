#!/usr/bin/env python3
"""Compare preserved retail world-frame vectors with geometry-derived axes."""

from __future__ import annotations

import argparse
import json
import math
from collections import defaultdict
from pathlib import Path

import numpy


def _normalize(values: numpy.ndarray) -> numpy.ndarray:
    lengths = numpy.linalg.norm(values, axis=1)
    result = numpy.zeros_like(values, dtype=numpy.float64)
    valid = lengths > 1.0e-8
    result[valid] = values[valid] / lengths[valid, None]
    return result


def _triangle_axes(
    positions: numpy.ndarray,
    uvs: numpy.ndarray,
    faces: numpy.ndarray,
) -> tuple[numpy.ndarray, numpy.ndarray, numpy.ndarray]:
    p0 = positions[faces[:, 0]]
    p1 = positions[faces[:, 1]]
    p2 = positions[faces[:, 2]]
    uv0 = uvs[faces[:, 0]]
    uv1 = uvs[faces[:, 1]]
    uv2 = uvs[faces[:, 2]]
    edge1 = p1 - p0
    edge2 = p2 - p0
    delta1 = uv1 - uv0
    delta2 = uv2 - uv0
    determinant = delta1[:, 0] * delta2[:, 1] - delta1[:, 1] * delta2[:, 0]
    valid = (
        numpy.isfinite(determinant)
        & (numpy.abs(determinant) > 1.0e-8)
        & (numpy.linalg.norm(numpy.cross(edge1, edge2), axis=1) > 1.0e-8)
    )
    inverse = numpy.zeros_like(determinant, dtype=numpy.float64)
    inverse[valid] = 1.0 / determinant[valid]
    tangent = (
        edge1 * delta2[:, 1, None] - edge2 * delta1[:, 1, None]
    ) * inverse[:, None]
    binormal = (
        edge2 * delta1[:, 0, None] - edge1 * delta2[:, 0, None]
    ) * inverse[:, None]
    return _normalize(tangent), _normalize(binormal), valid


def _mesh_measurements(
    arrays: numpy.lib.npyio.NpzFile,
    mesh_index: int,
) -> tuple[numpy.ndarray, numpy.ndarray]:
    positions = numpy.asarray(
        arrays[f"vertices_{mesh_index}"], dtype=numpy.float64
    )
    uvs = numpy.asarray(arrays[f"uvs_{mesh_index}"], dtype=numpy.float64)
    faces = numpy.asarray(arrays[f"faces_{mesh_index}"], dtype=numpy.int64)
    source = _normalize(
        numpy.asarray(
            arrays[
                f"retail_tangents_{mesh_index}"
                if f"retail_tangents_{mesh_index}" in arrays
                else f"retail_binormals_{mesh_index}"
            ],
            dtype=numpy.float64,
        )
    )
    normals = _normalize(
        numpy.asarray(
            arrays[f"retail_normals_{mesh_index}"], dtype=numpy.float64
        )
    )
    handedness = numpy.asarray(
        arrays[f"retail_tangent_handedness_{mesh_index}"],
        dtype=numpy.float64,
    )
    tangents, binormals, valid = _triangle_axes(positions, uvs, faces)
    if not numpy.any(valid):
        return numpy.empty((0, 5)), numpy.empty((0,), dtype=numpy.int64)

    corners = faces[valid].reshape(-1)
    triangle_tangents = numpy.repeat(tangents[valid], 3, axis=0)
    triangle_binormals = numpy.repeat(binormals[valid], 3, axis=0)
    source = source[corners]
    normals = normals[corners]
    handedness = handedness[corners]

    source = _normalize(
        source - normals * numpy.sum(source * normals, axis=1)[:, None]
    )
    triangle_tangents = _normalize(
        triangle_tangents
        - normals * numpy.sum(triangle_tangents * normals, axis=1)[:, None]
    )
    triangle_binormals = _normalize(
        triangle_binormals
        - normals * numpy.sum(triangle_binormals * normals, axis=1)[:, None]
    )

    tangent_dot = numpy.sum(source * triangle_tangents, axis=1)
    binormal_dot = numpy.sum(source * triangle_binormals, axis=1)
    reconstructed_binormal = _normalize(
        numpy.cross(normals, source) * handedness[:, None]
    )
    reconstructed_dot = numpy.sum(
        reconstructed_binormal * triangle_binormals, axis=1
    )
    measurements = numpy.column_stack(
        (
            numpy.abs(tangent_dot),
            numpy.abs(binormal_dot),
            tangent_dot,
            binormal_dot,
            reconstructed_dot,
        )
    )
    finite = numpy.isfinite(measurements).all(axis=1)
    return measurements[finite], corners[finite]


def _summary(rows: list[numpy.ndarray]) -> dict[str, float | int]:
    values = numpy.concatenate(rows, axis=0)
    tangent_abs = values[:, 0]
    binormal_abs = values[:, 1]
    return {
        "corners": int(len(values)),
        "mean_abs_dot_tangent": float(numpy.mean(tangent_abs)),
        "mean_abs_dot_binormal": float(numpy.mean(binormal_abs)),
        "median_abs_dot_tangent": float(numpy.median(tangent_abs)),
        "median_abs_dot_binormal": float(numpy.median(binormal_abs)),
        "tangent_wins_percent": float(
            numpy.mean(tangent_abs > binormal_abs) * 100.0
        ),
        "binormal_wins_percent": float(
            numpy.mean(binormal_abs > tangent_abs) * 100.0
        ),
        "mean_signed_dot_tangent": float(numpy.mean(values[:, 2])),
        "mean_signed_dot_binormal": float(numpy.mean(values[:, 3])),
        "mean_reconstructed_binormal_dot": float(numpy.mean(values[:, 4])),
    }


def analyze(root: Path) -> dict[str, object]:
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    groups: dict[str, list[numpy.ndarray]] = defaultdict(list)
    mesh_count = 0
    model_count = 0
    for model in manifest["models"]:
        relevant = [
            mesh
            for mesh in model["meshes"]
            if mesh.get("retail_world_frame")
        ]
        if not relevant:
            continue
        model_count += 1
        asset_id = int(model["asset_id"], 0)
        with numpy.load(root / "models" / f"{asset_id:016X}.npz") as arrays:
            for mesh in relevant:
                index = int(mesh["index"])
                tangent_keys = (
                    f"retail_tangents_{index}",
                    f"retail_binormals_{index}",
                )
                if (
                    not any(key in arrays for key in tangent_keys)
                    or f"uvs_{index}" not in arrays
                ):
                    continue
                measurements, _ = _mesh_measurements(arrays, index)
                if len(measurements) == 0:
                    continue
                mesh_count += 1
                groups["all"].append(measurements)
                groups[f"shader:{mesh.get('shader_name', '<none>')}"].append(
                    measurements
                )
                world_frame = mesh["retail_world_frame"]
                frame_offset = world_frame.get(
                    "tangent_offset",
                    world_frame.get("binormal_offset", -1),
                )
                layout = (
                    f"layout:{mesh.get('vertex_stride', 0)}/{frame_offset}"
                )
                groups[layout].append(measurements)
    summaries = {
        key: _summary(rows)
        for key, rows in sorted(groups.items())
        if rows
    }
    return {
        "source": str(root),
        "models": model_count,
        "meshes": mesh_count,
        "groups": summaries,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "intermediate",
        type=Path,
        help="Prepared map intermediate directory containing manifest.json",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    result = analyze(args.intermediate.resolve())
    if args.json:
        print(json.dumps(result, indent=2))
        return 0
    print(f"models={result['models']} meshes={result['meshes']}")
    for key, values in result["groups"].items():
        print(
            f"{key}: corners={values['corners']} "
            f"|dot(T)|={values['mean_abs_dot_tangent']:.4f} "
            f"|dot(B)|={values['mean_abs_dot_binormal']:.4f} "
            f"T-wins={values['tangent_wins_percent']:.2f}% "
            f"B-wins={values['binormal_wins_percent']:.2f}% "
            f"reconstructed-B={values['mean_reconstructed_binormal_dot']:.4f}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
