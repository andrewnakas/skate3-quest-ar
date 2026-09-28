"""Verify University B5G6R5 extraction and known retail constant maps."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image

from retail_texture_decode import B5G6R5_DECODER_NAME


EXPECTED_CONSTANTS = {
    "0x2c70170a000b0040": (255, 255, 255, 255),
    "0x0000043d03e3870a": (131, 129, 255, 255),
    "0x0000475d03e3870a": (131, 129, 255, 255),
    "0x000002b403e3870a": (131, 129, 131, 255),
}


def verify(manifest_path: Path) -> None:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    root = manifest_path.parent
    textures = manifest["textures"]
    decoded_count = 0
    for texture_id, texture in textures.items():
        if texture["format"] != "B5G6R5":
            continue
        decoded_count += 1
        if texture.get("decoder") != B5G6R5_DECODER_NAME:
            raise RuntimeError(
                f"{texture_id} did not use {B5G6R5_DECODER_NAME}"
            )

    for texture_id, expected_pixel in EXPECTED_CONSTANTS.items():
        texture = textures[texture_id]
        image_path = root / texture["png"]
        with Image.open(image_path) as image:
            pixel_bytes = image.convert("RGBA").tobytes()
        pixels = {
            tuple(pixel_bytes[offset : offset + 4])
            for offset in range(0, len(pixel_bytes), 4)
        }
        if pixels != {expected_pixel}:
            raise RuntimeError(
                f"{texture_id} is not the expected constant map: "
                f"actual={sorted(pixels)[:8]} expected={expected_pixel}"
            )

    print(
        "UNIVERSITY_B5G6R5_OK "
        f"textures={decoded_count} constants={len(EXPECTED_CONSTANTS)} "
        f"decoder={B5G6R5_DECODER_NAME}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    verify(args.manifest.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
