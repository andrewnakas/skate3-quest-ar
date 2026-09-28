#!/usr/bin/env python3
"""Verify complete retail material identity through a University SKATE export."""

from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import sys


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import analyze_package  # noqa: E402


def _integer(value: object) -> int:
    text = str(value or "").strip()
    return int(text, 0) if text else 0


def _family(shader_name: str) -> int:
    shader = shader_name.lower()
    prefixes = (
        ("environment.reflective_simple", 6),
        ("environment.reflective_trans", 13),
        ("environment.reflective", 5),
        ("environment.decal_tileable", 4),
        ("environment.decal", 3),
        ("environment.default", 1),
        ("environmentsimple.alphatest", 7),
        ("environmentsimple.diffuse", 8),
        ("environmentsimple.default", 2),
        ("tree.default", 9),
        ("animated.tree", 10),
        ("proxyworld.", 11),
        ("incandescent.backlituvscroll", 14),
        ("incandescent.default", 12),
        ("water.flowing", 30),
        ("ocean.default", 31),
        ("ocean.reflection", 32),
        ("sky.", 40),
    )
    return next(
        (family for prefix, family in prefixes if shader.startswith(prefix)),
        0,
    )


def _flags(shader_name: str, alpha_mode: int) -> int:
    shader = shader_name.lower()
    flags = 1 if alpha_mode == 1 else (2 if alpha_mode == 2 else 0)
    if shader.startswith(("tree.", "animated.tree")) or "alphatest" in shader:
        flags |= 1 | 4
    if shader.startswith("sky."):
        flags |= 8
    if shader.startswith("environment.decal"):
        flags |= 16
    if shader.startswith("environment.decal_tileable"):
        flags |= 32
    if shader.startswith(("water.", "ocean.")):
        flags |= 64
    return flags


def _source_metadata(
    model: dict[str, object],
    mesh: dict[str, object],
) -> dict[str, object]:
    return {
        "asset_id": model["asset_id"],
        "stream_file": model["stream_file"],
        "mesh_index": mesh["index"],
        "vertex_stride": mesh["vertex_stride"],
        "attributes": mesh.get("attributes", []),
        "source_offsets": mesh["source_offsets"],
        "bounds": mesh.get("bounds", {}),
        "vertex_count": mesh["vertex_count"],
        "triangle_count": mesh["triangle_count"],
        "retail_world_frame": mesh.get("retail_world_frame"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    package = analyze_package(args.package, include_payloads=True)
    if package["version"] not in (12, 13):
        raise RuntimeError(
            f"expected SKATE v12 or v13, found v{package['version']}"
        )

    textures = {
        int(entry["id"]): str(entry["name"])
        for entry in package["texture_dimensions"]
    }
    expected: dict[tuple[str, int], tuple[dict[str, object], dict[str, object]]] = {}
    for model in manifest["models"]:
        for mesh in model["meshes"]:
            key = (str(model["asset_id"]), int(mesh["index"]))
            if key in expected:
                raise RuntimeError(f"duplicate manifest material identity {key}")
            expected[key] = (model, mesh)

    actual: dict[tuple[str, int], dict[str, object]] = {}
    disabled = 0
    for material in package["_materials"]:
        if not material.get("retail_enabled", False):
            disabled += 1
            continue
        source_text = str(material.get("retail_source_metadata", ""))
        try:
            source = json.loads(source_text)
        except json.JSONDecodeError as exc:
            raise RuntimeError(
                f"material {material['name']!r} has invalid source metadata"
            ) from exc
        key = (str(source.get("asset_id", "")), int(source.get("mesh_index", -1)))
        if key in actual:
            raise RuntimeError(f"duplicate package material identity {key}")
        material["_decoded_source"] = source
        actual[key] = material

    missing = sorted(set(expected) - set(actual))
    extra = sorted(set(actual) - set(expected))
    if missing or extra:
        raise RuntimeError(
            "retail material identity set changed: "
            f"missing={missing[:5]} extra={extra[:5]}"
        )

    shader_counts: Counter[str] = Counter()
    role_counts: Counter[str] = Counter()
    parameter_counts: Counter[str] = Counter()
    for key, (model, mesh) in expected.items():
        material = actual[key]
        shader = str(mesh.get("shader_name") or "")
        shader_counts[shader] += 1
        checks = {
            "retail_material_guid": _integer(
                mesh.get("retail_material_guid")
            ),
            "retail_material_handle": _integer(
                mesh.get("retail_material_handle")
            ),
            "retail_material_group_index": int(
                mesh.get("retail_material_group_index", -1)
            ),
            "retail_shader_name": shader,
            "retail_shader_family": _family(shader),
            "retail_render_flags": _flags(
                shader, int(mesh.get("alpha_mode", 0))
            ),
        }
        for field, expected_value in checks.items():
            if material.get(field) != expected_value:
                raise RuntimeError(
                    f"{key} changed {field}: "
                    f"{material.get(field)!r} != {expected_value!r}"
                )

        expected_parameters = {
            str(name): [str(value) for value in values]
            for name, values in mesh.get("retail_parameters", {}).items()
        }
        if material.get("retail_parameters") != expected_parameters:
            raise RuntimeError(f"{key} changed retail parameter values")
        parameter_counts.update(expected_parameters.keys())

        expected_textures = {
            str(role): str(texture_id).lower()
            for role, texture_id in mesh.get(
                "retail_texture_ids", {}
            ).items()
        }
        bindings = {}
        for binding in material.get("retail_texture_bindings", []):
            role = str(binding["semantic"])
            texture_name = textures.get(int(binding["texture"]))
            if texture_name is None:
                raise RuntimeError(f"{key} binding {role!r} has no texture")
            bindings[role] = texture_name
            expected_uv = 1 if role in {"lightmap", "alpha"} else (
                2 if role == "decal" else 0
            )
            tileable = shader.lower().startswith(
                "environment.decal_tileable"
            )
            expected_address = 1 if role == "lightmap" or (
                role == "decal" and not tileable
            ) else 0
            if (
                int(binding["uv_set"]) != expected_uv
                or int(binding["address_u"]) != expected_address
                or int(binding["address_v"]) != expected_address
            ):
                raise RuntimeError(f"{key} changed {role!r} sampler binding")
        if bindings != expected_textures:
            raise RuntimeError(
                f"{key} changed texture roles: "
                f"{bindings!r} != {expected_textures!r}"
            )
        role_counts.update(expected_textures.keys())

        if material["_decoded_source"] != _source_metadata(model, mesh):
            raise RuntimeError(f"{key} changed source provenance")

    summary = {
        "status": "UNIVERSITY_RETAIL_MATERIALS_EXACT",
        "retail_materials": len(actual),
        "non_retail_collision_materials": disabled,
        "shader_families": dict(sorted(shader_counts.items())),
        "texture_role_references": dict(sorted(role_counts.items())),
        "parameter_name_references": dict(sorted(parameter_counts.items())),
        "world_metadata_extension": "WMET" in package["extension_tags"],
    }
    if not summary["world_metadata_extension"]:
        raise RuntimeError("package lost the WMET retail world manifest")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
