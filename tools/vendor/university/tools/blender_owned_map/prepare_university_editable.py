"""Prepare an editable University map from the extracted scene.

The University retail extraction contains material-batched presentation
meshes and separately extracted exact collision. A retail mesh part is not
automatically an authored object: most ``sur_`` records are floor, wall, or
material fragments. The curated profile exposes compact retail props and
named skate features. The aggressive profile intentionally exposes every
visual mesh record, up to a deterministic 20,000-object ceiling, for editor
performance stress testing. Both profiles report every classification
decision before export.

Usage:
  blender --background DIST_University_Owned.blend \
    --python prepare_university_editable.py -- --audit-only
"""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree


TOOL_ROOT = Path(__file__).resolve().parent
if str(TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOL_ROOT))

import owned_world_material_addon as addon  # noqa: E402
from owned_world_material_addon import exporter  # noqa: E402

COLLISION_TOOL_ROOT = (
    TOOL_ROOT.parent
    / "vanilla_map_extraction"
    / "tools"
)
if str(COLLISION_TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(COLLISION_TOOL_ROOT))
from attach_retail_collision_identity import attach_identity  # noqa: E402


# These are intentionally semantic world markers, not generic material words
# such as "concrete" or "metal" that also appear on movable skate props.
PERMANENT_WORLD_MARKERS = (
    "architecture",
    "background",
    "building",
    "bush",
    "ceiling",
    "cityblock",
    "cliff",
    "cloud",
    "decal",
    "facade",
    "foliage",
    "foundation",
    "ground",
    "grass",
    "landscape",
    "leaf",
    "mountain",
    "ocean",
    "pavement",
    "reflection",
    "road",
    "roof",
    "rock",
    "shadow",
    "sidewalk",
    "sky",
    "street",
    "terrain",
    "tree",
    "tunnel",
    "water",
    "wall",
    "window",
)

# Surface records are static by default. Only names that describe a coherent
# skate obstacle or practical prop become independently editable.
SKATE_FEATURE_SURFACE_MARKERS = (
    "bank",
    "barrier",
    "bench",
    "block",
    "bowl",
    "coping",
    "curb",
    "fence",
    "hubba",
    "kicker",
    "ledge",
    "manualpad",
    "pipe",
    "planter",
    "quarter",
    "rail",
    "ramp",
    "skate",
    "stair",
    "vent",
)

STANDALONE_PROP_PREFIXES = (
    "fire_hydrant",
    "sportscar",
    "traffic_",
    "traf4sedan",
    "trafsuva",
    "tmsag",
    "woodenpalette",
)

MAX_HORIZONTAL_SPAN = 40.0
MAX_VERTICAL_SPAN = 25.0
MAX_FOOTPRINT_AREA = 650.0
MAX_TRIANGLES = 20_000
AGGRESSIVE_EDITABLE_LIMIT = 20_000
COLLISION_GRID_SIZE = 20.0
COLLISION_MATCH_DISTANCE = 0.32
COLLISION_BOUNDS_MARGIN = 0.45
MAX_COLLISION_COMPONENT_SPAN = 45.0
MAX_COLLISION_COMPONENT_TRIANGLES = 20_000


@dataclass(frozen=True)
class Classification:
    object_name: str
    editable: bool
    reason: str
    dimensions: tuple[float, float, float]
    triangles: int
    bounds_min: tuple[float, float, float]
    bounds_max: tuple[float, float, float]


def parse_arguments() -> argparse.Namespace:
    arguments = (
        sys.argv[sys.argv.index("--") + 1 :]
        if "--" in sys.argv
        else []
    )
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audit-only", action="store_true")
    parser.add_argument(
        "--selection-profile",
        choices=("curated", "aggressive"),
        default="aggressive",
        help=(
            "curated selects coherent props; aggressive selects every visual "
            "mesh record up to the 20,000-object stress-test ceiling"
        ),
    )
    parser.add_argument("--report", type=Path)
    parser.add_argument("--output-blend", type=Path)
    parser.add_argument("--output-skate", type=Path)
    parser.add_argument("--retail-collision-archive", type=Path)
    return parser.parse_args(arguments)


