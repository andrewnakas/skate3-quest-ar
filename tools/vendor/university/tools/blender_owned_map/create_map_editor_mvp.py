"""Create the small independently-editable map-editor validation map."""

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


BLEND_PATH = ROOT / "maps" / "map_editor_mvp.blend"
PACKAGE_PATH = ROOT / "maps" / "map_editor_mvp.skate"


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images):
        for item in list(block):
            block.remove(item)


def add_editable_box(
    name: str,
    location: tuple[float, float, float],
    dimensions: tuple[float, float, float],
    material: bpy.types.Material,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=location)
    obj = bpy.context.object
    obj.name = name
    obj.dimensions = dimensions
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.data.materials.append(material)
    if obj.data.uv_layers.get("UVMap") is None:
        obj.data.uv_layers.new(name="UVMap")
    if obj.data.uv_layers.get("Lightmap") is None:
        obj.data.uv_layers.new(name="Lightmap", do_init=True)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if bpy.ops.skate_map.assign_selected(role="VISUAL") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign {name} as visual")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if bpy.ops.skate_map.assign_selected(role="COLLISION") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign {name} as collision")
    obj["ow_upward_surface"] = False
    return obj


def add_editable_floor(
    name: str,
    size: float,
    material: bpy.types.Material,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_plane_add(size=size, location=(0.0, 0.0, 0.0))
    obj = bpy.context.object
    obj.name = name
    obj.data.materials.append(material)
    if obj.data.uv_layers.get("UVMap") is None:
        obj.data.uv_layers.new(name="UVMap")
    if obj.data.uv_layers.get("Lightmap") is None:
        obj.data.uv_layers.new(name="Lightmap", do_init=True)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if bpy.ops.skate_map.assign_selected(role="VISUAL") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign {name} as visual")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if bpy.ops.skate_map.assign_selected(role="COLLISION") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign {name} as collision")
    obj["ow_upward_surface"] = True
    return obj


def add_parented_corner_grind(
    parent: bpy.types.Object,
    half_x: float,
    half_y: float,
    top_z: float,
) -> bpy.types.Object:
    curve = bpy.data.curves.new(f"{parent.name}_CornerGrind", "CURVE")
    curve.dimensions = "3D"
    spline = curve.splines.new("POLY")
    spline.points.add(3)
    for point, coordinate in zip(
        spline.points,
        (
            (-half_x, -half_y, top_z, 1.0),
            (half_x, -half_y, top_z, 1.0),
            (half_x, half_y, top_z, 1.0),
            (-half_x, half_y, top_z, 1.0),
        ),
        strict=True,
    ):
        point.co = coordinate
    spline.use_cyclic_u = True
    obj = bpy.data.objects.new(curve.name, curve)
    bpy.context.scene.collection.objects.link(obj)
    obj.parent = parent
    obj.location = (0.0, 0.0, 0.0)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if bpy.ops.skate_map.assign_selected(role="GRIND") != {"FINISHED"}:
        raise RuntimeError(f"unable to assign {obj.name} as grind")
    return obj


def build() -> None:
    addon.register()
    try:
        reset_scene()
        if bpy.ops.skate_map.prepare_scene() != {"FINISHED"}:
            raise RuntimeError("unable to prepare owned-world scene")

        concrete = bpy.data.materials.new("EditorConcrete")
        concrete.diffuse_color = (0.24, 0.29, 0.34, 1.0)
        concrete.owned_world.roughness = 0.82
        orange = bpy.data.materials.new("EditorOrange")
        orange.diffuse_color = (0.95, 0.26, 0.06, 1.0)
        orange.owned_world.roughness = 0.58
        cyan = bpy.data.materials.new("EditorCyan")
        cyan.diffuse_color = (0.04, 0.72, 0.88, 1.0)
        cyan.owned_world.roughness = 0.46

        add_editable_floor("EditorGround", 24.0, concrete)
        orange_block = add_editable_box(
            "MoveMe_OrangeBlock",
            (-2.5, 2.5, 1.0),
            (3.0, 3.0, 2.0),
            orange,
        )
        cyan_ledge = add_editable_box(
            "MoveMe_CyanLedge",
            (3.5, 1.0, 0.6),
            (5.0, 1.5, 1.2),
            cyan,
        )
        add_parented_corner_grind(
            orange_block, half_x=1.5, half_y=1.5, top_z=1.0
        )
        add_parented_corner_grind(
            cyan_ledge, half_x=2.5, half_y=0.75, top_z=0.6
        )

        spawn = bpy.data.objects.get("OW_SPAWN")
        if spawn is None:
            raise RuntimeError("prepared scene has no OW_SPAWN")
        spawn.location = (0.0, -6.0, 0.75)
        spawn["ow_heading_radians"] = 0.0

        scene = bpy.context.scene
        scene["ow_map_name"] = "map_editor_mvp"
        scene["ow_sky_zenith"] = (0.04, 0.10, 0.18)
        scene["ow_sky_horizon"] = (0.38, 0.58, 0.72)
        scene["ow_sky_nadir"] = (0.08, 0.10, 0.13)
        scene["ow_cycle_seconds"] = 120.0
        scene["ow_start_hour"] = 11.0
        scene["ow_end_hour"] = 17.0
        scene["ow_orbit_azimuth"] = 0.45

        bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH))
        export_scene(PACKAGE_PATH, force_rebuild=True)
        print(f"MAP_EDITOR_MVP_OK {BLEND_PATH} {PACKAGE_PATH}")
    finally:
        addon.unregister()


if __name__ == "__main__":
    build()
