from __future__ import annotations

import re
from pathlib import Path

from .binary import FormatError


_FIELD = re.compile(r"(?m)^([A-Za-z][A-Za-z0-9]*):[ \t]*(.*)$")


def _fields(text: str) -> dict[str, str]:
    matches = list(_FIELD.finditer(text))
    result = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        value = text[match.start(2) : end].replace("\r", "").replace("\n", " ")
        result[match.group(1)] = value.strip()
    return result


def _number(value: str) -> int | float:
    return float(value) if any(char in value for char in ".eE") else int(value)


def parse_bitmap_font(path: Path) -> dict:
    path = Path(path)
    fields = _fields(path.read_text(encoding="cp1252"))
    required = {"Version", "FullName", "Family", "GlyphMetricsMap", "CharMapSet"}
    missing = sorted(required - fields.keys())
    if missing:
        raise FormatError(f"{path}: missing bitmap-font fields: {', '.join(missing)}")
    if fields["Version"] != "3":
        raise FormatError(f"{path}: unsupported bitmap-font version")

    metric_chunks = [chunk.strip() for chunk in fields["GlyphMetricsMap"].split(",")]
    first = metric_chunks[0].split()
    if len(first) != 10:
        raise FormatError(f"{path}: malformed first glyph metric")
    declared_glyphs = int(first.pop(0))
    metric_chunks[0] = " ".join(first)
    glyphs = []
    for chunk in metric_chunks:
        values = [int(value) for value in chunk.split()]
        if len(values) != 9:
            raise FormatError(f"{path}: malformed glyph metric {chunk!r}")
        glyph = dict(
            zip(
                (
                    "glyph_index",
                    "texture_index",
                    "x",
                    "y",
                    "width",
                    "height",
                    "x_offset",
                    "y_offset",
                    "x_advance",
                ),
                values,
            )
        )
        # EA::Text::BmpGlyphMetrics stores X/Y as the glyph's pen origin.
        # EATextBmpFont.cpp derives the physical atlas rectangle by adding the
        # horizontal bearing and subtracting the vertical bearing.
        atlas_left = glyph["x"] + glyph["x_offset"]
        atlas_top = glyph["y"] - glyph["y_offset"]
        glyph["atlas_bounds"] = [
            atlas_left,
            atlas_top,
            atlas_left + glyph["width"],
            atlas_top + glyph["height"],
        ]
        glyphs.append(glyph)
    if len(glyphs) != declared_glyphs:
        raise FormatError(
            f"{path}: declared {declared_glyphs} glyphs, found {len(glyphs)}"
        )

    char_chunks = [chunk.strip() for chunk in fields["CharMapSet"].split(",")]
    first = char_chunks[0].split()
    if len(first) != 3:
        raise FormatError(f"{path}: malformed first character mapping")
    declared_chars = int(first.pop(0))
    char_chunks[0] = " ".join(first)
    characters = []
    for chunk in char_chunks:
        values = [int(value) for value in chunk.split()]
        if len(values) != 2:
            raise FormatError(f"{path}: malformed character mapping {chunk!r}")
        characters.append({"codepoint": values[0], "glyph_index": values[1]})
    if len(characters) != declared_chars:
        raise FormatError(
            f"{path}: declared {declared_chars} characters, found {len(characters)}"
        )

    scalar_keys = (
        "Weight",
        "Stretch",
        "Size",
        "HAdvanceXMax",
        "VAdvanceYMax",
        "Ascent",
        "Descent",
        "Leading",
        "Baseline",
        "LineHeight",
        "XHeight",
        "CapsHeight",
        "UnderlinePosition",
        "UnderlineThickness",
        "LinethroughPosition",
        "LinethroughThickness",
    )
    metrics = {key: _number(fields[key]) for key in scalar_keys if key in fields}
    textures = []
    for key, value in fields.items():
        if not key.startswith("Texture"):
            continue
        parts = value.split()
        if len(parts) < 4:
            raise FormatError(f"{path}: malformed texture declaration {value!r}")
        textures.append(
            {
                "index": int(key[7:]),
                "bytes": int(parts[0]),
                "width": int(parts[1]),
                "height": int(parts[2]),
                "name": " ".join(parts[3:]),
            }
        )

    return {
        "format": "skate3-bitmap-font",
        "source": str(path),
        "full_name": fields["FullName"],
        "family": fields["Family"],
        "style": fields.get("Style", ""),
        "smooth": fields.get("Smooth") == "Yes",
        "fixed_pitch": fields.get("FixedPitch") == "Yes",
        "metrics": metrics,
        "glyphs": glyphs,
        "characters": characters,
        "textures": sorted(textures, key=lambda item: item["index"]),
    }


def measure_bitmap_text(font: dict, text: str, height: float) -> dict:
    mappings = {
        item["codepoint"]: item["glyph_index"] for item in font["characters"]
    }
    glyphs = {item["glyph_index"]: item for item in font["glyphs"]}
    source_ascent = float(font["metrics"]["Ascent"])
    if source_ascent <= 0:
        raise FormatError("bitmap font has a non-positive ascent")
    # APT dynamic-text fontHeight is the Flash ascent height. The bitmap
    # glyph bearings and advances are expressed in the bmpFont's source-pixel
    # coordinate system, whose matching vertical metric is Ascent rather than
    # the nominal point Size.
    scale = height / source_ascent
    pen = 0.0
    minimum = 0.0
    maximum = 0.0
    used = []
    for character in text:
        codepoint = ord(character)
        glyph_index = mappings.get(codepoint)
        if glyph_index is None or glyph_index not in glyphs:
            raise FormatError(
                f"bitmap font {font['family']!r} lacks codepoint {codepoint}"
            )
        glyph = glyphs[glyph_index]
        left = pen + glyph["x_offset"] * scale
        right = left + glyph["width"] * scale
        minimum = min(minimum, left)
        maximum = max(maximum, right)
        used.append(
            {
                "codepoint": codepoint,
                "glyph_index": glyph_index,
                "pen_x": pen,
                "left": left,
                "right": right,
            }
        )
        pen += glyph["x_advance"] * scale
    # Flash TextField autoSize retains the two-pixel gutter on each side.
    width = max(pen, maximum) - min(0.0, minimum) + 4.0
    return {
        "width": width,
        "advance": pen,
        "ink_bounds_x": [minimum, maximum],
        "height": height,
        "scale": scale,
        "glyphs": used,
    }
