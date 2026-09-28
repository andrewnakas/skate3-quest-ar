"""Build the collision-only manifest `build_retail_collision_archive.py` consumes.

The full `prepare_university.py` pipeline decodes textures and models and therefore needs
UTT-1.1.7 (`rx2_parser`, `mdl_parser`). A playable map for *audio* work needs none of that — only
the simulation stream, whose decoders (`skate3_streams`, `retail_collision_mesh`) all live in this
repository. This writes the same `simulation_assets` records the full pipeline writes, and nothing
else, so `build_retail_collision_archive.py` can run unchanged.

    python tools/build_university_collision_manifest.py <stream-dir> <output-dir>

`stream-dir` holds `DIST_University_Sim.xst` and its `cSim_*.xsf` chunks, unpacked from
`data/content/worldDIST_University.big` (an ordinary EB v3 archive — see
`crates/skate-audio-formats/src/eb.rs` for the layout).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

TOOLCHAIN = (
    Path(__file__).resolve().parent
    / "vendor"
    / "university"
    / "tools"
    / "vanilla_map_extraction"
    / "tools"
)
sys.path.insert(0, str(TOOLCHAIN))

from retail_collision_mesh import decode_rx2_clustered_meshes  # noqa: E402
from skate3_streams import load_district_stream  # noqa: E402

DISTRICT = "DIST_University"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stream_dir", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--district", default=DISTRICT)
    parser.add_argument("--map-name", default="University District")
    args = parser.parse_args()

    output = args.output.resolve()
    (output / "rx2" / "simulation").mkdir(parents=True, exist_ok=True)

    assets = load_district_stream(args.stream_dir.resolve(), "Sim", args.district)
    print(f"{len(assets)} simulation assets in {args.district}_Sim.xst")

    simulation: list[dict] = []
    meshes = clusters = triangles = 0
    for asset in assets:
        decoded = decode_rx2_clustered_meshes(asset.data)
        if not decoded:
            continue
        name = f"{asset.record.asset_id:016X}.rx2"
        rx2_path = output / "rx2" / "simulation" / name
        rx2_path.write_bytes(asset.data)
        entries = []
        for index, mesh in enumerate(decoded):
            meshes += 1
            clusters += mesh.cluster_count
            triangles += len(mesh.triangles)
            entries.append(
                {
                    "index": index,
                    "bounds": {
                        "minimum": mesh.bounds_min,
                        "maximum": mesh.bounds_max,
                    },
                    "triangles": len(mesh.triangles),
                    "clusters": mesh.cluster_count,
                    "vertices": mesh.vertex_count,
                    "units": mesh.unit_count,
                    "compression_cluster_counts": {
                        str(compression): count
                        for compression, count in mesh.compression_counts
                    },
                }
            )
        simulation.append(
            {
                "asset_id": f"0x{asset.record.asset_id:016X}",
                "asset_type": f"0x{asset.record.asset_type:08X}",
                "stream_file": asset.source_path.name,
                "source_offset": asset.source_offset,
                "size": len(asset.data),
                "rx2": str(rx2_path.relative_to(output)),
                "collision_meshes": entries,
            }
        )

    manifest = {
        "map_name": args.map_name,
        "district": args.district,
        "simulation_assets": simulation,
    }
    manifest_path = output / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(
        f"{len(simulation)} assets with collision, {meshes} meshes, "
        f"{clusters} clusters, {triangles} triangles"
    )
    print(f"manifest -> {manifest_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
