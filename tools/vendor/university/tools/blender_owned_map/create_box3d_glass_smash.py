"""Generate a deterministic pre-fractured Box3D glass-pane SKATEOBJ."""

from __future__ import annotations

import math
from pathlib import Path
import sys

import bpy


ROOT = Path(__file__).resolve().parents[2]
TOOL_ROOT = Path(__file__).resolve().parent
if str(TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOL_ROOT))

import owned_world_material_addon as addon  # noqa: E402
from owned_world_material_addon.exporter import export_scene  # noqa: E402


OBJECT_ROOT = ROOT / "objects"
BLEND_PATH = OBJECT_ROOT / "box3d_glass_smash.blend"
PACKAGE_PATH = OBJECT_ROOT / "box3d_glass_smash.skateobj"
BREAK_GROUP = 1


def make_uniform_rgba(
    name: str, rgba: tuple[float, float, float, float]
) -> bpy.types.Image:
    image = bpy.data.images.new(name, width=1, height=1, alpha=True)
    image.colorspace_settings.name = "sRGB"
    image.pixels.foreach_set(rgba)
    image.update()
    return image


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images):
        for item in list(block):
            block.remove(item)


def assign_owned_object(
    obj: bpy.types.Object,
    *,
    physics_type: str,
    shape: str = "BOX",
    density: float = 100.0,
    friction: float = 0.55,
) -> None:
    obj.data.uv_layers.new(name="UVMap")
    obj.data.uv_layers.new(name="Lightmap", do_init=True)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if bpy.ops.skate_map.assign_selected(role="VISUAL") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign visual role to {obj.name}")
    obj.select_set(True)
    if bpy.ops.skate_map.assign_selected(role="COLLISION") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign collision role to {obj.name}")
    settings = obj.owned_world_physics
    settings.physics_type = physics_type
    settings.box3d_collision_shape = shape
    settings.box3d_density = density
    settings.box3d_friction = friction
    settings.box3d_restitution = 0.02
    settings.box3d_linear_damping = 0.08
    settings.box3d_angular_damping = 0.16
    settings.box3d_enable_sleep = True
    obj["ow_upward_surface"] = False


def add_shard(
    number: int,
    triangle: tuple[
        tuple[float, float],
        tuple[float, float],
        tuple[float, float],
    ],
    material: bpy.types.Material,
    hidden_edge_material: bpy.types.Material,
) -> bpy.types.Object:
    center_x = sum(point[0] for point in triangle) / 3.0
    center_z = sum(point[1] for point in triangle) / 3.0
    half_depth = 0.035
    local = [
        (point[0] - center_x, point[1] - center_z)
        for point in triangle
    ]
    vertices = [
        (x, -half_depth, z) for x, z in local
    ] + [
        (x, half_depth, z) for x, z in local
    ]
    faces = [
        (0, 2, 1),
        (3, 4, 5),
        (0, 1, 4, 3),
        (1, 2, 5, 4),
        (2, 0, 3, 5),
    ]
    mesh = bpy.data.meshes.new(f"GlassShardMesh_{number:02d}")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(f"Glass_Shard_{number:02d}", mesh)
    bpy.context.collection.objects.link(obj)
    obj.location = (center_x, 0.0, center_z)
    obj.data.materials.append(material)
    obj.data.materials.append(hidden_edge_material)
    # The first two polygons are the pane-facing triangles. The remaining
    # three close the collision prism but must not reveal the pre-fracture
    # pattern through alpha blending while the pane is intact.
    for polygon in obj.data.polygons[2:]:
        polygon.material_index = 1
    assign_owned_object(
        obj,
        physics_type="BOX3D_DYNAMIC",
        shape="CONVEX_HULL",
        density=42.0,
        friction=0.28,
    )
    settings = obj.owned_world_physics
    settings.box3d_gravity_scale = 0.0
    settings.box3d_initially_awake = False
    settings.box3d_break_group = BREAK_GROUP
    settings.box3d_break_speed_threshold = 2.15
    settings.box3d_break_impulse_scale = 0.38
    settings.box3d_break_angular_impulse = 0.055
    settings.box3d_break_gravity_scale = 1.0
    return obj


