#!/usr/bin/env python3
"""Verify exact retail University lightmaps through the SKATE package."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import struct
import sys

import numpy
from PIL import Image


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import Reader, VERTEX_BYTES_V12  # noqa: E402


def _selected_lightmaps(
    manifest: dict[str, object],
) -> tuple[list[dict[str, object]], set[str], int, int]:
    selected: list[dict[str, object]] = []
    excluded_no_uv = 0
    excluded_shader = 0
    for model in manifest["models"]:
        for mesh in model["meshes"]:
            lightmap_id = str(
                mesh.get("retail_texture_ids", {}).get("lightmap", "")
            )
            if not lightmap_id:
                continue
            if not mesh.get("lightmap_uv"):
                excluded_no_uv += 1
                continue
            shader = str(mesh.get("shader_name") or "")
            if shader.startswith(("water.", "ocean.")):
                excluded_shader += 1
                continue
            if not mesh.get("texture_id"):
                raise RuntimeError(
                    "a selected lightmapped mesh has no base texture/material"
                )
            selected.append(mesh)
    return (
        selected,
        {
            str(mesh["retail_texture_ids"]["lightmap"])
            for mesh in selected
        },
        excluded_no_uv,
        excluded_shader,
    )


def _png_rgba_blender_order(path: Path) -> bytes:
    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        width, height = rgba.size
        top_to_bottom = rgba.tobytes()
        row_bytes = width * 4
        # Blender exposes image.pixels bottom row first. The importer flips V
        # when it converts retail UVs, so this storage-order flip preserves
        # the exact sampled texel rather than altering the map orientation.
        return b"".join(
            top_to_bottom[row * row_bytes : (row + 1) * row_bytes]
            for row in range(height - 1, -1, -1)
    )


def _source_uv_bounds(
    manifest: dict[str, object],
) -> tuple[tuple[float, float], tuple[float, float]]:
    minimum = numpy.array((numpy.inf, numpy.inf), dtype=numpy.float64)
    maximum = numpy.array((-numpy.inf, -numpy.inf), dtype=numpy.float64)
    root = Path(manifest["_manifest_root"])
    for model in manifest["models"]:
        with numpy.load(root / str(model["npz"])) as arrays:
            for mesh in model["meshes"]:
                lightmap_id = str(
                    mesh.get("retail_texture_ids", {}).get("lightmap", "")
                )
                shader = str(mesh.get("shader_name") or "")
                if (
                    not lightmap_id
                    or not mesh.get("lightmap_uv")
                    or shader.startswith(("water.", "ocean."))
                ):
                    continue
                values = numpy.abs(
                    arrays[f"lightmap_uvs_{int(mesh['index'])}"].astype(
                        numpy.float64
                    )
                )
                values[:, 1] = 1.0 - values[:, 1]
                minimum = numpy.minimum(minimum, values.min(axis=0))
                maximum = numpy.maximum(maximum, values.max(axis=0))
    if not numpy.isfinite(minimum).all() or not numpy.isfinite(maximum).all():
        raise RuntimeError("could not derive retail lightmap UV bounds")
    return tuple(minimum), tuple(maximum)


def _verify_package(
    package_path: Path,
    manifest: dict[str, object],
    selected_ids: set[str],
    source_uv_bounds: tuple[tuple[float, float], tuple[float, float]],
) -> dict[str, int]:
    reader = Reader(package_path.read_bytes())
    magic = reader.take(8, "magic")
    if magic not in (b"SKATE12\0", b"SKATE13\0", b"SKATE14\0"):
        raise RuntimeError(f"expected SKATE12/13 package, found {magic!r}")
    if reader.u32("endian marker") != 0x12345678:
        raise RuntimeError("invalid SKATE endian marker")
    reader.string("map name")
    reader.skip(49 * 4, "map metadata")
    counts = struct.unpack("<9I", reader.take(9 * 4, "count table"))
    material_count, texture_count, vertex_count, index_count = counts[:4]

    materials: list[tuple[int, int, float]] = []
    for index in range(material_count):
        reader.string(f"material {index} name")
        fields = reader.take(76, f"material {index} fields")
        materials.append(
            (
                struct.unpack_from("<I", fields, 32)[0],
                struct.unpack_from("<I", fields, 36)[0],
                struct.unpack_from("<f", fields, 40)[0],
            )
        )
        retail_enabled = reader.u32(
            f"material {index} retail enabled"
        )
        if retail_enabled:
            reader.skip(8 + 4 + 4, f"material {index} retail identity")
            reader.string(f"material {index} retail shader")
            reader.skip(4 + 4, f"material {index} retail family/flags")
            binding_count = reader.u32(
                f"material {index} retail binding count"
            )
            for binding_index in range(binding_count):
                reader.string(
                    f"material {index} binding {binding_index} semantic"
                )
                reader.skip(
                    4 * 4,
                    f"material {index} binding {binding_index} fields",
                )
            parameter_count = reader.u32(
                f"material {index} retail parameter count"
            )
            for parameter_index in range(parameter_count):
                reader.string(
                    f"material {index} parameter {parameter_index} name"
                )
                value_count = reader.u32(
                    f"material {index} parameter {parameter_index} values"
                )
                for value_index in range(value_count):
                    reader.string(
                        f"material {index} parameter {parameter_index} "
                        f"value {value_index}"
                    )
            reader.string(f"material {index} retail source")

    texture_names: dict[int, str] = {}
    exact_payloads = 0
    exact_bytes = 0
    for index in range(texture_count):
        texture_id = index + 1
        name = reader.string(f"texture {index} name")
        width = reader.u32(f"texture {index} width")
        height = reader.u32(f"texture {index} height")
        color_space = reader.u32(f"texture {index} color space")
        rgba8 = reader.stored(width * height * 4, f"texture {index}")
        texture_names[texture_id] = name
        if name not in selected_ids:
            continue
        if color_space != 0:
            raise RuntimeError(f"retail lightmap {name!r} is not linear data")
        entry = manifest["textures"][name]
        if (width, height) != (int(entry["width"]), int(entry["height"])):
            raise RuntimeError(f"retail lightmap {name!r} dimensions changed")
        source = _png_rgba_blender_order(
            Path(manifest["_manifest_root"]) / str(entry["png"])
        )
        if rgba8 != source:
            raise RuntimeError(
                f"retail lightmap {name!r} changed during Blender export"
            )
        exact_payloads += 1
        exact_bytes += len(rgba8)

    lightmap_material_ids = {
        material_id
        for material_id, (_albedo, lightmap, _strength) in enumerate(
            materials,
            start=1,
        )
        if lightmap != 0
    }
    lightmap_strengths = {
        strength
        for _albedo, lightmap, strength in materials
        if lightmap != 0
    }
    if lightmap_strengths != {0.25}:
        raise RuntimeError(
            "retail lightmaps must use console energy scale 0.25, found "
            f"{sorted(lightmap_strengths)!r}"
        )
    package_lightmap_names = {
        texture_names[lightmap]
        for _albedo, lightmap, _strength in materials
        if lightmap != 0
    }
    if package_lightmap_names != selected_ids:
        missing = sorted(selected_ids - package_lightmap_names)
        extra = sorted(package_lightmap_names - selected_ids)
        raise RuntimeError(
            "package retail lightmap set differs from extraction: "
            f"missing={missing}, extra={extra}"
        )
    if exact_payloads != len(selected_ids):
        raise RuntimeError("not every retail lightmap payload was verified")

    vertex_bytes = reader.stored(
        vertex_count * VERTEX_BYTES_V12,
        "visual vertex block",
    )
    index_bytes = reader.stored(index_count * 4, "visual index block")
    indices = struct.unpack(f"<{index_count}I", index_bytes)
    lightmap_vertices = 0
    distinct_uv_vertices = 0
    used_lightmap_material_ids: set[int] = set()
    package_uv_min = [math.inf, math.inf]
    package_uv_max = [-math.inf, -math.inf]
    for offset in range(0, len(vertex_bytes), VERTEX_BYTES_V12):
        uv0 = struct.unpack_from("<2f", vertex_bytes, offset + 24)
        uv1 = struct.unpack_from("<2f", vertex_bytes, offset + 32)
        material_id = struct.unpack_from("<I", vertex_bytes, offset + 40)[0]
        if material_id not in lightmap_material_ids:
            continue
        if not all(math.isfinite(value) for value in (*uv0, *uv1)):
            raise RuntimeError("package contains non-finite lightmap UVs")
        for axis in range(2):
            package_uv_min[axis] = min(package_uv_min[axis], uv1[axis])
            package_uv_max[axis] = max(package_uv_max[axis], uv1[axis])
        lightmap_vertices += 1
        distinct_uv_vertices += uv0 != uv1
        used_lightmap_material_ids.add(material_id)
    if used_lightmap_material_ids != lightmap_material_ids:
        raise RuntimeError("package contains an unused lightmapped material")

    lightmap_corners = 0
    for index in indices:
        material_id = struct.unpack_from(
            "<I",
            vertex_bytes,
            index * VERTEX_BYTES_V12 + 40,
        )[0]
        lightmap_corners += material_id in lightmap_material_ids
    if not lightmap_vertices or not distinct_uv_vertices or not lightmap_corners:
        raise RuntimeError("package did not transport usable lightmap UVs")
    expected_min, expected_max = source_uv_bounds
    for axis in range(2):
        if (
            abs(package_uv_min[axis] - expected_min[axis]) > 1.0e-5
            or abs(package_uv_max[axis] - expected_max[axis]) > 1.0e-5
        ):
            raise RuntimeError(
                "package lightmap UV bounds changed on axis "
                f"{axis}: package=({package_uv_min[axis]}, "
                f"{package_uv_max[axis]}) source=({expected_min[axis]}, "
                f"{expected_max[axis]})"
            )

    return {
        "package_lightmap_texture_ids": len(package_lightmap_names),
        "package_lightmap_materials": len(lightmap_material_ids),
        "package_lightmap_strength": min(lightmap_strengths),
        "exact_payloads": exact_payloads,
        "exact_payload_bytes": exact_bytes,
        "package_lightmap_vertices": lightmap_vertices,
        "package_distinct_uv_vertices": distinct_uv_vertices,
        "package_lightmap_triangle_corners": lightmap_corners,
        "package_lightmap_uv_min": package_uv_min,
        "package_lightmap_uv_max": package_uv_max,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("package", type=Path)
    parser.add_argument("--expected", type=Path)
    args = parser.parse_args()

    manifest_path = args.manifest.resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["_manifest_root"] = str(manifest_path.parent)
    source_uv_bounds = _source_uv_bounds(manifest)
    selected, selected_ids, excluded_no_uv, excluded_shader = (
        _selected_lightmaps(manifest)
    )
    source_references = sum(
        bool(mesh.get("retail_texture_ids", {}).get("lightmap"))
        for model in manifest["models"]
        for mesh in model["meshes"]
    )
    result = {
        "status": "UNIVERSITY_RETAIL_LIGHTMAPS_VERIFIED",
        "source_lightmap_references": source_references,
        "source_lightmap_texture_ids": len(
            {
                str(mesh["retail_texture_ids"]["lightmap"])
                for model in manifest["models"]
                for mesh in model["meshes"]
                if mesh.get("retail_texture_ids", {}).get("lightmap")
            }
        ),
        "selected_mesh_parts": len(selected),
        "excluded_no_secondary_uv": excluded_no_uv,
        "excluded_shader_mesh_parts": excluded_shader,
        **_verify_package(
            args.package.resolve(),
            manifest,
            selected_ids,
            source_uv_bounds,
        ),
    }
    if args.expected is not None:
        expected = json.loads(
            args.expected.resolve().read_text(encoding="utf-8")
        )["retail_lightmaps"]
        for name, value in result.items():
            if name == "status":
                continue
            if expected[name] != value:
                raise RuntimeError(
                    f"retail lightmap contract {name} changed: "
                    f"{value!r} != {expected[name]!r}"
                )
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
