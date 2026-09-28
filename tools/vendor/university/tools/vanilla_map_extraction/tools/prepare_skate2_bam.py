"""Prepare Skate 2's main New San Vanelona world stream for Blender."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from prepare_hawaiian_dream import prepare


DISTRICT_NAME = "BAM"
EXCLUDED_NORMAL_TEXTURE_IDS = (
    # Referenced by Skate 2 materials but absent from every BAM texture stream.
    # Preserve the material reference in the manifest while leaving Blender's
    # generic tangent-space normal slot unbound.
    "0x00005fbe03e3870a",
)
EXCLUDED_STATIC_MODEL_ASSET_IDS = (
    # Sign_35 is an unplaced prop template. Its gameplay placement data is not
    # part of the static presentation model, so importing it at identity leaves
    # a floating speed-limit sign at the world origin.
    "0xD4538D2E1DA5434F",
)


def _workspace() -> Path:
    return Path(__file__).resolve().parents[1]


def main() -> int:
    workspace = _workspace()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--stream-dir",
        type=Path,
        default=(
            workspace
            / "raw"
            / "skate2"
            / "worldbam-extracted"
            / "data"
            / "content"
            / "world"
            / "stream"
            / DISTRICT_NAME
        ),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=workspace / "intermediate" / "skate2_bam",
    )
    parser.add_argument(
        "--utt-root",
        type=Path,
        default=(
            Path(os.environ["SKATE3_UTT_ROOT"])
            if os.environ.get("SKATE3_UTT_ROOT")
            else None
        ),
        required=not bool(os.environ.get("SKATE3_UTT_ROOT")),
        help=(
            "Path to UTT-1.1.7 (or set the SKATE3_UTT_ROOT environment "
            "variable)"
        ),
    )
    args = parser.parse_args()
    manifest_path = prepare(
        stream_directory=args.stream_dir.resolve(),
        output_root=args.output.resolve(),
        utt_root=args.utt_root.resolve(),
        district_name=DISTRICT_NAME,
        map_name="Skate 2 — New San Vanelona",
        package_name="Official Skate 2 base game",
        cache_format="skate2-bam-cache-v1",
        texture_stream_names=("Tex",),
        excluded_normal_texture_ids=EXCLUDED_NORMAL_TEXTURE_IDS,
        allow_material_import_order_fallback=True,
        grind_coordinate_mode="mixed_cell_local",
        excluded_static_model_asset_ids=EXCLUDED_STATIC_MODEL_ASSET_IDS,
    )
    print(manifest_path)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    print(json.dumps(manifest["summary"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
