from __future__ import annotations

import struct
import zlib
from dataclasses import asdict, dataclass
from pathlib import Path

from .binary import FormatError, align


XENOS_FORMATS = {
    6: "A8R8G8B8",
    18: "DXT1",
    19: "DXT3",
    20: "DXT5",
}


@dataclass(frozen=True)
class FetchConstant:
    width: int
    height: int
    format_code: int
    format_name: str
    mipmaps: int
    tiled: bool
    endian: int
    pitch: int = 0

    def manifest(self) -> dict:
        return asdict(self)


def reverse_bits(value: int, size: int) -> int:
    result = 0
    for bit in range(size):
        result |= ((value >> bit) & 1) << (size - bit - 1)
    return result


def parse_fetch_constant(data: bytes) -> FetchConstant:
    """Decode the 24-byte Xbox 360 texture fetch constant used by RX2."""
    if len(data) != 24:
        raise FormatError(f"Xbox texture fetch constant is {len(data)} bytes, expected 24")
    encoded = struct.unpack(">6I", data)
    words = [
        reverse_bits(encoded[0], 32),
        reverse_bits(encoded[1], 32),
        encoded[2],
        reverse_bits(encoded[3], 32),
        reverse_bits(encoded[4], 32),
        reverse_bits(encoded[5], 32),
    ]
    width = (words[2] & 0x1FFF) + 1
    height = ((words[2] >> 13) & 0x1FFF) + 1
    format_code = reverse_bits((words[1] >> 26) & 0x3F, 6)
    max_mip = reverse_bits((words[4] >> 22) & 0xF, 4)
    return FetchConstant(
        width=width,
        height=height,
        format_code=format_code,
        format_name=XENOS_FORMATS.get(format_code, f"XENOS_{format_code}"),
        mipmaps=max_mip + 1,
        tiled=bool(reverse_bits(words[0] & 1, 1)),
        endian=reverse_bits((words[1] >> 24) & 0x3, 2),
        pitch=((encoded[0] >> 22) & 0x1FF) << 5,
    )


def xbox360_tiled_offset(x: int, y: int, width: int, log_bpb: int) -> int:
    aligned_width = align(width, 32)
    macro = ((x >> 5) + (y >> 5) * (aligned_width >> 5)) << (log_bpb + 7)
    micro = ((x & 7) + ((y & 0xE) << 2)) << log_bpb
    offset = macro + ((micro & ~0xF) << 1) + (micro & 0xF) + ((y & 1) << 4)
    address = (
        ((offset & ~0x1FF) << 3)
        + ((y & 16) << 7)
        + ((offset & 0x1C0) << 2)
        + (((((y & 8) >> 2) + (x >> 3)) & 3) << 6)
        + (offset & 0x3F)
    )
    return address >> log_bpb


def _layout(fetch: FetchConstant) -> tuple[int, int]:
    if fetch.format_code == 18:
        return 4, 8
    if fetch.format_code in (19, 20):
        return 4, 16
    if fetch.format_code == 6:
        return 1, 4
    raise FormatError(f"unsupported Xbox texture format {fetch.format_name}")


def _endian_swap(block: bytes, compressed: bool) -> bytes:
    if compressed:
        swapped = bytearray(block)
        for index in range(0, len(swapped) - 1, 2):
            swapped[index], swapped[index + 1] = swapped[index + 1], swapped[index]
        return bytes(swapped)
    return block[::-1]


