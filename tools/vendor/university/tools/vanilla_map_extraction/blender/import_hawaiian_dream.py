"""Build and save Danny Way's Hawaiian Dream as a textured Blender scene."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import struct
import sys

import bpy
import numpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from retail_collision_mesh import decode_rx2_clustered_meshes
from retail_grind_splines import (
    classify_grind_coordinate_frame,
    grind_cell_translation,
    translate_native_segment_payload,
)


RETAIL_NORMAL_ATTRIBUTE = "skate3_retail_normal"
RETAIL_TANGENT_ATTRIBUTE = "skate3_retail_tangent"
RETAIL_HANDEDNESS_ATTRIBUTE = "skate3_retail_tangent_handedness"


def _safe_name(value: str, fallback: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("_")
    return (cleaned or fallback)[:120]


def _runtime_to_blender(values: numpy.ndarray) -> numpy.ndarray:
    converted = values[:, [0, 2, 1]].copy()
    converted[:, 1] *= -1.0
    return converted


def _set_point_vector_attribute(
    mesh: bpy.types.Mesh,
    name: str,
    values: numpy.ndarray,
) -> None:
    if values.shape != (len(mesh.vertices), 3):
        raise ValueError(
            f"{mesh.name} attribute {name!r} has shape {values.shape}, "
            f"expected ({len(mesh.vertices)}, 3)"
        )
    attribute = mesh.attributes.new(
        name=name,
        type="FLOAT_VECTOR",
        domain="POINT",
    )
    attribute.data.foreach_set(
        "vector",
        numpy.ascontiguousarray(values, dtype=numpy.float32).ravel(),
    )


def _set_point_float_attribute(
    mesh: bpy.types.Mesh,
    name: str,
    values: numpy.ndarray,
) -> None:
    if values.shape != (len(mesh.vertices),):
        raise ValueError(
            f"{mesh.name} attribute {name!r} has shape {values.shape}, "
            f"expected ({len(mesh.vertices)},)"
        )
    attribute = mesh.attributes.new(
        name=name,
        type="FLOAT",
        domain="POINT",
    )
    attribute.data.foreach_set(
        "value",
        numpy.ascontiguousarray(values, dtype=numpy.float32),
    )


def _clear_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)


def _new_child_collection(
    parent: bpy.types.Collection | bpy.types.Scene,
    name: str,
) -> bpy.types.Collection:
    collection = bpy.data.collections.new(name)
    if isinstance(parent, bpy.types.Scene):
        parent.collection.children.link(collection)
    else:
        parent.children.link(collection)
    return collection


def _load_images(
    manifest: dict[str, object],
    cache_root: Path,
) -> dict[str, bpy.types.Image]:
    images: dict[str, bpy.types.Image] = {}
    for texture_id, entry in manifest["textures"].items():
        image_path = cache_root / entry["png"]
        # Retail texture IDs are binding identities, even when two IDs resolve
        # to the same decoded PNG. Reusing a Blender image here allows a later
        # alias to rename the shared datablock and makes the earlier ID
        # impossible for the exporter to resolve.
        image = bpy.data.images.load(str(image_path), check_existing=False)
        image.name = texture_id
        image["skate3_retail_texture_id"] = texture_id
        # Most secondary retail roles are not connected to Blender preview
        # nodes. Keep them in the saved .blend so the SKATE exporter can bind
        # every original channel rather than losing zero-user datablocks.
        image.use_fake_user = True
        images[texture_id] = image
    return images


def _new_material(
    texture_id: str,
    image: bpy.types.Image | None,
    alpha_mode: int,
    *,
    lightmap_texture_id: str = "",
    lightmap_image: bpy.types.Image | None = None,
    normal_texture_id: str = "",
    normal_image: bpy.types.Image | None = None,
    shader_name: str = "",
    retail_material_guid: str = "",
    retail_material_handle: str = "",
    retail_material_group_index: int = -1,
    retail_texture_ids: dict[str, str] | None = None,
    retail_parameters: dict[str, list[str]] | None = None,
    retail_source: dict[str, object] | None = None,
    fallback: bool = False,
) -> bpy.types.Material:
    alpha_suffix = ("OPAQUE", "MASK", "BLEND")[alpha_mode]
    fallback_suffix = "_FALLBACK" if fallback else ""
    identity = "|".join(
        (
            texture_id,
            lightmap_texture_id,
            normal_texture_id,
            str(alpha_mode),
        )
    )
    identity_digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:12]
    material = bpy.data.materials.new(
        f"MAT_{texture_id}_{identity_digest}_"
        f"{alpha_suffix}{fallback_suffix}"
    )
    material.use_nodes = True
    material.use_backface_culling = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    principled = nodes.get("Principled BSDF")
    base_color_output = None
    if image is not None:
        texture = nodes.new("ShaderNodeTexImage")
        texture.name = f"TEX_{texture_id}"
        texture.label = texture_id
        texture.image = image
        texture.interpolation = "Linear"
        base_color_output = texture.outputs["Color"]
        links.new(texture.outputs["Alpha"], principled.inputs["Alpha"])
    else:
        principled.inputs["Base Color"].default_value = (
            1.0,
            1.0,
            1.0,
            1.0,
        )
    if lightmap_texture_id:
        if lightmap_image is None:
            raise ValueError(
                f"lightmap texture {lightmap_texture_id} is missing "
                "from the cache"
            )
        lightmap_image.colorspace_settings.name = "Non-Color"
        lightmap_image["ow_lightmap_encoding"] = (
            "skate3_retail_sqrt_linear_over_4"
        )
        lightmap_texture = nodes.new("ShaderNodeTexImage")
        lightmap_texture.name = f"LIGHTMAP_{lightmap_texture_id}"
        lightmap_texture.label = lightmap_texture_id
        lightmap_texture.image = lightmap_image
        lightmap_texture.interpolation = "Linear"
        lightmap_texture.extension = "EXTEND"
        lightmap_uv = nodes.new("ShaderNodeUVMap")
        lightmap_uv.name = "RETAIL_LIGHTMAP_UV"
        lightmap_uv.uv_map = "Lightmap"
        links.new(lightmap_uv.outputs["UV"], lightmap_texture.inputs["Vector"])
        lightmap_square = nodes.new("ShaderNodeVectorMath")
        lightmap_square.name = "RETAIL_LIGHTMAP_SQUARE"
        lightmap_square.operation = "MULTIPLY"
        links.new(
            lightmap_texture.outputs["Color"],
            lightmap_square.inputs[0],
        )
        links.new(
            lightmap_texture.outputs["Color"],
            lightmap_square.inputs[1],
        )
        lightmap_scale = nodes.new("ShaderNodeVectorMath")
        lightmap_scale.name = "RETAIL_LIGHTMAP_SCALE"
        lightmap_scale.operation = "SCALE"
        lightmap_scale.inputs["Scale"].default_value = 4.0
        links.new(lightmap_square.outputs["Vector"], lightmap_scale.inputs[0])
        if base_color_output is not None:
            baked_color = nodes.new("ShaderNodeVectorMath")
            baked_color.name = "RETAIL_BAKED_COLOR"
            baked_color.operation = "MULTIPLY"
            links.new(base_color_output, baked_color.inputs[0])
            links.new(lightmap_scale.outputs["Vector"], baked_color.inputs[1])
            base_color_output = baked_color.outputs["Vector"]
    if base_color_output is not None:
        links.new(base_color_output, principled.inputs["Base Color"])
    if normal_texture_id:
        if normal_image is None:
            raise ValueError(
                f"normal texture {normal_texture_id} is missing from the cache"
            )
        normal_image.colorspace_settings.name = "Non-Color"
        normal_texture = nodes.new("ShaderNodeTexImage")
        normal_texture.name = f"NORMAL_{normal_texture_id}"
        normal_texture.label = normal_texture_id
        normal_texture.image = normal_image
        normal_texture.interpolation = "Linear"
        normal_map = nodes.new("ShaderNodeNormalMap")
        normal_map.name = f"NORMAL_MAP_{normal_texture_id}"
        links.new(normal_texture.outputs["Color"], normal_map.inputs["Color"])
        links.new(normal_map.outputs["Normal"], principled.inputs["Normal"])
    principled.inputs["Roughness"].default_value = 0.68
    principled.inputs["Metallic"].default_value = 0.0
    if alpha_mode != 0:
        try:
            material.surface_render_method = "DITHERED"
        except Exception:
            try:
                material.blend_method = "HASHED"
            except Exception:
                pass
    material["skate3_texture_id"] = texture_id
    material["skate3_lightmap_texture_id"] = lightmap_texture_id
    material["skate3_normal_texture_id"] = normal_texture_id
    material["skate3_alpha_mode"] = alpha_mode
    material["skate3_alpha_cutoff"] = 0.5
    material["skate3_shader_name"] = shader_name
    material["skate3_retail_material_guid"] = retail_material_guid
    material["skate3_retail_material_handle"] = retail_material_handle
    material["skate3_retail_material_group_index"] = (
        retail_material_group_index
    )
    material["skate3_retail_texture_ids"] = json.dumps(
        retail_texture_ids or {},
        sort_keys=True,
        separators=(",", ":"),
    )
    material["skate3_retail_parameters"] = json.dumps(
        retail_parameters or {},
        sort_keys=True,
        separators=(",", ":"),
    )
    material["skate3_retail_source"] = json.dumps(
        retail_source or {},
        sort_keys=True,
        separators=(",", ":"),
    )
    material["ow_normal_image"] = (
        normal_image.name if normal_image is not None else ""
    )
    material["ow_lightmap_image"] = (
        lightmap_image.name if lightmap_image is not None else ""
    )
    material["ow_lightmap_encoding"] = (
        "skate3_retail_sqrt_linear_over_4"
        if lightmap_image is not None
        else ""
    )
    material["ow_baked_strength"] = 1.0 if lightmap_image is not None else 0.0
    if fallback:
        material["skate3_fallback_reason"] = (
            "Package references a shared texture that is not embedded."
        )
    return material


def _runtime_point_to_blender(
    point: tuple[float, float, float],
) -> tuple[float, float, float]:
    return point[0], -point[2], point[1]


def _retail_grind_controls(
    payload_hex: str,
) -> tuple[
    tuple[float, float, float],
    tuple[float, float, float],
    tuple[float, float, float],
    tuple[float, float, float],
]:
    payload = bytes.fromhex(payload_hex)
    if len(payload) != 120:
        raise ValueError("retail grind segment payload must contain 120 bytes")
    values = struct.unpack(">30f", payload)
    coefficient_a = values[0:3]
    coefficient_b = values[4:7]
    coefficient_c = values[8:11]
    coefficient_d = values[12:15]
    point_0 = coefficient_d
    point_1 = tuple(
        coefficient_d[axis] + coefficient_c[axis] / 3.0
        for axis in range(3)
    )
    point_2 = tuple(
        coefficient_d[axis]
        + (2.0 * coefficient_c[axis] + coefficient_b[axis]) / 3.0
        for axis in range(3)
    )
    point_3 = tuple(
        coefficient_d[axis]
        + coefficient_c[axis]
        + coefficient_b[axis]
        + coefficient_a[axis]
        for axis in range(3)
    )
    return tuple(
        _runtime_point_to_blender(point)
        for point in (point_0, point_1, point_2, point_3)
    )


def _import_retail_grinds(
    manifest: dict[str, object],
    collection: bpy.types.Collection,
) -> tuple[int, int, int, int]:
    rail_count = 0
    segment_count = 0
    cell_local_count = 0
    ambiguous_count = 0
    coordinate_policy = manifest.get("grind_coordinate_policy", {})
    default_coordinate_mode = (
        "mixed_cell_local"
        if manifest.get("format") == "skate2-bam-cache-v1"
        else "world_space"
    )
    coordinate_mode = str(
        coordinate_policy.get("mode", default_coordinate_mode)
    )
    classification_margin = float(
        coordinate_policy.get("classification_margin", 40.0)
    )
    for entry in manifest.get("grind_splines", []):
        source_payloads = entry["native_segment_payloads"]
        expected_segments = int(entry["segment_count"])
        if (
            len(source_payloads) != expected_segments
            or expected_segments == 0
        ):
            raise ValueError("retail grind manifest has invalid segment count")
        source_coordinate_frame = "world_space"
        payloads = source_payloads
        translation = None
        if coordinate_mode == "mixed_cell_local":
            source_coordinate_frame = classify_grind_coordinate_frame(
                entry["stream_file"],
                source_payloads,
                margin=classification_margin,
            )
            if source_coordinate_frame == "cell_local":
                translation = grind_cell_translation(entry["stream_file"])
                if translation is None:
                    raise ValueError(
                        "cell-local grind has no stream-cell translation"
                    )
                payloads = [
                    translate_native_segment_payload(payload, translation)
                    for payload in source_payloads
                ]
                cell_local_count += 1
            elif source_coordinate_frame == "ambiguous":
                ambiguous_count += 1
        elif coordinate_mode != "world_space":
            raise ValueError(
                f"unsupported grind coordinate mode {coordinate_mode!r}"
            )
        controls = [_retail_grind_controls(payload) for payload in payloads]
        closed = bool(entry["closed"])
        point_count = expected_segments if closed else expected_segments + 1
        curve_name = (
            f"GRIND_{entry['asset_id'][2:]}_"
            f"{int(entry['rail_index']):04d}"
        )
        curve_data = bpy.data.curves.new(curve_name, type="CURVE")
        curve_data.dimensions = "3D"
        curve_data.resolution_u = 8
        curve_data.bevel_depth = 0.015
        curve_data.bevel_resolution = 0
        spline = curve_data.splines.new("BEZIER")
        spline.bezier_points.add(point_count - 1)
        spline.use_cyclic_u = closed
        for point in spline.bezier_points:
            point.handle_left_type = "FREE"
            point.handle_right_type = "FREE"

        for segment_index, segment_controls in enumerate(controls):
            point_0, point_1, point_2, point_3 = segment_controls
            current_index = segment_index
            next_index = (segment_index + 1) % point_count
            current = spline.bezier_points[current_index]
            following = spline.bezier_points[next_index]
            current.co = point_0
            current.handle_right = point_1
            following.handle_left = point_2
            if not closed or next_index != 0:
                following.co = point_3
        if not closed:
            spline.bezier_points[0].handle_left = (
                spline.bezier_points[0].co
            )
            spline.bezier_points[-1].handle_right = (
                spline.bezier_points[-1].co
            )

        obj = bpy.data.objects.new(curve_name, curve_data)
        collection.objects.link(obj)
        obj.color = (1.0, 0.12, 0.02, 1.0)
        obj["skate3_retail_grind"] = True
        obj["skate3_asset_id"] = entry["asset_id"]
        obj["skate3_stream_file"] = entry["stream_file"]
        obj["skate3_retail_grind_rail_index"] = int(entry["rail_index"])
        obj["skate3_retail_grind_spline_id"] = entry["spline_id"]
        obj["skate3_retail_grind_type_signature"] = entry["type_signature"]
        obj["skate3_retail_grind_flags"] = int(entry["flags"])
        obj["skate3_retail_grind_trailing_word"] = int(
            entry["trailing_word"]
        )
        obj["skate3_retail_grind_segment_count"] = expected_segments
        obj["skate3_retail_grind_segment_payload"] = "".join(payloads)
        obj["skate3_retail_grind_source_coordinate_frame"] = (
            source_coordinate_frame
        )
        if translation is not None:
            obj["skate3_retail_grind_cell_translation"] = translation
        rail_count += 1
        segment_count += expected_segments
    return rail_count, segment_count, cell_local_count, ambiguous_count


def _retail_collision_material(surface: int) -> bpy.types.Material:
    name = f"MAT_RETAIL_COLLISION_{surface:04X}"
    material = bpy.data.materials.get(name)
    if material is None:
        material = bpy.data.materials.new(name)
    material.diffuse_color = (
        ((surface * 37) & 0xFF) / 255.0,
        ((surface * 67) & 0xFF) / 255.0,
        ((surface * 97) & 0xFF) / 255.0,
        1.0,
    )
    material["skate3_retail_surface_id"] = f"0x{surface:04X}"
    material["ow_flags"] = 1
    material["ow_friction"] = 0.82
    material["ow_restitution"] = 0.0
    material["ow_display_color"] = tuple(material.diffuse_color[:3])
    material["ow_roughness"] = 0.68
    material["ow_emissive"] = 0.0
    material["ow_baked_strength"] = 0.0
    material["ow_albedo_image"] = ""
    material["ow_lightmap_image"] = ""
    material["ow_normal_image"] = ""
    material["ow_orm_image"] = ""
    material["ow_emissive_image"] = ""
    material["ow_alpha_mode"] = 0
    material["ow_alpha_cutoff"] = 0.5
    material["ow_audio_surface"] = surface & 0x7F
    material["ow_physics_surface"] = (surface >> 7) & 0x1F
    material["ow_surface_pattern"] = (surface >> 12) & 0x0F
    material["ow_collision_enabled"] = True
    return material


def _import_retail_collision(
    manifest: dict[str, object],
    cache_root: Path,
    collection: bpy.types.Collection,
) -> tuple[int, int, int]:
    by_surface: dict[
        int,
        tuple[
            list[tuple[float, float, float]],
            list[tuple[int, int, int]],
            dict[tuple[float, float, float], int],
            list[tuple[int, int, int]],
        ],
    ] = {}
    mesh_count = 0
    source_triangle_count = 0
    for entry in manifest["simulation_assets"]:
        if not entry.get("collision_meshes"):
            continue
        path = cache_root / Path(str(entry["rx2"]).replace("\\", "/"))
        meshes = decode_rx2_clustered_meshes(path.read_bytes())
        if len(meshes) != len(entry["collision_meshes"]):
            raise ValueError(
                f"{entry['asset_id']} retail collision section count changed"
            )
        for mesh in meshes:
            mesh_count += 1
            source_triangle_count += len(mesh.triangles)
            for triangle in mesh.triangles:
                vertices, faces, indices, surface_edge_codes = (
                    by_surface.setdefault(
                        triangle.surface,
                        ([], [], {}, []),
                    )
                )
                face = []
                for runtime_point in (triangle.a, triangle.b, triangle.c):
                    point = _runtime_point_to_blender(runtime_point)
                    index = indices.get(point)
                    if index is None:
                        index = len(vertices)
                        indices[point] = index
                        vertices.append(point)
                    face.append(index)
                faces.append(tuple(face))
                if triangle.edge_codes is None:
                    raise ValueError(
                        f"{entry['asset_id']} retail collision triangle "
                        "omits native edge codes"
                    )
                surface_edge_codes.append(triangle.edge_codes)

    imported_triangles = 0
    for surface, (
        vertices,
        faces,
        _indices,
        edge_codes,
    ) in sorted(by_surface.items()):
        name = f"RETAIL_COLLISION_{surface:04X}"
        mesh_data = bpy.data.meshes.new(f"{name}_MESH")
        mesh_data.from_pydata(vertices, [], faces)
        mesh_data.update(calc_edges=False)
        for corner in range(3):
            attribute = mesh_data.attributes.new(
                f"skate3_retail_edge_code_{corner}",
                type="INT",
                domain="FACE",
            )
            attribute.data.foreach_set(
                "value",
                [codes[corner] for codes in edge_codes],
            )
        material = _retail_collision_material(surface)
        mesh_data.materials.append(material)
        obj = bpy.data.objects.new(name, mesh_data)
        collection.objects.link(obj)
        obj["skate3_retail_collision"] = True
        obj["skate3_retail_surface_id"] = f"0x{surface:04X}"
        obj["skate3_retail_triangle_count"] = len(faces)
        # Retail ClusteredMesh commonly stores reverse-wound partner faces.
        # They make a patch intentionally collidable from both sides and must
        # not be collapsed by the generic collision cleanup.
        obj["ow_preserve_opposite_wound_collision"] = True
        obj["ow_preserve_retail_edge_codes"] = True
        obj["ow_material"] = material.name
        imported_triangles += len(faces)

    if imported_triangles != source_triangle_count:
        raise ValueError(
            "retail collision import changed the decoded triangle count"
        )
    return mesh_count, len(by_surface), imported_triangles


def build_scene(manifest_path: Path) -> dict[str, int]:
    manifest_text = manifest_path.read_text(encoding="utf-8")
    manifest = json.loads(manifest_text)
    cache_root = manifest_path.parent
    _clear_scene()

    scene = bpy.context.scene
    scene_name = manifest["map_name"]
    scene.name = scene_name
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0
    manifest_block = bpy.data.texts.get("SKATE3_RETAIL_MANIFEST")
    if manifest_block is None:
        manifest_block = bpy.data.texts.new("SKATE3_RETAIL_MANIFEST")
    else:
        manifest_block.clear()
    manifest_block.write(manifest_text)

    root = _new_child_collection(scene, scene_name)
    presentation = _new_child_collection(root, "Presentation")
    retail_grinds = _new_child_collection(root, "Retail_Grinds")
    collision = _new_child_collection(root, "Collision_RAW")
    collision.hide_viewport = True
    collision.hide_render = True
    metadata = _new_child_collection(root, "Metadata")
    images = _load_images(manifest, cache_root)
    materials: dict[tuple[object, ...], bpy.types.Material] = {}
    excluded_normal_texture_ids = {
        str(texture_id).lower()
        for texture_id in manifest.get("normal_texture_policy", {}).get(
            "excluded_texture_ids",
            [],
        )
    }

    object_count = 0
    vertex_count = 0
    triangle_count = 0
    normal_mapped_object_count = 0
    used_normal_texture_ids: set[str] = set()
    lightmapped_object_count = 0
    used_lightmap_texture_ids: set[str] = set()
    retail_world_frame_object_count = 0
    default_excluded_static_model_asset_ids = (
        {"0xd4538d2e1da5434f"}
        if manifest.get("format") == "skate2-bam-cache-v1"
        else set()
    )
    excluded_static_model_asset_ids = default_excluded_static_model_asset_ids | {
        str(asset_id).lower()
        for asset_id in manifest.get("presentation_model_policy", {}).get(
            "excluded_asset_ids",
            [],
        )
    }
    excluded_static_mesh_parts = 0
    for model_entry in manifest["models"]:
        asset_id = model_entry["asset_id"]
        if str(asset_id).lower() in excluded_static_model_asset_ids:
            excluded_static_mesh_parts += len(model_entry["meshes"])
            continue
        source_stem = Path(model_entry["stream_file"]).stem
        cell_collection = presentation.children.get(source_stem)
        if cell_collection is None:
            cell_collection = _new_child_collection(presentation, source_stem)
        asset_collection = _new_child_collection(
            cell_collection,
            f"MODEL_{asset_id[2:]}",
        )
        npz_path = cache_root / model_entry["npz"]
        with numpy.load(npz_path) as arrays:
            for mesh_entry in model_entry["meshes"]:
                mesh_index = mesh_entry["index"]
                vertices = _runtime_to_blender(arrays[f"vertices_{mesh_index}"])
                faces = arrays[f"faces_{mesh_index}"]
                uvs = (
                    arrays[f"uvs_{mesh_index}"]
                    if f"uvs_{mesh_index}" in arrays
                    else None
                )
                lightmap_uvs = (
                    arrays[f"lightmap_uvs_{mesh_index}"]
                    if f"lightmap_uvs_{mesh_index}" in arrays
                    else None
                )
                decal_uvs = (
                    arrays[f"decal_uvs_{mesh_index}"]
                    if f"decal_uvs_{mesh_index}" in arrays
                    else None
                )
                retail_frame_available = all(
                    name in arrays
                    for name in (
                        f"retail_normals_{mesh_index}",
                        f"retail_tangents_{mesh_index}",
                        f"retail_tangent_handedness_{mesh_index}",
                    )
                )
                if retail_frame_available:
                    normals = _runtime_to_blender(
                        arrays[f"retail_normals_{mesh_index}"]
                    )
                    retail_tangents = _runtime_to_blender(
                        arrays[f"retail_tangents_{mesh_index}"]
                    )
                    retail_handedness = numpy.asarray(
                        arrays[
                            f"retail_tangent_handedness_{mesh_index}"
                        ],
                        dtype=numpy.float32,
                    )
                else:
                    normals = (
                        _runtime_to_blender(
                            arrays[f"normals_{mesh_index}"]
                        )
                        if f"normals_{mesh_index}" in arrays
                        else None
                    )
                    retail_tangents = None
                    retail_handedness = None

                object_name = _safe_name(
                    mesh_entry["material_name"] or mesh_entry["name"],
                    f"{asset_id[2:]}_{mesh_index:02d}",
                )
                mesh_data = bpy.data.meshes.new(f"{object_name}_MESH")
                mesh_data.from_pydata(vertices.tolist(), [], faces.tolist())
                mesh_data.update(calc_edges=True)
                if retail_frame_available:
                    _set_point_vector_attribute(
                        mesh_data,
                        RETAIL_NORMAL_ATTRIBUTE,
                        normals,
                    )
                    _set_point_vector_attribute(
                        mesh_data,
                        RETAIL_TANGENT_ATTRIBUTE,
                        retail_tangents,
                    )
                    _set_point_float_attribute(
                        mesh_data,
                        RETAIL_HANDEDNESS_ATTRIBUTE,
                        retail_handedness,
                    )
                    retail_world_frame_object_count += 1

                if uvs is not None:
                    uv_layer = mesh_data.uv_layers.new(name="UVMap")
                    for loop in mesh_data.loops:
                        uv = uvs[loop.vertex_index]
                        uv_layer.data[loop.index].uv = (
                            float(uv[0]),
                            1.0 - float(uv[1]),
                        )
                if lightmap_uvs is not None:
                    lightmap_layer = mesh_data.uv_layers.new(name="Lightmap")
                    shader_name = str(mesh_entry.get("shader_name") or "")
                    absolute_unwrap = shader_name != "ocean.default"
                    for loop in mesh_data.loops:
                        uv = lightmap_uvs[loop.vertex_index]
                        u = float(uv[0])
                        v = float(uv[1])
                        if absolute_unwrap:
                            u = abs(u)
                            v = abs(v)
                        lightmap_layer.data[loop.index].uv = (u, 1.0 - v)
                if decal_uvs is not None:
                    decal_layer = mesh_data.uv_layers.new(name="Decal")
                    for loop in mesh_data.loops:
                        uv = decal_uvs[loop.vertex_index]
                        decal_layer.data[loop.index].uv = (
                            float(uv[0]),
                            1.0 - float(uv[1]),
                        )

                for polygon in mesh_data.polygons:
                    polygon.use_smooth = True
                if normals is not None:
                    mesh_data.normals_split_custom_set_from_vertices(
                        normals.tolist()
                    )

                obj = bpy.data.objects.new(object_name, mesh_data)
                asset_collection.objects.link(obj)
                texture_id = mesh_entry["texture_id"]
                alpha_mode = int(mesh_entry.get("alpha_mode", 0))
                retail_texture_ids = mesh_entry.get(
                    "retail_texture_ids",
                    {},
                )
                source_lightmap_texture_id = str(
                    retail_texture_ids.get("lightmap", "")
                ).lower()
                shader_name = str(mesh_entry.get("shader_name") or "")
                lightmap_texture_id = (
                    source_lightmap_texture_id
                    if source_lightmap_texture_id
                    and lightmap_uvs is not None
                    and not shader_name.startswith(("water.", "ocean."))
                    else ""
                )
                source_normal_texture_id = str(
                    retail_texture_ids.get("normal", "")
                ).lower()
                normal_texture_id = (
                    source_normal_texture_id
                    if source_normal_texture_id
                    and source_normal_texture_id
                    not in excluded_normal_texture_ids
                    else ""
                )
                retail_parameters = mesh_entry.get(
                    "retail_parameters", {}
                )
                retail_source = {
                    "asset_id": asset_id,
                    "stream_file": model_entry["stream_file"],
                    "mesh_index": mesh_index,
                    "vertex_stride": mesh_entry["vertex_stride"],
                    "attributes": mesh_entry.get("attributes", []),
                    "source_offsets": mesh_entry["source_offsets"],
                    "bounds": mesh_entry.get("bounds", {}),
                    "vertex_count": mesh_entry["vertex_count"],
                    "triangle_count": mesh_entry["triangle_count"],
                    "retail_world_frame": mesh_entry.get(
                        "retail_world_frame"
                    ),
                }
                material_key = (
                    asset_id,
                    mesh_index,
                    str(mesh_entry.get("retail_material_guid", "")),
                    str(mesh_entry.get("retail_material_handle", "")),
                    int(
                        mesh_entry.get(
                            "retail_material_group_index", -1
                        )
                    ),
                    shader_name,
                    json.dumps(
                        retail_texture_ids,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        retail_parameters,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                    alpha_mode,
                )
                material = materials.get(material_key)
                if material is None:
                    image = images.get(texture_id) if texture_id else None
                    normal_image = (
                        images.get(normal_texture_id)
                        if normal_texture_id
                        else None
                    )
                    lightmap_image = (
                        images.get(lightmap_texture_id)
                        if lightmap_texture_id
                        else None
                    )
                    material = _new_material(
                        texture_id or "NO_DIFFUSE",
                        image,
                        alpha_mode,
                        lightmap_texture_id=lightmap_texture_id,
                        lightmap_image=lightmap_image,
                        normal_texture_id=normal_texture_id,
                        normal_image=normal_image,
                        shader_name=shader_name,
                        retail_material_guid=str(
                            mesh_entry.get(
                                "retail_material_guid", ""
                            )
                        ),
                        retail_material_handle=str(
                            mesh_entry.get(
                                "retail_material_handle", ""
                            )
                        ),
                        retail_material_group_index=int(
                            mesh_entry.get(
                                "retail_material_group_index", -1
                            )
                        ),
                        retail_texture_ids={
                            str(key): str(value).lower()
                            for key, value in retail_texture_ids.items()
                        },
                        retail_parameters=retail_parameters,
                        retail_source=retail_source,
                        fallback=image is None and bool(texture_id),
                    )
                    entry = (
                        manifest["textures"].get(texture_id)
                        if texture_id
                        else None
                    )
                    if entry is not None:
                        material["skate3_stream_asset_id"] = entry[
                            "stream_asset_id"
                        ]
                        material["skate3_stream_file"] = entry[
                            "stream_file"
                        ]
                    materials[material_key] = material
                mesh_data.materials.append(material)
                if normal_texture_id:
                    normal_mapped_object_count += 1
                    used_normal_texture_ids.add(normal_texture_id)
                if lightmap_texture_id:
                    lightmapped_object_count += 1
                    used_lightmap_texture_ids.add(lightmap_texture_id)
                obj["skate3_asset_id"] = asset_id
                obj["skate3_stream_file"] = model_entry["stream_file"]
                obj["skate3_mesh_index"] = mesh_index
                obj["skate3_material_name"] = mesh_entry["material_name"] or ""
                obj["skate3_retail_material_guid"] = mesh_entry.get(
                    "retail_material_guid",
                    "",
                )
                obj["skate3_retail_material_handle"] = mesh_entry.get(
                    "retail_material_handle",
                    "",
                )
                obj["skate3_retail_material_group_index"] = int(
                    mesh_entry.get("retail_material_group_index", -1)
                )
                obj["skate3_texture_id"] = texture_id or ""
                obj["skate3_retail_world_frame"] = retail_frame_available
                obj["skate3_lightmap_texture_id"] = lightmap_texture_id
                obj["skate3_source_lightmap_texture_id"] = (
                    source_lightmap_texture_id
                )
                if source_lightmap_texture_id and not lightmap_texture_id:
                    obj["skate3_lightmap_exclusion"] = (
                        "shader family does not consume a reliable static "
                        "lightmap"
                        if lightmap_uvs is not None
                        else "retail mesh has no secondary TEXCOORD"
                    )
                obj["skate3_normal_texture_id"] = normal_texture_id
                obj["skate3_source_normal_texture_id"] = (
                    source_normal_texture_id
                )
                obj["skate3_shader_name"] = mesh_entry.get("shader_name") or ""
                obj["skate3_retail_parameters"] = json.dumps(
                    retail_parameters,
                    sort_keys=True,
                    separators=(",", ":"),
                )
                obj["skate3_retail_texture_ids"] = json.dumps(
                    retail_texture_ids,
                    sort_keys=True,
                    separators=(",", ":"),
                )
                obj["skate3_texture_channel"] = (
                    mesh_entry.get("texture_channel") or ""
                )
                obj["skate3_alpha_mode"] = alpha_mode
                obj["skate3_vertex_stride"] = mesh_entry["vertex_stride"]
                obj["skate3_source_offsets"] = json.dumps(
                    mesh_entry["source_offsets"],
                    sort_keys=True,
                )
                object_count += 1
                vertex_count += len(vertices)
                triangle_count += len(faces)

    (
        grind_rail_count,
        grind_segment_count,
        cell_local_grind_count,
        ambiguous_grind_count,
    ) = _import_retail_grinds(manifest, retail_grinds)
    (
        collision_mesh_count,
        collision_surface_count,
        collision_triangle_count,
    ) = _import_retail_collision(manifest, cache_root, collision)

    for simulation_entry in manifest["simulation_assets"]:
        empty = bpy.data.objects.new(
            f"SIM_{simulation_entry['asset_id'][2:]}",
            None,
        )
        empty.empty_display_type = "CUBE"
        empty.empty_display_size = 0.25
        empty.hide_render = True
        empty["skate3_asset_id"] = simulation_entry["asset_id"]
        empty["skate3_stream_file"] = simulation_entry["stream_file"]
        empty["skate3_rx2"] = simulation_entry["rx2"]
        empty["skate3_size"] = simulation_entry["size"]
        collision.objects.link(empty)

    info = bpy.data.objects.new(f"{scene_name} Import Status", None)
    info.empty_display_type = "PLAIN_AXES"
    info.empty_display_size = 2.0
    info["status"] = (
        "Presentation geometry, diffuse textures, exact retail lightmaps, "
        "conservative retail normal textures, exact retail collision, and "
        "retail grind splines imported. Original RX2 assets and all material "
        "channel IDs are preserved in the cache."
    )
    info["source_manifest"] = str(manifest_path)
    metadata.objects.link(info)

    scene["skate3_map_name"] = manifest["map_name"]
    scene["skate3_package_name"] = manifest["package_name"]
    scene["skate3_district_name"] = manifest["district_name"]
    scene["skate3_manifest"] = str(manifest_path)
    scene["skate3_import_format"] = manifest["format"]
    scene["skate3_collision_status"] = (
        f"{collision_triangle_count} exact retail ClusteredMesh triangles "
        f"across {collision_surface_count} packed surfaces"
    )
    scene["skate3_grind_status"] = (
        f"{grind_rail_count} exact retail grind rails with "
        f"{grind_segment_count} native cubic segments; "
        f"{cell_local_grind_count} cell-local rails placed in world space; "
        f"{ambiguous_grind_count} boundary rails preserved unchanged"
    )
    scene["skate3_excluded_static_model_status"] = (
        f"{excluded_static_mesh_parts} unplaced static prop mesh parts "
        "excluded by import policy"
    )
    scene["skate3_normal_status"] = (
        f"{normal_mapped_object_count} mesh parts use "
        f"{len(used_normal_texture_ids)} conventional retail normal maps"
    )
    scene["skate3_lightmap_status"] = (
        f"{lightmapped_object_count} mesh parts use "
        f"{len(used_lightmap_texture_ids)} exact retail lightmap pages"
    )
    scene["skate3_retail_world_frame_status"] = (
        f"{retail_world_frame_object_count} mesh parts preserve exact retail "
        "normal, tangent, and handedness data"
    )
    return {
        "objects": object_count,
        "vertices": vertex_count,
        "triangles": triangle_count,
        "textures": len(images),
        "materials": len(materials),
        "normal_mapped_objects": normal_mapped_object_count,
        "normal_textures": len(used_normal_texture_ids),
        "lightmapped_objects": lightmapped_object_count,
        "lightmap_textures": len(used_lightmap_texture_ids),
        "retail_world_frames": retail_world_frame_object_count,
        "grind_rails": grind_rail_count,
        "grind_segments": grind_segment_count,
        "collision_meshes": collision_mesh_count,
        "collision_surfaces": collision_surface_count,
        "collision_triangles": collision_triangle_count,
        "simulation_assets": len(manifest["simulation_assets"]),
        "expected_objects": (
            manifest["summary"]["mesh_parts"]
            - excluded_static_mesh_parts
        ),
    }


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 2:
        raise SystemExit(
            "usage: blender --background --python import_hawaiian_dream.py -- "
            "MANIFEST OUTPUT_BLEND"
        )
    manifest_path = Path(arguments[0]).resolve()
    output_blend = Path(arguments[1]).resolve()
    summary = build_scene(manifest_path)
    if summary["objects"] != summary["expected_objects"]:
        raise RuntimeError(
            f"created {summary['objects']} objects; "
            f"expected {summary['expected_objects']}"
        )
    output_blend.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(output_blend))
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
