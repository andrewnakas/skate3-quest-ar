"""Build, bake, save, and export the small SKATE lighting showcase."""

from __future__ import annotations

from pathlib import Path
import math
import sys

import bpy
from mathutils import Vector


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from owned_world_material_addon.exporter import export_scene  # noqa: E402
import owned_world_material_addon as owned_world_materials  # noqa: E402

owned_world_materials.register()


BLEND_PATH = ROOT / "owned" / "maps" / "source" / "blender_bake_showcase.blend"
PACKAGE_PATH = ROOT / "owned" / "maps" / "blender_bake_showcase.skate"
TEXTURE_DIR = ROOT / "owned" / "maps" / "source" / "blender_bake_showcase_textures"


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (
        bpy.data.meshes,
        bpy.data.curves,
        bpy.data.materials,
        bpy.data.images,
        bpy.data.lights,
    ):
        for item in list(block):
            block.remove(item)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)


def make_collection(name: str) -> bpy.types.Collection:
    collection = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(collection)
    return collection


def move_to(obj: bpy.types.Object, collection: bpy.types.Collection) -> None:
    for current in list(obj.users_collection):
        current.objects.unlink(obj)
    collection.objects.link(obj)


def make_texture(
    name: str,
    first: tuple[float, float, float],
    second: tuple[float, float, float],
    cells: int,
    stripe: bool = False,
    *,
    alpha: float = 1.0,
    cutout_grid: bool = False,
) -> bpy.types.Image:
    size = 256
    image = bpy.data.images.new(name, width=size, height=size, alpha=True)
    pixels = [0.0] * (size * size * 4)
    for y in range(size):
        for x in range(size):
            checker = ((x * cells // size) + (y * cells // size)) & 1
            color = second if checker else first
            if stripe:
                color = second if ((x + y) // 28) & 1 else first
            grain = (((x * 17 + y * 31) % 29) / 28.0 - 0.5) * 0.055
            offset = (y * size + x) * 4
            pixels[offset + 0] = max(0.0, min(1.0, color[0] + grain))
            pixels[offset + 1] = max(0.0, min(1.0, color[1] + grain))
            pixels[offset + 2] = max(0.0, min(1.0, color[2] + grain))
            pixel_alpha = alpha
            if cutout_grid:
                pixel_alpha = (
                    1.0 if x % 32 < 5 or y % 32 < 5 else 0.0
                )
            pixels[offset + 3] = pixel_alpha
    image.pixels.foreach_set(pixels)
    image.update()
    # Generated image buffers did not survive Blender's pack/save cycle in
    # 5.0 and reopened as zero RGB. Persist them as ordinary project-owned
    # PNG assets, then reload them through Blender's file-backed image path
    # before any material or package references them.
    TEXTURE_DIR.mkdir(parents=True, exist_ok=True)
    texture_path = TEXTURE_DIR / f"{name}.png"
    image.filepath_raw = str(texture_path)
    image.file_format = "PNG"
    image.save()
    bpy.data.images.remove(image)
    loaded = bpy.data.images.load(str(texture_path), check_existing=False)
    loaded.name = name
    return loaded


def make_normal_texture(
    name: str,
    strength: float,
    frequency: float,
) -> bpy.types.Image:
    size = 256
    image = bpy.data.images.new(name, width=size, height=size, alpha=True)
    image.colorspace_settings.name = "Non-Color"
    pixels = [0.0] * (size * size * 4)
    for y in range(size):
        for x in range(size):
            u = x / size * math.tau * frequency
            v = y / size * math.tau * frequency
            nx = math.sin(u) * strength
            ny = math.cos(v) * strength
            nz = math.sqrt(max(0.0, 1.0 - nx * nx - ny * ny))
            offset = (y * size + x) * 4
            pixels[offset:offset + 4] = (
                nx * 0.5 + 0.5,
                ny * 0.5 + 0.5,
                nz * 0.5 + 0.5,
                1.0,
            )
    image.pixels.foreach_set(pixels)
    image.update()
    TEXTURE_DIR.mkdir(parents=True, exist_ok=True)
    path = TEXTURE_DIR / f"{name}.png"
    image.filepath_raw = str(path)
    image.file_format = "PNG"
    image.save()
    bpy.data.images.remove(image)
    loaded = bpy.data.images.load(str(path), check_existing=False)
    loaded.name = name
    loaded.colorspace_settings.name = "Non-Color"
    return loaded


def make_orm_texture(
    name: str,
    roughness: float,
    metallic: float,
    *,
    ao: float = 1.0,
) -> bpy.types.Image:
    size = 64
    image = bpy.data.images.new(name, width=size, height=size, alpha=True)
    image.colorspace_settings.name = "Non-Color"
    pixels = [0.0] * (size * size * 4)
    for y in range(size):
        for x in range(size):
            variation = (((x * 13 + y * 19) % 17) / 16.0 - 0.5) * 0.05
            offset = (y * size + x) * 4
            pixels[offset:offset + 4] = (
                max(0.0, min(1.0, ao + variation * 0.25)),
                max(0.0, min(1.0, roughness + variation)),
                max(0.0, min(1.0, metallic)),
                1.0,
            )
    image.pixels.foreach_set(pixels)
    image.update()
    TEXTURE_DIR.mkdir(parents=True, exist_ok=True)
    path = TEXTURE_DIR / f"{name}.png"
    image.filepath_raw = str(path)
    image.file_format = "PNG"
    image.save()
    bpy.data.images.remove(image)
    loaded = bpy.data.images.load(str(path), check_existing=False)
    loaded.name = name
    loaded.colorspace_settings.name = "Non-Color"
    return loaded


def make_material(
    name: str,
    image: bpy.types.Image,
    color: tuple[float, float, float],
    roughness: float,
    *,
    flags: int = 1,
    emissive: float = 0.0,
    normal_image: bpy.types.Image | None = None,
    orm_image: bpy.types.Image | None = None,
    emissive_image: bpy.types.Image | None = None,
    metallic: float = 0.0,
    alpha_mode: int = 0,
    alpha_cutoff: float = 0.5,
    audio_surface: int = 3,
    physics_surface: int = 1,
    surface_pattern: int = 0,
    collision_enabled: bool = True,
) -> bpy.types.Material:
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    material.diffuse_color = (*color, 1.0)
    material["ow_albedo_image"] = image.name
    material["ow_display_color"] = color
    material["ow_roughness"] = roughness
    material["ow_friction"] = 0.84
    material["ow_restitution"] = 0.0
    material["ow_flags"] = flags
    material["ow_emissive"] = emissive
    material["ow_baked_strength"] = 0.92
    material["ow_normal_image"] = (
        normal_image.name if normal_image is not None else ""
    )
    material["ow_orm_image"] = (
        orm_image.name if orm_image is not None else ""
    )
    material["ow_emissive_image"] = (
        emissive_image.name if emissive_image is not None else ""
    )
    material["ow_metallic"] = metallic
    material["ow_alpha_mode"] = alpha_mode
    material["ow_alpha_cutoff"] = alpha_cutoff
    material["ow_audio_surface"] = audio_surface
    material["ow_physics_surface"] = physics_surface
    material["ow_surface_pattern"] = surface_pattern
    material["ow_collision_enabled"] = collision_enabled
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    nodes.clear()
    output = nodes.new("ShaderNodeOutputMaterial")
    shader = nodes.new("ShaderNodeBsdfPrincipled")
    texture = nodes.new("ShaderNodeTexImage")
    texture.image = image
    texture.interpolation = "Linear"
    links.new(texture.outputs["Color"], shader.inputs["Base Color"])
    shader.inputs["Roughness"].default_value = roughness
    shader.inputs["Metallic"].default_value = metallic
    if alpha_mode != 0:
        links.new(texture.outputs["Alpha"], shader.inputs["Alpha"])
        try:
            material.surface_render_method = (
                "DITHERED" if alpha_mode == 2 else "DITHERED"
            )
        except Exception:
            pass
    if normal_image is not None:
        normal_texture = nodes.new("ShaderNodeTexImage")
        normal_texture.image = normal_image
        normal_texture.interpolation = "Linear"
        normal_node = nodes.new("ShaderNodeNormalMap")
        links.new(normal_texture.outputs["Color"], normal_node.inputs["Color"])
        links.new(normal_node.outputs["Normal"], shader.inputs["Normal"])
    if emissive > 0.0:
        emission_color = shader.inputs.get("Emission Color")
        emission_strength = shader.inputs.get("Emission Strength")
        if emission_color is not None:
            emission_color.default_value = (*color, 1.0)
        if emission_strength is not None:
            emission_strength.default_value = emissive
    links.new(shader.outputs["BSDF"], output.inputs["Surface"])
    settings = material.owned_world
    settings.audio_surface = str(audio_surface)
    settings.physics_surface = str(physics_surface)
    settings.surface_pattern = str(surface_pattern)
    settings.collision_enabled = collision_enabled
    settings.alpha_mode = str(alpha_mode)
    settings.alpha_cutoff = alpha_cutoff
    settings.roughness = roughness
    settings.metallic = metallic
    settings.normal_image = normal_image
    settings.orm_image = orm_image
    settings.emissive_image = emissive_image
    return material


def metric_uv(
    obj: bpy.types.Object,
    layer_name: str = "UVMap",
    metres_per_tile: float = 4.0,
) -> None:
    """Author deterministic metre-scaled UV0 instead of fitting each object."""
    mesh = obj.data
    uv_layer = mesh.uv_layers.get(layer_name)
    if uv_layer is None:
        uv_layer = mesh.uv_layers.new(name=layer_name)
    normal_matrix = obj.matrix_world.to_3x3().inverted().transposed()
    for polygon in mesh.polygons:
        normal = (normal_matrix @ polygon.normal).normalized()
        axis = max(range(3), key=lambda index: abs(normal[index]))
        for loop_index in polygon.loop_indices:
            loop = mesh.loops[loop_index]
            point = obj.matrix_world @ mesh.vertices[loop.vertex_index].co
            if axis == 2:
                uv = (point.x, point.y)
            elif axis == 0:
                uv = (point.y, point.z)
            else:
                uv = (point.x, point.z)
            uv_layer.data[loop_index].uv = (
                uv[0] / metres_per_tile,
                uv[1] / metres_per_tile,
            )


def collision_copy(
    source: bpy.types.Object,
    collision_collection: bpy.types.Collection,
    material_name: str,
    suffix: str = "",
) -> bpy.types.Object:
    collision = source.copy()
    collision.data = source.data.copy()
    collision.name = f"COL_{source.name}{suffix}"
    collision["ow_material"] = material_name
    collision.hide_render = True
    collision.display_type = "WIRE"
    collision_collection.objects.link(collision)
    return collision


def add_collision_mesh(
    name: str,
    vertices: list[tuple[float, float, float]],
    faces: list[tuple[int, ...]],
    material_name: str,
    collision_collection: bpy.types.Collection,
    *,
    upward_surface: bool = False,
) -> bpy.types.Object:
    mesh = bpy.data.meshes.new(f"COL_{name}_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.validate(verbose=True)
    mesh.update()
    obj = bpy.data.objects.new(f"COL_{name}", mesh)
    obj["ow_material"] = material_name
    obj["ow_upward_surface"] = upward_surface
    obj.hide_render = True
    obj.display_type = "WIRE"
    collision_collection.objects.link(obj)
    return obj


def add_collision_box(
    name: str,
    location: tuple[float, float, float],
    dimensions: tuple[float, float, float],
    material_name: str,
    collision_collection: bpy.types.Collection,
) -> bpy.types.Object:
    """Create a box proxy without a bottom face buried in the floor."""
    cx, cy, cz = location
    hx, hy, hz = (dimension * 0.5 for dimension in dimensions)
    vertices = [
        (cx - hx, cy - hy, cz - hz),
        (cx - hx, cy + hy, cz - hz),
        (cx + hx, cy + hy, cz - hz),
        (cx + hx, cy - hy, cz - hz),
        (cx - hx, cy - hy, cz + hz),
        (cx - hx, cy + hy, cz + hz),
        (cx + hx, cy + hy, cz + hz),
        (cx + hx, cy - hy, cz + hz),
    ]
    faces = [
        (4, 7, 6, 5),  # top, +Z
        (0, 4, 5, 1),
        (3, 2, 6, 7),
        (0, 3, 7, 4),
        (1, 5, 6, 2),
    ]
    return add_collision_mesh(
        name, vertices, faces, material_name, collision_collection
    )


def add_surface_tile(
    name: str,
    x0: float,
    x1: float,
    y0: float,
    y1: float,
    material: bpy.types.Material,
    visual_collection: bpy.types.Collection,
    collision_collection: bpy.types.Collection,
) -> bpy.types.Object:
    visual_vertices = [
        (x0, y0, 0.012),
        (x1, y0, 0.012),
        (x1, y1, 0.012),
        (x0, y1, 0.012),
    ]
    mesh = bpy.data.meshes.new(f"{name}_Mesh")
    mesh.from_pydata(visual_vertices, [], [(0, 1, 2, 3)])
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    visual_collection.objects.link(obj)
    obj.data.materials.append(material)
    metric_uv(obj, metres_per_tile=2.0)
    add_collision_mesh(
        name,
        [(x0, y0, 0.0), (x1, y0, 0.0),
         (x1, y1, 0.0), (x0, y1, 0.0)],
        [(0, 1, 2, 3)],
        material.name,
        collision_collection,
        upward_surface=True,
    )
    return obj


def add_box(
    name: str,
    location: tuple[float, float, float],
    dimensions: tuple[float, float, float],
    material: bpy.types.Material,
    visual_collection: bpy.types.Collection,
    collision_collection: bpy.types.Collection | None,
    *,
    bevel: float = 0.0,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cube_add(location=location)
    obj = bpy.context.object
    obj.name = name
    obj.dimensions = dimensions
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    move_to(obj, visual_collection)
    obj.data.materials.append(material)
    if collision_collection is not None:
        add_collision_box(
            name, location, dimensions, material.name, collision_collection
        )
    if bevel > 0.0:
        modifier = obj.modifiers.new("Edge softness", "BEVEL")
        modifier.width = bevel
        modifier.segments = 3
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    metric_uv(obj)
    return obj


def add_wedge(
    name: str,
    x0: float,
    x1: float,
    y0: float,
    y1: float,
    low: float,
    high: float,
    material: bpy.types.Material,
    visual_collection: bpy.types.Collection,
    collision_collection: bpy.types.Collection,
) -> bpy.types.Object:
    vertices = [
        (x0, y0, low),
        (x0, y1, low),
        (x1, y1, low),
        (x1, y0, low),
        (x1, y0, high),
        (x1, y1, high),
    ]
    faces = [
        (0, 4, 5, 1),
        (3, 4, 5, 2),
        (0, 3, 2, 1),
        (0, 4, 3),
        (1, 2, 5),
    ]
    mesh = bpy.data.meshes.new(f"{name}_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    visual_collection.objects.link(obj)
    obj.data.materials.append(material)
    add_collision_mesh(
        name,
        [vertices[index] for index in (0, 4, 5, 1)],
        [(0, 1, 2, 3)],
        material.name,
        collision_collection,
        upward_surface=True,
    )
    metric_uv(obj)
    return obj


def add_curved_ramp(
    name: str,
    center_x: float,
    y0: float,
    y1: float,
    length: float,
    height: float,
    material: bpy.types.Material,
    visual_collection: bpy.types.Collection,
    collision_collection: bpy.types.Collection,
) -> bpy.types.Object:
    segments = 14
    vertices = []
    faces = []
    for index in range(segments + 1):
        amount = index / segments
        x = center_x + length * amount
        z = height * (1.0 - math.cos(amount * math.pi * 0.5))
        vertices.extend(((x, y0, z), (x, y1, z)))
    for index in range(segments):
        base = index * 2
        faces.append((base, base + 2, base + 3, base + 1))
    mesh = bpy.data.meshes.new(f"{name}_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    visual_collection.objects.link(obj)
    obj.data.materials.append(material)
    collision = collision_copy(obj, collision_collection, material.name)
    collision["ow_upward_surface"] = True
    metric_uv(obj)
    return obj


def add_stair_set(
    name: str,
    x0: float,
    y0: float,
    y1: float,
    step_depth: float,
    step_height: float,
    step_count: int,
    deck_depth: float,
    material: bpy.types.Material,
    visual_collection: bpy.types.Collection,
    collision_collection: bpy.types.Collection,
) -> bpy.types.Object:
    """Create one continuous stair shell with no intersecting step boxes."""
    vertices: list[tuple[float, float, float]] = []
    faces: list[tuple[int, ...]] = []

    def quad(
        a: tuple[float, float, float],
        b: tuple[float, float, float],
        c: tuple[float, float, float],
        d: tuple[float, float, float],
    ) -> tuple[int, int, int, int]:
        base = len(vertices)
        vertices.extend((a, b, c, d))
        return base, base + 1, base + 2, base + 3

    previous_height = 0.0
    current_x = x0
    for step in range(step_count):
        height = step_height * (step + 1)
        next_x = current_x + step_depth
        faces.append(
            quad(
                (current_x, y0, previous_height),
                (current_x, y0, height),
                (current_x, y1, height),
                (current_x, y1, previous_height),
            )
        )
        faces.append(
            quad(
                (current_x, y0, height),
                (next_x, y0, height),
                (next_x, y1, height),
                (current_x, y1, height),
            )
        )
        previous_height = height
        current_x = next_x

    deck_end = current_x + deck_depth
    faces.append(
        quad(
            (current_x, y0, previous_height),
            (deck_end, y0, previous_height),
            (deck_end, y1, previous_height),
            (current_x, y1, previous_height),
        )
    )
    faces.append(
        quad(
            (deck_end, y0, 0.0),
            (deck_end, y1, 0.0),
            (deck_end, y1, previous_height),
            (deck_end, y0, previous_height),
        )
    )

    # Tile each side in height bands. This gives Blender only convex quads,
    # and every neighbouring edge has matching endpoints for native welding.
    side_sections = [
        (x0 + step * step_depth, x0 + (step + 1) * step_depth, step + 1)
        for step in range(step_count)
    ]
    side_sections.append((current_x, deck_end, step_count))
    for side_y, positive_y in ((y0, False), (y1, True)):
        for section_x0, section_x1, bands in side_sections:
            for band in range(bands):
                low = band * step_height
                high = (band + 1) * step_height
                if positive_y:
                    faces.append(
                        quad(
                            (section_x0, side_y, low),
                            (section_x0, side_y, high),
                            (section_x1, side_y, high),
                            (section_x1, side_y, low),
                        )
                    )
                else:
                    faces.append(
                        quad(
                            (section_x0, side_y, low),
                            (section_x1, side_y, low),
                            (section_x1, side_y, high),
                            (section_x0, side_y, high),
                        )
                    )

    mesh = bpy.data.meshes.new(f"{name}_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.validate(verbose=True)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    visual_collection.objects.link(obj)
    obj.data.materials.append(material)
    metric_uv(obj)
    add_collision_mesh(
        name, vertices, faces, material.name, collision_collection
    )
    return obj


def add_grind_curve(
    name: str,
    points: list[tuple[float, float, float]],
    collection: bpy.types.Collection,
) -> None:
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    spline = curve.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for target, point in zip(spline.points, points):
        target.co = (*point, 1.0)
    obj = bpy.data.objects.new(name, curve)
    collection.objects.link(obj)


def join_visuals(
    visual_collection: bpy.types.Collection,
) -> bpy.types.Object:
    visuals = [
        obj for obj in visual_collection.objects if obj.type == "MESH"
    ]
    bpy.ops.object.select_all(action="DESELECT")
    for obj in visuals:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = visuals[0]
    bpy.ops.object.join()
    joined = bpy.context.object
    joined.name = "OW_Visual_Showcase"
    if joined.data.uv_layers.get("Lightmap") is None:
        joined.data.uv_layers.new(name="Lightmap")
    joined.data.uv_layers.active = joined.data.uv_layers["Lightmap"]
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66.0), island_margin=0.012)
    bpy.ops.object.mode_set(mode="OBJECT")
    return joined


def prepare_shared_lightmap_uvs(
    visual_collection: bpy.types.Collection,
) -> list[bpy.types.Object]:
    """Pack one lightmap atlas while preserving independent mesh objects."""
    visuals = [
        obj for obj in visual_collection.objects if obj.type == "MESH"
    ]
    if not visuals:
        raise RuntimeError("OW_VISUAL contains no mesh objects")
    bpy.ops.object.select_all(action="DESELECT")
    for obj in visuals:
        layer = obj.data.uv_layers.get("Lightmap")
        if layer is None:
            layer = obj.data.uv_layers.new(name="Lightmap")
        obj.data.uv_layers.active = layer
        obj.select_set(True)
    bpy.context.view_layer.objects.active = visuals[0]
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    # Blender's multi-object edit mode packs every selected object's islands
    # into one shared atlas without collapsing authored object identity.
    bpy.ops.uv.smart_project(
        angle_limit=math.radians(66.0), island_margin=0.012
    )
    bpy.ops.object.mode_set(mode="OBJECT")
    return visuals


def add_bake_target(
    materials: list[bpy.types.Material],
    lightmap: bpy.types.Image,
) -> None:
    for material in materials:
        material["ow_lightmap_image"] = lightmap.name
        nodes = material.node_tree.nodes
        for node in nodes:
            node.select = False
        target = nodes.new("ShaderNodeTexImage")
        target.name = "OW_BakedIndirect"
        target.label = "OW Baked Indirect (UV: Lightmap)"
        target.image = lightmap
        target.interpolation = "Linear"
        target.select = True
        nodes.active = target


def persist_lightmap(
    lightmap: bpy.types.Image,
    materials: list[bpy.types.Material],
) -> bpy.types.Image:
    TEXTURE_DIR.mkdir(parents=True, exist_ok=True)
    lightmap_path = TEXTURE_DIR / "LM_BakedIndirect_Showcase.exr"
    exported_name = lightmap.name
    lightmap.filepath_raw = str(lightmap_path)
    lightmap.file_format = "OPEN_EXR"
    lightmap.save()

    # Replace the transient bake target with a normal file-backed image.
    # This makes both the editable .blend and later standalone exports
    # deterministic after Blender has been closed and reopened.
    lightmap.name = f"{exported_name}_BakeBuffer"
    loaded = bpy.data.images.load(str(lightmap_path), check_existing=False)
    loaded.name = exported_name
    loaded.colorspace_settings.name = "Non-Color"
    for material in materials:
        material["ow_lightmap_image"] = loaded.name
        target = material.node_tree.nodes.get("OW_BakedIndirect")
        if target is None:
            raise RuntimeError(
                f"material {material.name!r} lost its bake target"
            )
        target.image = loaded
    bpy.data.images.remove(lightmap)
    return loaded


def configure_bake(
    targets: bpy.types.Object | list[bpy.types.Object],
) -> None:
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    # Blender 5 removed the old Cycles bake path. Eevee's light-probe bake is
    # not an image bake, so use the supported workbench-independent emission
    # bake setup when Cycles is unavailable and Cycles otherwise.
    try:
        scene.render.engine = "CYCLES"
        scene.cycles.device = "CPU"
        scene.cycles.samples = 24
        scene.cycles.max_bounces = 4
        scene.cycles.diffuse_bounces = 3
    except Exception:
        scene.render.engine = "BLENDER_EEVEE"
    scene.render.bake.margin = 10
    scene.render.bake.use_clear = True
    scene.render.bake.use_pass_direct = False
    scene.render.bake.use_pass_indirect = True
    scene.render.bake.use_pass_color = False
    bpy.ops.object.select_all(action="DESELECT")
    objects = targets if isinstance(targets, list) else [targets]
    if not objects:
        raise RuntimeError("indirect bake has no target objects")
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]


def build_scene() -> None:
    reset_scene()
    visual = make_collection("OW_VISUAL")
    collision = make_collection("OW_COLLISION")
    grinds = make_collection("OW_GRIND")

    concrete_tex = make_texture(
        "T_Concrete",
        (0.34, 0.36, 0.39),
        (0.48, 0.50, 0.53),
        12,
    )
    orange_tex = make_texture(
        "T_OrangePaint",
        (0.72, 0.16, 0.035),
        (0.95, 0.38, 0.055),
        8,
        stripe=True,
    )
    brick_tex = make_texture(
        "T_DarkBrick",
        (0.11, 0.055, 0.045),
        (0.27, 0.10, 0.065),
        18,
    )
    metal_tex = make_texture(
        "T_Metal",
        (0.19, 0.22, 0.25),
        (0.34, 0.38, 0.42),
        24,
    )
    cyan_tex = make_texture(
        "T_CyanEmitter",
        (0.02, 0.85, 1.0),
        (0.10, 1.0, 0.94),
        4,
    )
    magenta_tex = make_texture(
        "T_OrangeEmitter",
        (1.0, 0.08, 0.015),
        (1.0, 0.42, 0.025),
        4,
    )
    asphalt_tex = make_texture(
        "T_Asphalt", (0.055, 0.06, 0.07), (0.12, 0.13, 0.14), 28
    )
    wood_tex = make_texture(
        "T_WoodRamp", (0.28, 0.105, 0.035), (0.58, 0.28, 0.08), 10,
        stripe=True,
    )
    tile_tex = make_texture(
        "T_CeramicTile", (0.14, 0.38, 0.68), (0.72, 0.88, 0.96), 8
    )
    grass_tex = make_texture(
        "T_Grass", (0.035, 0.18, 0.045), (0.12, 0.38, 0.09), 32
    )
    ice_tex = make_texture(
        "T_Ice", (0.38, 0.68, 0.86), (0.72, 0.92, 1.0), 6
    )
    glass_tex = make_texture(
        "T_Glass", (0.08, 0.35, 0.48), (0.22, 0.68, 0.78), 4,
        alpha=0.30,
    )
    fence_tex = make_texture(
        "T_FenceCutout", (0.08, 0.09, 0.10), (0.72, 0.76, 0.80), 8,
        cutout_grid=True,
    )
    hazard_tex = make_texture(
        "T_Hazard", (0.55, 0.015, 0.01), (1.0, 0.72, 0.02), 8,
        stripe=True,
    )

    normal_concrete = make_normal_texture("N_Concrete", 0.13, 18.0)
    normal_wood = make_normal_texture("N_Wood", 0.18, 7.0)
    normal_metal = make_normal_texture("N_Metal", 0.035, 24.0)
    normal_grass = make_normal_texture("N_Grass", 0.28, 21.0)
    normal_tile = make_normal_texture("N_Tile", 0.08, 8.0)
    normal_flat = make_normal_texture("N_Flat", 0.0, 1.0)

    orm_concrete = make_orm_texture("ORM_Concrete", 0.84, 0.0)
    orm_rough = make_orm_texture("ORM_Rough", 0.94, 0.0, ao=0.92)
    orm_wood = make_orm_texture("ORM_Wood", 0.72, 0.0)
    orm_metal = make_orm_texture("ORM_Metal", 0.25, 0.88)
    orm_tile = make_orm_texture("ORM_Tile", 0.32, 0.0)
    orm_grass = make_orm_texture("ORM_Grass", 0.96, 0.0, ao=0.82)
    orm_glass = make_orm_texture("ORM_Glass", 0.06, 0.0)
    orm_ice = make_orm_texture("ORM_Ice", 0.08, 0.0)

    concrete = make_material(
        "Concrete", concrete_tex, (0.72, 0.75, 0.79), 0.86,
        normal_image=normal_concrete, orm_image=orm_concrete,
        audio_surface=4, physics_surface=2, surface_pattern=11,
    )
    orange = make_material(
        "OrangeRamp", orange_tex, (1.0, 0.52, 0.12), 0.68,
        normal_image=normal_wood, orm_image=orm_wood,
        audio_surface=6, physics_surface=1, surface_pattern=10,
    )
    brick = make_material(
        "Brick", brick_tex, (0.62, 0.24, 0.13), 0.82, flags=1 | 4,
        normal_image=normal_concrete, orm_image=orm_rough,
        audio_surface=66, physics_surface=2, surface_pattern=12,
    )
    metal = make_material(
        "GrindMetal", metal_tex, (0.72, 0.78, 0.84), 0.24, flags=1 | 2,
        normal_image=normal_metal, orm_image=orm_metal, metallic=0.88,
        audio_surface=11, physics_surface=1,
    )
    cyan = make_material(
        "CyanBounce", cyan_tex, (0.04, 0.94, 1.0), 0.18,
        emissive=4.5, normal_image=normal_flat, orm_image=orm_glass,
        emissive_image=cyan_tex, collision_enabled=False,
    )
    amber = make_material(
        "AmberBounce", magenta_tex, (1.0, 0.21, 0.025), 0.18,
        emissive=5.0, normal_image=normal_flat, orm_image=orm_glass,
        emissive_image=magenta_tex, collision_enabled=False,
    )
    asphalt = make_material(
        "TEST_01_Asphalt_Rough", asphalt_tex, (0.72, 0.74, 0.78), 0.94,
        normal_image=normal_concrete, orm_image=orm_rough,
        audio_surface=2, physics_surface=2, surface_pattern=7,
    )
    polished = make_material(
        "TEST_02_Concrete_Polished", concrete_tex, (0.92, 0.92, 0.92), 0.28,
        normal_image=normal_concrete, orm_image=orm_tile,
        audio_surface=3, physics_surface=1,
    )
    wood = make_material(
        "TEST_03_Wood_Ramp", wood_tex, (0.95, 0.76, 0.42), 0.72,
        normal_image=normal_wood, orm_image=orm_wood,
        audio_surface=6, physics_surface=1, surface_pattern=10,
    )
    metal_sheet = make_material(
        "TEST_04_Metal_Sheet", metal_tex, (0.78, 0.84, 0.90), 0.25,
        normal_image=normal_metal, orm_image=orm_metal, metallic=0.88,
        audio_surface=31, physics_surface=1,
    )
    tile = make_material(
        "TEST_05_Ceramic_Tile", tile_tex, (0.78, 0.90, 1.0), 0.32,
        normal_image=normal_tile, orm_image=orm_tile,
        audio_surface=63, physics_surface=1, surface_pattern=4,
    )
    grass = make_material(
        "TEST_06_Grass_Slow", grass_tex, (0.58, 0.88, 0.52), 0.96,
        normal_image=normal_grass, orm_image=orm_grass,
        audio_surface=10, physics_surface=3, surface_pattern=7,
    )
    ice = make_material(
        "TEST_07_Ice_Slippery", ice_tex, (0.72, 0.92, 1.0), 0.08,
        normal_image=normal_flat, orm_image=orm_ice,
        audio_surface=72, physics_surface=4,
    )
    stairs_material = make_material(
        "TEST_Stairs", concrete_tex, (0.78, 0.80, 0.84), 0.84,
        normal_image=normal_concrete, orm_image=orm_concrete,
        audio_surface=53, physics_surface=8, surface_pattern=11,
    )
    glass = make_material(
        "TEST_Glass_Transparent", glass_tex, (0.65, 0.92, 1.0), 0.06,
        normal_image=normal_flat, orm_image=orm_glass,
        alpha_mode=2, audio_surface=51, physics_surface=1,
    )
    fence = make_material(
        "TEST_Fence_Cutout", fence_tex, (0.82, 0.86, 0.90), 0.35,
        normal_image=normal_metal, orm_image=orm_metal, metallic=0.72,
        alpha_mode=1, alpha_cutoff=0.5, audio_surface=68,
    )
    hazard = make_material(
        "TEST_Hazard_InstantBail", hazard_tex, (1.0, 0.32, 0.05), 0.82,
        normal_image=normal_concrete, orm_image=orm_rough,
        audio_surface=4, physics_surface=9,
    )
    materials = [
        concrete, orange, brick, metal, cyan, amber, asphalt, polished,
        wood, metal_sheet, tile, grass, ice, stairs_material, glass,
        fence, hazard,
    ]

    add_box(
        "PlazaFloor", (0.0, 0.0, -0.18), (48.0, 36.0, 0.36),
        concrete, visual, None, bevel=0.08
    )
    # The center lane is physically partitioned by material. The surrounding
    # plaza collision leaves a real hole rather than sitting underneath the
    # test tiles as a competing coplanar surface.
    for name, x0, x1, y0, y1 in (
        ("FloorLeft", -24.0, -3.5, -18.0, 18.0),
        ("FloorRight", 3.5, 24.0, -18.0, 18.0),
        ("FloorEntry", -3.5, 3.5, -18.0, -16.0),
        ("FloorExit", -3.5, 3.5, 12.0, 18.0),
    ):
        add_collision_mesh(
            name,
            [(x0, y0, 0.0), (x1, y0, 0.0),
             (x1, y1, 0.0), (x0, y1, 0.0)],
            [(0, 1, 2, 3)],
            concrete.name,
            collision,
            upward_surface=True,
        )
    for index, material in enumerate(
        (asphalt, polished, wood, metal_sheet, tile, grass, ice)
    ):
        y0 = -16.0 + index * 4.0
        add_surface_tile(
            f"SurfaceLane_{index + 1:02d}_{material.name}",
            -3.5, 3.5, y0, y0 + 4.0,
            material, visual, collision,
        )
    add_wedge(
        "OrangeBank", -19.0, -11.0, -7.0, 5.0, 0.0, 3.8,
        orange, visual, collision
    )
    add_curved_ramp(
        "CurvedWall", 11.0, -7.0, 5.0, 8.0, 4.5,
        orange, visual, collision
    )
    add_box(
        "BackWall", (0.0, 16.4, 3.0), (48.0, 1.0, 6.0),
        brick, visual, collision, bevel=0.14
    )
    add_box(
        "CanopyRoof", (-10.0, 10.0, 4.8), (17.0, 10.0, 0.55),
        metal, visual, collision, bevel=0.18
    )
    for x in (-17.5, -2.5):
        add_box(
            f"CanopyPost_{x}", (x, 13.2, 2.4), (0.55, 0.55, 4.8),
            metal, visual, collision, bevel=0.08
        )
    add_box(
        "CyanBouncePanel", (-16.8, 14.7, 2.25), (0.18, 4.5, 3.6),
        cyan, visual, None, bevel=0.04
    )
    add_box(
        "AmberBouncePanel", (-3.1, 14.7, 2.25), (0.18, 4.5, 3.6),
        amber, visual, None, bevel=0.04
    )

    # One welded stair shell replaces five overlapping cubes plus an
    # intersecting deck, eliminating hidden internal contact planes.
    add_stair_set(
        "StairSet", 7.0, 6.5, 11.5, 1.05, 0.36, 5, 3.0,
        stairs_material, visual, collision
    )

    # Cylindrical rail visuals; conservative box collision is authored
    # independently, while the curve below is the actual grind contract.
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=20, radius=0.075, depth=8.0,
        location=(-8.0, 6.2, 0.92), rotation=(0.0, math.pi * 0.5, 0.0)
    )
    rail = bpy.context.object
    rail.name = "CentralRail"
    move_to(rail, visual)
    rail.data.materials.append(metal)
    metric_uv(rail)
    rail_collision = add_box(
        "RailCollisionVisualHidden", (-8.0, 6.2, 0.92), (8.0, 0.15, 0.15),
        metal, visual, collision
    )
    # The collision helper created a visible box too; remove only that
    # visual representation, preserving its independent collision duplicate.
    bpy.data.objects.remove(rail_collision, do_unlink=True)
    for x in (-11.4, -4.6):
        add_box(
            f"RailPost_{x}", (x, 6.2, 0.46), (0.13, 0.13, 0.92),
            metal, visual, collision
        )
    add_grind_curve(
        "CentralRailGrind",
        [(-12.0, 6.2, 1.01), (-4.0, 6.2, 1.01)],
        grinds,
    )

    # A low manual pad makes collision and texture continuity easy to test.
    add_box(
        "ManualPad", (-10.0, -7.5, 0.28), (7.5, 4.0, 0.56),
        brick, visual, collision, bevel=0.10
    )
    add_box(
        "TransparentGlassWall", (8.0, -11.0, 2.0), (0.18, 6.0, 4.0),
        glass, visual, collision, bevel=0.03
    )
    add_box(
        "AlphaCutoutFence", (13.0, -11.0, 2.0), (0.12, 6.0, 4.0),
        fence, visual, collision
    )
    add_box(
        "InstantBailHazard", (10.0, -2.0, 0.12), (5.0, 4.0, 0.24),
        hazard, visual, collision, bevel=0.04
    )

    joined = join_visuals(visual)
    lightmap = bpy.data.images.new(
        "LM_BakedIndirect_Showcase", width=512, height=512, alpha=True
    )
    lightmap.colorspace_settings.name = "Non-Color"
    add_bake_target(materials, lightmap)

    world = bpy.data.worlds.new("OW_ShowcaseWorld")
    bpy.context.scene.world = world
    world.use_nodes = True
    background = world.node_tree.nodes.get("Background")
    background.inputs["Color"].default_value = (0.08, 0.12, 0.20, 1.0)
    background.inputs["Strength"].default_value = 0.32

    # Static lights illuminate the bake only. The runtime never imports
    # Blender lights; its moving sun/moon supplies the direct component.
    for name, location, color, energy in (
        ("BakeFillWarm", (10.0, -5.0, 10.0), (1.0, 0.48, 0.18), 1500.0),
        ("BakeFillCool", (-14.0, 8.0, 7.0), (0.08, 0.55, 1.0), 1250.0),
    ):
        data = bpy.data.lights.new(name, "AREA")
        data.energy = energy
        data.color = color
        data.shape = "DISK"
        data.size = 8.0
        light = bpy.data.objects.new(name, data)
        bpy.context.scene.collection.objects.link(light)
        light.location = location
        direction = Vector((0.0, 5.0, 0.0)) - light.location
        light.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()

    spawn = bpy.data.objects.new("OW_SPAWN", None)
    bpy.context.scene.collection.objects.link(spawn)
    spawn.location = (0.0, -17.0, 0.75)
    spawn.empty_display_type = "ARROWS"
    spawn.empty_display_size = 1.2
    spawn["ow_heading_radians"] = 0.0

    scene = bpy.context.scene
    scene["ow_map_name"] = "blender_material_lab"
    scene["ow_sky_zenith"] = (0.09, 0.34, 0.72)
    scene["ow_sky_horizon"] = (0.58, 0.78, 0.98)
    scene["ow_sky_nadir"] = (0.18, 0.25, 0.34)
    scene["ow_cycle_seconds"] = 96.0
    scene["ow_start_hour"] = 9.0
    scene["ow_orbit_azimuth"] = 0.62

    configure_bake(joined)
    print("Starting Cycles indirect-light bake...")
    bpy.ops.object.bake(type="DIFFUSE")
    values = list(lightmap.pixels)
    rgb = [values[index] for index in range(0, len(values), 4)]
    print(
        "Bake result:",
        f"minimum={min(rgb):.4f}",
        f"maximum={max(rgb):.4f}",
        f"mean={sum(rgb) / len(rgb):.4f}",
    )
    lightmap = persist_lightmap(lightmap, materials)

    BLEND_PATH.parent.mkdir(parents=True, exist_ok=True)
    collision.hide_viewport = False
    grinds.hide_viewport = False
    bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH))
    export_scene(PACKAGE_PATH)


if __name__ == "__main__":
    build_scene()
