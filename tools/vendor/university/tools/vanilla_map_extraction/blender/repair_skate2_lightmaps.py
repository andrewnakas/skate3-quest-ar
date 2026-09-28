"""Bind complete Skate 2 monochrome lightmaps in an existing Blender file."""

from __future__ import annotations

import json
from pathlib import Path
import sys

import bpy


RETAIL_ENCODING = "skate3_retail_sqrt_linear_over_4"


def _json_property(owner: object, name: str) -> dict[str, object]:
    raw = str(owner.get(name, "{}"))
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError(f"{owner.name!r} has invalid {name}")
    return value


def _source_lightmap(material: bpy.types.Material) -> str:
    parameters = _json_property(material, "skate3_retail_parameters")
    values = parameters.get("lightmap", [])
    if not isinstance(values, list):
        raise ValueError(
            f"{material.name!r} has an invalid retail lightmap parameter"
        )
    return str(values[0]) if values and values[0] else ""


def _bind_material(
    material: bpy.types.Material,
    source_name: str,
    texture_id: str,
    image: bpy.types.Image,
    chromaticity_name: str,
    chromaticity_texture_id: str,
    component: int,
) -> None:
    retail_ids = _json_property(
        material,
        "skate3_retail_texture_ids",
    )
    retail_ids["lightmap"] = texture_id
    retail_ids["chromaticity"] = chromaticity_texture_id
    material["skate3_retail_texture_ids"] = json.dumps(
        retail_ids,
        sort_keys=True,
        separators=(",", ":"),
    )
    parameters = _json_property(material, "skate3_retail_parameters")
    parameters["skate2_lightmap_component"] = [str(component)]
    material["skate3_retail_parameters"] = json.dumps(
        parameters,
        sort_keys=True,
        separators=(",", ":"),
    )
    material["skate3_source_lightmap_name"] = source_name
    material["skate3_source_chromaticity_name"] = chromaticity_name
    material["skate3_lightmap_texture_id"] = texture_id
    material["skate3_chromaticity_texture_id"] = chromaticity_texture_id
    material["skate2_lightmap_component"] = component
    material["ow_lightmap_image"] = image.name
    material["ow_lightmap_encoding"] = RETAIL_ENCODING
    material["ow_baked_strength"] = 1.0