def world_bounds(
    obj: bpy.types.Object,
) -> tuple[Vector, Vector]:
    corners = [obj.matrix_world @ Vector(corner) for corner in obj.bound_box]
    minimum = Vector(
        tuple(min(corner[axis] for corner in corners) for axis in range(3))
    )
    maximum = Vector(
        tuple(max(corner[axis] for corner in corners) for axis in range(3))
    )
    return minimum, maximum


def semantic_identity(obj: bpy.types.Object) -> str:
    values = [
        obj.name,
        str(obj.get("skate3_material_name", "")),
        str(obj.get("skate3_shader_name", "")),
    ]
    raw_parameters = str(obj.get("skate3_retail_parameters", ""))
    if raw_parameters:
        try:
            parameters = json.loads(raw_parameters)
            values.extend(str(value) for value in parameters.get("Name", []))
        except (TypeError, ValueError):
            values.append(raw_parameters)
    return " ".join(values).lower()


def proper_object_kind(obj: bpy.types.Object) -> str | None:
    name = obj.name_full.lower()
    if name.startswith("obj_"):
        return "retail_object"
    if name.startswith("sur_"):
        if any(marker in name for marker in SKATE_FEATURE_SURFACE_MARKERS):
            return "skate_feature_surface"
        return None
    if name.startswith(STANDALONE_PROP_PREFIXES):
        return "standalone_prop"
    return None


def classify(
    obj: bpy.types.Object,
    selection_profile: str,
) -> Classification:
    minimum, maximum = world_bounds(obj)
    dimensions_vector = maximum - minimum
    dimensions = tuple(float(value) for value in dimensions_vector)
    obj.data.calc_loop_triangles()
    triangles = len(obj.data.loop_triangles)

    if selection_profile == "aggressive":
        editable = True
        reason = "aggressive_visual_record"
    else:
        object_kind = proper_object_kind(obj)
        reason = (
            f"proper_{object_kind}" if object_kind is not None else ""
        )
        editable = True
        identity = semantic_identity(obj)
        marker = next(
            (
                candidate
                for candidate in PERMANENT_WORLD_MARKERS
                if candidate in identity
            ),
            None,
        )
        if object_kind is None:
            editable = False
            reason = "not_proper_object"
        elif marker is not None:
            editable = False
            reason = f"permanent_marker:{marker}"
        elif max(dimensions[0], dimensions[1]) > MAX_HORIZONTAL_SPAN:
            editable = False
            reason = "large_horizontal_span"
        elif dimensions[2] > MAX_VERTICAL_SPAN:
            editable = False
            reason = "large_vertical_span"
        elif dimensions[0] * dimensions[1] > MAX_FOOTPRINT_AREA:
            editable = False
            reason = "large_footprint"
        elif triangles > MAX_TRIANGLES:
            editable = False
            reason = "large_triangle_count"

    return Classification(
        object_name=obj.name_full,
        editable=editable,
        reason=reason,
        dimensions=dimensions,
        triangles=triangles,
        bounds_min=tuple(float(value) for value in minimum),
        bounds_max=tuple(float(value) for value in maximum),
    )


def audit_scene(
    selection_profile: str,
) -> tuple[list[bpy.types.Object], list[Classification]]:
    visual_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.PRESENTATION_COLLISION_COLLECTION,
            exporter.NO_COLLISION_COLLECTION,
            exporter.LEGACY_VISUAL_COLLECTION,
        )
        if obj.type == "MESH"
        and bool(obj.get("ow_export_visual", True))
        and not exporter._is_helper_object(obj)
    ]
    classifications = [
        classify(obj, selection_profile) for obj in visual_objects
    ]
    if (
        selection_profile == "aggressive"
        and len(classifications) > AGGRESSIVE_EDITABLE_LIMIT
    ):
        retained_names = {
            item.object_name
            for item in sorted(
                classifications,
                key=lambda item: item.object_name.casefold(),
            )[:AGGRESSIVE_EDITABLE_LIMIT]
        }
        classifications = [
            (
                item
                if item.object_name in retained_names
                else Classification(
                    object_name=item.object_name,
                    editable=False,
                    reason="aggressive_object_limit",
                    dimensions=item.dimensions,
                    triangles=item.triangles,
                    bounds_min=item.bounds_min,
                    bounds_max=item.bounds_max,
                )
            )
            for item in classifications
        ]
    return visual_objects, classifications


