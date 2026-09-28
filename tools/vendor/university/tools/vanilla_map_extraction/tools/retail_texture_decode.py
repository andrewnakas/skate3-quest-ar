"""Decode Xbox 360 retail texture formats that need RX2-specific handling."""

from __future__ import annotations


B5G6R5_FORMAT_ID = 0x44
B5G6R5_DECODER_NAME = "xenos-tiled-packed-b5g6r5-v1"


def _align(value: int, alignment: int) -> int:
    return (value + alignment - 1) & ~(alignment - 1)


def _log2_ceil(value: int) -> int:
    if value <= 0:
        raise ValueError("texture dimensions must be positive")
    return (value - 1).bit_length()


def _packed_base_offset(width: int, height: int) -> tuple[int, int]:
    """Return the level-zero offset in a Xenos packed mip tail.

    Xenos packs a texture into a shared 32x32 tile once either dimension is
    16 texels or smaller. Level zero is placed at x=16 for square/tall
    textures and y=16 for wide textures. This is the level-zero specialization
    of Xenia's GetPackedMipOffset.
    """

    log2_width = _log2_ceil(width)
    log2_height = _log2_ceil(height)
    if min(log2_width, log2_height) > 4:
        return 0, 0
    if log2_width > log2_height:
        return 0, 16
    return 16, 0


def _tiled_offset_2d(
    x: int,
    y: int,
    pitch: int,
    bytes_per_texel_log2: int,
) -> int:
    """Return the Xenos tiled byte address for a 2D texel."""

    pitch = _align(pitch, 32)
    macro = (
        (x >> 5) + (y >> 5) * (pitch >> 5)
    ) << (bytes_per_texel_log2 + 7)
    micro = ((x & 7) + ((y & 0xE) << 2)) << bytes_per_texel_log2
    offset = (
        macro
        + ((micro & ~0xF) << 1)
        + (micro & 0xF)
        + ((y & 1) << 4)
    )
    return (
        ((offset & ~0x1FF) << 3)
        + ((y & 16) << 7)
        + ((offset & 0x1C0) << 2)
        + (((((y & 8) >> 2) + (x >> 3)) & 3) << 6)
        + (offset & 0x3F)
    )


def decode_b5g6r5(
    raw: bytes,
    width: int,
    height: int,
) -> bytes:
    """Decode a tiled Xenos B5G6R5 base level to row-major RGBA8888.

    RX2 stores each 16-bit word in big-endian byte order. The external UTT
    parser previously swapped those bytes and then interpreted them as
    big-endian again. It also read packed base levels from tile origin,
    exposing zero padding as black stripes.
    """

    if width <= 0 or height <= 0:
        raise ValueError("texture dimensions must be positive")
    packed_x, packed_y = _packed_base_offset(width, height)
    rgba = bytearray(width * height * 4)
    for y in range(height):
        for x in range(width):
            source_offset = _tiled_offset_2d(
                x + packed_x,
                y + packed_y,
                width,
                1,
            )
            if source_offset + 2 > len(raw):
                raise ValueError(
                    "B5G6R5 payload is too short for "
                    f"{width}x{height} texel ({x}, {y}): "
                    f"need {source_offset + 2} bytes, have {len(raw)}"
                )
            value = int.from_bytes(
                raw[source_offset : source_offset + 2],
                "big",
            )
            output_offset = (y * width + x) * 4
            rgba[output_offset] = ((value >> 11) & 0x1F) * 255 // 31
            rgba[output_offset + 1] = (
                ((value >> 5) & 0x3F) * 255 // 63
            )
            rgba[output_offset + 2] = (value & 0x1F) * 255 // 31
            rgba[output_offset + 3] = 255
    return bytes(rgba)