def main() -> int:
    arguments = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if len(arguments) != 2:
        raise SystemExit(
            "usage: blender INPUT.blend --background --python "
            "repair_skate2_lightmaps.py -- ALIASES_JSON OUTPUT_BLEND"
        )
    aliases_path = Path(arguments[0]).resolve()
    output = Path(arguments[1]).resolve()
    alias_document = json.loads(aliases_path.read_text(encoding="utf-8"))
    if alias_document.get("format") != "skate2-lightmap-aliases-v2":
        raise ValueError("unsupported Skate 2 lightmap alias document")
    aliases = alias_document["aliases"]
    if not isinstance(aliases, dict):
        raise ValueError("lightmap aliases must be an object")
    chromaticity_aliases = alias_document.get("chromaticity_aliases")
    if not isinstance(chromaticity_aliases, dict):
        raise ValueError("chromaticity aliases must be an object")
    mesh_bindings = alias_document.get("mesh_bindings")
    if not isinstance(mesh_bindings, dict):
        raise ValueError("lightmap aliases have no per-mesh bindings")
    cache_root = Path(str(alias_document["cache_root"])).resolve()

    scene = bpy.context.scene
    if "Skate 2" not in str(
        scene.get("ow_map_name", scene.get("skate3_map_name", scene.name))
    ):
        raise RuntimeError("the open scene is not the Skate 2 map")

    source_references = 0
    excluded_shader_materials = 0
    excluded_no_uv_materials = 0
    bound_materials = 0
    bound_images: set[str] = set()
    missing_aliases: set[str] = set()
    missing_images: set[str] = set()
    materials_by_pointer: dict[
        int, tuple[str, str, str, str, int]
    ] = {}
    source_bindings_by_pointer: dict[int, dict[str, object]] = {}
    no_uv_materials: set[int] = set()
    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        asset_id = str(obj.get("skate3_asset_id", "")).lower()
        mesh_index = obj.get("skate3_mesh_index")
        if not asset_id or mesh_index is None:
            continue
        binding = mesh_bindings.get(f"{asset_id}:{int(mesh_index)}")
        if not isinstance(binding, dict):
            continue
        for material in obj.data.materials:
            if material is None:
                continue
            if bool(binding["eligible"]):
                pointer = material.as_pointer()
                previous = source_bindings_by_pointer.setdefault(
                    pointer, binding
                )
                if previous != binding:
                    raise RuntimeError(
                        f"{material.name!r} maps to conflicting Skate 2 "
                        "lightmap records"
                    )
            elif binding.get("exclusion") == "no_secondary_uv":
                no_uv_materials.add(material.as_pointer())
    for material in bpy.data.materials:
        if "skate3_retail_parameters" not in material:
            continue
        source_name = _source_lightmap(material)
        if not source_name:
            continue
        source_references += 1
        shader_name = str(material.get("skate3_shader_name", "")).lower()
        if shader_name.startswith(("water.", "ocean.")):
            excluded_shader_materials += 1
            continue
        if material.as_pointer() in no_uv_materials:
            excluded_no_uv_materials += 1
            continue
        binding = source_bindings_by_pointer.get(material.as_pointer())
        if binding is None:
            raise RuntimeError(
                f"{material.name!r} has no original Skate 2 mesh binding"
            )
        if str(binding["source_name"]) != source_name:
            raise RuntimeError(
                f"{material.name!r} source lightmap does not match its "
                "raw Skate 2 material record"
            )
        alias = aliases.get(source_name)
        if not isinstance(alias, dict):
            missing_aliases.add(source_name)
            continue
        chromaticity_name = str(binding["chromaticity_name"])
        chromaticity_alias = chromaticity_aliases.get(chromaticity_name)
        if not isinstance(chromaticity_alias, dict):
            missing_aliases.add(chromaticity_name)
            continue
        component = int(binding["lightmap_component"])
        if component < 0 or component > 2:
            raise RuntimeError(
                f"{material.name!r} has invalid Skate 2 lightmap "
                f"component {component}"
            )
        texture_id = str(alias["texture_id"])
        image = bpy.data.images.get(texture_id)
        if image is None:
            missing_images.add(texture_id)
            continue
        expected_size = (int(alias["width"]), int(alias["height"]))
        if tuple(image.size) != expected_size:
            raise RuntimeError(
                f"{texture_id!r} is {tuple(image.size)}, expected "
                f"{expected_size}"
            )
        image.colorspace_settings.name = "Non-Color"
        image["ow_lightmap_encoding"] = RETAIL_ENCODING
        image.use_fake_user = True
        chromaticity_texture_id = str(chromaticity_alias["texture_id"])
        chromaticity_image = bpy.data.images.get(chromaticity_texture_id)
        if chromaticity_image is None:
            chromaticity_path = (
                cache_root / str(chromaticity_alias["png"])
            )
            if not chromaticity_path.is_file():
                missing_images.add(chromaticity_texture_id)
                continue
            chromaticity_image = bpy.data.images.load(
                str(chromaticity_path),
                check_existing=False,
            )
            chromaticity_image.name = chromaticity_texture_id
        expected_chromaticity_size = (
            int(chromaticity_alias["width"]),
            int(chromaticity_alias["height"]),
        )
        if tuple(chromaticity_image.size) != expected_chromaticity_size:
            raise RuntimeError(
                f"{chromaticity_texture_id!r} is "
                f"{tuple(chromaticity_image.size)}, expected "
                f"{expected_chromaticity_size}"
            )
        chromaticity_image.colorspace_settings.name = "Non-Color"
        chromaticity_image["skate2_chromaticity"] = True
        chromaticity_image.use_fake_user = True
        _bind_material(
            material,
            source_name,
            texture_id,
            image,
            chromaticity_name,
            chromaticity_texture_id,
            component,
        )
        materials_by_pointer[material.as_pointer()] = (
            source_name,
            texture_id,
            chromaticity_name,
            chromaticity_texture_id,
            component,
        )
        bound_materials += 1
        bound_images.add(texture_id)
        if bound_materials % 5_000 == 0:
            print(
                f"Bound {bound_materials} Skate 2 lightmapped materials",
                flush=True,
            )

    if missing_aliases or missing_images:
        raise RuntimeError(
            "lightmap repair has unresolved resources: "
            f"aliases={sorted(missing_aliases)[:10]}, "
            f"images={sorted(missing_images)[:10]}"
        )
    if bound_materials == 0 or not bound_images:
        raise RuntimeError("lightmap repair produced no usable bindings")

    lightmapped_objects = 0
    lightmapped_loops = 0
    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        object_binding: tuple[str, str, str, str, int] | None = None
        for material in obj.data.materials:
            if material is None:
                continue
            binding = materials_by_pointer.get(material.as_pointer())
            if binding is None:
                continue
            if object_binding is not None and object_binding != binding:
                raise RuntimeError(
                    f"{obj.name!r} uses multiple retail lightmap pages"
                )
            object_binding = binding
        if object_binding is None:
            continue
        layer = obj.data.uv_layers.get("Lightmap")
        if layer is None or len(layer.data) != len(obj.data.loops):
            raise RuntimeError(
                f"{obj.name!r} is missing its exact Lightmap UV layer"
            )
        (
            source_name,
            texture_id,
            chromaticity_name,
            chromaticity_texture_id,
            component,
        ) = object_binding
        obj["skate3_source_lightmap_name"] = source_name
        obj["skate3_source_chromaticity_name"] = chromaticity_name
        obj["skate3_source_lightmap_texture_id"] = texture_id
        obj["skate3_lightmap_texture_id"] = texture_id
        obj["skate3_chromaticity_texture_id"] = chromaticity_texture_id
        obj["skate2_lightmap_component"] = component
        retail_ids = _json_property(
            obj,
            "skate3_retail_texture_ids",
        )
        retail_ids["lightmap"] = texture_id
        retail_ids["chromaticity"] = chromaticity_texture_id
        obj["skate3_retail_texture_ids"] = json.dumps(
            retail_ids,
            sort_keys=True,
            separators=(",", ":"),
        )
        lightmapped_objects += 1
        lightmapped_loops += len(layer.data)

    if lightmapped_objects != bound_materials:
        raise RuntimeError(
            f"bound {bound_materials} materials but found "
            f"{lightmapped_objects} lightmapped objects"
        )

    scene["skate3_lightmap_status"] = (
        f"{lightmapped_objects} Skate 2 mesh parts use "
        f"{len(bound_images)} exact retail layer pages and chromaticity"
    )
    scene["skate2_lightmap_aliases"] = str(aliases_path)
    scene["skate2_lightmap_repair"] = json.dumps(
        {
            "source_references": source_references,
            "excluded_shader_materials": excluded_shader_materials,
            "excluded_no_uv_materials": excluded_no_uv_materials,
            "lightmapped_materials": bound_materials,
            "lightmapped_objects": lightmapped_objects,
            "lightmap_images": len(bound_images),
            "chromaticity_images": len(
                {
                    binding[3]
                    for binding in materials_by_pointer.values()
                }
            ),
            "lightmapped_loops": lightmapped_loops,
        },
        sort_keys=True,
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(
        filepath=str(output),
        check_existing=False,
    )
    print(
        json.dumps(
            {
                "output": str(output),
                "source_references": source_references,
                "excluded_shader_materials": excluded_shader_materials,
                "excluded_no_uv_materials": excluded_no_uv_materials,
                "lightmapped_materials": bound_materials,
                "lightmapped_objects": lightmapped_objects,
                "lightmap_images": len(bound_images),
                "chromaticity_images": len(
                    {
                        binding[3]
                        for binding in materials_by_pointer.values()
                    }
                ),
                "lightmapped_loops": lightmapped_loops,
            },
            indent=2,
        ),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