def report_payload(
    classifications: list[Classification],
    selection_profile: str,
) -> dict[str, object]:
    editable = [item for item in classifications if item.editable]
    static = [item for item in classifications if not item.editable]
    return {
        "format": "university-editable-classification-v2",
        "source_blend": str(Path(bpy.data.filepath).resolve()),
        "selection_profile": selection_profile,
        "aggressive_editable_limit": AGGRESSIVE_EDITABLE_LIMIT,
        "thresholds": {
            "maximum_horizontal_span": MAX_HORIZONTAL_SPAN,
            "maximum_vertical_span": MAX_VERTICAL_SPAN,
            "maximum_footprint_area": MAX_FOOTPRINT_AREA,
            "maximum_triangles": MAX_TRIANGLES,
        },
        "counts": {
            "visual_objects": len(classifications),
            "editable_objects": len(editable),
            "static_objects": len(static),
            "editable_triangles": sum(item.triangles for item in editable),
            "static_triangles": sum(item.triangles for item in static),
        },
        "reasons": dict(
            sorted(Counter(item.reason for item in classifications).items())
        ),
        "largest_editable": [
            {
                "name": item.object_name,
                "dimensions": item.dimensions,
                "triangles": item.triangles,
            }
            for item in sorted(
                editable,
                key=lambda item: (
                    math.prod(item.dimensions),
                    item.triangles,
                ),
                reverse=True,
            )[:50]
        ],
        "objects": [
            {
                "name": item.object_name,
                "editable": item.editable,
                "reason": item.reason,
                "dimensions": item.dimensions,
                "triangles": item.triangles,
                "bounds_min": item.bounds_min,
                "bounds_max": item.bounds_max,
            }
            for item in classifications
        ],
    }


def grid_cells(
    minimum: Vector,
    maximum: Vector,
    cell_size: float = COLLISION_GRID_SIZE,
):
    starts = [
        math.floor(minimum[axis] / cell_size) for axis in range(3)
    ]
    ends = [
        math.floor(maximum[axis] / cell_size) for axis in range(3)
    ]
    for x in range(starts[0], ends[0] + 1):
        for y in range(starts[1], ends[1] + 1):
            for z in range(starts[2], ends[2] + 1):
                yield (x, y, z)


def bounds_overlap(
    first_minimum: Vector,
    first_maximum: Vector,
    second_minimum: Vector,
    second_maximum: Vector,
    margin: float = 0.0,
) -> bool:
    return all(
        first_maximum[axis] + margin >= second_minimum[axis]
        and second_maximum[axis] + margin >= first_minimum[axis]
        for axis in range(3)
    )


def object_bvh(obj: bpy.types.Object) -> BVHTree:
    obj.data.calc_loop_triangles()
    vertices = [
        obj.matrix_world @ vertex.co for vertex in obj.data.vertices
    ]
    polygons = [
        tuple(int(index) for index in triangle.vertices)
        for triangle in obj.data.loop_triangles
    ]
    return BVHTree.FromPolygons(vertices, polygons, all_triangles=True)


def stable_object_id(name: str) -> int:
    return int(exporter._stable_object_id(name))


def signed_i32(value: int) -> int:
    return value if value < 0x80000000 else value - 0x100000000


def collision_components(
    mesh: bpy.types.Mesh,
) -> list[list[int]]:
    polygon_count = len(mesh.polygons)
    parent = list(range(polygon_count))
    rank = bytearray(polygon_count)

    def find(index: int) -> int:
        while parent[index] != index:
            parent[index] = parent[parent[index]]
            index = parent[index]
        return index

    def union(first: int, second: int) -> None:
        first_root = find(first)
        second_root = find(second)
        if first_root == second_root:
            return
        if rank[first_root] < rank[second_root]:
            first_root, second_root = second_root, first_root
        parent[second_root] = first_root
        if rank[first_root] == rank[second_root]:
            rank[first_root] += 1

    vertex_owner: dict[int, int] = {}
    for polygon in mesh.polygons:
        polygon_index = int(polygon.index)
        for vertex_index in polygon.vertices:
            previous = vertex_owner.setdefault(
                int(vertex_index), polygon_index
            )
            union(polygon_index, previous)

    grouped: dict[int, list[int]] = {}
    for polygon_index in range(polygon_count):
        grouped.setdefault(find(polygon_index), []).append(polygon_index)
    return list(grouped.values())


