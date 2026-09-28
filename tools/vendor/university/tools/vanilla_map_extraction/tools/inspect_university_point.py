"""Inspect visual and collision geometry near a University map coordinate."""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
from pathlib import Path
import struct
import sys
from typing import Iterable


COLLISION_RECORD = struct.Struct("<9fII4B")
VISUAL_VERTEX = struct.Struct("<10fI")


def _load_analyzer(source_root: Path):
    analyzer_path = (
        source_root / "tools" / "blender_owned_map" / "analyze_skate.py"
    )
    spec = importlib.util.spec_from_file_location(
        "owned_map_analyze_skate", analyzer_path
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load package analyzer: {analyzer_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _sub(a: tuple[float, float, float], b: tuple[float, float, float]):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _dot(a: tuple[float, float, float], b: tuple[float, float, float]):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def _cross(a: tuple[float, float, float], b: tuple[float, float, float]):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _length_squared(value: tuple[float, float, float]) -> float:
    return _dot(value, value)


def _normal(
    a: tuple[float, float, float],
    b: tuple[float, float, float],
    c: tuple[float, float, float],
) -> tuple[float, float, float]:
    value = _cross(_sub(b, a), _sub(c, a))
    length = math.sqrt(_length_squared(value))
    if length <= 1.0e-12:
        return (0.0, 0.0, 0.0)
    return (value[0] / length, value[1] / length, value[2] / length)


def _point_triangle_distance_squared(
    point: tuple[float, float, float],
    a: tuple[float, float, float],
    b: tuple[float, float, float],
    c: tuple[float, float, float],
) -> float:
    # Real-Time Collision Detection, Christer Ericson, section 5.1.5.
    ab = _sub(b, a)
    ac = _sub(c, a)
    ap = _sub(point, a)
    d1 = _dot(ab, ap)
    d2 = _dot(ac, ap)
    if d1 <= 0.0 and d2 <= 0.0:
        return _length_squared(ap)

    bp = _sub(point, b)
    d3 = _dot(ab, bp)
    d4 = _dot(ac, bp)
    if d3 >= 0.0 and d4 <= d3:
        return _length_squared(bp)

    vc = d1 * d4 - d3 * d2
    if vc <= 0.0 and d1 >= 0.0 and d3 <= 0.0:
        v = d1 / (d1 - d3)
        closest = (a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2])
        return _length_squared(_sub(point, closest))

    cp = _sub(point, c)
    d5 = _dot(ab, cp)
    d6 = _dot(ac, cp)
    if d6 >= 0.0 and d5 <= d6:
        return _length_squared(cp)

    vb = d5 * d2 - d1 * d6
    if vb <= 0.0 and d2 >= 0.0 and d6 <= 0.0:
        w = d2 / (d2 - d6)
        closest = (a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2])
        return _length_squared(_sub(point, closest))

    va = d3 * d6 - d5 * d4
    if va <= 0.0 and d4 - d3 >= 0.0 and d5 - d6 >= 0.0:
        bc = _sub(c, b)
        w = (d4 - d3) / ((d4 - d3) + (d5 - d6))
        closest = (b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2])
        return _length_squared(_sub(point, closest))

    denominator = 1.0 / (va + vb + vc)
    v = vb * denominator
    w = vc * denominator
    closest = (
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    )
    return _length_squared(_sub(point, closest))


def _within_expanded_bounds(
    point: tuple[float, float, float],
    radius: float,
    vertices: Iterable[tuple[float, float, float]],
) -> bool:
    values = tuple(vertices)
    return all(
        min(vertex[axis] for vertex in values) - radius <= point[axis]
        <= max(vertex[axis] for vertex in values) + radius
        for axis in range(3)
    )


def _triangle_result(
    index: int,
    point: tuple[float, float, float],
    a: tuple[float, float, float],
    b: tuple[float, float, float],
    c: tuple[float, float, float],
    **metadata,
) -> dict[str, object]:
    return {
        "index": index,
        "distance": math.sqrt(
            _point_triangle_distance_squared(point, a, b, c)
        ),
        "normal": _normal(a, b, c),
        "vertices": (a, b, c),
        **metadata,
    }


def _nearest_collision(
    collision_bytes: bytes,
    point: tuple[float, float, float],
    radius: float,
    limit: int,
) -> list[dict[str, object]]:
    nearby: list[dict[str, object]] = []
    for index, record in enumerate(
        COLLISION_RECORD.iter_unpack(collision_bytes)
    ):
        a = tuple(record[0:3])
        b = tuple(record[3:6])
        c = tuple(record[6:9])
        if not _within_expanded_bounds(point, radius, (a, b, c)):
            continue
        result = _triangle_result(
            index,
            point,
            a,
            b,
            c,
            surface=record[9],
            material=record[10],
            edge_codes=tuple(record[11:14]),
            has_native_edge_codes=bool(record[14]),
        )
        if result["distance"] <= radius:
            nearby.append(result)
    nearby.sort(key=lambda item: (item["distance"], item["index"]))
    return nearby[:limit]