def linearize_base_level(data: bytes, fetch: FetchConstant) -> bytes:
    """Untile and endian-correct the base mip of an Xbox 360 texture."""
    block_size, bytes_per_block = _layout(fetch)
    block_width = max(1, (fetch.width + block_size - 1) // block_size)
    block_height = max(1, (fetch.height + block_size - 1) // block_size)
    linear = bytearray(block_width * block_height * bytes_per_block)
    compressed = block_size == 4

    if not fetch.tiled:
        pitch_texels = fetch.pitch or fetch.width
        pitch_blocks = max(
            block_width, (pitch_texels + block_size - 1) // block_size
        )
        row_pitch = pitch_blocks * bytes_per_block
        needed = (block_height - 1) * row_pitch + block_width * bytes_per_block
        if len(data) < needed:
            raise FormatError(f"texture payload is {len(data)} bytes, expected at least {needed}")
        for y in range(block_height):
            source_row = y * row_pitch
            target_row = y * block_width * bytes_per_block
            for x in range(block_width):
                source = source_row + x * bytes_per_block
                target = target_row + x * bytes_per_block
                linear[target : target + bytes_per_block] = _endian_swap(
                    data[source : source + bytes_per_block], compressed
                )
        return bytes(linear)

    tiled_width = align(fetch.width, 128 if compressed else 32) // block_size
    log_bpb = bytes_per_block.bit_length() - 1
    for y in range(block_height):
        for x in range(block_width):
            tiled_block = xbox360_tiled_offset(x, y, tiled_width, log_bpb)
            source = tiled_block * bytes_per_block
            target = (y * block_width + x) * bytes_per_block
            if source + bytes_per_block > len(data):
                raise FormatError(
                    f"tiled texture address {source:#x} exceeds {len(data):#x}-byte payload"
                )
            linear[target : target + bytes_per_block] = _endian_swap(
                data[source : source + bytes_per_block], compressed
            )
    return bytes(linear)


def _expand_565(value: int) -> tuple[int, int, int]:
    red = (value >> 11) & 0x1F
    green = (value >> 5) & 0x3F
    blue = value & 0x1F
    return (
        (red << 3) | (red >> 2),
        (green << 2) | (green >> 4),
        (blue << 3) | (blue >> 2),
    )


def _color_table(c0: int, c1: int, allow_transparency: bool) -> list[tuple[int, ...]]:
    color0 = _expand_565(c0)
    color1 = _expand_565(c1)
    if not allow_transparency or c0 > c1:
        return [
            (*color0, 255),
            (*color1, 255),
            tuple((2 * color0[i] + color1[i]) // 3 for i in range(3)) + (255,),
            tuple((color0[i] + 2 * color1[i]) // 3 for i in range(3)) + (255,),
        ]
    return [
        (*color0, 255),
        (*color1, 255),
        tuple((color0[i] + color1[i]) // 2 for i in range(3)) + (255,),
        (0, 0, 0, 0),
    ]


def _write_block(
    output: bytearray,
    width: int,
    height: int,
    block_x: int,
    block_y: int,
    colors: list[tuple[int, ...]],
    color_bits: int,
    alphas: list[int] | None = None,
) -> None:
    for py in range(4):
        for px in range(4):
            x = block_x * 4 + px
            y = block_y * 4 + py
            if x >= width or y >= height:
                continue
            pixel = py * 4 + px
            color = colors[(color_bits >> (pixel * 2)) & 3]
            alpha = alphas[pixel] if alphas is not None else color[3]
            offset = (y * width + x) * 4
            output[offset : offset + 4] = bytes((color[0], color[1], color[2], alpha))


def _decode_dxt(width: int, height: int, data: bytes, format_code: int) -> bytes:
    output = bytearray(width * height * 4)
    block_bytes = 8 if format_code == 18 else 16
    blocks_wide = max(1, (width + 3) // 4)
    blocks_high = max(1, (height + 3) // 4)
    source = 0
    for by in range(blocks_high):
        for bx in range(blocks_wide):
            if source + block_bytes > len(data):
                raise FormatError("compressed texture base mip is truncated")
            alphas = None
            color_offset = source
            if format_code == 19:
                alpha_bytes = data[source : source + 8]
                alphas = []
                for value in alpha_bytes:
                    alphas.extend(((value & 0xF) * 17, (value >> 4) * 17))
                color_offset += 8
            elif format_code == 20:
                alpha0, alpha1 = data[source], data[source + 1]
                palette = [alpha0, alpha1]
                if alpha0 > alpha1:
                    palette.extend(
                        ((7 - index) * alpha0 + index * alpha1) // 7
                        for index in range(1, 7)
                    )
                else:
                    palette.extend(
                        ((5 - index) * alpha0 + index * alpha1) // 5
                        for index in range(1, 5)
                    )
                    palette.extend((0, 255))
                alpha_bits = int.from_bytes(data[source + 2 : source + 8], "little")
                alphas = [palette[(alpha_bits >> (pixel * 3)) & 7] for pixel in range(16)]
                color_offset += 8
            c0, c1, color_bits = struct.unpack_from("<HHI", data, color_offset)
            colors = _color_table(c0, c1, format_code == 18)
            _write_block(output, width, height, bx, by, colors, color_bits, alphas)
            source += block_bytes
    return bytes(output)


def decode_rgba(data: bytes, fetch: FetchConstant) -> bytes:
    linear = linearize_base_level(data, fetch)
    if fetch.format_code in (18, 19, 20):
        return _decode_dxt(fetch.width, fetch.height, linear, fetch.format_code)
    if fetch.format_code == 6:
        output = bytearray(fetch.width * fetch.height * 4)
        for offset in range(0, len(output), 4):
            blue, green, red, alpha = linear[offset : offset + 4]
            output[offset : offset + 4] = bytes((red, green, blue, alpha))
        return bytes(output)
    raise FormatError(f"unsupported Xbox texture format {fetch.format_name}")


def _png_chunk(chunk_type: bytes, payload: bytes) -> bytes:
    checksum = zlib.crc32(chunk_type)
    checksum = zlib.crc32(payload, checksum)
    return (
        struct.pack(">I", len(payload))
        + chunk_type
        + payload
        + struct.pack(">I", checksum & 0xFFFFFFFF)
    )


def encode_png(width: int, height: int, rgba: bytes) -> bytes:
    expected = width * height * 4
    if len(rgba) != expected:
        raise FormatError(f"RGBA payload is {len(rgba)} bytes, expected {expected}")
    scanlines = bytearray()
    stride = width * 4
    for y in range(height):
        scanlines.append(0)
        scanlines.extend(rgba[y * stride : (y + 1) * stride])
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", header)
        + _png_chunk(b"IDAT", zlib.compress(bytes(scanlines), 9))
        + _png_chunk(b"IEND", b"")
    )


def write_png(path: Path, width: int, height: int, rgba: bytes) -> None:
    Path(path).write_bytes(encode_png(width, height, rgba))
