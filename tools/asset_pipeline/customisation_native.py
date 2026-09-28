"""Read native CAC settings without changing the gameplay collections schema.

The XML catalog contains models/materials; BIN/VLT contains preset arrays,
colour parameters, morph order/ranges, cameras and menu filters. Preserve both.
Raw reflected fields remain authoritative; decoded helpers only cover layouts
corroborated with the native code. See docs/character-customisation.md.
"""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from tools.asset_pipeline.vlt import convert, hash64
from tools.owned_game.big import BigArchive


DATABASE_FILES = {
    "skaterschema.bin", "skaterschema.vlt", "skatercollections.bin",
    "skatercollections.vlt", "skaterschema_summaryreport.txt",
}

# Reflected layout offsets are independently corroborated by 8253CA60..CD18.
MORPH_CLASS = 0x7F47CA62F4100F41
MORPH_FIELDS = {
    "target": 0xAAC0F30B119BD14F,  # layout +0
    "target_order": 0x0D381DFA82C751E1,  # layout +4
    "ui_zone": 0x3D15CA957C745188,  # layout +8; not always target_order!
    "min_a": 0x0279ED3135F03661,  # layout +12
    "max_a": 0xD4340AC527D228E1,  # layout +16
    "default_a": 0x49E3727FEB2F6475,  # layout +20
    "min_b": 0x11691A8EEF816175,  # layout +24
    "max_b": 0x7680C10C0F43EB3E,  # layout +28
    "default_b": 0xBE3909745E8A3935,  # layout +32
}


def key_hash(key: str) -> int:
    return int(key[5:], 16) if key.startswith("Hash_") else hash64(key)


def fields_by_hash(row: dict) -> dict[int, dict]:
    return {key_hash(key): value for key, value in row["fields"].items()}


def float32(field: dict) -> float:
    data = bytes.fromhex(field["data"])
    if field["type"] != "EA::Reflection::Float" or len(data) != 4:
        raise ValueError("Expected one native Float field")
    value, = struct.unpack(">f", data)
    if not math.isfinite(value):
        raise ValueError("Non-finite native Float field")
    return value


def decode_morphs(rows: list[dict]) -> list[dict]:
    result = []
    for row in rows:
        if key_hash(row["class"]) != MORPH_CLASS:
            continue
        fields = fields_by_hash(row)
        values = {name: fields[key] for name, key in MORPH_FIELDS.items()}
        order, = struct.unpack(">i", bytes.fromhex(values["target_order"]["data"]))
        zone, = struct.unpack(">I", bytes.fromhex(values["ui_zone"]["data"]))
        if order == -1:
            continue  # schema default, not an additional user morph
        if not 0 <= order < 19 or not 0 <= zone < 19:
            raise ValueError("Native morph index outside the 19-field container")
        if values["target"]["type"] != "EA::Reflection::Text":
            raise ValueError("Undecoded native morph target name")
        entry = {"key": row["key"], "target": values["target"]["data"],
                 "target_order": order, "ui_zone": zone}
        # Retain native branch labels until the gender boolean's meaning is
        # verified. The inspected retail bank gives identical A/B ranges.
        for branch in ("a", "b"):
            bounds = {name: float32(values[f"{name}_{branch}"])
                      for name in ("min", "max", "default")}
            if not bounds["min"] <= bounds["default"] <= bounds["max"] or bounds["min"] == bounds["max"]:
                raise ValueError(f"Invalid native morph range: {row['key']}")
            entry[f"branch_{branch}"] = bounds
        result.append(entry)
    if len(result) != 19 or {m["target_order"] for m in result} != set(range(19)):
        raise ValueError("Native morph target order is incomplete or duplicated")
    if len({m["target"] for m in result}) != 19:
        raise ValueError("Duplicate native morph target name")
    return sorted(result, key=lambda m: m["target_order"])


def read_database(game_root: Path, staging: Path) -> dict:
    # Import locally to avoid a module cycle with the catalog's optional stage.
    from tools.asset_pipeline.customisation_catalog import write_private

    game_root = game_root.resolve(strict=True)
    staging = staging.resolve()
    if staging == game_root or staging.is_relative_to(game_root) or game_root.is_relative_to(staging):
        raise ValueError("Database staging must be separate from owned source")
    archive = BigArchive(game_root / "data/big/db.big")
    entries = [e for e in archive.entries if e.path.lower().startswith("data/db/")
               and Path(e.path).name.lower() in DATABASE_FILES]
    if len(entries) != len(DATABASE_FILES):
        raise ValueError("Missing or ambiguous owned CAC BIN/VLT inputs")
    sources = []
    for entry in entries:
        data = archive.read(entry)
        name = Path(entry.path).name.lower()
        write_private(staging / name, data)
        sources.append({"entry": entry.path, "size": len(data),
                        "sha256": hashlib.sha256(data).hexdigest()})
    # The game ships its class/collection names in this report. No local dump
    # or machine-specific reverse-engineering name cache is needed.
    names = Path(__file__).with_name("names.txt").read_text(encoding="utf-8").splitlines()
    for line in (staging / "skaterschema_summaryreport.txt").read_text(encoding="utf-8").splitlines():
        names.extend(line.replace(".class", "").replace(".xml", "").split("/"))
    names.extend(["Sk8::CAC::MorphPreset", "Sk8::CAC::ColourPreset", "Sk8::CAC::MORPH_ZONES"])
    data = convert(staging / "skaterschema", staging / "skatercollections", names)
    # Roster names also come from this owned schema report. The gameplay-only
    # name list deliberately leaves these collections hashed and cannot supply
    # a complete pro/special character catalogue.
    write_private(staging / "collections.json", json.dumps(data).encode())
    rows = [r for r in data["collections"] if r["class"].startswith(("cac_", "cas_"))]
    return {"version": 1, "sources": sources, "collections": rows, "morphs": decode_morphs(rows)}