def assign_collision_ownership(
    editable_objects: list[bpy.types.Object],
) -> dict[str, object]:
    object_bounds = {
        obj: world_bounds(obj) for obj in editable_objects
    }
    grid: dict[tuple[int, int, int], list[bpy.types.Object]] = {}
    for obj, (minimum, maximum) in object_bounds.items():
        for cell in grid_cells(minimum, maximum):
            grid.setdefault(cell, []).append(obj)

    cached_bvhs: dict[bpy.types.Object, BVHTree] = {}
    owned_triangles: Counter[str] = Counter()
    owned_components: Counter[str] = Counter()
    collision_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.PRESENTATION_COLLISION_COLLECTION,
            exporter.NO_PRESENTATION_COLLECTION,
            exporter.LEGACY_COLLISION_COLLECTION,
        )
        if obj.type == "MESH"
        and not exporter._is_helper_object(obj)
    ]
    source_components = 0
    source_triangles = 0
    matched_components = 0
    matched_triangles = 0

    for object_index, collision_object in enumerate(
        collision_objects, start=1
    ):
        material = bpy.data.materials.get(
            str(collision_object.get("ow_material", ""))
        )
        if material is not None and not bool(
            material.get("ow_collision_enabled", True)
        ):
            continue
        mesh = collision_object.data
        mesh.calc_loop_triangles()
        if any(len(polygon.vertices) != 3 for polygon in mesh.polygons):
            raise RuntimeError(
                f"{collision_object.name!r} contains non-triangle "
                "retail collision"
            )
        existing = mesh.attributes.get(
            exporter.EDITOR_COLLISION_OWNER_ATTRIBUTE
        )
        if existing is not None:
            mesh.attributes.remove(existing)
        owner_attribute = mesh.attributes.new(
            exporter.EDITOR_COLLISION_OWNER_ATTRIBUTE,
            "INT",
            "FACE",
        )
        owner_values = [0] * len(mesh.polygons)
        world_vertices = [
            collision_object.matrix_world @ vertex.co
            for vertex in mesh.vertices
        ]
        components = collision_components(mesh)
        source_components += len(components)
        source_triangles += len(mesh.polygons)

        for component in components:
            minimum = Vector((math.inf, math.inf, math.inf))
            maximum = Vector((-math.inf, -math.inf, -math.inf))
            for polygon_index in component:
                for vertex_index in mesh.polygons[polygon_index].vertices:
                    point = world_vertices[int(vertex_index)]
                    for axis in range(3):
                        minimum[axis] = min(minimum[axis], point[axis])
                        maximum[axis] = max(maximum[axis], point[axis])
            dimensions = maximum - minimum
            if (
                len(component) > MAX_COLLISION_COMPONENT_TRIANGLES
                or max(dimensions.x, dimensions.y)
                > MAX_COLLISION_COMPONENT_SPAN
                or dimensions.z > MAX_COLLISION_COMPONENT_SPAN
            ):
                continue

            candidates: set[bpy.types.Object] = set()
            for cell in grid_cells(minimum, maximum):
                candidates.update(grid.get(cell, ()))
            candidates = {
                candidate
                for candidate in candidates
                if bounds_overlap(
                    minimum,
                    maximum,
                    *object_bounds[candidate],
                    COLLISION_BOUNDS_MARGIN,
                )
            }
            if not candidates:
                continue

            sample_count = min(24, len(component))
            sample_indices = [
                component[
                    min(
                        len(component) - 1,
                        sample * len(component) // sample_count,
                    )
                ]
                for sample in range(sample_count)
            ]
            samples = []
            for polygon_index in sample_indices:
                polygon = mesh.polygons[polygon_index]
                points = [
                    world_vertices[int(index)]
                    for index in polygon.vertices
                ]
                center = sum(points, Vector()) / 3.0
                normal = (points[1] - points[0]).cross(
                    points[2] - points[0]
                ).normalized()
                samples.append((center, normal))

            best_object = None
            best_score = None
            for candidate in candidates:
                bvh = cached_bvhs.get(candidate)
                if bvh is None:
                    bvh = object_bvh(candidate)
                    cached_bvhs[candidate] = bvh
                matches = 0
                distance_sum = 0.0
                for center, collision_normal in samples:
                    nearest = bvh.find_nearest(
                        center, COLLISION_MATCH_DISTANCE
                    )
                    if nearest is None or nearest[0] is None:
                        continue
                    _location, visual_normal, _face, distance = nearest
                    if (
                        distance <= COLLISION_MATCH_DISTANCE
                        and abs(visual_normal.dot(collision_normal)) >= 0.35
                    ):
                        matches += 1
                        distance_sum += float(distance)
                minimum_matches = 1 if sample_count <= 3 else 2
                if matches < minimum_matches:
                    continue
                score = (
                    -matches / sample_count,
                    distance_sum / matches,
                    math.prod(
                        value + 0.01
                        for value in (
                            object_bounds[candidate][1]
                            - object_bounds[candidate][0]
                        )
                    ),
                    candidate.name_full,
                )
                if best_score is None or score < best_score:
                    best_score = score
                    best_object = candidate

            if best_object is None:
                continue
            encoded_owner = signed_i32(
                stable_object_id(best_object.name_full)
            )
            for polygon_index in component:
                owner_values[polygon_index] = encoded_owner
            matched_components += 1
            matched_triangles += len(component)
            owned_components[best_object.name_full] += 1
            owned_triangles[best_object.name_full] += len(component)

        owner_attribute.data.foreach_set("value", owner_values)
        print(
            "UNIVERSITY_COLLISION_MATCH",
            f"object={object_index}/{len(collision_objects)}",
            f"name={collision_object.name}",
            f"triangles={len(mesh.polygons)}",
            f"components={len(components)}",
            flush=True,
        )

    return {
        "source_collision_objects": len(collision_objects),
        "source_components": source_components,
        "source_triangles": source_triangles,
        "matched_components": matched_components,
        "matched_triangles": matched_triangles,
        "owners": len(owned_triangles),
        "owner_triangles": dict(owned_triangles),
        "owner_components": dict(owned_components),
    }


