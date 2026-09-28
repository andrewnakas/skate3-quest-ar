"""Restore retail CAC UVs, normals, texture maps, and material policy in Blender."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

import bpy
from mathutils import Vector


TYPE_RAW_BUFFER = 0x00010031
TYPE_VTX_DESC = 0x000200E9
TYPE_VB_DESC = 0x000200EA
TYPE_VTX_DECL = 0x00020081
TYPE_MESH_DESC = 0x00EB0023
TYPE_MORPH_DESC = 0x00EB000E
MORPH_VERTEX_STRIDE = 16
MORPH_POSITION_SCALE = 1.0 / 16384.0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rx2-dir", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--material-dir", required=True)
    parser.add_argument("--parser-dir", required=True)
    parser.add_argument("--report", required=True)
    return parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])


def load_image(path: Path, *, color: bool) -> bpy.types.Image:
    image = bpy.data.images.load(str(path), check_existing=True)
    image.colorspace_settings.name = "sRGB" if color else "Non-Color"
    image.pack()
    return image


def principled_input(node: bpy.types.Node, *names: str):
    for name in names:
        socket = node.inputs.get(name)
        if socket is not None:
            return socket
    raise RuntimeError(
        f"Principled BSDF {node.name} is missing inputs {', '.join(names)}"
    )


def create_material(
    component: dict, material_dir: Path
) -> bpy.types.Material:
    slot = component["slot"]
    material = bpy.data.materials.new(
        f"Retail_{slot}_{component['material_id']}"
    )
    material.use_nodes = True
    material.diffuse_color = (*component["tint"], 1.0)
    material.use_backface_culling = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    nodes.clear()

    output = nodes.new("ShaderNodeOutputMaterial")
    shader = nodes.new("ShaderNodeBsdfPrincipled")
    links.new(shader.outputs["BSDF"], output.inputs["Surface"])

    base_path = material_dir / f"{slot}_base_color.png"
    base_texture = nodes.new("ShaderNodeTexImage")
    base_texture.name = f"{slot}_BaseColor"
    base_texture.image = load_image(base_path, color=True)
    # Use Blender's glTF-recognized RGBA Mix node so the retail linear tint
    # is exported as pbrMetallicRoughness.baseColorFactor. The legacy
    # ShaderNodeMixRGB looks correct in Blender but is ignored by glTF.
    tint = nodes.new("ShaderNodeMix")
    tint.data_type = "RGBA"
    tint.blend_type = "MULTIPLY"
    tint.inputs[0].default_value = 1.0
    tint.inputs[7].default_value = (*component["tint"], 1.0)
    links.new(base_texture.outputs["Color"], tint.inputs[6])
    links.new(tint.outputs[2], principled_input(shader, "Base Color"))

    if component.get("alpha_mode") == "MASK":
        cutoff = float(component.get("alpha_cutoff", 0.5))
        # Blender 5's glTF exporter derives alphaMode from the shader graph.
        # A direct texture-alpha link exports BLEND even when Eevee previews a
        # dithered surface. The explicit comparison is recognized as MASK and
        # preserves the retail hard-cutout policy in Bevy.
        alpha_clip = nodes.new("ShaderNodeMath")
        alpha_clip.name = f"{slot}_AlphaCutoff"
        alpha_clip.operation = "GREATER_THAN"
        alpha_clip.inputs[1].default_value = cutoff
        links.new(base_texture.outputs["Alpha"], alpha_clip.inputs[0])
        links.new(alpha_clip.outputs[0], principled_input(shader, "Alpha"))
        material.surface_render_method = "DITHERED"
        material["skate3_alpha_mode"] = "MASK"
        material["skate3_alpha_cutoff"] = cutoff
    else:
        material["skate3_alpha_mode"] = "OPAQUE"

    normal_id = component["textures"].get("normal")
    if normal_id:
        normal_path = material_dir / f"{slot}_normal.png"
        if not normal_path.is_file():
            raise RuntimeError(
                f"{slot} reconstructed DXT5nm texture is missing: {normal_path}"
            )
        normal_texture = nodes.new("ShaderNodeTexImage")
        normal_texture.name = f"{slot}_Normal"
        normal_texture.image = load_image(normal_path, color=False)
        normal = nodes.new("ShaderNodeNormalMap")
        normal.space = "TANGENT"
        normal.inputs["Strength"].default_value = 1.0
        links.new(normal_texture.outputs["Color"], normal.inputs["Color"])
        links.new(normal.outputs["Normal"], principled_input(shader, "Normal"))

    roughness_path = material_dir / f"{slot}_roughness.png"
    if roughness_path.is_file():
        roughness_texture = nodes.new("ShaderNodeTexImage")
        roughness_texture.name = f"{slot}_Roughness"
        roughness_texture.image = load_image(roughness_path, color=False)
        links.new(
            roughness_texture.outputs["Color"],
            principled_input(shader, "Roughness"),
        )
    else:
        principled_input(shader, "Roughness").default_value = (
            0.48 if slot in {"SkateTruck", "SkateWheel"} else 0.72
        )
    principled_input(shader, "Metallic").default_value = (
        0.65 if slot == "SkateTruck" else 0.0
    )

    material["skate3_slot"] = slot
    material["skate3_asset_id"] = component["asset_id"]
    material["skate3_model_id"] = component["model_id"]
    material["skate3_material_id"] = component["material_id"]
    material["skate3_texture_ids"] = json.dumps(
        component["textures"], sort_keys=True
    )
    return material


def max_position_error(
    obj: bpy.types.Object, positions: list[tuple[float, float, float]], convert
) -> float:
    return max(
        (
            Vector(obj.data.vertices[index].co) - Vector(convert(position))
        ).length
        for index, position in enumerate(positions)
    )


def be_u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]


def morph_weight(name: str, manifest: dict) -> float:
    body = manifest["preset"]["body_mods"]
    if name == "fat" or name.startswith("fat_"):
        return float(body["fatness"])
    if name == "thin" or name.startswith("thin_"):
        return float(body["skinniness"])
    if name in manifest["morph_assembly"]["face_targets"]:
        return float(body["face_fields"])
    raise RuntimeError(f"Retail morph target has no recipe mapping: {name}")


def decode_dense_morphs(
    path: Path, parsed: dict, vertex_count: int, rx2_skeleton
) -> list[dict]:
    data = path.read_bytes()
    sections = parsed["sections"]
    mesh_descs = [
        section
        for section in sections
        if section["type_code"] == TYPE_MESH_DESC
    ]
    if len(mesh_descs) != 1:
        raise RuntimeError(
            f"{path.name} expected one mesh descriptor, found {len(mesh_descs)}"
        )
    base = mesh_descs[0]["file_offset"]
    morph_count = be_u32(data, base + 0x50)
    morph_table = be_u32(data, base + 0x54)
    morphs = []
    for morph_index in range(morph_count):
        entry = base + morph_table + morph_index * 16
        target_hash, descriptor_index, name_offset = struct.unpack_from(
            ">QII", data, entry
        )
        name_start = base + name_offset
        name_end = data.find(b"\0", name_start)
        if name_end < 0:
            raise RuntimeError(
                f"{path.name} morph {morph_index} has no terminated name"
            )
        name = data[name_start:name_end].decode("ascii")
        if descriptor_index < 4 or descriptor_index >= len(sections):
            raise RuntimeError(
                f"{path.name} morph {name} has invalid descriptor index "
                f"{descriptor_index}"
            )
        expected_types = (
            TYPE_RAW_BUFFER,
            TYPE_VB_DESC,
            TYPE_VTX_DECL,
            TYPE_VTX_DESC,
            TYPE_MORPH_DESC,
        )
        stream_sections = sections[descriptor_index - 4 : descriptor_index + 1]
        actual_types = tuple(
            section["type_code"] for section in stream_sections
        )
        if actual_types != expected_types:
            raise RuntimeError(
                f"{path.name} morph {name} stream layout changed: "
                f"{actual_types} != {expected_types}"
            )
        raw, vb_desc, _vtx_decl, vtx_desc, _morph_desc = stream_sections
        byte_size = be_u32(data, vb_desc["file_offset"] + 0x20)
        if byte_size != vertex_count * MORPH_VERTEX_STRIDE:
            raise RuntimeError(
                f"{path.name} morph {name} byte size changed: "
                f"{byte_size} != {vertex_count * MORPH_VERTEX_STRIDE}"
            )
        if raw["size"] != byte_size:
            raise RuntimeError(
                f"{path.name} morph {name} raw-buffer size changed"
            )
        descriptor = rx2_skeleton._parse_vertex_descriptor(
            data, vtx_desc["file_offset"], vtx_desc["size"]
        )
        expected_elements = [
            ("POSITION", 0, "SHORT4", 0),
            ("NORMAL", 0, "PACKED11_11_10N", 8),
            ("BLENDWEIGHT", 0, "UNKNOWN_0x002C83A4", 12),
        ]
        actual_elements = [
            (
                element["usage_name"],
                element["usage_index"],
                element["format_name"],
                element["offset"],
            )
            for element in descriptor["elements"]
        ]
        if (
            descriptor["stride"] != MORPH_VERTEX_STRIDE
            or actual_elements != expected_elements
        ):
            raise RuntimeError(
                f"{path.name} morph {name} vertex declaration changed"
            )

        deltas = []
        nonzero_vertices = 0
        nonzero_normal_deltas = 0
        max_delta = 0.0
        for vertex_index in range(vertex_count):
            vertex_offset = (
                raw["file_offset"] + vertex_index * MORPH_VERTEX_STRIDE
            )
            x, y, z, _remap = struct.unpack_from(
                ">4h", data, vertex_offset
            )
            if be_u32(data, vertex_offset + 8) != 0:
                nonzero_normal_deltas += 1
            if be_u32(data, vertex_offset + 12) != 0x3F800000:
                raise RuntimeError(
                    f"{path.name} morph {name} has an unhandled vertex factor"
                )
            delta = (
                x * MORPH_POSITION_SCALE,
                y * MORPH_POSITION_SCALE,
                z * MORPH_POSITION_SCALE,
            )
            length = math.sqrt(sum(value * value for value in delta))
            if length > 0.0:
                nonzero_vertices += 1
                max_delta = max(max_delta, length)
            deltas.append(delta)
        morphs.append(
            {
                "name": name,
                "hash": f"{target_hash:016x}",
                "descriptor_index": descriptor_index,
                "deltas": deltas,
                "nonzero_vertices": nonzero_vertices,
                "nonzero_normal_deltas": nonzero_normal_deltas,
                "max_source_delta": max_delta,
            }
        )
    return morphs


def apply_retail_morphs(
    obj: bpy.types.Object,
    source_positions: list[tuple[float, float, float]],
    morphs: list[dict],
    manifest: dict,
    convert,
) -> dict:
    combined = [[0.0, 0.0, 0.0] for _ in source_positions]
    weights = {}
    for morph in morphs:
        weight = morph_weight(morph["name"], manifest)
        weights[morph["name"]] = weight
        if weight == 0.0:
            continue
        if morph["nonzero_normal_deltas"] != 0:
            raise RuntimeError(
                f"Active retail morph {morph['name']} has "
                f"{morph['nonzero_normal_deltas']} unhandled normal deltas"
            )
        for vertex_index, delta in enumerate(morph["deltas"]):
            for axis in range(3):
                combined[vertex_index][axis] += weight * delta[axis]

    morphed_positions = []
    moved_vertices = 0
    max_delta = 0.0
    digest = hashlib.sha256()
    for vertex_index, (position, delta) in enumerate(
        zip(source_positions, combined)
    ):
        morphed = tuple(
            float(position[axis] + delta[axis]) for axis in range(3)
        )
        morphed_positions.append(morphed)
        source_delta_length = math.sqrt(sum(value * value for value in delta))
        if source_delta_length > 0.0:
            moved_vertices += 1
            max_delta = max(max_delta, source_delta_length)
            obj.data.vertices[vertex_index].co += convert(delta)
        digest.update(struct.pack(">3f", *morphed))

    return {
        "targets": [
            {
                "name": morph["name"],
                "hash": morph["hash"],
                "descriptor_index": morph["descriptor_index"],
                "weight": weights[morph["name"]],
                "nonzero_vertices": morph["nonzero_vertices"],
                "nonzero_normal_deltas": morph[
                    "nonzero_normal_deltas"
                ],
                "max_source_delta": morph["max_source_delta"],
            }
            for morph in morphs
        ],
        "moved_vertices": moved_vertices,
        "maximum_applied_source_delta": max_delta,
        "first_morphed_source_position": list(morphed_positions[0]),
        "morphed_source_positions_sha256": digest.hexdigest().upper(),
    }


def main() -> None:
    args = parse_args()
    rx2_dir = Path(args.rx2_dir).resolve()
    manifest_path = Path(args.manifest).resolve()
    material_dir = Path(args.material_dir).resolve()
    parser_dir = Path(args.parser_dir).resolve()
    report_path = Path(args.report).resolve()
    sys.path.insert(0, str(parser_dir))
    import blender_rx2_abin_export as exporter
    import rx2_skeleton

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    components = {item["slot"]: item for item in manifest["components"]}
    objects = {
        obj.name.split("_", 1)[0]: obj
        for obj in bpy.data.objects
        if obj.type == "MESH"
    }
    expected_slots = set(components)
    missing = sorted(expected_slots - set(objects))
    extra = sorted(set(objects) - expected_slots)
    if missing or extra:
        raise RuntimeError(
            f"Default-skater mesh set mismatch: missing={missing} extra={extra}"
        )

    reports = []
    total_vertices = 0
    total_triangles = 0
    for slot, component in components.items():
        path = rx2_dir / slot / f"0x{component['model_id']}.rx2"
        parsed = rx2_skeleton.parse_rx2(str(path))
        errors = rx2_skeleton.validate(parsed)
        tolerated = [
            error
            for error in errors
            if "no canonical parent known" not in error
        ]
        if tolerated:
            raise RuntimeError(f"{slot} RX2 validation failed: {tolerated}")
        meshes = [
            mesh
            for mesh in parsed["meshes"]
            if mesh.get("positions") and mesh.get("indices")
        ]
        if len(meshes) != 1:
            raise RuntimeError(
                f"{slot} expected one renderable mesh, found {len(meshes)}"
            )
        source = meshes[0]
        obj = objects[slot]
        positions = source["positions"]
        if len(obj.data.vertices) != len(positions):
            raise RuntimeError(
                f"{slot} vertex count changed: "
                f"{len(obj.data.vertices)} != {len(positions)}"
            )
        position_error = max_position_error(
            obj, positions, exporter.point_source_to_blender
        )
        if position_error > 1.0e-6:
            raise RuntimeError(
                f"{slot} vertex-order mismatch: {position_error:.9g} m"
            )
        morphs = decode_dense_morphs(
            path, parsed, len(positions), rx2_skeleton
        )
        expected_morphs = manifest["morph_assembly"]["expected_targets"][slot]
        actual_morphs = [morph["name"] for morph in morphs]
        if actual_morphs != expected_morphs:
            raise RuntimeError(
                f"{slot} morph target order changed: "
                f"{actual_morphs} != {expected_morphs}"
            )
        morph_report = apply_retail_morphs(
            obj,
            positions,
            morphs,
            manifest,
            exporter.point_source_to_blender,
        )

        uvs = source.get("uvs")
        if not uvs or len(uvs) != len(positions):
            raise RuntimeError(f"{slot} has no complete retail UV set")
        while obj.data.uv_layers:
            obj.data.uv_layers.remove(obj.data.uv_layers[0])
        uv_layer = obj.data.uv_layers.new(name="TEXCOORD_0")
        for loop in obj.data.loops:
            uv = uvs[loop.vertex_index]
            # RX2 uses the Direct3D/top-left texture convention. Blender UVs
            # are bottom-left and its glTF exporter flips them back on export.
            uv_layer.data[loop.index].uv = (
                float(uv[0]),
                1.0 - float(uv[1]),
            )

        normals = source.get("normals")
        if not normals or len(normals) != len(positions):
            raise RuntimeError(f"{slot} has no complete retail normal set")
        converted_normals = [
            Vector((normal[0], -normal[2], normal[1])).normalized()
            for normal in normals
        ]
        if hasattr(obj.data, "normals_split_custom_set_from_vertices"):
            obj.data.normals_split_custom_set_from_vertices(converted_normals)
        elif hasattr(obj.data, "normals_split_custom_set"):
            obj.data.normals_split_custom_set(
                [converted_normals[loop.vertex_index] for loop in obj.data.loops]
            )
        else:
            raise RuntimeError("This Blender build cannot store custom normals")

        obj.data.materials.clear()
        obj.data.materials.append(create_material(component, material_dir))
        obj["skate3_retail_default_component"] = True
        obj["skate3_lod"] = 0
        obj["skate3_source_model_id"] = component["model_id"]
        obj["skate3_native_rx2_bones"] = len(parsed["bones"])

        influence_groups = {
            group.name
            for group in obj.vertex_groups
            if any(
                group.index == assignment.group and assignment.weight > 1.0e-6
                for vertex in obj.data.vertices
                for assignment in vertex.groups
            )
        }
        if not influence_groups:
            raise RuntimeError(f"{slot} has no bound vertex groups")
        if any(
            not math.isfinite(value)
            for vertex in obj.data.vertices
            for value in vertex.co
        ):
            raise RuntimeError(f"{slot} contains a non-finite vertex")

        triangles = len(source["indices"]) // 3
        total_vertices += len(positions)
        total_triangles += triangles
        reports.append(
            {
                "slot": slot,
                "vertices": len(positions),
                "triangles": triangles,
                "native_bones": len(parsed["bones"]),
                "runtime_vertex_groups": len(influence_groups),
                "position_error": position_error,
                "morphs": morph_report,
                "uvs": len(uvs),
                "first_rx2_uv": [float(uvs[0][0]), float(uvs[0][1])],
                "normals": len(normals),
                "tangents": len(source.get("tangents") or []),
                "degenerate_tangent_frames": source.get(
                    "degenerate_tangent_frames", 0
                ),
                "material": obj.data.materials[0].name,
            }
        )

    if total_vertices != manifest["rig_evidence"]["selected_vertices"]:
        raise RuntimeError(
            f"Vertex total changed: {total_vertices} != "
            f"{manifest['rig_evidence']['selected_vertices']}"
        )
    if total_triangles != manifest["rig_evidence"]["selected_triangles"]:
        raise RuntimeError(
            f"Triangle total changed: {total_triangles} != "
            f"{manifest['rig_evidence']['selected_triangles']}"
        )

    bpy.context.scene["skate3_default_preset"] = manifest["preset"]["name"]
    bpy.context.scene["skate3_default_recipe_sha256"] = manifest["preset"][
        "recipe_sha256"
    ]
    bpy.context.scene["skate3_retail_modular_parts"] = len(reports)
    bpy.context.scene["skate3_retail_vertices"] = total_vertices
    bpy.context.scene["skate3_retail_triangles"] = total_triangles
    bpy.context.scene["skate3_retail_uvs_restored"] = True
    bpy.context.scene["skate3_retail_normals_restored"] = True
    bpy.context.scene["skate3_generated_weights"] = False
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)

    report = {
        "schema": 1,
        "normal_encoding": "dxt5nm-ag-to-gltf-rgb-v1",
        "uv_encoding": "rx2-top-left-to-blender-bottom-left-v1",
        "tint_encoding": "gltf-linear-base-color-factor-v1",
        "alpha_encoding": "gltf-mask-retail-alpha-v1",
        "morph_encoding": manifest["morph_assembly"]["encoding"],
        "preset": manifest["preset"]["name"],
        "recipe_sha256": manifest["preset"]["recipe_sha256"],
        "parts": reports,
        "vertices": total_vertices,
        "triangles": total_triangles,
        "generated_weights": False,
        "runtime_bind_policy": manifest["rig_evidence"]["runtime_policy"],
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        "DEFAULT_SKATER_MATERIALS_OK "
        f"parts={len(reports)} vertices={total_vertices} "
        f"triangles={total_triangles}"
    )


main()
