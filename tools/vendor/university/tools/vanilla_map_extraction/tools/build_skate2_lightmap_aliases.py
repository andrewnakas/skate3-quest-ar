"""Map Skate 2 symbolic lightmap pages onto the existing decoded image cache."""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
import struct


LAYERPAGE_TEXTURE_PATTERN = re.compile(
    rb"(layerpage_[^\x00\r\n]+?)\.Texture"
)
CHROMATICITY_TEXTURE_PATTERN = re.compile(
    rb"(chromo_[^\x00\r\n]+?)\.Texture"
)
RX2_TOC_RECORD_SIZE = 24
RX2_TYPE_MATERIAL = 0x00EB0005


def _png_dimensions(path: Path) -> tuple[int, int]:
    header = path.read_bytes()[:24]
    if (
        len(header) != 24
        or header[:8] != b"\x89PNG\r\n\x1a\n"
        or header[12:16] != b"IHDR"
    ):
        raise ValueError(f"{path} is not a valid PNG")
    return struct.unpack(">II", header[16:24])


def _select_aliases(
    candidates: dict[str, list[dict[str, object]]],
) -> tuple[dict[str, dict[str, object]], int]:
    aliases: dict[str, dict[str, object]] = {}
    duplicate_names = 0
    for source_name, pages in sorted(candidates.items()):
        if len(pages) > 1:
            duplicate_names += 1
        largest_area = max(
            int(page["width"]) * int(page["height"])
            for page in pages
        )
        largest = [
            page
            for page in pages
            if int(page["width"]) * int(page["height"]) == largest_area
        ]
        largest_payloads = {
            (
                int(page["width"]),
                int(page["height"]),
                str(page["png_sha256"]),
            )
            for page in largest
        }
        if len(largest_payloads) != 1:
            raise RuntimeError(
                f"{source_name!r} has conflicting highest-resolution pages"
            )
        selected = min(
            largest,
            key=lambda page: str(page["asset_id"]),
        )
        aliases[source_name] = {
            **selected,
            "source_name": source_name,
            "candidate_count": len(pages),
        }
    return aliases, duplicate_names


def _raw_lightmap_groups(
    data: bytes,
) -> list[dict[str, object]]:
    if len(data) < 0x34:
        raise ValueError("RX2 is too small to contain a section table")
    file_count = struct.unpack_from(">I", data, 0x20)[0]
    file_table = struct.unpack_from(">I", data, 0x30)[0]
    if file_table + file_count * RX2_TOC_RECORD_SIZE > len(data):
        raise ValueError("RX2 section table extends beyond the file")

    groups: list[dict[str, object]] = []
    for record_index in range(file_count):
        record_offset = file_table + record_index * RX2_TOC_RECORD_SIZE
        section_offset, _, section_size, _, _ = struct.unpack_from(
            ">5I", data, record_offset
        )
        section_type = struct.unpack_from(">I", data, record_offset + 20)[0]
        if section_type != RX2_TYPE_MATERIAL:
            continue
        if section_offset + 32 > len(data):
            raise ValueError("RX2 material header extends beyond the file")
        header = struct.unpack_from(">8I", data, section_offset)
        parameter_count = header[1]
        header_size = header[3]
        parameters_size = header[4]
        if parameter_count == 0:
            continue
        payload_size = parameters_size - header_size
        if payload_size < 0 or payload_size % parameter_count:
            raise ValueError("RX2 material parameter table has invalid sizes")
        block_size = payload_size // parameter_count
        if block_size != 32:
            raise ValueError(
                "RX2 material lightmap components require 32-byte records, "
                f"got {block_size}"
            )
        current: dict[str, object] | None = None
        for parameter_index in range(parameter_count):
            parameter_offset = (
                section_offset + header_size + parameter_index * block_size
            )
            values = struct.unpack_from(">8I", data, parameter_offset)

            def source_string(relative_offset: int) -> str:
                start = section_offset + relative_offset
                if not section_offset <= start < section_offset + section_size:
                    raise ValueError(
                        "RX2 material string points outside its section"
                    )
                end = data.index(b"\0", start, section_offset + section_size)
                return data[start:end].decode("ascii")

            kind = source_string(values[0])
            if kind == "Name":
                current = {}
                groups.append(current)
            if current is None or kind != "lightmap":
                continue
            component = int(values[1])
            if component not in (0, 1, 2):
                raise ValueError(
                    f"Skate 2 lightmap uses invalid component {component}"
                )
            current["lightmap_component"] = component
            current["lightmap"] = source_string(values[6])
    return groups


