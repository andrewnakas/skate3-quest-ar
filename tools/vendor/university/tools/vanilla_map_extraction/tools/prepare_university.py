"""Prepare the official Skate 3 University district for Blender."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from prepare_hawaiian_dream import prepare


DISTRICT_NAME = "DIST_University"
EXCLUDED_NORMAL_TEXTURE_IDS = (
    # The retail shaders treat these resources specially. The two decoded
    # neutral defaults do not promote a retail simple material to the normal
    # mapped path. The water pair uses a shader-specific/PCA path, and the palm
    # textures are not conventional tangent-space normal maps. Retain all IDs
    # in the manifest, but do not bind them to the generic owned-world normal
    # slot.
    "0x0000043d03e3870a",
    "0x0000475d03e3870a",
    "0x2c70170a00171210",
    "0x2c70170a00171211",
    "0x0000676403e3870a",
    "0x0000676703e3870a",
    "0x0000677003e3870a",
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
            / "university"
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
        default=workspace / "intermediate" / "university",
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
        map_name="University District",
        package_name="Official Skate 3 base game",
        cache_format="skate3-university-cache-v1",
        texture_stream_names=("Tex",),
        excluded_normal_texture_ids=EXCLUDED_NORMAL_TEXTURE_IDS,
    )
    print(manifest_path)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    print(json.dumps(manifest["summary"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