def ray_to_rectangle(
    angle: float, half_width: float, half_height: float
) -> tuple[float, float]:
    dx = math.cos(angle)
    dz = math.sin(angle)
    scales = []
    if abs(dx) > 1.0e-6:
        scales.append(half_width / abs(dx))
    if abs(dz) > 1.0e-6:
        scales.append(half_height / abs(dz))
    scale = min(scales)
    return (dx * scale, dz * scale)


def fracture_triangles() -> list[tuple[tuple[float, float], ...]]:
    half_width = 2.7
    half_height = 1.6
    center_z = 2.05
    spoke_count = 16
    center = (0.08, center_z - 0.04)
    inner: list[tuple[float, float]] = []
    outer: list[tuple[float, float]] = []
    for index in range(spoke_count):
        angle = 2.0 * math.pi * index / spoke_count
        radius = 0.48 + 0.12 * ((index * 7) % 5) / 4.0
        inner.append(
            (
                center[0] + math.cos(angle) * radius,
                center[1] + math.sin(angle) * radius,
            )
        )
        edge = ray_to_rectangle(angle, half_width, half_height)
        outer.append((edge[0], center_z + edge[1]))

    triangles: list[tuple[tuple[float, float], ...]] = []
    for index in range(spoke_count):
        next_index = (index + 1) % spoke_count
        triangles.append((center, inner[index], inner[next_index]))
        if index % 2 == 0:
            triangles.append(
                (inner[index], outer[index], outer[next_index])
            )
            triangles.append(
                (inner[index], outer[next_index], inner[next_index])
            )
        else:
            triangles.append(
                (inner[index], outer[index], inner[next_index])
            )
            triangles.append(
                (inner[next_index], outer[index], outer[next_index])
            )
    return triangles


def build() -> None:
    OBJECT_ROOT.mkdir(parents=True, exist_ok=True)
    addon.register()
    try:
        reset_scene()
        if bpy.ops.skate_map.prepare_scene() != {"FINISHED"}:
            raise RuntimeError("unable to prepare glass SKATEOBJ scene")

        glass = bpy.data.materials.new("Breakable_Glass")
        # Uniform 1x1 RGBA swatches select the imported-material path without
        # adding any visible texture pattern. That path supports real opacity;
        # the procedural path deliberately adds surface noise and is opaque.
        glass_swatch = make_uniform_rgba(
            "Glass_Uniform_RGBA", (1.0, 1.0, 1.0, 0.28)
        )
        hidden_swatch = make_uniform_rgba(
            "Glass_Hidden_RGBA", (1.0, 1.0, 1.0, 0.0)
        )

        glass.diffuse_color = (0.78, 0.91, 0.96, 0.17)
        glass["ow_albedo_image"] = glass_swatch.name
        glass.owned_world.albedo_image = glass_swatch
        glass.owned_world.alpha_mode = "2"
        glass.owned_world.roughness = 0.04
        glass.owned_world.metallic = 0.0
        glass.owned_world.friction = 0.25
        glass.owned_world.restitution = 0.02

        hidden_edges = bpy.data.materials.new("Glass_Hidden_Shard_Edges")
        hidden_edges.diffuse_color = (0.0, 0.0, 0.0, 0.0)
        hidden_edges["ow_albedo_image"] = hidden_swatch.name
        hidden_edges.owned_world.albedo_image = hidden_swatch
        hidden_edges.owned_world.alpha_mode = "2"
        hidden_edges.owned_world.roughness = 1.0
        hidden_edges.owned_world.metallic = 0.0

        shard_count = 0
        for shard_count, triangle in enumerate(
            fracture_triangles(), start=1
        ):
            add_shard(shard_count, triangle, glass, hidden_edges)

        scene = bpy.context.scene
        scene["ow_map_name"] = "Box3D Glass Smash"
        spawn = bpy.data.objects.get("OW_SPAWN")
        if spawn is None:
            raise RuntimeError("prepared scene has no OW_SPAWN pivot")
        spawn.location = (0.0, 0.0, 0.0)

        bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH))
        export_scene(PACKAGE_PATH, force_rebuild=True)
        print(
            "SKATEOBJ_BOX3D_GLASS_OK "
            f"shards={shard_count} frame_parts=0 "
            "visible_patterns=0 uniform_rgba_swatches=2 "
            f"{BLEND_PATH} {PACKAGE_PATH}"
        )
    finally:
        addon.unregister()


if __name__ == "__main__":
    build()
