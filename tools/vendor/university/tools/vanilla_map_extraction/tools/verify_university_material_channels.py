#!/usr/bin/env python3
"""Verify University normal and lightmap roles through the SKATE package."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import struct
import sys


REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "tools" / "blender_owned_map"))

from analyze_skate import (  # noqa: E402
    VERTEX_BYTES,
    VERTEX_BYTES_V12,
    analyze_package,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("package", type=Path)
    parser.add_argument("--expected", type=Path)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.resolve().read_text(encoding="utf-8"))
    excluded = {
        str(texture_id).lower()
        for texture_id in manifest["normal_texture_policy"][
            "excluded_texture_ids"
        ]
    }
    source_normal_ids: set[str] = set()
    selected_normal_ids: set[str] = set()
    source_normal_references = 0
    selected_mesh_parts = 0
    for model in manifest["models"]:
        for mesh in model["meshes"]:
            normal_id = str(
                mesh.get("retail_texture_ids", {}).get("normal", "")
            ).lower()
            if not normal_id:
                continue
            source_normal_references += 1
            source_normal_ids.add(normal_id)
            if normal_id not in excluded:
                selected_mesh_parts += 1
                selected_normal_ids.add(normal_id)

    analysis = analyze_package(args.package.resolve(), include_payloads=True)
    textures_by_id = {
        int(texture["id"]): texture
        for texture in analysis["texture_dimensions"]
    }
    materials = analysis["_materials"]
    normal_materials = [
        material
        for material in materials
        if int(material["normal_texture"]) != 0
    ]
    package_normal_ids: set[str] = set()
    for material in normal_materials:
        texture_id = int(material["normal_texture"])
        texture = textures_by_id.get(texture_id)
        if texture is None:
            raise RuntimeError(
                f"material {material['name']!r} references missing normal "
                f"texture {texture_id}"
            )
        if int(texture["color_space"]) != 0:
            raise RuntimeError(
                f"normal texture {texture['name']!r} is not linear"
            )
        package_normal_ids.add(str(texture["name"]).lower())

    if package_normal_ids != selected_normal_ids:
        missing = sorted(selected_normal_ids - package_normal_ids)
        extra = sorted(package_normal_ids - selected_normal_ids)
        raise RuntimeError(
            "SKATE normal texture set differs from conservative retail "
            f"selection: missing={missing}, extra={extra}"
        )
    if package_normal_ids & excluded:
        raise RuntimeError("SKATE package binds an excluded special normal map")

    unexpected_roles = {
        role: sum(int(material[role]) != 0 for material in materials)
        for role in (
            "lightmap_texture",
            "orm_texture",
            "emissive_texture",
        )
    }
    if unexpected_roles["orm_texture"] or unexpected_roles[
        "emissive_texture"
    ]:
        raise RuntimeError(
            "University unexpectedly populated an unsupported material "
            f"role: {unexpected_roles}"
        )

    used_material_ids: set[int] = set()
    vertex_bytes = analysis["_vertex_bytes"]
    vertex_stride = (
        VERTEX_BYTES_V12
        if int(analysis["version"]) >= 12
        else VERTEX_BYTES
    )
    for offset in range(0, len(vertex_bytes), vertex_stride):
        used_material_ids.add(
            struct.unpack_from("<I", vertex_bytes, offset + 40)[0]
        )
    unused_normal_materials = [
        str(material["name"])
        for material in normal_materials
        if int(material["id"]) not in used_material_ids
    ]
    if unused_normal_materials:
        raise RuntimeError(
            "SKATE package contains unreferenced normal materials: "
            f"{unused_normal_materials}"
        )

    result = {
        "status": "UNIVERSITY_RETAIL_NORMALS_VERIFIED",
        "source_normal_references": source_normal_references,
        "source_normal_texture_ids": len(source_normal_ids),
        "excluded_special_texture_ids": len(excluded),
        "selected_mesh_parts": selected_mesh_parts,
        "package_normal_texture_ids": len(package_normal_ids),
        "package_normal_materials": len(normal_materials),
        "unexpected_role_references": unexpected_roles,
    }
    if args.expected is not None:
        expected = json.loads(
            args.expected.resolve().read_text(encoding="utf-8")
        )["retail_normals"]
        for name, value in result.items():
            if name == "status":
                continue
            if expected[name] != value:
                raise RuntimeError(
                    f"retail normal contract {name} changed: "
                    f"{value!r} != {expected[name]!r}"
                )
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
