"""Rename the extracted grind curves to the provenance name the Rust runtime validates.

`import_hawaiian_dream.build_scene` names each rail curve
``GRIND_{asset_id_without_0x}_{rail_index:04d}``. The runtime's static grind provider
(`crates/skate-game/src/grind_world/provider.rs`) instead requires the package rail's name to be
``{asset_id}_{section_index}_{rail_index}``, built from the WMET record's own provenance, and
refuses the map with "Stock rail provenance/name mismatch" otherwise. The vendored extraction
toolchain predates that check.

Renaming is safe and does not disturb the runtime's positional `records.iter().zip(&map.rails)`:
the exporter emits the WMET `grind_splines` array and the package rail list in the same pass from
the same objects, so any consistent reordering moves both together.

The mapping is unambiguous — over the University manifest's 4,201 rails, `(asset_id, rail_index)`
has 4,201 distinct values — so the old name identifies its record exactly.

    blender --background university-owned.blend --python tools/rename_university_grinds.py \
        -- manifest.json output.blend
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import bpy


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 2:
        raise SystemExit(
            "usage: blender --background scene.blend --python "
            "rename_university_grinds.py -- MANIFEST OUTPUT_BLEND"
        )
    manifest_path, output_blend = Path(arguments[0]).resolve(), Path(arguments[1]).resolve()
    records = json.loads(manifest_path.read_text(encoding="utf-8"))["grind_splines"]

    wanted = {}
    for record in records:
        asset = str(record["asset_id"])
        rail = int(record["rail_index"])
        old = f"GRIND_{asset[2:]}_{rail:04d}"
        wanted[old] = f"{asset}_{int(record['section_index'])}_{rail}"

    renamed = missing = 0
    # Two passes: park every target name out of the way first, so a rename can never collide with
    # an object that has not been processed yet.
    for obj in list(bpy.data.objects):
        if obj.name in wanted:
            obj.name = f"__pending__{obj.name}"
    for obj in list(bpy.data.objects):
        if not obj.name.startswith("__pending__"):
            continue
        old = obj.name[len("__pending__") :]
        obj.name = wanted[old]
        if obj.data is not None:
            obj.data.name = wanted[old]
        renamed += 1

    present = {o.name for o in bpy.data.objects}
    for target in wanted.values():
        if target not in present:
            missing += 1

    print(
        json.dumps(
            {
                "status": "GRIND_RENAME_OK" if missing == 0 else "GRIND_RENAME_INCOMPLETE",
                "records": len(records),
                "renamed": renamed,
                "missing": missing,
                "output": str(output_blend),
            },
            indent=2,
        )
    )
    if missing:
        raise SystemExit(f"{missing} rail curves were not found in the scene")
    bpy.ops.wm.save_as_mainfile(filepath=str(output_blend))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
