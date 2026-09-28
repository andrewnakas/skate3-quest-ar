"""Validate Skate 2 retail lightmaps after reopening the repaired Blender file."""

from __future__ import annotations

from array import array
import json
import sys

import bpy


RETAIL_ENCODING = "skate3_retail_sqrt_linear_over_4"


def main() -> int:
    lightmapped_materials = 0
    lightmap_images: set[str] = set()
    chromaticity_images: set[str] = set()
    component_counts = [0, 0, 0]
    for material in bpy.data.materials:
        image_name = str(material.get("ow_lightmap_image", ""))
        if not image_name:
            continue
        image = bpy.data.images.get(image_name)
        if image is None:
            raise RuntimeError(
                f"{material.name!r} references missing image {image_name!r}"
            )
        if (
            str(material.get("ow_lightmap_encoding", ""))
            != RETAIL_ENCODING
            or float(material.get("ow_baked_strength", 0.0)) != 1.0
            or image.colorspace_settings.name != "Non-Color"
        ):
            raise RuntimeError(
                f"{material.name!r} has an invalid retail lightmap setup"
            )
        retail_ids = json.loads(
            str(material.get("skate3_retail_texture_ids", "{}"))
        )
        if retail_ids.get("lightmap") != image_name:
            raise RuntimeError(
                f"{material.name!r} has inconsistent lightmap metadata"
            )
        chromaticity_name = str(retail_ids.get("chromaticity", ""))
        chromaticity = bpy.data.images.get(chromaticity_name)
        if (
            not chromaticity_name
            or chromaticity is None
            or chromaticity.colorspace_settings.name != "Non-Color"
        ):
            raise RuntimeError(
                f"{material.name!r} has invalid chromaticity metadata"
            )
        parameters = json.loads(
            str(material.get("skate3_retail_parameters", "{}"))
        )
        component_values = parameters.get(
            "skate2_lightmap_component", []
        )
        if len(component_values) != 1:
            raise RuntimeError(
                f"{material.name!r} has no Skate 2 lightmap component"
            )
        component = int(component_values[0])
        if component < 0 or component > 2:
            raise RuntimeError(
                f"{material.name!r} has invalid component {component}"
            )
        component_counts[component] += 1
        lightmapped_materials += 1
        lightmap_images.add(image_name)
        chromaticity_images.add(chromaticity_name)

    lightmapped_objects = 0
    distinct_uv_objects = 0
    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        texture_id = str(obj.get("skate3_lightmap_texture_id", ""))
        if not texture_id:
            continue
        layer = obj.data.uv_layers.get("Lightmap")
        base = obj.data.uv_layers.get("UVMap")
        if layer is None or len(layer.data) != len(obj.data.loops):
            raise RuntimeError(f"{obj.name!r} lost its Lightmap UV layer")
        if base is not None:
            value_count = len(layer.data) * 2
            light_values = array("f", [0.0]) * value_count
            base_values = array("f", [0.0]) * value_count
            layer.data.foreach_get("uv", light_values)
            base.data.foreach_get("uv", base_values)
            distinct_uv_objects += light_values != base_values
        lightmapped_objects += 1

    if lightmapped_materials == 0 or lightmapped_objects == 0:
        raise RuntimeError("the repaired file contains no retail lightmaps")
    if lightmapped_materials != lightmapped_objects:
        raise RuntimeError(
            "lightmapped material/object counts differ after reopen"
        )
    if not distinct_uv_objects:
        raise RuntimeError(
            "the repaired file contains no distinct lightmap UVs"
        )
    result = {
        "lightmapped_materials": lightmapped_materials,
        "lightmapped_objects": lightmapped_objects,
        "lightmap_images": len(lightmap_images),
        "chromaticity_images": len(chromaticity_images),
        "lightmap_component_counts": component_counts,
        "distinct_lightmap_uv_objects": distinct_uv_objects,
        "status": bpy.context.scene.get("skate3_lightmap_status", ""),
    }
    print(json.dumps(result, indent=2), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
