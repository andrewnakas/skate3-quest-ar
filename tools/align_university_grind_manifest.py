"""Reorder the retail manifest's grind records into the order the exporter emits rails.

The runtime pairs the two positionally --
`records.iter().zip(&map.rails)` in `crates/skate-game/src/grind_world/provider.rs` -- but the
exporter builds them from different sources:

* `map.rails` comes from `_objects_from_collections(GRIND_COLLECTION, ...)`, and Blender's
  `collection.all_objects` yields objects **sorted by name**;
* the `WMET` payload is `bpy.data.texts["SKATE3_RETAIL_MANIFEST"]` written out verbatim, in the
  **extraction manifest's** order.

Those orders differ, so every rail is validated against another rail's provenance and the map is
refused with "Stock rail provenance/name mismatch". Sorting the manifest to the scene's own object
order fixes the pairing without touching any rail data.

The scene order is read from Blender rather than recomputed, so this does not depend on guessing
Blender's name collation.

Run after `tools/rename_university_grinds.py`, which gives each curve the
`{asset_id}_{section_index}_{rail_index}` name the runtime expects:

    blender --background university-named.blend --python tools/align_university_grind_manifest.py \
        -- output.blend
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import bpy

GRIND_COLLECTIONS = ("OW_GROUP_4_GRINDS", "SKATE_GRINDS")
MANIFEST_TEXT = "SKATE3_RETAIL_MANIFEST"


def scene_rail_order() -> list[str]:
    """The exporter's own order: `collection.all_objects`, first collection that exists."""
    seen: set[int] = set()
    order: list[str] = []
    for name in GRIND_COLLECTIONS:
        collection = bpy.data.collections.get(name)
        if collection is None:
            continue
        for obj in collection.all_objects:
            if obj.type != "CURVE":
                continue
            identity = obj.as_pointer()
            if identity not in seen:
                seen.add(identity)
                order.append(obj.name)
    return order


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 1:
        raise SystemExit(
            "usage: blender --background scene.blend --python "
            "align_university_grind_manifest.py -- OUTPUT_BLEND"
        )
    output_blend = Path(arguments[0]).resolve()

    text = bpy.data.texts.get(MANIFEST_TEXT)
    if text is None:
        raise SystemExit(f"scene has no {MANIFEST_TEXT} text datablock")
    manifest = json.loads(text.as_string())
    records = manifest.get("grind_splines") or []

    by_name = {}
    for record in records:
        asset = str(record["asset_id"])
        name = f"{asset}_{int(record['section_index'])}_{int(record['rail_index'])}"
        by_name[name] = record

    order = scene_rail_order()
    ordered = []
    unmatched_objects = []
    for name in order:
        record = by_name.pop(name, None)
        if record is None:
            unmatched_objects.append(name)
        else:
            ordered.append(record)

    report = {
        "status": "GRIND_MANIFEST_ALIGNED",
        "records": len(records),
        "scene_curves": len(order),
        "reordered": len(ordered),
        "objects_without_record": len(unmatched_objects),
        "records_without_object": len(by_name),
        "output": str(output_blend),
    }
    if unmatched_objects:
        report["example_object_without_record"] = unmatched_objects[0]
    if by_name:
        report["example_record_without_object"] = next(iter(by_name))
    if unmatched_objects or by_name:
        report["status"] = "GRIND_MANIFEST_MISMATCH"
    print(json.dumps(report, indent=2))
    if report["status"] != "GRIND_MANIFEST_ALIGNED":
        raise SystemExit("scene curves and manifest records do not correspond one to one")

    manifest["grind_splines"] = ordered
    text.clear()
    text.write(json.dumps(manifest))
    bpy.ops.wm.save_as_mainfile(filepath=str(output_blend))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
