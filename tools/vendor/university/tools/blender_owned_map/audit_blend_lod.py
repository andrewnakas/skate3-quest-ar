"""Print large-map object, LOD, and retained-source metadata statistics."""

from __future__ import annotations

import collections
import re

import bpy


def main() -> None:
    meshes = [obj for obj in bpy.data.objects if obj.type == "MESH"]
    exported = [
        obj
        for obj in meshes
        if bool(obj.get("ow_export_visual", True)) and not obj.hide_render
    ]
    lod_pattern = re.compile(r"(^|[_. -])lod[0-9]+($|[_. -])")
    lod_named = [
        obj for obj in meshes if lod_pattern.search(obj.name.lower())
    ]
    metadata = collections.Counter(
        key
        for obj in meshes
        for key in obj.keys()
        if any(
            marker in key.lower()
            for marker in ("lod", "sector", "stream", "cluster")
        )
    )
    stream_files = collections.Counter(
        str(obj.get("skate3_stream_file", ""))
        for obj in meshes
        if obj.get("skate3_stream_file", "")
    )
    print(
        "SKATE_BLEND_LARGE_MAP_STATS",
        f"meshes={len(meshes)}",
        f"exported={len(exported)}",
        f"polygons={sum(len(obj.data.polygons) for obj in exported)}",
        f"lod_named={len(lod_named)}",
        f"lod_exported={sum(obj in exported for obj in lod_named)}",
        f"metadata={dict(metadata)}",
        f"stream_files={len(stream_files)}",
    )
    print(
        "SKATE_BLEND_LOD_SAMPLES",
        [obj.name for obj in lod_named[:50]],
    )
    print("SKATE_BLEND_STREAM_FILES", stream_files.most_common(50))
    print(
        "SKATE_BLEND_OBJECT_SAMPLES",
        [
            {
                "name": obj.name,
                "polygons": len(obj.data.polygons),
                "properties": {
                    key: obj.get(key)
                    for key in obj.keys()
                    if key != "_RNA_UI"
                },
            }
            for obj in meshes[:20]
        ],
    )


if __name__ == "__main__":
    main()
