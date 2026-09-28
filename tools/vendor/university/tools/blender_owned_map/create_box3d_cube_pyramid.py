"""Generate the multi-root Box3D cube-pyramid SKATEOBJ sample."""

from __future__ import annotations

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
BLEND_PATH = OBJECT_ROOT / "box3d_cube_pyramid.blend"
PACKAGE_PATH = OBJECT_ROOT / "box3d_cube_pyramid.skateobj"


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
    density: float = 100.0,
    friction: float = 0.65,
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
    settings.box3d_collision_shape = "BOX"
    settings.box3d_density = density
    settings.box3d_friction = friction
    settings.box3d_restitution = 0.015
    settings.box3d_linear_damping = 0.04
    settings.box3d_angular_damping = 0.18
    settings.box3d_gravity_scale = 1.0
    settings.box3d_enable_sleep = True
    settings.box3d_initially_awake = True
    obj["ow_upward_surface"] = False


def add_box(
    name: str,
    location: tuple[float, float, float],
    dimensions: tuple[float, float, float],
    material: bpy.types.Material,
    physics_type: str,
    *,
    density: float = 100.0,
    friction: float = 0.65,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=location)
    obj = bpy.context.object
    obj.name = name
    obj.dimensions = dimensions
    bpy.ops.object.transform_apply(
        location=False, rotation=False, scale=True
    )
    obj.data.materials.append(material)
    assign_owned_object(
        obj,
        physics_type=physics_type,
        density=density,
        friction=friction,
    )
    return obj


def build() -> None:
    OBJECT_ROOT.mkdir(parents=True, exist_ok=True)
    addon.register()
    try:
        reset_scene()
        if bpy.ops.skate_map.prepare_scene() != {"FINISHED"}:
            raise RuntimeError("unable to prepare Box3D SKATEOBJ scene")

        base_material = bpy.data.materials.new("Box3D_Base")
        base_material.diffuse_color = (0.09, 0.11, 0.15, 1.0)
        base_material.owned_world.roughness = 0.68
        cube_material = bpy.data.materials.new("Box3D_Cubes")
        cube_material.diffuse_color = (0.96, 0.30, 0.08, 1.0)
        cube_material.owned_world.roughness = 0.44

        # The prefab pivot is OW_SPAWN at zero. The base therefore has its
        # bottom exactly at placement height when spawned by the map editor.
        add_box(
            "Box3D_Static_Base",
            (0.0, 0.0, 0.15),
            (6.0, 4.0, 0.3),
            base_material,
            "BOX3D_STATIC",
            friction=0.82,
        )

        cube_size = 1.0
        horizontal_step = 1.08
        first_center_height = 1.40
        vertical_step = 1.12
        cube_number = 0
        for row_index, count in enumerate((4, 3, 2, 1)):
            height = first_center_height + row_index * vertical_step
            first_x = -0.5 * (count - 1) * horizontal_step
            for column in range(count):
                cube_number += 1
                x = first_x + column * horizontal_step
                # A tiny depth stagger avoids a perfectly constrained 2D
                # arrangement while keeping the 4/3/2/1 silhouette clear.
                depth = 0.025 if cube_number % 2 == 0 else -0.025
                add_box(
                    f"Box3D_Cube_{cube_number:02d}",
                    (x, depth, height),
                    (cube_size, cube_size, cube_size),
                    cube_material,
                    "BOX3D_DYNAMIC",
                    density=180.0,
                    friction=0.68,
                )

        scene = bpy.context.scene
        scene["ow_map_name"] = "Box3D Cube Pyramid"
        spawn = bpy.data.objects.get("OW_SPAWN")
        if spawn is None:
            raise RuntimeError("prepared scene has no OW_SPAWN pivot")
        spawn.location = (0.0, 0.0, 0.0)

        bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH))
        export_scene(PACKAGE_PATH, force_rebuild=True)
        print(
            "SKATEOBJ_BOX3D_PYRAMID_OK "
            f"roots=11 dynamic_cubes=10 {BLEND_PATH} {PACKAGE_PATH}"
        )
    finally:
        addon.unregister()


if __name__ == "__main__":
    build()
