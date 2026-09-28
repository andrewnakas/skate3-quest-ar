"""Build and save the official Skate 3 University district."""

from __future__ import annotations

import json
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
from import_hawaiian_dream import build_scene


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 2:
        raise SystemExit(
            "usage: blender --background --python import_university.py -- "
            "MANIFEST OUTPUT_BLEND"
        )
    manifest_path = Path(arguments[0]).resolve()
    output_blend = Path(arguments[1]).resolve()
    summary = build_scene(manifest_path)
    if summary["objects"] != summary["expected_objects"]:
        raise RuntimeError(
            f"created {summary['objects']} objects; "
            f"expected {summary['expected_objects']}"
        )
    output_blend.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(output_blend))
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
