"""Create the spawn-menu's small grindable SKATEOBJ validation prefab."""

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
BLEND_PATH = OBJECT_ROOT / "test_grind_ledge.blend"
PACKAGE_PATH = OBJECT_ROOT / "test_grind_ledge.skateobj"


def reset_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images):
        for item in list(block):
            block.remove(item)


def build() -> None:
    OBJECT_ROOT.mkdir(parents=True, exist_ok=True)
    addon.register()
    try:
        reset_scene()
        if bpy.ops.skate_map.prepare_scene() != {"FINISHED"}:
            raise RuntimeError("unable to prepare SKATEOBJ scene")

        material = bpy.data.materials.new("SpawnLedgePaintedMetal")
        material.diffuse_color = (0.10, 0.52, 0.92, 1.0)
        material.owned_world.roughness = 0.42

        bpy.ops.mesh.primitive_cube_add(
            size=1.0, location=(0.0, 0.0, 0.5)
        )
        ledge = bpy.context.object
        ledge.name = "TestGrindLedge"
        ledge.dimensions = (4.0, 1.0, 1.0)
        # Bake the center offset into the mesh so the prefab root/pivot is at
        # the bottom. Surface placement can then put the object fully above
        # ground instead of burying its lower half.
        bpy.ops.object.transform_apply(
            location=True, rotation=False, scale=True
        )
        ledge.data.materials.append(material)
        ledge.data.uv_layers.new(name="UVMap")
        ledge.data.uv_layers.new(name="Lightmap", do_init=True)
        bpy.ops.object.select_all(action="DESELECT")
        ledge.select_set(True)
        bpy.context.view_layer.objects.active = ledge
        if bpy.ops.skate_map.assign_selected(role="VISUAL") != {"FINISHED"}:
            raise RuntimeError("unable to assign visual role")
        ledge.select_set(True)
        if bpy.ops.skate_map.assign_selected(
            role="COLLISION"
        ) != {"FINISHED"}:
            raise RuntimeError("unable to assign collision role")
        ledge["ow_upward_surface"] = False

        curve = bpy.data.curves.new("TestGrindLedge_TopGrind", "CURVE")
        curve.dimensions = "3D"
        spline = curve.splines.new("POLY")
        spline.points.add(1)
        # The ledge root is at its bottom. Put the path on the exposed long
        # top corner (runtime Y=1, Z=+0.5), not through the middle of the top
        # face where native grind acquisition is blocked by collision.
        spline.points[0].co = (-2.0, -0.5, 1.0, 1.0)
        spline.points[1].co = (2.0, -0.5, 1.0, 1.0)
        grind = bpy.data.objects.new(curve.name, curve)
        bpy.context.scene.collection.objects.link(grind)
        grind.parent = ledge
        grind.location = (0.0, 0.0, 0.0)
        bpy.ops.object.select_all(action="DESELECT")
        grind.select_set(True)
        bpy.context.view_layer.objects.active = grind
        if bpy.ops.skate_map.assign_selected(role="GRIND") != {"FINISHED"}:
            raise RuntimeError("unable to assign grind role")

        scene = bpy.context.scene
        scene["ow_map_name"] = "Test Grind Ledge"
        spawn = bpy.data.objects.get("OW_SPAWN")
        if spawn is not None:
            # SKATEOBJ v2 uses OW_SPAWN as the package pivot.
            spawn.location = (0.0, 0.0, 0.0)

        bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH))
        export_scene(PACKAGE_PATH, force_rebuild=True)
        print(f"SKATEOBJ_TEST_OK {BLEND_PATH} {PACKAGE_PATH}")
    finally:
        addon.unregister()


if __name__ == "__main__":
    build()