def recenter_object(obj: bpy.types.Object) -> None:
    minimum, maximum = world_bounds(obj)
    pivot = (minimum + maximum) * 0.5
    old_matrix = obj.matrix_world.copy()
    new_matrix = old_matrix.copy()
    new_matrix.translation = pivot
    transform = new_matrix.inverted() @ old_matrix
    if obj.data.users > 1:
        obj.data = obj.data.copy()
    obj.data.transform(transform)
    obj.matrix_world = new_matrix
    obj.data.update()


def associate_grinds(
    editable_objects: list[bpy.types.Object],
) -> dict[str, int]:
    object_bounds = {
        obj: world_bounds(obj) for obj in editable_objects
    }
    grid: dict[tuple[int, int, int], list[bpy.types.Object]] = {}
    for obj, (minimum, maximum) in object_bounds.items():
        for cell in grid_cells(minimum, maximum):
            grid.setdefault(cell, []).append(obj)
    grind_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.GRIND_COLLECTION,
            exporter.LEGACY_GRIND_COLLECTION,
        )
        if obj.type == "CURVE"
    ]
    associated = 0
    for grind in grind_objects:
        minimum, maximum = world_bounds(grind)
        margin = 0.85
        expanded_minimum = minimum - Vector((margin, margin, margin))
        expanded_maximum = maximum + Vector((margin, margin, margin))
        candidates: set[bpy.types.Object] = set()
        for cell in grid_cells(expanded_minimum, expanded_maximum):
            candidates.update(grid.get(cell, ()))
        candidates = {
            candidate
            for candidate in candidates
            if bounds_overlap(
                minimum,
                maximum,
                *object_bounds[candidate],
                margin,
            )
        }
        if not candidates:
            continue
        best_owner = None
        best_score = None
        for candidate in candidates:
            owner_minimum, owner_maximum = object_bounds[candidate]
            if any(
                minimum[axis] < owner_minimum[axis] - margin
                or maximum[axis] > owner_maximum[axis] + margin
                for axis in range(3)
            ):
                continue
            dimensions = owner_maximum - owner_minimum
            volume = max(
                float(dimensions.x * dimensions.y * dimensions.z),
                0.000001,
            )
            slack = sum(
                max(float(minimum[axis] - owner_minimum[axis]), 0.0)
                + max(float(owner_maximum[axis] - maximum[axis]), 0.0)
                for axis in range(3)
            )
            identity = semantic_identity(candidate)
            semantic_rank = (
                0
                if any(
                    marker in identity
                    for marker in (
                        "rail",
                        "ledge",
                        "coping",
                        "bench",
                        "fence",
                        "bar",
                    )
                )
                else 1
            )
            score = (
                semantic_rank,
                volume,
                slack,
                candidate.name_full,
            )
            if best_score is None or score < best_score:
                best_score = score
                best_owner = candidate
        if best_owner is None:
            continue
        owner = best_owner
        world_matrix = grind.matrix_world.copy()
        grind.parent = owner
        grind.matrix_world = world_matrix
        associated += 1
    return {
        "source_grinds": len(grind_objects),
        "associated_grinds": associated,
    }


