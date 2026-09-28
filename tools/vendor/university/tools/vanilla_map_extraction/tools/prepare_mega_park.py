"""Prepare lossless RX2 assets and a Blender-friendly Mega Park cache."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys

import numpy

from skate3_streams import (
    ASSET_TYPE_MODEL,
    ASSET_TYPE_SIMULATION,
    ASSET_TYPE_TEXTURE,
    StreamAsset,
    load_global_stream,
)


TEXTURE_NAME_PATTERN = re.compile(rb"(0x[0-9A-Fa-f]{16})\.Texture")
MATERIAL_TEXTURE_PATTERN = re.compile(r"(0x[0-9A-Fa-f]{16})$")


def _default_workspace() -> Path:
    return Path(__file__).resolve().parents[1]


def _default_stream_directory(workspace: Path) -> Path:
    return (
        workspace
        / "raw"
        / "mega_park"
        / "data"
        / "content"
        / "world"
        / "stream"
        / "DIST_MegaPark"
    )


def _write_rx2(output_root: Path, category: str, asset: StreamAsset) -> Path:
    target = output_root / "rx2" / category / f"{asset.record.asset_id:016X}.rx2"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(asset.data)
    return target


def _texture_id(data: bytes, asset_id: int) -> str:
    matches = TEXTURE_NAME_PATTERN.findall(data)
    if matches:
        return matches[-1].decode("ascii").lower()
    return f"asset_{asset_id:016x}"


def _material_texture_id(material_name: str | None) -> str | None:
    if not material_name:
        return None
    match = MATERIAL_TEXTURE_PATTERN.search(material_name)
    return match.group(1).lower() if match else None


def prepare(
    workspace: Path,
    stream_directory: Path,
    output_root: Path,
    utt_root: Path,
) -> Path:
    sys.path.insert(0, str(utt_root))
    import rx2_parser
    from mdl_parser import parse_rx2 as parse_model

    output_root.mkdir(parents=True, exist_ok=True)
    presentation_assets = load_global_stream(stream_directory, "Pres")
    simulation_assets = load_global_stream(stream_directory, "Sim")

    manifest: dict[str, object] = {
        "format": "skate3-mega-park-cache-v1",
        "source_stream_directory": str(stream_directory),
        "axis_conversion": {
            "runtime_to_blender": ["x", "-z", "y"],
            "uv_v_flipped": True,
        },
        "textures": {},
        "models": [],
        "simulation_assets": [],
        "other_presentation_assets": [],
    }
    textures: dict[str, object] = manifest["textures"]  # type: ignore[assignment]
    models: list[object] = manifest["models"]  # type: ignore[assignment]
    simulation: list[object] = manifest["simulation_assets"]  # type: ignore[assignment]
    other: list[object] = manifest["other_presentation_assets"]  # type: ignore[assignment]

    for asset in presentation_assets:
        asset_id = asset.record.asset_id
        if asset.record.asset_type == ASSET_TYPE_TEXTURE:
            rx2_path = _write_rx2(output_root, "textures", asset)
            texture_id = _texture_id(asset.data, asset_id)
            parsed = rx2_parser.parse_rx2(asset.data)
            if not parsed.textures:
                raise RuntimeError(
                    f"texture asset 0x{asset_id:016X} contains no decoded texture"
                )
            texture = parsed.textures[0]
            png_path = output_root / "textures" / f"{texture_id}.png"
            png_path.parent.mkdir(parents=True, exist_ok=True)
            texture.save_png(png_path)
            textures[texture_id] = {
                "stream_asset_id": f"0x{asset_id:016X}",
                "rx2": str(rx2_path.relative_to(output_root)),
                "png": str(png_path.relative_to(output_root)),
                "width": texture.width,
                "height": texture.height,
                "format": texture.fmt_name,
                "warnings": list(parsed.warnings),
            }
        elif asset.record.asset_type == ASSET_TYPE_MODEL:
            rx2_path = _write_rx2(output_root, "models", asset)
            parsed = parse_model(
                asset.data,
                source_path=rx2_path,
                strict=False,
            )
            npz_path = output_root / "models" / f"{asset_id:016X}.npz"
            npz_path.parent.mkdir(parents=True, exist_ok=True)
            arrays: dict[str, numpy.ndarray] = {}
            mesh_entries: list[dict[str, object]] = []
            for mesh_index, mesh in enumerate(parsed.meshes):
                arrays[f"vertices_{mesh_index}"] = numpy.asarray(
                    mesh.vertices, dtype=numpy.float32
                )
                arrays[f"faces_{mesh_index}"] = numpy.asarray(
                    mesh.faces, dtype=numpy.uint32
                )
                if mesh.uvs is not None:
                    arrays[f"uvs_{mesh_index}"] = numpy.asarray(
                        mesh.uvs, dtype=numpy.float32
                    )
                if mesh.normals is not None:
                    arrays[f"normals_{mesh_index}"] = numpy.asarray(
                        mesh.normals, dtype=numpy.float32
                    )
                minimum, maximum = mesh.bounds
                mesh_entries.append(
                    {
                        "index": mesh_index,
                        "name": mesh.name,
                        "material_name": mesh.material_name,
                        "texture_id": _material_texture_id(mesh.material_name),
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
                        "source_offsets": dict(mesh.source_offsets),
                        "bounds": {
                            "minimum": minimum.tolist(),
                            "maximum": maximum.tolist(),
                        },
                    }
                )
            numpy.savez_compressed(npz_path, **arrays)
            minimum, maximum = parsed.bounds
            models.append(
                {
                    "asset_id": f"0x{asset_id:016X}",
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
                    "meshes": mesh_entries,
                }
            )
        else:
            rx2_path = _write_rx2(output_root, "presentation_other", asset)
            other.append(
                {
                    "asset_id": f"0x{asset_id:016X}",
                    "asset_type": f"0x{asset.record.asset_type:08X}",
                    "source_offset": asset.source_offset,
                    "rx2": str(rx2_path.relative_to(output_root)),
                }
            )

    for asset in simulation_assets:
        rx2_path = _write_rx2(output_root, "simulation", asset)
        simulation.append(
            {
                "asset_id": f"0x{asset.record.asset_id:016X}",
                "asset_type": f"0x{asset.record.asset_type:08X}",
                "source_offset": asset.source_offset,
                "size": len(asset.data),
                "rx2": str(rx2_path.relative_to(output_root)),
            }
        )

    used_texture_ids = {
        mesh["texture_id"]
        for model in models
        for mesh in model["meshes"]  # type: ignore[index]
        if mesh["texture_id"] is not None
    }
    manifest["summary"] = {
        "presentation_assets": len(presentation_assets),
        "model_assets": len(models),
        "mesh_parts": sum(len(model["meshes"]) for model in models),  # type: ignore[arg-type,index]
        "vertices": sum(model["vertex_count"] for model in models),  # type: ignore[arg-type]
        "triangles": sum(model["triangle_count"] for model in models),  # type: ignore[arg-type]
        "texture_assets": len(textures),
        "used_diffuse_textures": len(used_texture_ids),
        "simulation_assets": len(simulation_assets),
        "unmatched_diffuse_textures": sorted(used_texture_ids - set(textures)),
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
        default=workspace / "intermediate" / "mega_park",
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
        workspace=workspace,
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
