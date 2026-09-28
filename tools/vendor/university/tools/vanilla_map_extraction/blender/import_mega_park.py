"""Build and save a textured Mega Park Blender scene from the prepared cache."""

from __future__ import annotations

import json
import math
from pathlib import Path
import re
import sys

import bpy
from mathutils import Vector
import numpy


def _safe_name(value: str, fallback: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("_")
    return (cleaned or fallback)[:120]


def _runtime_to_blender(values: numpy.ndarray) -> numpy.ndarray:
    converted = values[:, [0, 2, 1]].copy()
    converted[:, 1] *= -1.0
    return converted


def _look_at(camera: bpy.types.Object, target: Vector) -> None:
    direction = target - camera.location
    camera.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()


def _clear_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        if collection.name != "Collection":
            bpy.data.collections.remove(collection)
    default = bpy.data.collections.get("Collection")
    if default is not None:
        bpy.data.collections.remove(default)


def _new_child_collection(
    parent: bpy.types.Collection | bpy.types.Scene, name: str
) -> bpy.types.Collection:
    collection = bpy.data.collections.new(name)
    parent.collection.children.link(collection) if isinstance(
        parent, bpy.types.Scene
    ) else parent.children.link(collection)
    return collection


def _load_materials(
    manifest: dict[str, object], cache_root: Path
) -> dict[str, bpy.types.Material]:
    materials: dict[str, bpy.types.Material] = {}
    for texture_id, entry_value in manifest["textures"].items():
        entry = entry_value
        image_path = cache_root / entry["png"]
        image = bpy.data.images.load(str(image_path), check_existing=True)
        image.name = texture_id

        material = bpy.data.materials.new(f"MAT_{texture_id}")
        material.use_nodes = True
        material.use_backface_culling = True
        nodes = material.node_tree.nodes
        links = material.node_tree.links
        principled = nodes.get("Principled BSDF")
        texture = nodes.new("ShaderNodeTexImage")
        texture.name = f"TEX_{texture_id}"
        texture.label = texture_id
        texture.image = image
        texture.interpolation = "Linear"
        links.new(texture.outputs["Color"], principled.inputs["Base Color"])
        if "Alpha" in principled.inputs:
            links.new(texture.outputs["Alpha"], principled.inputs["Alpha"])
        principled.inputs["Roughness"].default_value = 0.68
        principled.inputs["Metallic"].default_value = 0.0
        if entry["format"] in {"DXT3", "DXT5"}:
            try:
                material.surface_render_method = "DITHERED"
            except Exception:
                try:
                    material.blend_method = "HASHED"
                except Exception:
                    pass
        material["skate3_texture_id"] = texture_id
        material["skate3_stream_asset_id"] = entry["stream_asset_id"]
        materials[texture_id] = material
    return materials


def build_scene(manifest_path: Path) -> dict[str, int]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    cache_root = manifest_path.parent
    _clear_scene()

    scene = bpy.context.scene
    scene.name = "DIST_MegaPark"
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 720
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.film_transparent = False
    scene.view_settings.look = "AgX - Medium High Contrast"
    scene.view_settings.exposure = 1.0

    world = bpy.data.worlds.new("Mega Park World")
    world.use_nodes = True
    background = world.node_tree.nodes.get("Background")
    background.inputs["Color"].default_value = (0.025, 0.035, 0.05, 1.0)
    background.inputs["Strength"].default_value = 0.35
    scene.world = world

    root = _new_child_collection(scene, "DIST_MegaPark")
    presentation = _new_child_collection(root, "Presentation")
    collision = _new_child_collection(root, "Collision_RAW")
    metadata = _new_child_collection(root, "Metadata")
    materials = _load_materials(manifest, cache_root)

    object_count = 0
    vertex_count = 0
    triangle_count = 0

    for model_entry in manifest["models"]:
        asset_id = model_entry["asset_id"]
        asset_collection = _new_child_collection(
            presentation, f"MODEL_{asset_id[2:]}"
        )
        npz_path = cache_root / model_entry["npz"]
        with numpy.load(npz_path) as arrays:
            for mesh_entry in model_entry["meshes"]:
                mesh_index = mesh_entry["index"]
                runtime_vertices = arrays[f"vertices_{mesh_index}"]
                vertices = _runtime_to_blender(runtime_vertices)
                faces = arrays[f"faces_{mesh_index}"]
                uvs = (
                    arrays[f"uvs_{mesh_index}"]
                    if f"uvs_{mesh_index}" in arrays
                    else None
                )

                object_name = _safe_name(
                    mesh_entry["material_name"] or mesh_entry["name"],
                    f"{asset_id[2:]}_{mesh_index:02d}",
                )
                mesh_data = bpy.data.meshes.new(f"{object_name}_MESH")
                mesh_data.from_pydata(
                    vertices.tolist(),
                    [],
                    faces.tolist(),
                )
                mesh_data.update(calc_edges=True)

                if uvs is not None:
                    uv_layer = mesh_data.uv_layers.new(name="UVMap")
                    for loop in mesh_data.loops:
                        uv = uvs[loop.vertex_index]
                        uv_layer.data[loop.index].uv = (
                            float(uv[0]),
                            1.0 - float(uv[1]),
                        )

                for polygon in mesh_data.polygons:
                    polygon.use_smooth = True

                obj = bpy.data.objects.new(object_name, mesh_data)
                asset_collection.objects.link(obj)
                texture_id = mesh_entry["texture_id"]
                if texture_id in materials:
                    mesh_data.materials.append(materials[texture_id])
                obj["skate3_asset_id"] = asset_id
                obj["skate3_mesh_index"] = mesh_index
                obj["skate3_material_name"] = mesh_entry["material_name"] or ""
                obj["skate3_texture_id"] = texture_id or ""
                obj["skate3_vertex_stride"] = mesh_entry["vertex_stride"]
                obj["skate3_source_offsets"] = json.dumps(
                    mesh_entry["source_offsets"], sort_keys=True
                )
                object_count += 1
                vertex_count += len(vertices)
                triangle_count += len(faces)

    for simulation_entry in manifest["simulation_assets"]:
        empty = bpy.data.objects.new(
            f"SIM_{simulation_entry['asset_id'][2:]}",
            None,
        )
        empty.empty_display_type = "CUBE"
        empty.empty_display_size = 0.35
        empty.hide_render = True
        empty["skate3_asset_id"] = simulation_entry["asset_id"]
        empty["skate3_rx2"] = simulation_entry["rx2"]
        empty["skate3_size"] = simulation_entry["size"]
        collision.objects.link(empty)

    info = bpy.data.objects.new("Mega Park Import Status", None)
    info.empty_display_type = "PLAIN_AXES"
    info.empty_display_size = 2.0
    info["status"] = (
        "Presentation geometry and diffuse textures imported. "
        "Simulation RX2 assets preserved but collision decoding is pending."
    )
    info["source_manifest"] = str(manifest_path)
    metadata.objects.link(info)

    summary = manifest["summary"]
    runtime_minimum = numpy.min(
        numpy.asarray(
            [model["bounds"]["minimum"] for model in manifest["models"]],
            dtype=numpy.float32,
        ),
        axis=0,
    )
    runtime_maximum = numpy.max(
        numpy.asarray(
            [model["bounds"]["maximum"] for model in manifest["models"]],
            dtype=numpy.float32,
        ),
        axis=0,
    )
    corners = numpy.asarray(
        [
            [x, y, z]
            for x in (runtime_minimum[0], runtime_maximum[0])
            for y in (runtime_minimum[1], runtime_maximum[1])
            for z in (runtime_minimum[2], runtime_maximum[2])
        ],
        dtype=numpy.float32,
    )
    blender_corners = _runtime_to_blender(corners)
    minimum = blender_corners.min(axis=0)
    maximum = blender_corners.max(axis=0)
    center = Vector(((minimum + maximum) * 0.5).tolist())
    dimensions = maximum - minimum
    extent = float(max(dimensions))

    exterior_data = bpy.data.cameras.new("Mega Park Exterior Camera")
    exterior = bpy.data.objects.new("Mega Park Exterior Camera", exterior_data)
    root.objects.link(exterior)
    exterior.location = center + Vector(
        (extent * 0.72, -extent * 0.95, extent * 0.58)
    )
    exterior_data.lens = 48.0
    exterior_data.clip_start = 0.1
    exterior_data.clip_end = max(2000.0, extent * 10.0)
    _look_at(exterior, center)

    interior_data = bpy.data.cameras.new("Mega Park Interior Camera")
    interior = bpy.data.objects.new("Mega Park Interior Camera", interior_data)
    root.objects.link(interior)
    interior.location = Vector(
        (
            float(minimum[0] + dimensions[0] * 0.18),
            float(minimum[1] + dimensions[1] * 0.28),
            float(minimum[2] + max(9.0, dimensions[2] * 0.13)),
        )
    )
    interior_target = Vector(
        (
            float(minimum[0] + dimensions[0] * 0.72),
            float(minimum[1] + dimensions[1] * 0.58),
            float(minimum[2] + max(7.0, dimensions[2] * 0.09)),
        )
    )
    interior_data.lens = 31.0
    interior_data.clip_start = 0.05
    interior_data.clip_end = max(2000.0, extent * 10.0)
    _look_at(interior, interior_target)
    scene.camera = interior

    sun_data = bpy.data.lights.new("Mega Park Sun", type="SUN")
    sun_data.energy = 3.0
    sun_data.angle = math.radians(18.0)
    sun = bpy.data.objects.new("Mega Park Sun", sun_data)
    root.objects.link(sun)
    sun.rotation_euler = (math.radians(32.0), math.radians(-18.0), math.radians(-28.0))

    area_data = bpy.data.lights.new("Mega Park Fill", type="AREA")
    area_data.energy = 1800.0
    area_data.shape = "DISK"
    area_data.size = extent * 0.45
    area = bpy.data.objects.new("Mega Park Fill", area_data)
    root.objects.link(area)
    area.location = center + Vector((-extent * 0.35, extent * 0.2, extent * 0.7))
    _look_at(area, center)

    interior_key_data = bpy.data.lights.new("Mega Park Interior Key", type="AREA")
    interior_key_data.energy = 5200.0
    interior_key_data.shape = "DISK"
    interior_key_data.size = extent * 0.34
    interior_key_data.use_shadow = False
    interior_key = bpy.data.objects.new("Mega Park Interior Key", interior_key_data)
    root.objects.link(interior_key)
    interior_key.location = Vector(
        (
            float(center.x),
            float(center.y),
            float(minimum[2] + dimensions[2] * 0.62),
        )
    )
    _look_at(
        interior_key,
        Vector((float(center.x), float(center.y), float(minimum[2]))),
    )

    camera_fill_data = bpy.data.lights.new("Mega Park Camera Fill", type="AREA")
    camera_fill_data.energy = 2600.0
    camera_fill_data.shape = "DISK"
    camera_fill_data.size = extent * 0.18
    camera_fill_data.use_shadow = False
    camera_fill = bpy.data.objects.new("Mega Park Camera Fill", camera_fill_data)
    root.objects.link(camera_fill)
    camera_fill.location = interior.location + Vector((0.0, 0.0, extent * 0.08))
    _look_at(camera_fill, interior_target)

    interior_sun_data = bpy.data.lights.new("Mega Park Interior Sun", type="SUN")
    interior_sun_data.energy = 2.2
    interior_sun_data.angle = math.radians(35.0)
    interior_sun_data.use_shadow = False
    interior_sun = bpy.data.objects.new(
        "Mega Park Interior Sun", interior_sun_data
    )
    root.objects.link(interior_sun)
    interior_sun.rotation_euler = (
        math.radians(18.0),
        math.radians(-12.0),
        math.radians(142.0),
    )

    scene["skate3_manifest"] = str(manifest_path)
    scene["skate3_import_format"] = manifest["format"]
    scene["skate3_collision_status"] = "raw assets preserved; decoder pending"
    return {
        "objects": object_count,
        "vertices": vertex_count,
        "triangles": triangle_count,
        "textures": len(materials),
        "simulation_assets": len(manifest["simulation_assets"]),
        "expected_objects": summary["mesh_parts"],
    }


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) < 2:
        raise SystemExit(
            "usage: blender --background --python import_mega_park.py -- "
            "MANIFEST OUTPUT_BLEND [PREVIEW_PNG]"
        )
    manifest_path = Path(arguments[0]).resolve()
    output_blend = Path(arguments[1]).resolve()
    preview_png = Path(arguments[2]).resolve() if len(arguments) > 2 else None

    summary = build_scene(manifest_path)
    if summary["objects"] != summary["expected_objects"]:
        raise RuntimeError(
            f"created {summary['objects']} objects; "
            f"expected {summary['expected_objects']}"
        )
    output_blend.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(output_blend))
    if preview_png is not None:
        preview_png.parent.mkdir(parents=True, exist_ok=True)
        bpy.context.scene.render.filepath = str(preview_png)
        bpy.ops.render.render(write_still=True)
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
