"""Index owned CAC authoring data; extract only explicitly selected resources.

This is a lossless metadata boundary, not a character assembler or a material
compositor. It deliberately does not equate an authored variant with a usable
in-game option. Gender, unlock, layering, material-loop and stamp rules still
have to be applied by the native CAC implementation.

Run with ``python -m tools.asset_pipeline.customisation_catalog --help``.
Never distribute its output: the catalog is derived from the user's disc.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import xml.etree.ElementTree as ET

from tools.owned_game.big import BigArchive


CATALOG_PATH = "data/content/recipe/createacharacter/cas_db_xfd1ca62f_000.xml"
MODEL_PREFIX = "data/content/createacharacter/model/cas_db"
TEXTURE_PREFIX = "data/content/createacharacter/texture"

# Navigation labels requested by the user. This is a scope contract, not a list
# of implemented pages. Do not generate working controls from this list alone.
HIERARCHY = [
    {"name": "Body mods", "children": [
        "Gender", "Skin", "Hair", "Body shape", "Facial presets",
        "Face modification", "Facial hair", "Upper body tattoo", "Lower body tattoo",
    ]},
    {"name": "Merchandise", "children": [
        "Hats", "Tshirts", "Button shirts", "Hoodies", "Jackets", "Sweaters",
        "Pants", "Shoes", "Socks", "Accessories", "Skateboards",
    ]},
    {"name": "Edit style", "children": ["Gestures", "Stance", "Style", "Posture"]},
    {"name": "Trucks adjustment", "children": []},
    {"name": "Wheels adjustment", "children": []},
]


def asset_id(value: str) -> str:
    if not re.fullmatch(r"(?:0x)?[0-9a-fA-F]{16}", value):
        raise ValueError(f"Invalid retail asset ID: {value!r}")
    return value.removeprefix("0x").lower()


def authored(node: ET.Element) -> dict:
    """Keep ordering, attributes, and scalar strings; never invent shader data."""
    result = {"tag": node.tag, "attributes": dict(node.attrib)}
    if node.text and node.text.strip():
        result["text"] = node.text
    if len(node):
        result["children"] = [authored(child) for child in node]
    return result


def authoring_flags(node: ET.Element) -> dict[str, str]:
    result = {}
    for group in node.findall("ud"):
        if group.get("n") != "cas":
            continue
        for field in group:
            if field.tag in result:
                raise ValueError(f"Duplicate CAC field {field.tag}")
            result[field.tag] = field.get("value", "")
    return result


def parse_catalog(xml: bytes, archive_paths: set[str]) -> dict:
    root = ET.fromstring(xml)
    if (root.tag, root.get("n"), root.get("type"), root.get("schemaversion")) != (
        "compositeasset", "cas_db", "CreateACharacter", "6.0"
    ):
        raise ValueError("Unsupported CAC authoring schema")
    if any(child.tag not in {"mat", "comp"} for child in root):
        raise ValueError("Unrecognized CAC root record")
    available = {p.lower() for p in archive_paths}
    materials = {}
    for material in root.findall("mat"):
        key = asset_id(material.attrib["id"])
        if key in materials:
            raise ValueError(f"Duplicate material {key}")
        textures = []
        for parameter in material.findall("sp"):
            # Keep duplicate channels and their order rather than silently
            # collapsing separate authored bindings into a channel dictionary.
            texture = asset_id(parameter.attrib["id"])
            path = f"{TEXTURE_PREFIX}/0x{texture}.rx2"
            textures.append({"channel": parameter.attrib["chn"], "id": texture,
                             "path": path, "in_archive": path.lower() in available})
        materials[key] = {
            "shader": material.attrib["type"], "textures": textures,
            "flags": authoring_flags(material), "authored": authored(material),
        }
    components = []
    slots = set()
    for component in root.findall("comp"):
        slot = component.attrib["n"]
        if not re.fullmatch(r"[A-Za-z]+", slot) or slot in slots:
            raise ValueError(f"Invalid or duplicate component slot {slot!r}")
        slots.add(slot)
        if any(child.tag != "mod" for child in component):
            raise ValueError(f"Unrecognized record in {slot}")
        models = []
        ids = set()
        for model in component.findall("mod"):
            key = asset_id(model.attrib["id"])
            if key in ids:
                raise ValueError(f"Duplicate model {slot}/{key}")
            ids.add(key)
            lods = []
            lod_indices = set()
            for lod in model.findall("lod"):
                index = int(lod.attrib["idx"])
                if index < 0 or index in lod_indices:
                    raise ValueError(f"Invalid or duplicate LOD {slot}/{key}/{index}")
                lod_indices.add(index)
                # XML arenaid names the RX2 FILE; XML id is the arena asset ID.
                # The fallback .recipe stores them in the opposite field order.
                model_id = asset_id(lod.attrib["arenaid"])
                path = f"{MODEL_PREFIX}/{slot}/0x{model_id}.rx2"
                instances = []
                for instance in lod.findall("matinst"):
                    variants = []
                    variant_ids = set()
                    for variant in instance.findall("matvar"):
                        material_id = asset_id(variant.attrib["id"])
                        if material_id not in materials:
                            raise ValueError(f"Missing material definition {material_id}")
                        if material_id in variant_ids:
                            raise ValueError(f"Duplicate material variant {slot}/{key}/{material_id}")
                        variant_ids.add(material_id)
                        variants.append({"id": material_id, "name": variant.attrib["n"],
                                         "authored": authored(variant)})
                    if not variants:
                        raise ValueError(f"Empty material instance {slot}/{key}")
                    instances.append(variants)
                if not instances:
                    raise ValueError(f"Missing material instances {slot}/{key}")
                lods.append({"index": index, "arena_asset_id": asset_id(lod.attrib["id"]),
                             "model_id": model_id, "path": path,
                             "in_archive": path.lower() in available,
                             "material_instances": instances})
            if not lods:
                raise ValueError(f"Missing LODs {slot}/{key}")
            models.append({"id": key, "name": model.attrib["n"],
                           "flags": authoring_flags(model), "lods": lods,
                           "authored": authored(model)})
        components.append({"slot": slot, "models": models})
    return {"version": 1, "source": {"entry": CATALOG_PATH,
            "xml_sha256": hashlib.sha256(xml).hexdigest(), "attributes": dict(root.attrib)},
            "scope": HIERARCHY, "materials": materials, "components": components}


def selected_paths(catalog: dict, selections: list[dict]) -> list[str]:
    """Validate extraction requests, not gender/outfit/material compatibility.

    Each request supplies slot, asset_id, lod and one material_id per ordered
    material instance. Requiring every instance prevents losing female head's
    second material loop. Requests do not authorize arbitrary archive paths.
    """
    paths = set()
    slots = set()
    for selection in selections:
        slot = selection["slot"]
        if slot in slots:
            raise ValueError(f"Repeated selected slot {slot}")
        slots.add(slot)
        candidates = [m for c in catalog["components"] if c["slot"] == slot
                      for m in c["models"] if m["id"] == asset_id(selection["asset_id"])]
        if len(candidates) != 1:
            raise ValueError(f"Unknown selected component {slot}/{selection['asset_id']}")
        lods = [lod for lod in candidates[0]["lods"] if lod["index"] == selection["lod"]]
        if len(lods) != 1:
            raise ValueError(f"Unknown selected LOD for {slot}")
        lod = lods[0]
        if not lod["in_archive"]:
            raise ValueError(f"Selected model is absent from archive: {lod['path']}")
        choices = selection["material_ids"]
        if len(choices) != len(lod["material_instances"]):
            raise ValueError(f"{slot} requires {len(lod['material_instances'])} material selections")
        paths.add(lod["path"])
        for choice, instance in zip(choices, lod["material_instances"]):
            key = asset_id(choice)
            if key not in {v["id"] for v in instance}:
                raise ValueError(f"Material {key} is not a variant of this {slot} instance")
            for texture in catalog["materials"][key]["textures"]:
                if not texture["in_archive"]:
                    raise ValueError(f"Selected texture is absent from archive: {texture['path']}")
                paths.add(texture["path"])
    return sorted(paths)


def write_private(path: Path, data: bytes) -> str:
    """Idempotent, no-clobber staging, including failed or interrupted runs."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.read_bytes() != data:
            raise ValueError(f"Refusing to replace different staged content: {path}")
        return "reused"
    # Publish a complete file without replacing another extractor's output.
    # Windows rename is atomic and refuses an existing destination. POSIX
    # rename replaces it, so use an exclusive hard link there instead.
    descriptor, temporary = tempfile.mkstemp(prefix=".cac-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
        try:
            if os.name == "nt":
                os.rename(temporary, path)
            else:
                os.link(temporary, path)
        except FileExistsError:
            if path.read_bytes() != data:
                raise ValueError(f"Conflicting concurrent staged content: {path}")
            return "reused"
    finally:
        Path(temporary).unlink(missing_ok=True)
    return "written"


def prepare(game_root: Path, output: Path, selections: list[dict] | None = None) -> dict:
    game_root = game_root.resolve(strict=True)
    output = output.resolve()
    if output == game_root or output.is_relative_to(game_root) or game_root.is_relative_to(output):
        raise ValueError("The private staging directory must be separate from the owned source")
    archive = BigArchive(game_root / "data/content/createacharacter.big")
    entries = {e.path.lower(): e for e in archive.entries}
    if len(entries) != len(archive.entries):
        raise ValueError("Ambiguous case-insensitive paths in CAC archive")
    xml = archive.read(entries[CATALOG_PATH.lower()])
    catalog = parse_catalog(xml, set(entries))
    # Resolve every selected dependency before writing any selected resources.
    paths = selected_paths(catalog, selections or [])
    from tools.asset_pipeline.customisation_native import read_database
    native = read_database(game_root, output / "database")
    paths.append(CATALOG_PATH)
    records = []
    for name in paths:
        entry = entries[name.lower()]
        data = xml if name == CATALOG_PATH else archive.read(entry)
        destination = (output / "source" / name).resolve()
        if not destination.is_relative_to(output):
            raise ValueError("Staged asset escaped the private directory")
        state = write_private(destination, data)
        records.append({"path": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                        "state": state})
    models = [m for c in catalog["components"] for m in c["models"]]
    report = {"version": 1, "source_xml_sha256": catalog["source"]["xml_sha256"],
              "component_slots": len(catalog["components"]), "model_variants": len(models),
              "materials": len(catalog["materials"]),
              "native_collections": len(native["collections"]),
              "native_morphs": len(native["morphs"]),
              "multiple_material_instance_lods": sum(len(lod["material_instances"]) > 1
                                                       for m in models for lod in m["lods"]),
              "missing_model_paths": sorted({lod["path"] for m in models for lod in m["lods"]
                                              if not lod["in_archive"]}),
              "missing_texture_paths": sorted({t["path"] for m in catalog["materials"].values()
                                                for t in m["textures"] if not t["in_archive"]}),
              "extracted": records,
              "runtime_ready": False,
              "requires": ["native CAC assembly and rig validation", "native material and tattoo composition",
                           "native recipe colour/morph/style defaults and menu filtering"]}
    # Catalog output is deterministic. A different disc/version needs its own
    # staging directory rather than silently reusing another catalog's files.
    write_private(output / "catalog.json", (json.dumps(catalog, indent=2) + "\n").encode())
    write_private(output / "native.json", (json.dumps(native, indent=2) + "\n").encode())
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-root", required=True, type=Path, help="Owned extracted Xbox game directory")
    parser.add_argument("--output", required=True, type=Path, help="Private task staging directory, outside game source")
    parser.add_argument("--selection", type=Path, help="JSON array of explicit component extraction requests")
    args = parser.parse_args()
    selections = json.loads(args.selection.read_text(encoding="utf-8")) if args.selection else None
    if selections is not None and not isinstance(selections, list):
        parser.error("--selection must contain a JSON array")
    report = prepare(args.game_root, args.output, selections)
    report_path = (args.output / "extraction-report.json").resolve()
    if not report_path.is_relative_to(args.output.resolve()):
        raise ValueError("Report path escaped the private staging directory")
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"CAC_CATALOG_READY slots={report['component_slots']} models={report['model_variants']} "
          f"materials={report['materials']} staged={len(report['extracted'])} runtime_ready=false")


if __name__ == "__main__":
    main()
