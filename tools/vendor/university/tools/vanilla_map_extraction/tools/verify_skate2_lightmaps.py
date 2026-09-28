#!/usr/bin/env python3
"""Verify Skate 2 retail layer pages and UVs in an exported SKATE package."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import struct
import sys

from PIL import Image


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import Reader, VERTEX_BYTES_V12  # noqa: E402


EXPECTED_LIGHTMAPPED_MATERIALS = 42_249
EXPECTED_LIGHTMAP_IMAGES = 2_892


def _png_rgba_blender_order(path: Path) -> bytes:
    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        width, height = rgba.size
        top_to_bottom = rgba.tobytes()
        row_bytes = width * 4
        return b"".join(
            top_to_bottom[row * row_bytes : (row + 1) * row_bytes]
            for row in range(height - 1, -1, -1)
        )


def _read_retail_material(
    reader: Reader, index: int
) -> tuple[dict[str, tuple[int, int, int, int]], dict[str, list[str]]]:
    retail_enabled = reader.u32(f"material {index} retail enabled")
    if not retail_enabled:
        return {}, {}
    reader.skip(8 + 4 + 4, f"material {index} retail identity")
    reader.string(f"material {index} retail shader")
    reader.skip(4 + 4, f"material {index} retail family/flags")
    binding_count = reader.u32(f"material {index} retail binding count")
    bindings: dict[str, tuple[int, int, int, int]] = {}
    for binding_index in range(binding_count):
        semantic = reader.string(
            f"material {index} binding {binding_index} semantic"
        )
        bindings[semantic] = struct.unpack(
            "<4I",
            reader.take(
                4 * 4,
                f"material {index} binding {binding_index} fields",
            ),
        )
    parameter_count = reader.u32(
        f"material {index} retail parameter count"
    )
    parameters: dict[str, list[str]] = {}
    for parameter_index in range(parameter_count):
        name = reader.string(
            f"material {index} parameter {parameter_index} name"
        )
        value_count = reader.u32(
            f"material {index} parameter {parameter_index} values"
        )
        parameters[name] = [
            reader.string(
                f"material {index} parameter {parameter_index} "
                f"value {value_index}"
            )
            for value_index in range(value_count)
        ]
    reader.string(f"material {index} retail source")
    return bindings, parameters


def verify(package_path: Path, aliases_path: Path) -> dict[str, object]:
    alias_document = json.loads(aliases_path.read_text(encoding="utf-8"))
    aliases = alias_document["aliases"]
    aliases_by_texture = {
        str(entry["texture_id"]): entry
        for entry in aliases.values()
    }
    chromaticity_aliases_by_texture = {
        str(entry["texture_id"]): entry
        for entry in alias_document["chromaticity_aliases"].values()
    }
    cache_root = Path(alias_document["cache_root"])

    reader = Reader(package_path.read_bytes())
    magic = reader.take(8, "magic")
    if magic not in (b"SKATE12\0", b"SKATE13\0", b"SKATE14\0"):
        raise RuntimeError(f"expected SKATE12/13 package, found {magic!r}")
    if reader.u32("endian marker") != 0x12345678:
        raise RuntimeError("invalid SKATE endian marker")
    map_name = reader.string("map name")
    reader.skip(49 * 4, "map metadata")
    counts = struct.unpack("<9I", reader.take(9 * 4, "count table"))
    material_count, texture_count, vertex_count, index_count = counts[:4]

    lightmap_material_ids: set[int] = set()
    material_lightmaps: dict[int, int] = {}
    material_chromaticity: dict[int, int] = {}
    component_counts = [0, 0, 0]
    lightmap_strengths: set[float] = set()
    for index in range(material_count):
        reader.string(f"material {index} name")
        fields = reader.take(76, f"material {index} fields")
        lightmap_texture = struct.unpack_from("<I", fields, 36)[0]
        baked_strength = struct.unpack_from("<f", fields, 40)[0]
        material_id = index + 1
        if lightmap_texture:
            lightmap_material_ids.add(material_id)
            material_lightmaps[material_id] = lightmap_texture
            lightmap_strengths.add(baked_strength)
        bindings, parameters = _read_retail_material(reader, index)
        if lightmap_texture:
            chromaticity_binding = bindings.get("chromaticity")
            if chromaticity_binding is None:
                raise RuntimeError(
                    f"lightmapped material {index} has no chromaticity"
                )
            chromaticity_texture, uv_set, address_u, address_v = (
                chromaticity_binding
            )
            if (
                chromaticity_texture == 0
                or uv_set != 1
                or address_u != 1
                or address_v != 1
            ):
                raise RuntimeError(
                    f"material {index} has invalid chromaticity binding "
                    f"{chromaticity_binding}"
                )
            component_values = parameters.get(
                "skate2_lightmap_component", []
            )
            if len(component_values) != 1:
                raise RuntimeError(
                    f"material {index} has no Skate 2 component"
                )
            component = int(component_values[0])
            if component < 0 or component > 2:
                raise RuntimeError(
                    f"material {index} has invalid component {component}"
                )
            component_counts[component] += 1
            material_chromaticity[material_id] = chromaticity_texture

    if len(lightmap_material_ids) != EXPECTED_LIGHTMAPPED_MATERIALS:
        raise RuntimeError(
            f"package has {len(lightmap_material_ids)} lightmapped "
            f"materials, expected {EXPECTED_LIGHTMAPPED_MATERIALS}"
        )
    if lightmap_strengths != {0.25}:
        raise RuntimeError(
            f"retail lightmap strengths changed: {lightmap_strengths}"
        )

    lightmap_texture_ids = set(material_lightmaps.values())
    chromaticity_texture_ids = set(material_chromaticity.values())
    texture_names: dict[int, str] = {}
    texture_records: dict[int, tuple[int, int, int, bytes]] = {}
    for index in range(texture_count):
        texture_id = index + 1
        name = reader.string(f"texture {index} name")
        width = reader.u32(f"texture {index} width")
        height = reader.u32(f"texture {index} height")
        color_space = reader.u32(f"texture {index} color space")
        rgba8 = reader.stored(width * height * 4, f"texture {index}")
        texture_names[texture_id] = name
        if (
            texture_id in lightmap_texture_ids
            or texture_id in chromaticity_texture_ids
        ):
            texture_records[texture_id] = (
                width,
                height,
                color_space,
                rgba8,
            )

    package_lightmap_names = {
        texture_names[texture_id]
        for texture_id in material_lightmaps.values()
    }
    if len(package_lightmap_names) != EXPECTED_LIGHTMAP_IMAGES:
        raise RuntimeError(
            f"package has {len(package_lightmap_names)} lightmap images, "
            f"expected {EXPECTED_LIGHTMAP_IMAGES}"
        )
    unknown_names = package_lightmap_names - set(aliases_by_texture)
    if unknown_names:
        raise RuntimeError(
            f"package contains unknown Skate 2 pages: "
            f"{sorted(unknown_names)[:10]}"
        )

    package_chromaticity_names = {
        texture_names[texture_id]
        for texture_id in material_chromaticity.values()
    }
    if len(package_chromaticity_names) < 7_000:
        raise RuntimeError(
            "package contains too few Skate 2 chromaticity images: "
            f"{len(package_chromaticity_names)}"
        )
    unknown_chromaticity = (
        package_chromaticity_names
        - set(chromaticity_aliases_by_texture)
    )
    if unknown_chromaticity:
        raise RuntimeError(
            "package contains unknown Skate 2 chromaticity pages: "
            f"{sorted(unknown_chromaticity)[:10]}"
        )

    exact_payload_bytes = 0
    for texture_id in sorted(lightmap_texture_ids):
        name = texture_names[texture_id]
        alias = aliases_by_texture[name]
        width, height, color_space, rgba8 = texture_records[texture_id]
        expected_size = (int(alias["width"]), int(alias["height"]))
        if (width, height) != expected_size or color_space != 0:
            raise RuntimeError(
                f"{name!r} changed format: {(width, height, color_space)}"
            )
        source = _png_rgba_blender_order(cache_root / str(alias["png"]))
        if rgba8 != source:
            raise RuntimeError(
                f"{name!r} changed pixels during Blender export"
            )
        exact_payload_bytes += len(rgba8)

    exact_chromaticity_payload_bytes = 0
    for texture_id in sorted(chromaticity_texture_ids):
        name = texture_names[texture_id]
        alias = chromaticity_aliases_by_texture[name]
        width, height, color_space, rgba8 = texture_records[texture_id]
        expected_size = (int(alias["width"]), int(alias["height"]))
        if (width, height) != expected_size or color_space != 0:
            raise RuntimeError(
                f"{name!r} changed format: "
                f"{(width, height, color_space)}"
            )
        source = _png_rgba_blender_order(cache_root / str(alias["png"]))
        if rgba8 != source:
            raise RuntimeError(
                f"{name!r} changed chromaticity pixels during export"
            )
        exact_chromaticity_payload_bytes += len(rgba8)

    vertex_bytes = reader.stored(
        vertex_count * VERTEX_BYTES_V12,
        "visual vertex block",
    )
    reader.stored(index_count * 4, "visual index block")
    used_materials: set[int] = set()
    lightmapped_vertices = 0
    distinct_uv_vertices = 0
    uv_min = [math.inf, math.inf]
    uv_max = [-math.inf, -math.inf]
    for offset in range(0, len(vertex_bytes), VERTEX_BYTES_V12):
        material_id = struct.unpack_from("<I", vertex_bytes, offset + 40)[0]
        if material_id not in lightmap_material_ids:
            continue
        uv0 = struct.unpack_from("<2f", vertex_bytes, offset + 24)
        uv1 = struct.unpack_from("<2f", vertex_bytes, offset + 32)
        if not all(math.isfinite(value) for value in (*uv0, *uv1)):
            raise RuntimeError("package contains non-finite lightmap UVs")
        lightmapped_vertices += 1
        distinct_uv_vertices += uv0 != uv1
        used_materials.add(material_id)
        for axis in range(2):
            uv_min[axis] = min(uv_min[axis], uv1[axis])
            uv_max[axis] = max(uv_max[axis], uv1[axis])

    if used_materials != lightmap_material_ids:
        raise RuntimeError("package contains unused lightmapped materials")
    if lightmapped_vertices == 0 or distinct_uv_vertices == 0:
        raise RuntimeError("package did not transport usable lightmap UVs")

    return {
        "map_name": map_name,
        "package_bytes": package_path.stat().st_size,
        "materials": material_count,
        "textures": texture_count,
        "lightmapped_materials": len(lightmap_material_ids),
        "lightmap_images": len(package_lightmap_names),
        "chromaticity_images": len(package_chromaticity_names),
        "lightmap_component_counts": component_counts,
        "lightmap_strength": min(lightmap_strengths),
        "exact_lightmap_payload_bytes": exact_payload_bytes,
        "exact_chromaticity_payload_bytes": (
            exact_chromaticity_payload_bytes
        ),
        "lightmapped_vertices": lightmapped_vertices,
        "distinct_lightmap_uv_vertices": distinct_uv_vertices,
        "lightmap_uv_min": uv_min,
        "lightmap_uv_max": uv_max,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("aliases", type=Path)
    args = parser.parse_args()
    result = verify(args.package.resolve(), args.aliases.resolve())
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
