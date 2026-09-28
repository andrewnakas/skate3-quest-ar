"""Decode and audit every retail University ClusteredMesh collision section."""

from __future__ import annotations

from collections import Counter
import argparse
import hashlib
import json
from pathlib import Path

from retail_collision_mesh import decode_rx2_clustered_meshes


def main() -> int:
    workspace = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=workspace / "intermediate" / "university" / "manifest.json",
    )
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    manifest_path = args.manifest.resolve()
    cache_root = manifest_path.parent
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets = manifest["simulation_assets"]
    mesh_count = 0
    cluster_count = 0
    vertex_count = 0
    unit_count = 0
    triangle_count = 0
    source_bytes = 0
    surfaces: Counter[int] = Counter()
    compressions: Counter[int] = Counter()
    bounds_min = [float("inf")] * 3
    bounds_max = [float("-inf")] * 3
    source_digest = hashlib.sha256()
    mesh_assets: list[str] = []

    for asset in assets:
        path = cache_root / Path(str(asset["rx2"]).replace("\\", "/"))
        data = path.read_bytes()
        try:
            meshes = decode_rx2_clustered_meshes(data)
        except ValueError as error:
            raise ValueError(
                f"{asset['asset_id']} ({path.name}): {error}"
            ) from error
        if not meshes:
            continue
        source_digest.update(bytes.fromhex(str(asset["asset_id"])[2:]))
        source_digest.update(hashlib.sha256(data).digest())
        source_bytes += len(data)
        mesh_assets.append(str(asset["asset_id"]))
        for mesh in meshes:
            mesh_count += 1
            cluster_count += mesh.cluster_count
            vertex_count += mesh.vertex_count
            unit_count += mesh.unit_count
            triangle_count += len(mesh.triangles)
            for compression, count in mesh.compression_counts:
                compressions[compression] += count
            for triangle in mesh.triangles:
                surfaces[triangle.surface] += 1
            for axis in range(3):
                bounds_min[axis] = min(bounds_min[axis], mesh.bounds_min[axis])
                bounds_max[axis] = max(bounds_max[axis], mesh.bounds_max[axis])

    result = {
        "format": "skate3-retail-collision-audit-v1",
        "district": manifest["district_name"],
        "simulation_assets": len(assets),
        "clustered_mesh_assets": len(mesh_assets),
        "clustered_mesh_sections": mesh_count,
        "clusters": cluster_count,
        "cluster_vertices": vertex_count,
        "units": unit_count,
        "triangles": triangle_count,
        "surfaces": len(surfaces),
        "surface_triangle_counts": {
            f"0x{surface:04X}": count
            for surface, count in sorted(surfaces.items())
        },
        "compression_cluster_counts": {
            str(compression): count
            for compression, count in sorted(compressions.items())
        },
        "bounds_min": bounds_min,
        "bounds_max": bounds_max,
        "source_rx2_bytes": source_bytes,
        "source_set_sha256": source_digest.hexdigest(),
        "asset_ids": mesh_assets,
    }
    encoded = json.dumps(result, indent=2) + "\n"
    if args.output is not None:
        output = args.output.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
