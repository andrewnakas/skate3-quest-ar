#!/usr/bin/env python3
"""Recover the retail Xbox 360 fallback skater into ignored private paths.

This script never modifies retail sources. It validates the fallback recipe,
selects the exact LOD0 modular pieces recorded in the checked-in manifest,
decodes every referenced RX2 texture with the user's proven local tooling, and
writes only to assets/private or work/private-assets.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import shutil
import struct
import sys
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST = Path(__file__).with_name("default_skater_retail_manifest.json")
DEFAULT_WORK = PROJECT_ROOT / "work" / "private-assets" / "default_skater"
DEFAULT_PRIVATE = PROJECT_ROOT / "assets" / "private" / "default_skater"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def require_file(path: Path, label: str) -> Path:
    if not path.is_file():
        raise RuntimeError(f"{label} is missing: {path}")
    return path


def find_exact_file(candidates: list[Path], expected_hash: str, label: str) -> Path:
    found = []
    for candidate in candidates:
        if candidate.is_file():
            actual = sha256(candidate)
            found.append(f"{candidate} ({actual})")
            if actual == expected_hash:
                return candidate
    detail = "\n  ".join(found) if found else "<none found>"
    raise RuntimeError(
        f"No authorized local {label} matched SHA-256 {expected_hash}.\n"
        f"Candidates:\n  {detail}"
    )


def be_u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]


def parse_fallback_recipe(path: Path, payload_offset: int) -> dict[str, Any]:
    wrapped = path.read_bytes()
    data = wrapped[payload_offset:]
    name_len = data[7]
    name = data[8 : 8 + name_len].decode("ascii")
    recipe_type = data[0x0B + name_len]
    full_payload = data[0x13 + name_len] == 2
    index = 0x18 + name_len
    list_count = data[index - 1]
    lists: dict[str, list[dict[str, Any]]] = {}

    for _ in range(list_count):
        list_name_len = data[index + 3]
        list_name = data[index + 4 : index + 4 + list_name_len].decode("ascii")
        index += 8 + list_name_len
        asset_count = data[index - 1]
        assets = []
        for _ in range(asset_count):
            asset_id = data[index : index + 8].hex()
            model_count = data[index + 11]
            index += 12
            models = []
            for _ in range(model_count):
                block = data[index : index + 0x25]
                if len(block) != 0x25:
                    raise RuntimeError("Fallback recipe ended inside a model block")
                model = {
                    "lod": block[0],
                    "model_arena_id": block[1:9].hex(),
                    "model_id": block[9:17].hex(),
                    "material_id": block[0x19 : 0x21].hex(),
                    "textures": {},
                }
                texture_loops = block[0x14]
                texture_count = block[-1]
                index += 0x25
                for _ in range(texture_count):
                    channel_len = data[index + 3]
                    channel = data[index + 4 : index + 4 + channel_len].decode(
                        "ascii"
                    )
                    texture_id = data[
                        index + 4 + channel_len : index + 12 + channel_len
                    ].hex()
                    model["textures"][channel] = texture_id
                    index += 12 + channel_len
                # Female rostrals can carry additional material loops. They are
                # retained in the format even though this selected male preset
                # uses only the first material instance.
                for _ in range(max(0, texture_loops - 1)):
                    index += 0x10
                    extra_count = data[index - 1]
                    for _ in range(extra_count):
                        channel_len = data[index + 3]
                        index += 12 + channel_len
                models.append(model)
            assets.append({"asset_id": asset_id, "models": models})
        lists[list_name] = assets

    gender = None
    body_mods: list[float] = []
    if full_payload:
        index += 5
        gender = data[index]
        index += 9
        rgb_count = data[index - 1]
        index += rgb_count * 0x2C
        index += 8
        graphic_count = data[index - 1]
        for _ in range(graphic_count):
            url_len = data[index + 3]
            index += 5 + 24 + url_len
            index += 1
        index += 4
        body_mods = list(struct.unpack_from(">19f", data, index))

    return {
        "name": name,
        "recipe_type": recipe_type,
        "full_payload": full_payload,
        "gender": gender,
        "asset_lists": lists,
        "body_mods": body_mods,
    }


def assert_recipe(parsed: dict[str, Any], manifest: dict[str, Any]) -> None:
    preset = manifest["preset"]
    if parsed["name"] != preset["recipe_name"]:
        raise RuntimeError(
            f"Recipe name mismatch: {parsed['name']} != {preset['recipe_name']}"
        )
    if parsed["recipe_type"] != preset["recipe_type"]:
        raise RuntimeError("Fallback recipe is not a CreateACharacter recipe")
    if parsed["gender"] != preset["gender"]:
        raise RuntimeError(
            f"Fallback gender mismatch: {parsed['gender']} != {preset['gender']}"
        )

    selected_slots = {component["slot"] for component in manifest["components"]}
    actual_slots = set(parsed["asset_lists"])
    missing_slots = sorted(selected_slots - actual_slots)
    if missing_slots:
        raise RuntimeError(f"Fallback recipe is missing slots: {missing_slots}")

    for component in manifest["components"]:
        slot = component["slot"]
        matches = [
            asset
            for asset in parsed["asset_lists"][slot]
            if asset["asset_id"] == component["asset_id"]
        ]
        if len(matches) != 1:
            raise RuntimeError(
                f"{slot} expected asset {component['asset_id']}, found {len(matches)}"
            )
        models = matches[0]["models"]
        if len(models) < 2:
            raise RuntimeError(f"{slot} is missing its retail LOD pair")
        high, low = models[0], models[1]
        expected_high = {
            "model_id": component["model_id"],
            "model_arena_id": component["model_arena_id"],
            "material_id": component["material_id"],
            "textures": component["textures"],
        }
        for key, expected in expected_high.items():
            if high[key] != expected:
                raise RuntimeError(
                    f"{slot} {key} mismatch:\n"
                    f"  recipe={high[key]}\n  expected={expected}"
                )
        if low["model_id"] != component["low_model_id"]:
            raise RuntimeError(f"{slot} low LOD model ID changed")
        if low["model_arena_id"] != component["low_model_arena_id"]:
            raise RuntimeError(f"{slot} low LOD arena ID changed")

    body = parsed["body_mods"]
    if len(body) != 19:
        raise RuntimeError(f"Expected 19 body modifiers, found {len(body)}")
    if abs(body[0] - preset["body_mods"]["skinniness"]) > 1.0e-7:
        raise RuntimeError("Fallback skinniness changed")
    if abs(body[1] - preset["body_mods"]["fatness"]) > 1.0e-7:
        raise RuntimeError("Fallback fatness changed")
    expected_face = preset["body_mods"]["face_fields"]
    if any(abs(value - expected_face) > 1.0e-7 for value in body[2:]):
        raise RuntimeError("Fallback face/body modifier baseline changed")


def import_rx2_parser(utt_root: Path):
    module_path = require_file(utt_root / "rx2_parser.py", "RX2 texture decoder")
    spec = importlib.util.spec_from_file_location("skate3_local_rx2_parser", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not load RX2 decoder: {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # Loading by filename does not put the sibling directory on sys.path.
    # In particular, character-only setup used the slow scalar fallback while
    # map setup happened to make rx2_fast importable. Load the matching bundled
    # implementation explicitly, independent of import order or global paths.
    fast_spec = importlib.util.spec_from_file_location("skate3_local_rx2_fast", utt_root / "rx2_fast.py")
    fast = importlib.util.module_from_spec(fast_spec)
    fast_spec.loader.exec_module(fast)
    for name in ('decode_dxt1', 'decode_dxt1_normal', 'decode_dxt3', 'decode_dxt5',
                 'decode_ati2', '_untile360', '_decode_raw_a8r8g8b8',
                 '_decode_raw_b5g6r5', '_decode_raw_a8'):
        setattr(module, name, getattr(fast, name))
    return module


def decode_texture(module, source: Path, destination: Path) -> dict[str, Any]:
    parsed = module.parse_rx2(source)
    if len(parsed.textures) != 1:
        raise RuntimeError(
            f"Expected one texture in {source.name}, found {len(parsed.textures)}"
        )
    texture = parsed.textures[0]
    destination.parent.mkdir(parents=True, exist_ok=True)
    texture.save_png(destination)
    return {
        "width": texture.width,
        "height": texture.height,
        "format_id": texture.fmt_id,
        "format": texture.fmt_name,
        "png_sha256": sha256(destination),
        "warnings": list(parsed.warnings),
    }


def build_composite_images(
    manifest: dict[str, Any], decoded_root: Path, output_root: Path
) -> dict[str, dict[str, str]]:
    from PIL import Image, ImageChops

    def reconstruct_dxt5nm(source: Image.Image) -> Image.Image:
        """Convert Skate 3's DXT5nm `.ag` storage to a glTF RGB normal."""
        packed = source.convert("RGBA")
        converted: list[tuple[int, int, int]] = []
        pixels = (
            packed.get_flattened_data()
            if hasattr(packed, "get_flattened_data")
            else packed.getdata()
        )
        for _red, green, _blue, alpha in pixels:
            x = alpha / 127.5 - 1.0
            y = green / 127.5 - 1.0
            xy_length_squared = x * x + y * y
            if xy_length_squared > 1.0:
                inverse_length = 1.0 / math.sqrt(xy_length_squared)
                x *= inverse_length
                y *= inverse_length
                z = 0.0
            else:
                z = math.sqrt(1.0 - xy_length_squared)
            converted.append(
                (
                    round((x * 0.5 + 0.5) * 255.0),
                    round((y * 0.5 + 0.5) * 255.0),
                    round((z * 0.5 + 0.5) * 255.0),
                )
            )
        result = Image.new("RGB", packed.size)
        result.putdata(converted)
        return result

    outputs: dict[str, dict[str, str]] = {}
    for component in manifest["components"]:
        slot = component["slot"]
        textures = component["textures"]
        diffuse = Image.open(decoded_root / f"{textures['diffuse']}.png").convert(
            "RGBA"
        )
        # The decal/decal2 references are shader inputs, not unconditional
        # overlays. The fallback recipe has no active graphic URL, so baking
        # those generic template maps into base color would create false face
        # markings and logos. Preserve them as decoded maps while using the
        # selected diffuse as the retail base-color layer.
        result = diffuse
        if slot == "Hair" and "alpha" in textures:
            alpha = Image.open(decoded_root / f"{textures['alpha']}.png").convert("RGBA")
            if alpha.size != result.size:
                alpha = alpha.resize(result.size, Image.Resampling.LANCZOS)
            # Retail hair alpha files are A8-like masks. Prefer their alpha
            # channel, falling back to luminance when the decoder reports a
            # fully opaque channel.
            mask = alpha.getchannel("A")
            if mask.getextrema() == (255, 255):
                mask = alpha.convert("L")
            result.putalpha(ImageChops.multiply(result.getchannel("A"), mask))
        destination = output_root / f"{slot}_base_color.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        result.save(destination, "PNG")
        outputs[slot] = {"base_color_sha256": sha256(destination)}

        # The character shaders sample DXT5 normals as `.ag`: X is stored
        # in alpha and Y in green, then Z is reconstructed. Feeding the
        # decoded grayscale RGB directly to glTF produces near-horizontal
        # normals and harsh black/white mottling in Bevy.
        normal_id = textures.get("normal")
        if normal_id:
            normal = reconstruct_dxt5nm(
                Image.open(decoded_root / f"{normal_id}.png")
            )
            normal_destination = output_root / f"{slot}_normal.png"
            normal.save(normal_destination, "PNG")
            outputs[slot]["normal_sha256"] = sha256(normal_destination)

        specular_id = textures.get("specular")
        if specular_id:
            specular = Image.open(
                decoded_root / f"{specular_id}.png"
            ).convert("L")
            roughness = ImageChops.invert(specular)
            roughness_destination = output_root / f"{slot}_roughness.png"
            roughness.save(roughness_destination, "PNG")
            outputs[slot]["roughness_sha256"] = sha256(roughness_destination)
    return outputs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument(
        "--owned-data-root",
        type=Path,
        default=PROJECT_ROOT / "work" / "private-assets" / "owned-game",
    )
    parser.add_argument(
        "--utt-root",
        type=Path,
        default=PROJECT_ROOT / "tools" / "vendor" / "utt",
    )
    parser.add_argument("--work-root", type=Path, default=DEFAULT_WORK)
    parser.add_argument("--private-root", type=Path, default=DEFAULT_PRIVATE)
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()

    manifest = json.loads(require_file(args.manifest, "retail manifest").read_text())
    recipe_hash = manifest["preset"]["recipe_sha256"]
    recipe = require_file(
        args.owned_data_root
        / "data"
        / "cacrecipes"
        / "SavedRecipeFallbackMale.bin",
        "ISO-extracted SavedRecipeFallbackMale.bin",
    )
    if sha256(recipe) != recipe_hash:
        raise RuntimeError("Fallback recipe hash changed")
    if recipe.stat().st_size != manifest["preset"]["recipe_size"]:
        raise RuntimeError("Fallback recipe size changed")
    parsed = parse_fallback_recipe(recipe, manifest["preset"]["payload_offset"])
    assert_recipe(parsed, manifest)

    utt_root = args.utt_root
    cac_root = (
        args.owned_data_root / "data" / "content" / "createacharacter"
    )
    model_root = cac_root / "model" / "cas_db"
    texture_root = cac_root / "texture"

    selected_models = args.work_root / "selected" / "models"
    selected_raw_textures = args.work_root / "selected" / "textures"
    decoded_root = args.private_root / "textures" / "decoded"
    composite_root = args.private_root / "textures" / "materials"
    generated: dict[str, Any] = {
        "schema": 1,
        "normal_encoding": "dxt5nm-ag-to-gltf-rgb-v1",
        "body_mods": parsed["body_mods"],
        "recipe": str(recipe),
        "recipe_sha256": recipe_hash,
        "archive": "owned-retail-createacharacter.big",
        "archive_sha256": manifest["archive"]["sha256"],
        "models": {},
        "textures": {},
    }

    rx2_parser = import_rx2_parser(utt_root)
    all_texture_ids = sorted(
        {
            texture_id
            for component in manifest["components"]
            for texture_id in component["textures"].values()
        }
    )
    for component in manifest["components"]:
        slot = component["slot"]
        model_id = component["model_id"]
        source = require_file(
            model_root / slot / f"0x{model_id}.rx2", f"{slot} LOD0 model"
        )
        actual_hash = sha256(source)
        expected_hash = manifest["model_sha256"][model_id]
        if actual_hash != expected_hash:
            raise RuntimeError(
                f"{slot} model hash mismatch: {actual_hash} != {expected_hash}"
            )
        destination = selected_models / slot / source.name
        if not args.verify_only:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        generated["models"][slot] = {
            "id": model_id,
            "sha256": actual_hash,
            "bytes": source.stat().st_size,
        }

    for texture_id in all_texture_ids:
        source = require_file(
            texture_root / f"0x{texture_id}.rx2", f"texture {texture_id}"
        )
        actual_hash = sha256(source)
        expected_hash = manifest["texture_sha256"][texture_id]
        if actual_hash != expected_hash:
            raise RuntimeError(
                f"Texture {texture_id} hash mismatch: "
                f"{actual_hash} != {expected_hash}"
            )
        raw_destination = selected_raw_textures / source.name
        if not args.verify_only:
            raw_destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, raw_destination)
        png_destination = decoded_root / f"{texture_id}.png"
        metadata = decode_texture(rx2_parser, source, png_destination)
        metadata.update(
            {
                "rx2_sha256": actual_hash,
                "rx2_bytes": source.stat().st_size,
            }
        )
        generated["textures"][texture_id] = metadata

    generated["material_images"] = build_composite_images(
        manifest, decoded_root, composite_root
    )
    stable_assembly = {
        key: value
        for key, value in generated.items()
        if key not in {"recipe", "archive"}
    }
    generated["assembly_sha256"] = hashlib.sha256(
        json.dumps(stable_assembly, sort_keys=True).encode("utf-8")
    ).hexdigest().upper()
    generated_path = args.work_root / "default_skater.generated.json"
    generated_path.parent.mkdir(parents=True, exist_ok=True)
    generated_path.write_text(
        json.dumps(generated, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        "DEFAULT_SKATER_EXTRACTION_OK "
        f"models={len(generated['models'])} "
        f"textures={len(generated['textures'])} "
        f"assembly={generated['assembly_sha256']} "
        f"verify_only={args.verify_only}"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"DEFAULT_SKATER_EXTRACTION_FAILED: {error}", file=sys.stderr)
        raise
