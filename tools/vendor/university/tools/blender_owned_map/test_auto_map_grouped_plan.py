"""Headless regression test for grouped agent inventory and map plans."""

from pathlib import Path
import sys

import bpy


TOOL_ROOT = Path(__file__).resolve().parent
SCRIPT_ROOT = TOOL_ROOT / "agent_workflow" / "sk8-auto-map" / "scripts"
for path in (TOOL_ROOT, SCRIPT_ROOT):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import owned_world_material_addon as addon  # noqa: E402
from apply_map_plan import apply_plan  # noqa: E402
from inventory_scene import build_inventory  # noqa: E402


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def make_plane(
    collection: bpy.types.Collection,
    name: str,
    x_offset: float,
    material: bpy.types.Material,
) -> bpy.types.Object:
    mesh = bpy.data.meshes.new(f"{name}Mesh")
    mesh.from_pydata(
        [
            (-1.0, -1.0, 0.0),
            (1.0, -1.0, 0.0),
            (1.0, 1.0, 0.0),
            (-1.0, 1.0, 0.0),
        ],
        [],
        [(0, 1, 2, 3)],
    )
    mesh.materials.append(material)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    obj.location.x = x_offset
    collection.objects.link(obj)
    return obj


def material_plan(
    name: str,
    collision: bool,
    emissive_strength: float | None = None,
) -> dict:
    result = {
        "material": name,
        "audio_surface": 4,
        "physics_surface": 2,
        "surface_pattern": 0,
        "collision_enabled": collision,
    }
    if emissive_strength is not None:
        result["emissive_strength"] = emissive_strength
    return result


def main() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)
    for material in list(bpy.data.materials):
        bpy.data.materials.remove(material)

    source = bpy.data.collections.new("Imported_Source")
    bpy.context.scene.collection.children.link(source)
    fixture_material = bpy.data.materials.new("FixtureMetal")
    plaza_material = bpy.data.materials.new("PlazaConcrete")
    fixture = make_plane(source, "Fixture", 0.0, fixture_material)
    fixture_duplicate = make_plane(
        source, "Fixture.001", 5.0, fixture_material
    )
    plaza = make_plane(source, "Plaza", 12.0, plaza_material)

    inventory, members = build_inventory()
    require(inventory["version"] == 2, "Grouped inventory version changed")
    require(
        inventory["summary"]["objects"] == 3
        and inventory["summary"]["mesh_groups"] == 2
        and inventory["summary"]["agent_object_records"] == 2
        and inventory["summary"]["collapsed_duplicate_instances"] == 1,
        f"Unexpected grouping summary: {inventory['summary']}",
    )
    fixture_group = next(
        group
        for group in inventory["mesh_groups"]
        if group["name_family"] == "Fixture"
    )
    plaza_group = next(
        group
        for group in inventory["mesh_groups"]
        if group["name_family"] == "Plaza"
    )
    require(
        fixture_group["instance_count"] == 2
        and fixture_group["representative"]["name"] == fixture.name,
        f"Fixture instances were not collapsed: {fixture_group}",
    )
    require(
        "objects" not in inventory["materials"][0],
        "Material inventory still duplicates object-name lists",
    )
    fixture_usage = next(
        material["usage"]
        for material in inventory["materials"]
        if material["name"] == fixture_material.name
    )
    require(
        fixture_usage["object_count"] == 2
        and fixture_usage["group_count"] == 1,
        f"Material usage was not collapsed by group: {fixture_usage}",
    )
    require(
        members["mesh_groups"][fixture_group["group"]]
        == [fixture.name, fixture_duplicate.name],
        "Script-only membership sidecar lost exact object names",
    )

    incomplete_plan = {
        "version": 3,
        "object_group_roles": [
            {"group": fixture_group["group"], "role": "VISUAL_ONLY"},
        ],
        "grinds": {"enabled": False},
    }
    try:
        apply_plan(incomplete_plan)
    except ValueError as exc:
        require(
            "must classify every mesh group" in str(exc),
            f"Unexpected incomplete grouped-plan error: {exc}",
        )
    else:
        raise RuntimeError("Version 3 accepted an incomplete mesh group plan")

    plan = {
        "version": 3,
        "map_name": "Grouped Plan Test",
        "materials": [
            material_plan(
                fixture_material.name,
                True,
                emissive_strength=1.0,
            ),
            material_plan(plaza_material.name, True),
        ],
        "object_group_roles": [
            {"group": fixture_group["group"], "role": "VISUAL_ONLY"},
            {"group": plaza_group["group"], "role": "DEFAULT"},
        ],
        "object_roles": [
            {"object": fixture_duplicate.name, "role": "DEFAULT"},
        ],
        "lights": [
            {
                "source_group": fixture_group["group"],
                "type": "POINT",
                "energy": 100.0,
                "range": 4.0,
                "offset": [0.0, 0.0, 1.0],
            }
        ],
        "spawn": {
            "location": [12.0, 0.0, 0.15],
            "heading_degrees": 0.0,
        },
        "grinds": {"enabled": False},
    }
    result = apply_plan(plan)
    require(
        addon._object_groups(fixture)
        == [addon.exporter.NO_COLLISION_COLLECTION],
        "Grouped VISUAL_ONLY role was not expanded",
    )
    require(
        addon._object_groups(fixture_duplicate)
        == [addon.exporter.PRESENTATION_COLLISION_COLLECTION],
        "Exact role override did not replace its grouped role",
    )
    require(
        addon._object_groups(plaza)
        == [addon.exporter.PRESENTATION_COLLISION_COLLECTION],
        "Grouped DEFAULT role was not expanded",
    )
    require(
        result["object_groups"]["mesh_groups"] == 2
        and result["object_groups"]["mesh_object_expansions"] == 3
        and result["object_groups"]["exact_object_overrides"] == 1,
        f"Unexpected grouped apply result: {result}",
    )
    require(result["lights"] == 2, "Grouped fixture lights were not expanded")
    require(
        abs(float(fixture_material.get("ow_emissive", 0.0)) - 1.0)
        < 1.0e-6,
        "Visible fixture material was not made emissive",
    )
    require(result["spawn"], "Explicit visual spawn was not applied")
    require(
        tuple(round(float(value), 4) for value in bpy.data.objects["OW_SPAWN"].location)
        == (12.0, 0.0, 0.15),
        "Explicit visual spawn location changed during application",
    )
    print("AUTO_MAP_GROUPED_PLAN_OK", result)
    addon.unregister()


if __name__ == "__main__":
    try:
        main()
    except Exception:
        import traceback

        traceback.print_exc()
        sys.exit(1)