def main() -> int:
    arguments = parse_arguments()
    visual_objects, classifications = audit_scene(
        arguments.selection_profile
    )
    payload = report_payload(
        classifications,
        arguments.selection_profile,
    )
    encoded = json.dumps(payload, indent=2)
    print(
        "UNIVERSITY_EDITABLE_AUDIT",
        json.dumps(payload["counts"], sort_keys=True),
        json.dumps(payload["reasons"], sort_keys=True),
        flush=True,
    )
    if arguments.report is not None:
        report = arguments.report.resolve()
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_text(encoded + "\n", encoding="utf-8")
        print(f"UNIVERSITY_EDITABLE_REPORT {report}", flush=True)

    if arguments.audit_only:
        return 0
    if arguments.output_blend is None or arguments.output_skate is None:
        raise RuntimeError(
            "--output-blend and --output-skate are required for conversion"
        )
    if arguments.retail_collision_archive is None:
        raise RuntimeError(
            "--retail-collision-archive is required for editable University "
            "collision provenance"
        )

    by_name = {item.object_name: item for item in classifications}
    for obj in visual_objects:
        obj["ow_editor_editable"] = by_name[obj.name_full].editable

    provisional = [
        obj
        for obj in visual_objects
        if bool(obj.get("ow_editor_editable", False))
    ]
    collision = assign_collision_ownership(provisional)
    owners = set(collision["owner_triangles"])
    editable_objects = provisional
    for obj in editable_objects:
        recenter_object(obj)
    # Blender does not immediately refresh every curve/object world bound after
    # thousands of mesh datablocks are transformed in a background session.
    # Grind ownership depends on those evaluated bounds, so force one scene
    # update before matching them.
    bpy.context.view_layer.update()
    grinds = associate_grinds(editable_objects)
    payload["collision_ownership"] = collision
    payload["grind_ownership"] = grinds
    payload["counts"]["collision_owned_editable_objects"] = len(
        owners
    )
    payload["counts"]["visual_only_editable_objects"] = (
        len(editable_objects) - len(owners)
    )
    if arguments.report is not None:
        arguments.report.resolve().write_text(
            json.dumps(payload, indent=2) + "\n",
            encoding="utf-8",
        )
    print(
        "UNIVERSITY_EDITABLE_OWNERSHIP",
        json.dumps(
            {
                "editable_objects": len(editable_objects),
                "visual_only_objects": (
                    len(editable_objects) - len(owners)
                ),
                "collision_components": collision["matched_components"],
                "collision_triangles": collision["matched_triangles"],
                "associated_grinds": grinds["associated_grinds"],
            },
            sort_keys=True,
        ),
        flush=True,
    )

    addon.register()
    try:
        output_blend = arguments.output_blend.resolve()
        output_skate = arguments.output_skate.resolve()
        output_blend.parent.mkdir(parents=True, exist_ok=True)
        output_skate.parent.mkdir(parents=True, exist_ok=True)
        bpy.context.scene["ow_map_name"] = "University District - Editable"
        bpy.ops.wm.save_as_mainfile(filepath=str(output_blend))
        exporter.export_scene(output_skate, force_rebuild=True)
        identity_output = output_skate.with_name(
            output_skate.stem + ".rcid" + output_skate.suffix
        )
        identity = attach_identity(
            output_skate,
            arguments.retail_collision_archive.resolve(),
            identity_output,
        )
        identity_output.replace(output_skate)
        print(
            "UNIVERSITY_EDITABLE_COLLISION_IDENTITY",
            json.dumps(identity, sort_keys=True),
            flush=True,
        )
    finally:
        addon.unregister()
    print(
        "UNIVERSITY_EDITABLE_EXPORT",
        output_blend,
        output_skate,
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
