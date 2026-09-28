"""Headless regression tests for automatic grind source and density limits."""

from pathlib import Path
import sys

import bpy
from mathutils import Vector


SCRIPT_ROOT = (
    Path(__file__).resolve().parent
    / "agent_workflow"
    / "sk8-auto-map"
    / "scripts"
)
if str(SCRIPT_ROOT) not in sys.path:
    sys.path.insert(0, str(SCRIPT_ROOT))

from generate_grinds import (  # noqa: E402
    GrindSettings,
    _candidate_segments,
    _limit_chain_density,
    generate_grinds,
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def make_plane(
    collection: bpy.types.Collection, name: str, x: float
) -> bpy.types.Object:
    mesh = bpy.data.meshes.new(f"{name}Mesh")
    mesh.from_pydata(
        [
            (x - 1.0, -1.0, 0.0),
            (x + 1.0, -1.0, 0.0),
            (x + 1.0, 1.0, 0.0),
            (x - 1.0, 1.0, 0.0),
        ],
        [],
        [(0, 1, 2, 3)],
    )
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    return obj


def main() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        bpy.data.collections.remove(collection)

    collision = bpy.data.collections.new(
        "OW_GROUP_1_PRESENTATION_COLLISION"
    )
    visual_only = bpy.data.collections.new("OW_GROUP_3_NO_COLLISION")
    legacy_visual = bpy.data.collections.new("OW_VISUAL")
    legacy_collision = bpy.data.collections.new("OW_COLLISION")
    bpy.context.scene.collection.children.link(collision)
    bpy.context.scene.collection.children.link(visual_only)
    bpy.context.scene.collection.children.link(legacy_visual)
    bpy.context.scene.collection.children.link(legacy_collision)
    collision_plane = make_plane(collision, "CollisionPlane", 0.0)
    make_plane(visual_only, "VisualPlane", 10.0)
    legacy_owner = make_plane(legacy_visual, "LegacyVisual", 20.0)
    legacy_collider = make_plane(
        legacy_collision, "LegacyCollider", 20.0
    )
    legacy_collider["ow_map_object_owner"] = legacy_owner.name

    settings = GrindSettings(
        minimum_segment_length=0.1,
        maximum_slope_degrees=90.0,
        density_cell_size=0.0,
    )
    candidates = _candidate_segments(settings)
    require(
        len(candidates) == 8,
        f"Expected eight current/legacy boundary edges: {candidates}",
    )
    require(
        {segment.source for segment in candidates}
        == {"CollisionPlane", "LegacyCollider"},
        "Visual-only geometry was used or legacy collision was ignored",
    )
    generated = generate_grinds(settings)
    generated_by_source = {
        str(obj["sk8_source_object"]): obj
        for obj in bpy.data.objects
        if bool(obj.get("sk8_auto_generated_grinds", False))
    }
    require(
        generated["splines"] == 2
        and generated_by_source["CollisionPlane"].parent
        is collision_plane
        and generated_by_source["LegacyCollider"].parent is legacy_owner,
        "generated grinds did not retain editable visual ownership",
    )

    manual_collection = bpy.data.collections.new("OW_GRIND")
    bpy.context.scene.collection.children.link(manual_collection)
    manual_curve = bpy.data.curves.new("ManualLegacyGrind", "CURVE")
    manual_spline = manual_curve.splines.new("POLY")
    manual_spline.points.add(1)
    manual_spline.points[0].co = (-1.0, 0.0, 0.0, 1.0)
    manual_spline.points[1].co = (1.0, 0.0, 0.0, 1.0)
    manual_grind = bpy.data.objects.new(
        "ManualLegacyGrind", manual_curve
    )
    manual_collection.objects.link(manual_grind)
    manual_grind.parent = legacy_owner
    candidates_with_manual = _candidate_segments(settings)
    require(
        {segment.source for segment in candidates_with_manual}
        == {"CollisionPlane"},
        "automatic grinds duplicated a manually authored owner",
    )
    legacy_collider["sk8_auto_grind_exclude"] = True
    manual_collection.objects.unlink(manual_grind)
    bpy.data.objects.remove(manual_grind)
    candidates_with_exclusion = _candidate_segments(settings)
    require(
        {segment.source for segment in candidates_with_exclusion}
        == {"CollisionPlane"},
        "explicit manual-grind exclusion was ignored",
    )

    chains = {
        "DenseRail": [
            [Vector((0.0, 0.01 * index, 0.0)), Vector((length, 0.01 * index, 0.0))]
            for index, length in enumerate((1.0, 2.0, 3.0, 4.0))
        ]
    }
    limited, rejected = _limit_chain_density(
        chains,
        GrindSettings(
            density_cell_size=10.0,
            maximum_splines_per_cell=2,
            maximum_splines_per_source=128,
        ),
    )
    lengths = sorted(
        round((chain[1] - chain[0]).length, 4)
        for chain in limited["DenseRail"]
    )
    require(lengths == [3.0, 4.0], f"Density limit kept {lengths}")
    require(rejected == 2, f"Density limit rejected {rejected} chains")
    print(
        "AUTO_MAP_GRINDS_OK",
        len(candidates),
        generated["splines"],
        lengths,
        rejected,
    )


if __name__ == "__main__":
    try:
        main()
    except Exception:
        import traceback

        traceback.print_exc()
        sys.exit(1)
