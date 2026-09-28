"""Headless regression test for explicit version 2 agent collision roles."""

from pathlib import Path
import sys

import bpy


TOOL_ROOT = Path(__file__).resolve().parent
SCRIPT_ROOT = (
    TOOL_ROOT
    / "agent_workflow"
    / "sk8-auto-map"
    / "scripts"
)
for path in (TOOL_ROOT, SCRIPT_ROOT):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import owned_world_material_addon as addon
from apply_map_plan import apply_plan


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def make_plane(
    collection: bpy.types.Collection,
    name: str,
    x_offset: float,
    material: bpy.types.Material | None,
) -> bpy.types.Object:
    mesh = bpy.data.meshes.new(f"{name}Mesh")
    mesh.from_pydata(
        [
            (x_offset - 1.0, -1.0, 0.0),
            (x_offset + 1.0, -1.0, 0.0),
            (x_offset + 1.0, 1.0, 0.0),
            (x_offset - 1.0, 1.0, 0.0),
        ],
        [],
        [(0, 1, 2, 3)],
    )
    mesh.update()
    if material is not None:
        mesh.materials.append(material)
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    return obj


def material_plan(name: str, collision: bool) -> dict:
    return {
        "material": name,
        "audio_surface": 4,
        "physics_surface": 2,
        "surface_pattern": 0,
        "collision_enabled": collision,
    }


def main() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)
    for material in list(bpy.data.materials):
        bpy.data.materials.remove(material)

    source = bpy.data.collections.new("Imported_Source_Organization")
    bpy.context.scene.collection.children.link(source)
    grass = bpy.data.materials.new("Soft_Grass_Planter")
    leaves = bpy.data.materials.new("Dense_Shrub_Foliage")
    proxy_material = bpy.data.materials.new("Invisible_Collision_Material")
    planter = make_plane(source, "Soft_Grass_Planter_Block", 0.0, grass)
    foliage = make_plane(source, "Dense_Shrub_Foliage", 3.0, leaves)
    proxy = make_plane(source, "UCX_Plaza_Proxy", 6.0, proxy_material)
    proxy.hide_render = True
    helper = make_plane(source, "character_size", 9.0, None)

    incomplete = {
        "version": 2,
        "object_roles": [
            {"object": planter.name, "role": "DEFAULT"},
        ],
    }
    try:
        apply_plan(incomplete)
    except ValueError as exc:
        require(
            "must classify every mesh" in str(exc),
            f"Unexpected incomplete-plan error: {exc}",
        )
    else:
        raise RuntimeError("Version 2 accepted an incomplete mesh role plan")

    plan = {
        "version": 2,
        "map_name": "Agent Collision Role Test",
        "materials": [
            material_plan(grass.name, True),
            material_plan(leaves.name, False),
            material_plan(proxy_material.name, True),
        ],
        "object_roles": [
            {"object": planter.name, "role": "DEFAULT"},
            {"object": foliage.name, "role": "VISUAL_ONLY"},
            {"object": proxy.name, "role": "COLLISION_ONLY"},
            {"object": helper.name, "role": "IGNORE"},
        ],
        "grinds": {"enabled": False},
    }
    result = apply_plan(plan)
    require(
        addon._object_groups(planter)
        == [addon.exporter.PRESENTATION_COLLISION_COLLECTION],
        "Agent DEFAULT role was not applied",
    )
    require(
        addon._object_groups(foliage)
        == [addon.exporter.NO_COLLISION_COLLECTION],
        "Agent VISUAL_ONLY role was not applied",
    )
    require(
        addon._object_groups(proxy)
        == [addon.exporter.NO_PRESENTATION_COLLECTION],
        "Agent COLLISION_ONLY role was not applied",
    )
    require(
        not addon._object_groups(helper),
        "Agent IGNORE role did not remove SKATE memberships",
    )
    for obj in (planter, foliage, proxy, helper):
        require(
            obj.name in source.objects,
            f"Applying a SKATE role destroyed {obj.name}'s source collection",
        )
    require(
        planter.get("ow_material") == grass.name,
        "DEFAULT object did not receive its explicit collision material",
    )
    require(
        proxy.get("ow_material") == proxy_material.name,
        "COLLISION_ONLY object did not receive its explicit material",
    )
    require(
        bool(grass.get("ow_collision_enabled", False))
        and not bool(leaves.get("ow_collision_enabled", True))
        and bool(proxy_material.get("ow_collision_enabled", False)),
        "Agent material collision decisions were not preserved",
    )
    require(
        not bool(grass.get("ow_auto_imported", True)),
        "Agent material decision remained auto-refreshable",
    )
    require(
        result["object_roles"] == 4
        and result["collision_objects"] == 2,
        f"Unexpected apply result: {result}",
    )
    print("AGENT_COLLISION_PLAN_OK", result)
    addon.unregister()


if __name__ == "__main__":
    try:
        main()
    except Exception:
        import traceback

        traceback.print_exc()
        sys.exit(1)
