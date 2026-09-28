#!/usr/bin/env python3
"""Verify exact retail world tangent frames through a University SKATE export."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

import numpy


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import (  # noqa: E402
    VERTEX_BYTES_V12,
    analyze_package,
)


def _source_key(source: dict[str, object]) -> tuple[str, int]:
    return str(source.get("asset_id", "")), int(
        source.get("mesh_index", -1)
    )


def _source_frame_digests(
    manifest: dict[str, object],
    root: Path,
) -> tuple[dict[tuple[str, int], tuple[int, str]], int]:
    result: dict[tuple[str, int], tuple[int, str]] = {}
    source_vertices = 0
    for model in manifest["models"]:
        with numpy.load(root / str(model["npz"])) as arrays:
            for mesh in model["meshes"]:
                if not mesh.get("retail_world_frame"):
                    continue
                mesh_index = int(mesh["index"])
                key = (str(model["asset_id"]), mesh_index)
                faces = numpy.asarray(
                    arrays[f"faces_{mesh_index}"], dtype=numpy.uint32
                )
                corners = faces.reshape(-1)
                normals = numpy.asarray(
                    arrays[f"retail_normals_{mesh_index}"],
                    dtype=numpy.float32,
                )[corners].copy()
                # Coordinate-system round trips can turn +0 into -0 without
                # changing the represented vector. Canonicalize signed zero
                # before the byte-level comparison.
                normals[normals == 0.0] = 0.0
                tangent_key = (
                    f"retail_tangents_{mesh_index}"
                    if f"retail_tangents_{mesh_index}" in arrays
                    else f"retail_binormals_{mesh_index}"
                )
                tangents = numpy.asarray(
                    arrays[tangent_key],
                    dtype=numpy.float32,
                )[corners].astype(numpy.float64)
                handedness = numpy.asarray(
                    arrays[f"retail_tangent_handedness_{mesh_index}"],
                    dtype=numpy.float32,
                )[corners]
                tangent_lengths = numpy.linalg.norm(tangents, axis=1)
                tangents -= normals * numpy.sum(
                    tangents * normals, axis=1
                )[:, None]
                projected_lengths = numpy.linalg.norm(tangents, axis=1)
                valid = projected_lengths > numpy.maximum(
                    1.0e-12, tangent_lengths * 1.0e-6
                )
                tangents[valid] /= projected_lengths[valid, None]
                handedness = handedness.copy()
                handedness[~valid] = 0.0
                binormals = (
                    numpy.cross(normals, tangents)
                    * handedness[:, None]
                )
                packed_binormals = numpy.rint(
                    numpy.clip(binormals, -1.0, 1.0) * 127.0
                ).astype(numpy.int8)
                packed_handedness = numpy.rint(
                    numpy.clip(handedness, -1.0, 1.0) * 127.0
                ).astype(numpy.int8)
                records = numpy.empty(
                    len(corners),
                    dtype=numpy.dtype(
                        [
                            ("normal", "<f4", (3,)),
                            ("frame", "i1", (4,)),
                        ],
                        align=False,
                    ),
                )
                records["normal"] = normals
                records["frame"][:, :3] = packed_binormals
                records["frame"][:, 3] = packed_handedness
                result[key] = (
                    len(records),
                    hashlib.sha256(records.tobytes()).hexdigest(),
                )
                source_vertices += int(
                    arrays[f"retail_normals_{mesh_index}"].shape[0]
                )
    return result, source_vertices


def _package_frame_digests(
    package: dict[str, object],
) -> dict[tuple[str, int], tuple[int, str]]:
    if int(package["version"]) not in (12, 13):
        raise RuntimeError(
            f"expected SKATE v12 or v13, found v{package['version']}"
        )
    material_keys: dict[int, tuple[str, int]] = {}
    for material_id, material in enumerate(
        package["_materials"], start=1
    ):
        if not material.get("retail_enabled", False):
            continue
        source_text = str(material.get("retail_source_metadata", ""))
        source = json.loads(source_text)
        if source.get("retail_world_frame"):
            material_keys[material_id] = _source_key(source)

    digests = {
        key: hashlib.sha256() for key in material_keys.values()
    }
    counts = {key: 0 for key in material_keys.values()}
    vertex_bytes = package["_vertex_bytes"]
    for vertex_index in package["_indices"]:
        offset = int(vertex_index) * VERTEX_BYTES_V12
        material_id = struct.unpack_from("<I", vertex_bytes, offset + 40)[0]
        key = material_keys.get(material_id)
        if key is None:
            continue
        normal = list(
            struct.unpack_from("<3f", vertex_bytes, offset + 12)
        )
        normal = [0.0 if value == 0.0 else value for value in normal]
        digests[key].update(struct.pack("<3f", *normal))
        digests[key].update(vertex_bytes[offset + 52 : offset + 56])
        counts[key] += 1
    return {
        key: (counts[key], digest.hexdigest())
        for key, digest in digests.items()
    }


def _aggregate_digest(
    entries: dict[tuple[str, int], tuple[int, str]],
) -> str:
    digest = hashlib.sha256()
    for key in sorted(entries):
        asset_id, mesh_index = key
        count, frame_digest = entries[key]
        digest.update(asset_id.encode("ascii"))
        digest.update(struct.pack("<II", mesh_index, count))
        digest.update(bytes.fromhex(frame_digest))
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("package", type=Path)
    parser.add_argument("--expected", type=Path)
    args = parser.parse_args()

    manifest_path = args.manifest.resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    source, source_vertices = _source_frame_digests(
        manifest, manifest_path.parent
    )
    package = analyze_package(args.package.resolve(), include_payloads=True)
    exported = _package_frame_digests(package)
    missing = sorted(set(source) - set(exported))
    extra = sorted(set(exported) - set(source))
    if missing or extra:
        raise RuntimeError(
            "retail world-frame material set changed: "
            f"missing={missing[:5]} extra={extra[:5]}"
        )
    mismatches = [
        (key, source[key], exported[key])
        for key in sorted(source)
        if source[key] != exported[key]
    ]
    if mismatches:
        raise RuntimeError(
            "retail world-frame corner data changed; first mismatch: "
            f"{mismatches[0]!r}"
        )

    result = {
        "status": "UNIVERSITY_RETAIL_WORLD_FRAMES_RECONSTRUCTED",
        "mesh_parts": len(source),
        "source_vertices": source_vertices,
        "triangle_corners": sum(count for count, _digest in source.values()),
        "expanded_frame_sha256": _aggregate_digest(source),
    }
    if args.expected:
        expected = json.loads(
            args.expected.read_text(encoding="utf-8")
        ).get("retail_world_frames")
        if expected is None:
            raise RuntimeError(
                "expected manifest has no retail_world_frames section"
            )
        for name, value in result.items():
            if name == "status":
                continue
            if expected.get(name) != value:
                raise RuntimeError(
                    f"retail world-frame {name} changed: "
                    f"{value!r} != {expected.get(name)!r}"
                )
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
