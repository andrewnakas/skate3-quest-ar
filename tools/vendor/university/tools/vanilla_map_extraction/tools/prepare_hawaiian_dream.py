"""Prepare lossless RX2 assets and a Blender-friendly Hawaiian Dream cache."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import sys

import numpy

from retail_collision_mesh import decode_rx2_clustered_meshes
from retail_grind_splines import decode_grind_splines
from retail_lightmap_uv import (
    decode_decal_uvs,
    decode_lightmap_uvs,
    decode_retail_world_frame,
)
from retail_texture_decode import (
    B5G6R5_DECODER_NAME,
    B5G6R5_FORMAT_ID,
    decode_b5g6r5,
)
from skate3_streams import (
    ASSET_TYPE_MODEL,
    ASSET_TYPE_TEXTURE,
    StreamAsset,
    load_district_stream,
)


DISTRICT_NAME = "DIST_DHS 32221"
TEXTURE_NAME_PATTERN = re.compile(rb"(0x[0-9A-Fa-f]{16})\.Texture")
MATERIAL_TEXTURE_PATTERN = re.compile(r"(0x[0-9A-Fa-f]{16})$")
LAYERPAGE_TEXTURE_PATTERN = re.compile(
    rb"(layerpage_[^\x00\r\n]+?)\.Texture"
)
CHROMATICITY_TEXTURE_PATTERN = re.compile(
    rb"(chromo_[^\x00\r\n]+?)\.Texture"
)
ALPHA_BLEND_SHADERS = {
    "environment.reflective_trans",
    "environment.transparent",
}
RETAIL_TEXTURE_CHANNELS = (
    "diffuse",
    "transparent",
    "normal",
    "normal2",
    "specular",
    "lightmap",
    "chromaticity",
    "detail",
    "macrooverlay",
    "decal",
    "environment",
    "noise",
)
RETAIL_WORLD_FRAME_SHADER_PREFIXES = (
    "environment.default",
    "environment.decal",
    "environment.reflective",
)
RX2_TOC_RECORD_SIZE = 24
RX2_TYPE_MATERIAL = 0x00EB0005
RX2_TYPE_EXTERNAL_REFERENCES = 0x00EB000B
RX2_TYPE_MESH_DESCRIPTOR = 0x00EB0023
RX2_TYPE_MATERIAL_IMPORT = 0x00EB0066


def _default_workspace() -> Path:
    return Path(__file__).resolve().parents[1]


def _default_stream_directory(workspace: Path) -> Path:
    return (
        workspace
        / "raw"
        / "hawaiian_dream"
        / "extracted"
        / "data"
        / "content"
        / "world"
        / "stream"
        / DISTRICT_NAME
    )


def _write_rx2(output_root: Path, category: str, asset: StreamAsset, *, write: bool = True) -> Path:
    target = output_root / "rx2" / category / f"{asset.record.asset_id:016X}.rx2"
    if write:
        target.write_bytes(asset.data)
    return target


def _texture_id(data: bytes, asset_id: int) -> str:
    matches = TEXTURE_NAME_PATTERN.findall(data)
    if matches:
        return matches[-1].decode("ascii").lower()
    layerpages = LAYERPAGE_TEXTURE_PATTERN.findall(data)
    if layerpages:
        return _symbolic_texture_id(
            layerpages[-1].decode("ascii")
        )
    chromaticity = CHROMATICITY_TEXTURE_PATTERN.findall(data)
    if chromaticity:
        return _symbolic_texture_id(
            chromaticity[-1].decode("ascii")
        )
    return f"asset_{asset_id:016x}"


def _symbolic_texture_id(name: str) -> str:
    """Return a compact stable ID for a non-hashed retail texture name.

    Skate 2's baked layer and chromaticity pages are referenced by long
    symbolic names instead of the trailing 64-bit IDs used by ordinary
    retail textures.
    Blender truncates long datablock names, so use a deterministic digest as
    the interchange identity while preserving the source spelling in retail
    parameters.
    """

    normalized = name.strip().removesuffix(".Texture").lower()
    digest = hashlib.sha256(normalized.encode("utf-8")).hexdigest()[:24]
    return f"retail_symbolic_{digest}"


def _material_texture_id(material_name: str | None) -> str | None:
    if not material_name:
        return None
    match = MATERIAL_TEXTURE_PATTERN.search(material_name)
    if match:
        return match.group(1).lower()
    if material_name.lower().startswith(("layerpage_", "chromo_")):
        return _symbolic_texture_id(material_name)
    return None


def _group_material_parameters(
    parameters: object,
) -> list[dict[str, list[str]]]:
    """Recover the per-mesh material groups flattened by mdl_parser.

    A material starts at each ``Name`` parameter. Selecting the Nth value
    from a global list of ``diffuse`` parameters is unsafe because materials
    such as water have no diffuse channel; that shifts every later mesh onto
    the wrong texture.
    """
    groups: list[dict[str, list[str]]] = []
    current: dict[str, list[str]] | None = None
    for parameter in parameters:
        kind = str(parameter.kind)
        value = str(parameter.value)
        if kind == "Name":
            current = {}
            groups.append(current)
        if current is not None:
            current.setdefault(kind, []).append(value)
    return groups


def _first_parameter(
    group: dict[str, list[str]],
    name: str,
) -> str | None:
    values = group.get(name, [])
    return values[0] if values and values[0] else None


def _bind_material_groups_by_guid(
    data: bytes,
    groups: list[dict[str, list[str]]],
    mesh_count: int,
    *,
    allow_import_order_fallback: bool = False,
) -> tuple[list[dict[str, list[str]]], list[dict[str, object]]]:
    """Resolve every mesh to its retail material through RX2 GUID handles.

    Material records are not stored in render-mesh order. Each ``Name``
    parameter carries a 64-bit GUID, the external-reference table associates
    that GUID with a local handle, and each ``0x00EB0023`` mesh descriptor
    stores the handle it draws with. Following that chain is authoritative
    and also disambiguates repeated materials with different lightmaps.
    """

    if len(data) < 0x34:
        raise ValueError("RX2 is too small to contain a section table")
    file_count = struct.unpack_from(">I", data, 0x20)[0]
    file_table = struct.unpack_from(">I", data, 0x30)[0]
    if file_table + file_count * RX2_TOC_RECORD_SIZE > len(data):
        raise ValueError("RX2 section table extends beyond the file")

    records: list[tuple[int, int, int]] = []
    for record_index in range(file_count):
        record_offset = file_table + record_index * RX2_TOC_RECORD_SIZE
        section_offset, _, section_size, _, _ = struct.unpack_from(
            ">5I",
            data,
            record_offset,
        )
        section_type = struct.unpack_from(">I", data, record_offset + 20)[0]
        records.append((section_type, section_offset, section_size))

    material_guids: list[int] = []
    for section_type, section_offset, _ in records:
        if section_type != RX2_TYPE_MATERIAL:
            continue
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
                f"RX2 material GUID binding requires 32-byte records, got {block_size}"
            )
        for parameter_index in range(parameter_count):
            parameter_offset = (
                section_offset + header_size + parameter_index * block_size
            )
            values = struct.unpack_from(">8I", data, parameter_offset)
            kind_offset = section_offset + values[0]
            kind_end = data.index(b"\0", kind_offset)
            if data[kind_offset:kind_end] == b"Name":
                material_guids.append((values[4] << 32) | values[5])

    if len(material_guids) != len(groups):
        raise ValueError(
            "RX2 material GUID count does not match parsed material groups: "
            f"{len(material_guids)} versus {len(groups)}"
        )

    imports: list[tuple[int, int]] = []
    for section_type, section_offset, _ in records:
        if section_type != RX2_TYPE_EXTERNAL_REFERENCES:
            continue
        import_count, import_offset = struct.unpack_from(
            ">2I",
            data,
            section_offset,
        )
        for import_index in range(import_count):
            entry_offset = section_offset + import_offset + import_index * 24
            _, _, guid_high, guid_low, import_type, handle = struct.unpack_from(
                ">6I",
                data,
                entry_offset,
            )
            if import_type == RX2_TYPE_MATERIAL_IMPORT:
                imports.append(((guid_high << 32) | guid_low, handle))

    handle_groups: dict[int, tuple[int, int, int, str]] = {}
    import_cursor = 0
    guid_binding_failed = False
    for group_index, material_guid in enumerate(material_guids):
        while (
            import_cursor < len(imports)
            and imports[import_cursor][0] != material_guid
        ):
            import_cursor += 1
        if import_cursor >= len(imports):
            guid_binding_failed = True
            break
        external_guid, handle = imports[import_cursor]
        handle_groups[handle] = (
            group_index,
            material_guid,
            external_guid,
            "external_reference_guid",
        )
        import_cursor += 1

    if guid_binding_failed:
        if not allow_import_order_fallback or len(imports) != len(material_guids):
            raise ValueError(
                "RX2 material GUID is missing from the external-reference table: "
                f"0x{material_guids[len(handle_groups)]:016X}"
            )
        # Skate 2 sometimes uses a different GUID namespace for a material's
        # Name parameter and its external reference. The tables still have a
        # one-to-one order, and mesh descriptors continue to reference the
        # imported handles. Keep both GUIDs so the mismatch remains visible.
        handle_groups.clear()
        for group_index, (material_guid, imported) in enumerate(
            zip(material_guids, imports)
        ):
            external_guid, handle = imported
            handle_groups[handle] = (
                group_index,
                material_guid,
                external_guid,
                "external_reference_order",
            )

    bound_groups: list[dict[str, list[str]]] = []
    bindings: list[dict[str, object]] = []
    for section_type, section_offset, _ in records:
        if section_type != RX2_TYPE_MESH_DESCRIPTOR:
            continue
        handle = struct.unpack_from(">I", data, section_offset + 0x24)[0]
        if handle not in handle_groups:
            raise ValueError(
                "RX2 mesh material handle is unresolved: "
                f"0x{handle:08X}"
            )
        group_index, material_guid, external_guid, strategy = handle_groups[handle]
        bound_groups.append(groups[group_index])
        bindings.append(
            {
                "group_index": group_index,
                "material_guid": material_guid,
                "external_reference_guid": external_guid,
                "material_handle": handle,
                "strategy": strategy,
            }
        )

    if len(bound_groups) != mesh_count:
        raise ValueError(
            "RX2 mesh descriptor count does not match parsed render meshes: "
            f"{len(bound_groups)} versus {mesh_count}"
        )
    return bound_groups, bindings


def _material_metadata(
    groups: list[dict[str, list[str]]],
    mesh_index: int,
) -> dict[str, object]:
    group = groups[mesh_index] if mesh_index < len(groups) else {}
    diffuse = _first_parameter(group, "diffuse")
    transparent = _first_parameter(group, "transparent")
    shader_name = _first_parameter(group, "AttribulatorMaterialName")
    material_name = diffuse or transparent
    shader_key = (shader_name or "").lower()
    if shader_key in ALPHA_BLEND_SHADERS:
        alpha_mode = 2
    elif (
        transparent is not None
        or "alphatest" in shader_key
        or shader_key in {"animated.tree", "tree.default"}
    ):
        alpha_mode = 1
    else:
        alpha_mode = 0
    retail_texture_ids = {
        channel: texture_id
        for channel in RETAIL_TEXTURE_CHANNELS
        if (
            texture_id := _material_texture_id(
                _first_parameter(group, channel)
            )
        )
        is not None
    }
    return {
        "material_name": material_name,
        "texture_id": _material_texture_id(material_name),
        "texture_channel": (
            "diffuse"
            if diffuse is not None
            else "transparent"
            if transparent is not None
            else None
        ),
        "shader_name": shader_name,
        "alpha_mode": alpha_mode,
        "alpha_cutoff": 0.5,
        # Preserve every material channel, including constants and fields the
        # current renderer does not understand yet. Values remain in parser
        # order as strings so GUIDs and packed numeric spellings round-trip
        # without type guessing.
        "retail_parameters": {
            name: list(values)
            for name, values in group.items()
        },
        # Preserve every retail texture association in the extraction
        # manifest even when the current Blender/export policy does not yet
        # consume that role. This keeps extraction lossless and makes later
        # fidelity work independent of another RX2 material reparse.
        "retail_texture_ids": retail_texture_ids,
    }


def prepare(
    stream_directory: Path,
    output_root: Path,
    utt_root: Path,
    *,
    district_name: str = DISTRICT_NAME,
    map_name: str = "Danny Way's Hawaiian Dream",
    package_name: str = "DHS by DH13",
    cache_format: str = "skate3-hawaiian-dream-cache-v1",
    texture_stream_names: tuple[str, ...] = (),
    excluded_normal_texture_ids: tuple[str, ...] = (),
    allow_material_import_order_fallback: bool = False,
    grind_coordinate_mode: str = "world_space",
    excluded_static_model_asset_ids: tuple[str, ...] = (),
    raw_texture_cache: bool = False,
    collision_consumer=None,
    write_render_sources: bool = True,
) -> Path:
    sys.path.insert(0, str(utt_root))
    import rx2_parser
    from mdl_parser import parse_rx2 as parse_model

    output_root.mkdir(parents=True, exist_ok=True)
    # Create each output directory once, rather than thousands of failing
    # mkdir calls against directories already created for previous assets.
    for directory in ("rx2/textures", "rx2/models", "rx2/presentation_other",
                      "rx2/simulation", "textures/by_asset", "models"):
        (output_root / directory).mkdir(parents=True, exist_ok=True)
    presentation_assets = load_district_stream(
        stream_directory,
        "Pres",
        district_name,
    )
    simulation_assets = load_district_stream(
        stream_directory,
        "Sim",
        district_name,
    )
    extra_texture_assets = [
        asset
        for stream_name in texture_stream_names
        for asset in load_district_stream(
            stream_directory,
            stream_name,
            district_name,
        )
    ]

    manifest: dict[str, object] = {
        "format": cache_format,
        "map_name": map_name,
        "package_name": package_name,
        "district_name": district_name,
        "source_stream_directory": str(stream_directory),
        "axis_conversion": {
            "runtime_to_blender": ["x", "-z", "y"],
            "uv_v_flipped": True,
        },
        "normal_texture_policy": {
            "excluded_texture_ids": sorted(
                texture_id.lower()
                for texture_id in excluded_normal_texture_ids
            ),
        },
        "grind_coordinate_policy": {
            "mode": grind_coordinate_mode,
            "classification_margin": 40.0,
        },
        "presentation_model_policy": {
            "excluded_asset_ids": sorted(
                asset_id.lower()
                for asset_id in excluded_static_model_asset_ids
            ),
        },
        "textures": {},
        "models": [],
        "simulation_assets": [],
        "grind_splines": [],
        "other_presentation_assets": [],
    }
    textures: dict[str, object] = manifest["textures"]  # type: ignore[assignment]
    models: list[object] = manifest["models"]  # type: ignore[assignment]
    simulation: list[object] = manifest["simulation_assets"]  # type: ignore[assignment]
    grind_splines: list[object] = manifest["grind_splines"]  # type: ignore[assignment]
    other: list[object] = manifest["other_presentation_assets"]  # type: ignore[assignment]
    texture_digests: dict[str, set[bytes]] = {}

    for asset in presentation_assets + extra_texture_assets:
        asset_id = asset.record.asset_id
        source_file = asset.source_path.name
        if asset.record.asset_type == ASSET_TYPE_TEXTURE:
            rx2_path = _write_rx2(output_root, "textures", asset, write=write_render_sources)
            base_texture_id = _texture_id(asset.data, asset_id)
            parsed = rx2_parser.parse_rx2(asset.data)
            if not parsed.textures:
                raise RuntimeError(
                    f"texture asset 0x{asset_id:016X} contains no decoded texture"
                )
            for texture_index, texture in enumerate(parsed.textures):
                texture_decoder = None
                if texture.fmt_id == B5G6R5_FORMAT_ID:
                    raw_texture = parsed.data[
                        texture.data_offset :
                        texture.data_offset + texture.buffer_size
                    ]
                    face_height = texture.height // texture.cube_faces
                    texture.rgba = b''.join(decode_b5g6r5(
                        raw_texture[face*texture.face_stride:] if texture.cube_faces == 6 else raw_texture,
                        texture.width, face_height,
                    ) for face in range(texture.cube_faces))
                    texture_decoder = B5G6R5_DECODER_NAME
                texture_id = (
                    base_texture_id
                    if texture_index == 0
                    else f"{base_texture_id}_{texture_index}"
                )
                png_path = (
                    output_root
                    / "textures"
                    / "by_asset"
                    / f"{asset_id:016X}_{texture_index}{'.rgba' if raw_texture_cache else '.png'}"
                )
                if raw_texture_cache:png_path.write_bytes(texture.rgba)
                else:texture.save_png(png_path)
                new_entry = {
                    "stream_asset_id": f"0x{asset_id:016X}",
                    "stream_file": source_file,
                    "rx2": str(rx2_path.relative_to(output_root)),
                    "rgba" if raw_texture_cache else "png": str(png_path.relative_to(output_root)),
                    "texture_index": texture_index,
                    "width": texture.width,
                    "height": texture.height,
                    "cube_faces": texture.cube_faces,
                    "format": texture.fmt_name,
                    "warnings": list(parsed.warnings),
                }
                if texture_decoder is not None:
                    new_entry["decoder"] = texture_decoder
                texture_digest = hashlib.sha256(texture.rgba).digest()
                known_digests = texture_digests.setdefault(texture_id, set())
                if texture_id in textures:
                    existing = textures[texture_id]
                    if texture_digest in known_digests:
                        existing.setdefault("alternate_stream_assets", []).append(  # type: ignore[union-attr]
                            new_entry
                        )
                    elif texture.width * texture.height > (  # type: ignore[index]
                        existing["width"] * existing["height"]  # type: ignore[index,operator]
                    ):
                        previous_entry = {
                            key: value
                            for key, value in existing.items()  # type: ignore[union-attr]
                            if key not in {"variants", "alternate_stream_assets"}
                        }
                        new_entry["variants"] = list(  # type: ignore[index]
                            existing.get("variants", [])  # type: ignore[union-attr]
                        ) + [previous_entry]
                        if existing.get("alternate_stream_assets"):  # type: ignore[union-attr]
                            new_entry["alternate_stream_assets"] = existing[  # type: ignore[index]
                                "alternate_stream_assets"
                            ]
                        textures[texture_id] = new_entry
                    else:
                        existing.setdefault("variants", []).append(new_entry)  # type: ignore[union-attr]
                    known_digests.add(texture_digest)
                    continue

                known_digests.add(texture_digest)
                textures[texture_id] = new_entry
        elif asset.record.asset_type == ASSET_TYPE_MODEL:
            rx2_path = _write_rx2(output_root, "models", asset, write=write_render_sources)
            parsed = parse_model(
                asset.data,
                source_path=rx2_path,
                strict=False,
            )
            npz_path = output_root / "models" / f"{asset_id:016X}.npz"
            arrays: dict[str, numpy.ndarray] = {}
            mesh_entries: list[dict[str, object]] = []
            source_material_groups = _group_material_parameters(parsed.materials)
            material_groups, material_bindings = (
                _bind_material_groups_by_guid(
                    asset.data,
                    source_material_groups,
                    len(parsed.meshes),
                    allow_import_order_fallback=(
                        allow_material_import_order_fallback
                    ),
                )
            )
            for mesh_index, mesh in enumerate(parsed.meshes):
                material = _material_metadata(material_groups, mesh_index)
                material_group = (
                    material_groups[mesh_index]
                    if mesh_index < len(material_groups)
                    else {}
                )
                material_binding = material_bindings[mesh_index]
                arrays[f"vertices_{mesh_index}"] = numpy.asarray(
                    mesh.vertices,
                    dtype=numpy.float32,
                )
                arrays[f"faces_{mesh_index}"] = numpy.asarray(
                    mesh.faces,
                    dtype=numpy.uint32,
                )
                if mesh.uvs is not None:
                    arrays[f"uvs_{mesh_index}"] = numpy.asarray(
                        mesh.uvs,
                        dtype=numpy.float32,
                    )
                lightmap_uv = decode_lightmap_uvs(
                    asset.data,
                    vertex_buffer_offset=mesh.source_offsets[
                        "vertex_buffer"
                    ],
                    vertex_count=mesh.vertex_count,
                    vertex_stride=mesh.vertex_stride,
                    attributes=mesh.attributes,
                )
                if lightmap_uv is not None:
                    arrays[f"lightmap_uvs_{mesh_index}"] = (
                        lightmap_uv.values
                    )
                shader_name = str(material.get("shader_name") or "")
                retail_world_frame = (
                    decode_retail_world_frame(
                        asset.data,
                        vertex_buffer_offset=mesh.source_offsets[
                            "vertex_buffer"
                        ],
                        vertex_count=mesh.vertex_count,
                        vertex_stride=mesh.vertex_stride,
                        attributes=mesh.attributes,
                    )
                    if shader_name.startswith(
                        RETAIL_WORLD_FRAME_SHADER_PREFIXES
                    )
                    else None
                )
                if retail_world_frame is not None:
                    arrays[f"retail_normals_{mesh_index}"] = (
                        retail_world_frame.normals
                    )
                    arrays[f"retail_tangents_{mesh_index}"] = (
                        retail_world_frame.tangents
                    )
                    arrays[f"retail_tangent_handedness_{mesh_index}"] = (
                        retail_world_frame.tangent_handedness
                    )
                decal_uv = decode_decal_uvs(
                    asset.data,
                    vertex_buffer_offset=mesh.source_offsets[
                        "vertex_buffer"
                    ],
                    vertex_count=mesh.vertex_count,
                    vertex_stride=mesh.vertex_stride,
                    attributes=mesh.attributes,
                )
                if decal_uv is not None:
                    arrays[f"decal_uvs_{mesh_index}"] = decal_uv.values
                if mesh.normals is not None:
                    arrays[f"normals_{mesh_index}"] = numpy.asarray(
                        mesh.normals,
                        dtype=numpy.float32,
                    )
                minimum, maximum = mesh.bounds
                mesh_entries.append(
                    {
                        "index": mesh_index,
                        "name": (
                            _first_parameter(material_group, "Name")
                            or mesh.name
                        ),
                        "retail_material_guid": (
                            f"0x{material_binding['material_guid']:016X}"
                        ),
                        "retail_external_reference_guid": (
                            "0x"
                            f"{material_binding['external_reference_guid']:016X}"
                        ),
                        "retail_material_handle": (
                            f"0x{material_binding['material_handle']:08X}"
                        ),
                        "retail_material_group_index": material_binding[
                            "group_index"
                        ],
                        **material,
                        "vertex_count": mesh.vertex_count,
                        "triangle_count": mesh.triangle_count,
                        "vertex_stride": mesh.vertex_stride,
                        "attributes": [
                            {
                                "semantic": attribute.semantic,
                                "offset": attribute.offset,
                                "data_type": attribute.data_type,
                                "components": attribute.components,
                                "descriptor": attribute.descriptor.hex(),
                            }
                            for attribute in mesh.attributes
                        ],
                        "lightmap_uv": (
                            {
                                "format_code": (
                                    f"0x{lightmap_uv.format_code:08X}"
                                ),
                                "offset": lightmap_uv.offset,
                                "usage_index": lightmap_uv.usage_index,
                                "signed_tangent_handedness": True,
                            }
                            if lightmap_uv is not None
                            else None
                        ),
                        "retail_world_frame": (
                            {
                                "normal_source": (
                                    "secondary_texcoord_zw_and_y_sign"
                                ),
                                "tangent_format_code": (
                                    f"0x{retail_world_frame.tangent_format_code:08X}"
                                ),
                                "tangent_offset": (
                                    retail_world_frame.tangent_offset
                                ),
                                "tangent_handedness_source": (
                                    "secondary_texcoord_x_sign"
                                ),
                            }
                            if retail_world_frame is not None
                            else None
                        ),
                        "decal_uv": (
                            {
                                "format_code": (
                                    f"0x{decal_uv.format_code:08X}"
                                ),
                                "offset": decal_uv.offset,
                                "usage_index": decal_uv.usage_index,
                            }
                            if decal_uv is not None
                            else None
                        ),
                        "source_offsets": dict(mesh.source_offsets),
                        "bounds": {
                            "minimum": minimum.tolist(),
                            "maximum": maximum.tolist(),
                        },
                    }
                )
            # Temporary arrays are read again immediately by the map writer.
            numpy.savez(npz_path, **arrays)
            minimum, maximum = parsed.bounds
            models.append(
                {
                    "asset_id": f"0x{asset_id:016X}",
                    "stream_file": source_file,
                    "source_offset": asset.source_offset,
                    "rx2": str(rx2_path.relative_to(output_root)),
                    "npz": str(npz_path.relative_to(output_root)),
                    "bounds": {
                        "minimum": minimum.tolist(),
                        "maximum": maximum.tolist(),
                    },
                    "vertex_count": parsed.vertex_count,
                    "triangle_count": parsed.triangle_count,
                    "warnings": list(parsed.warnings),
                    "material_binding": {
                        "strategy": (
                            "external_reference_order"
                            if any(
                                binding["strategy"]
                                == "external_reference_order"
                                for binding in material_bindings
                            )
                            else "external_reference_guid"
                        ),
                        "source_group_count": len(source_material_groups),
                        "selected_group_count": len(material_groups),
                        "mesh_count": len(parsed.meshes),
                    },
                    "meshes": mesh_entries,
                }
            )
        else:
            rx2_path = _write_rx2(output_root, "presentation_other", asset)
            other.append(
                {
                    "asset_id": f"0x{asset_id:016X}",
                    "asset_type": f"0x{asset.record.asset_type:08X}",
                    "stream_file": source_file,
                    "source_offset": asset.source_offset,
                    "rx2": str(rx2_path.relative_to(output_root)),
                }
            )

    grind_spline_asset_count = 0
    collision_mesh_asset_count = 0
    collision_mesh_count = 0
    collision_cluster_count = 0
    collision_triangle_count = 0
    for asset in simulation_assets:
        rx2_path = _write_rx2(output_root, "simulation", asset)
        asset_grinds = decode_grind_splines(asset.data)
        asset_collision_meshes = decode_rx2_clustered_meshes(asset.data)
        if collision_consumer is not None:
            collision_consumer(asset_collision_meshes)
        grind_spline_asset_count += bool(asset_grinds)
        collision_mesh_asset_count += bool(asset_collision_meshes)
        collision_mesh_count += len(asset_collision_meshes)
        for rail in asset_grinds:
            grind_splines.append(
                {
                    "asset_id": f"0x{asset.record.asset_id:016X}",
                    "stream_file": asset.source_path.name,
                    **rail,
                }
            )
        collision_entries = []
        for mesh_index, mesh in enumerate(asset_collision_meshes):
            triangle_count = len(mesh.triangles)
            collision_cluster_count += mesh.cluster_count
            collision_triangle_count += triangle_count
            collision_entries.append(
                {
                    "index": mesh_index,
                    "bounds": {
                        "minimum": mesh.bounds_min,
                        "maximum": mesh.bounds_max,
                    },
                    "triangles": triangle_count,
                    "clusters": mesh.cluster_count,
                    "vertices": mesh.vertex_count,
                    "units": mesh.unit_count,
                    "compression_cluster_counts": {
                        str(compression): count
                        for compression, count in mesh.compression_counts
                    },
                }
            )
        simulation.append(
            {
                "asset_id": f"0x{asset.record.asset_id:016X}",
                "asset_type": f"0x{asset.record.asset_type:08X}",
                "stream_file": asset.source_path.name,
                "source_offset": asset.source_offset,
                "size": len(asset.data),
                "rx2": str(rx2_path.relative_to(output_root)),
                "collision_meshes": collision_entries,
            }
        )

    used_texture_ids = {
        mesh["texture_id"]
        for model in models
        for mesh in model["meshes"]  # type: ignore[index]
        if mesh["texture_id"] is not None
    }
    used_texture_ids_by_channel = {
        channel: {
            texture_id
            for model in models
            for mesh in model["meshes"]  # type: ignore[index]
            if (
                texture_id := mesh["retail_texture_ids"].get(channel)  # type: ignore[union-attr]
            )
            is not None
        }
        for channel in RETAIL_TEXTURE_CHANNELS
    }
    manifest["summary"] = {
        "presentation_assets": len(presentation_assets),
        "model_assets": len(models),
        "mesh_parts": sum(len(model["meshes"]) for model in models),  # type: ignore[arg-type,index]
        "excluded_static_mesh_parts": sum(
            len(model["meshes"])  # type: ignore[arg-type,index]
            for model in models
            if str(model["asset_id"]).lower()  # type: ignore[index]
            in {
                asset_id.lower()
                for asset_id in excluded_static_model_asset_ids
            }
        ),
        "vertices": sum(model["vertex_count"] for model in models),  # type: ignore[arg-type]
        "triangles": sum(model["triangle_count"] for model in models),  # type: ignore[arg-type]
        "texture_stream_assets": sum(
            asset.record.asset_type == ASSET_TYPE_TEXTURE
            for asset in presentation_assets + extra_texture_assets
        ),
        "decoded_textures": len(textures),
        "used_diffuse_textures": len(used_texture_ids),
        "bound_texture_references_by_channel": {
            channel: sum(
                channel in mesh["retail_texture_ids"]  # type: ignore[operator]
                for model in models
                for mesh in model["meshes"]  # type: ignore[index]
            )
            for channel in RETAIL_TEXTURE_CHANNELS
        },
        "used_texture_ids_by_channel": {
            channel: len(texture_ids)
            for channel, texture_ids in used_texture_ids_by_channel.items()
        },
        "secondary_uv_mesh_parts": sum(
            mesh["lightmap_uv"] is not None  # type: ignore[index]
            for model in models
            for mesh in model["meshes"]  # type: ignore[index]
        ),
        "secondary_uv_format_counts": {
            format_code: sum(
                mesh["lightmap_uv"] is not None  # type: ignore[index]
                and mesh["lightmap_uv"]["format_code"] == format_code  # type: ignore[index]
                for model in models
                for mesh in model["meshes"]  # type: ignore[index]
            )
            for format_code in sorted(
                {
                    mesh["lightmap_uv"]["format_code"]  # type: ignore[index]
                    for model in models
                    for mesh in model["meshes"]  # type: ignore[index]
                    if mesh["lightmap_uv"] is not None  # type: ignore[index]
                }
            )
        },
        "material_binding_strategies": {
            strategy: sum(
                model["material_binding"]["strategy"] == strategy  # type: ignore[index]
                for model in models
            )
            for strategy in {
                model["material_binding"]["strategy"]  # type: ignore[index]
                for model in models
            }
        },
        "simulation_assets": len(simulation_assets),
        "collision_mesh_assets": collision_mesh_asset_count,
        "collision_meshes": collision_mesh_count,
        "collision_clusters": collision_cluster_count,
        "collision_triangles": collision_triangle_count,
        "grind_spline_assets": grind_spline_asset_count,
        "grind_rails": len(grind_splines),
        "grind_segments": sum(
            rail["segment_count"] for rail in grind_splines  # type: ignore[index]
        ),
        "closed_grind_rails": sum(
            bool(rail["closed"]) for rail in grind_splines  # type: ignore[index]
        ),
        "unmatched_diffuse_textures": sorted(used_texture_ids - set(textures)),
        "unmatched_texture_ids_by_channel": {
            channel: sorted(texture_ids - set(textures))
            for channel, texture_ids in used_texture_ids_by_channel.items()
        },
    }

    manifest_path = output_root / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def main(argv: list[str] | None = None) -> int:
    workspace = _default_workspace()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--stream-dir",
        type=Path,
        default=_default_stream_directory(workspace),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=workspace / "intermediate" / "hawaiian_dream",
    )
    parser.add_argument(
        "--utt-root",
        type=Path,
        default=(
            Path(os.environ["SKATE3_UTT_ROOT"])
            if os.environ.get("SKATE3_UTT_ROOT")
            else None
        ),
        required=not bool(os.environ.get("SKATE3_UTT_ROOT")),
        help=(
            "Path to UTT-1.1.7 (or set the SKATE3_UTT_ROOT environment "
            "variable)"
        ),
    )
    args = parser.parse_args(argv)
    manifest_path = prepare(
        stream_directory=args.stream_dir.resolve(),
        output_root=args.output.resolve(),
        utt_root=args.utt_root.resolve(),
    )
    print(manifest_path)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    print(json.dumps(manifest["summary"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