def _nearest_visual(
    vertex_bytes: bytes,
    indices: tuple[int, ...],
    point: tuple[float, float, float],
    radius: float,
    limit: int,
) -> list[dict[str, object]]:
    position_cache: dict[int, tuple[float, float, float]] = {}

    def position(index: int) -> tuple[float, float, float]:
        value = position_cache.get(index)
        if value is None:
            value = tuple(
                VISUAL_VERTEX.unpack_from(
                    vertex_bytes, index * VISUAL_VERTEX.size
                )[0:3]
            )
            position_cache[index] = value
        return value

    nearby: list[dict[str, object]] = []
    for triangle_index in range(0, len(indices), 3):
        vertex_indices = indices[triangle_index : triangle_index + 3]
        if len(vertex_indices) != 3:
            break
        a, b, c = (position(index) for index in vertex_indices)
        if not _within_expanded_bounds(point, radius, (a, b, c)):
            continue
        material = VISUAL_VERTEX.unpack_from(
            vertex_bytes, vertex_indices[0] * VISUAL_VERTEX.size
        )[10]
        result = _triangle_result(
            triangle_index // 3,
            point,
            a,
            b,
            c,
            material=material,
            vertex_indices=vertex_indices,
        )
        if result["distance"] <= radius:
            nearby.append(result)
    nearby.sort(key=lambda item: (item["distance"], item["index"]))
    return nearby[:limit]


def _source_matches(
    source_root: Path,
    manifest_path: Path,
    collision_triangles: list[dict[str, object]],
) -> list[dict[str, object]]:
    if not collision_triangles:
        return []
    sys.path.insert(0, str(manifest_path.parent.parent.parent / "tools"))
    from retail_collision_mesh import decode_rx2_clustered_meshes

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    cache_root = manifest_path.parent
    wanted = {
        tuple(
            round(component, 6)
            for vertex in triangle["vertices"]
            for component in vertex
        )
        for triangle in collision_triangles
    }
    matches: list[dict[str, object]] = []
    for asset in manifest["simulation_assets"]:
        for mesh_metadata in asset.get("collision_meshes", []):
            minimum = mesh_metadata["bounds"]["minimum"]
            maximum = mesh_metadata["bounds"]["maximum"]
            if not any(
                all(
                    minimum[axis] - 0.01 <= vertex[axis]
                    <= maximum[axis] + 0.01
                    for axis in range(3)
                )
                for triangle in collision_triangles
                for vertex in triangle["vertices"]
            ):
                continue
            rx2 = cache_root / Path(str(asset["rx2"]).replace("\\", "/"))
            meshes = decode_rx2_clustered_meshes(rx2.read_bytes())
            for mesh_index, mesh in enumerate(meshes):
                for triangle_index, triangle in enumerate(mesh.triangles):
                    key = tuple(
                        round(component, 6)
                        for vertex in (triangle.a, triangle.b, triangle.c)
                        for component in vertex
                    )
                    if key in wanted:
                        matches.append(
                            {
                                "asset_id": asset["asset_id"],
                                "stream_file": asset["stream_file"],
                                "rx2": str(rx2.relative_to(source_root)),
                                "mesh_index": mesh_index,
                                "triangle_index": triangle_index,
                                "cluster_index": (
                                    mesh.triangle_cluster_indices[
                                        triangle_index
                                    ]
                                ),
                                "mesh_flags": mesh.mesh_flags,
                                "unit_flags": triangle.unit_flags,
                                "group_id": triangle.group_id,
                                "surface": triangle.surface,
                                "edge_codes": triangle.edge_codes,
                                "vertices": (
                                    triangle.a,
                                    triangle.b,
                                    triangle.c,
                                ),
                            }
                        )
    return matches


def main() -> int:
    source_root = Path(__file__).resolve().parents[3]
    workspace = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--package",
        type=Path,
        default=workspace / "intermediate" / "university" / "University.skate",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=workspace / "intermediate" / "university" / "manifest.json",
    )
    parser.add_argument("--point", nargs=3, type=float, required=True)
    parser.add_argument("--radius", type=float, default=3.0)
    parser.add_argument("--limit", type=int, default=20)
    parser.add_argument("--skip-source", action="store_true")
    args = parser.parse_args()

    analyzer = _load_analyzer(source_root)
    package = analyzer.analyze_package(
        args.package.resolve(), include_payloads=True
    )
    point = tuple(args.point)
    collision = _nearest_collision(
        package["_collision_bytes"], point, args.radius, args.limit
    )
    materials = {
        material["id"]: material for material in package["_materials"]
    }
    for triangle in collision:
        material = materials.get(triangle["material"])
        if material is None:
            continue
        triangle["material_name"] = material["name"]
        triangle["retail_surface"] = (
            int(material["audio_surface"]) & 0x7F
        ) | ((int(material["physics_surface"]) & 0x1F) << 7) | (
            (int(material["surface_pattern"]) & 0x0F) << 12
        )
        triangle["retail_surface_channels"] = {
            "audio": material["audio_surface"],
            "physics": material["physics_surface"],
            "pattern": material["surface_pattern"],
        }
    visual = _nearest_visual(
        package["_vertex_bytes"],
        package["_indices"],
        point,
        args.radius,
        args.limit,
    )
    result = {
        "format": "university-point-inspection-v1",
        "package": str(args.package.resolve()),
        "point": point,
        "radius": args.radius,
        "collision": collision,
        "visual": visual,
        "source_matches": (
            []
            if args.skip_source
            else _source_matches(
                source_root, args.manifest.resolve(), collision
            )
        ),
    }
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