def build_aliases(cache_root: Path) -> dict[str, object]:
    texture_root = cache_root / "rx2" / "textures"
    image_root = cache_root / "textures" / "by_asset"
    model_root = cache_root / "rx2" / "models"
    if (
        not texture_root.is_dir()
        or not image_root.is_dir()
        or not model_root.is_dir()
    ):
        raise FileNotFoundError(
            f"{cache_root} does not contain the Skate 2 texture/model cache"
        )

    layer_candidates: dict[str, list[dict[str, object]]] = defaultdict(list)
    chromaticity_candidates: dict[
        str, list[dict[str, object]]
    ] = defaultdict(list)
    layer_occurrence_count = 0
    chromaticity_occurrence_count = 0
    for rx2_path in sorted(texture_root.glob("*.rx2")):
        data = rx2_path.read_bytes()
        layer_names = {
            match.decode("ascii")
            for match in LAYERPAGE_TEXTURE_PATTERN.findall(data)
        }
        chromaticity_names = {
            match.decode("ascii")
            for match in CHROMATICITY_TEXTURE_PATTERN.findall(data)
        }
        if not layer_names and not chromaticity_names:
            continue
        png_path = image_root / f"{rx2_path.stem.upper()}_0.png"
        if not png_path.is_file():
            raise FileNotFoundError(
                f"decoded Skate 2 lighting image is missing: {png_path}"
            )
        width, height = _png_dimensions(png_path)
        digest = hashlib.sha256(png_path.read_bytes()).hexdigest()
        entry = {
            "texture_id": f"asset_{rx2_path.stem.lower()}",
            "asset_id": f"0x{rx2_path.stem.upper()}",
            "rx2": str(rx2_path.relative_to(cache_root)),
            "png": str(png_path.relative_to(cache_root)),
            "width": width,
            "height": height,
            "png_sha256": digest,
        }
        for source_name in layer_names:
            layer_occurrence_count += 1
            layer_candidates[source_name].append(entry)
        for source_name in chromaticity_names:
            chromaticity_occurrence_count += 1
            chromaticity_candidates[source_name].append(entry)

    aliases, duplicate_names = _select_aliases(layer_candidates)
    chromaticity_aliases, duplicate_chromaticity_names = _select_aliases(
        chromaticity_candidates
    )
    if not aliases:
        raise RuntimeError("the cache contains no Skate 2 layerpage textures")
    if not chromaticity_aliases:
        raise RuntimeError(
            "the cache contains no Skate 2 chromaticity textures"
        )
    manifest_path = cache_root / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    mesh_bindings: dict[str, dict[str, object]] = {}
    source_references = 0
    eligible_meshes = 0
    excluded_shader_meshes = 0
    excluded_no_uv_meshes = 0
    referenced_names: set[str] = set()
    eligible_names: set[str] = set()
    referenced_chromaticities: set[str] = set()
    eligible_chromaticities: set[str] = set()
    eligible_pairs: set[tuple[str, str, int]] = set()
    component_counts = [0, 0, 0]
    for model in manifest["models"]:
        asset_id = str(model["asset_id"]).lower()
        model_path = model_root / f"{asset_id[2:].upper()}.rx2"
        if not model_path.is_file():
            raise FileNotFoundError(
                f"decoded Skate 2 model is missing: {model_path}"
            )
        raw_groups = _raw_lightmap_groups(model_path.read_bytes())
        for mesh in model["meshes"]:
            values = mesh.get("retail_parameters", {}).get(
                "lightmap",
                [],
            )
            if not values or not values[0]:
                continue
            source_name = str(values[0])
            chromaticity_values = mesh.get(
                "retail_parameters", {}
            ).get("chromaticity", [])
            if not chromaticity_values or not chromaticity_values[0]:
                raise RuntimeError(
                    f"{asset_id}:{mesh['index']} has a layer page without "
                    "chromaticity"
                )
            chromaticity_name = str(chromaticity_values[0])
            group_index = int(
                mesh.get("retail_material_group_index", -1)
            )
            if not 0 <= group_index < len(raw_groups):
                raise RuntimeError(
                    f"{asset_id}:{mesh['index']} has invalid material group "
                    f"{group_index} of {len(raw_groups)}"
                )
            raw_group = raw_groups[group_index]
            if raw_group.get("lightmap") != source_name:
                raise RuntimeError(
                    f"{asset_id}:{mesh['index']} raw layer page disagrees "
                    "with the extraction manifest"
                )
            component = int(raw_group["lightmap_component"])
            source_references += 1
            referenced_names.add(source_name)
            referenced_chromaticities.add(chromaticity_name)
            shader_name = str(mesh.get("shader_name") or "").lower()
            reason = ""
            eligible = True
            if shader_name.startswith(("water.", "ocean.")):
                eligible = False
                reason = "shader"
                excluded_shader_meshes += 1
            elif mesh.get("lightmap_uv") is None:
                eligible = False
                reason = "no_secondary_uv"
                excluded_no_uv_meshes += 1
            else:
                eligible_meshes += 1
                eligible_names.add(source_name)
                eligible_chromaticities.add(chromaticity_name)
                eligible_pairs.add(
                    (source_name, chromaticity_name, component)
                )
                component_counts[component] += 1
            mesh_key = f"{asset_id}:{int(mesh['index'])}"
            if mesh_key in mesh_bindings:
                raise RuntimeError(
                    f"duplicate Skate 2 mesh identity {mesh_key}"
                )
            mesh_bindings[mesh_key] = {
                "source_name": source_name,
                "chromaticity_name": chromaticity_name,
                "lightmap_component": component,
                "eligible": eligible,
                "exclusion": reason,
            }
    missing_references = referenced_names - set(aliases)
    if missing_references:
        raise RuntimeError(
            "decoded layer pages are missing for material references: "
            f"{sorted(missing_references)[:10]}"
        )
    missing_chromaticities = referenced_chromaticities - set(
        chromaticity_aliases
    )
    if missing_chromaticities:
        raise RuntimeError(
            "decoded chromaticity pages are missing for material references: "
            f"{sorted(missing_chromaticities)[:10]}"
        )
    return {
        "format": "skate2-lightmap-aliases-v2",
        "cache_root": str(cache_root.resolve()),
        "aliases": aliases,
        "chromaticity_aliases": chromaticity_aliases,
        "mesh_bindings": mesh_bindings,
        "summary": {
            "unique_lightmap_pages": len(aliases),
            "asset_occurrences": layer_occurrence_count,
            "duplicate_page_names": duplicate_names,
            "unique_chromaticity_pages": len(chromaticity_aliases),
            "chromaticity_asset_occurrences": (
                chromaticity_occurrence_count
            ),
            "duplicate_chromaticity_names": (
                duplicate_chromaticity_names
            ),
            "decoded_rgba_bytes": sum(
                int(page["width"]) * int(page["height"]) * 4
                for page in aliases.values()
            ),
            "decoded_chromaticity_rgba_bytes": sum(
                int(page["width"]) * int(page["height"]) * 4
                for page in chromaticity_aliases.values()
            ),
            "source_references": source_references,
            "eligible_meshes": eligible_meshes,
            "eligible_lightmap_pages": len(eligible_names),
            "eligible_chromaticity_pages": len(
                eligible_chromaticities
            ),
            "eligible_lightmap_chromaticity_pairs": len(eligible_pairs),
            "eligible_component_counts": component_counts,
            "excluded_shader_meshes": excluded_shader_meshes,
            "excluded_no_uv_meshes": excluded_no_uv_meshes,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cache_root", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    result = build_aliases(args.cache_root.resolve())
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(result, indent=2) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(result["summary"], indent=2))
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
