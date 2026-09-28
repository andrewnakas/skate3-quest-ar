from __future__ import annotations

from .binary import FormatError
from tools.asset_pipeline.fast_refpack import decode as fast_decode


def _copy_backref(output: bytearray, distance: int, count: int) -> None:
    if distance <= 0 or distance > len(output):
        raise FormatError(
            f"RefPack back-reference distance {distance} exceeds "
            f"{len(output)} output bytes"
        )
    for _ in range(count):
        output.append(output[-distance])


def decompress(data: bytes, expected_size: int | None = None) -> bytes:
    """Decode an EA RefPack stream, including 0x10FB/0x90FB headers."""
    if len(data) < 2:
        raise FormatError("RefPack stream is too short")

    position = 0
    header_size = None
    if data[1] == 0xFB and data[0] in (0x10, 0x90):
        if data[0] & 0x80:
            if len(data) < 6:
                raise FormatError("truncated extended RefPack header")
            header_size = int.from_bytes(data[2:6], "big")
            position = 6
        else:
            if len(data) < 5:
                raise FormatError("truncated RefPack header")
            header_size = int.from_bytes(data[2:5], "big")
            position = 5

    if expected_size is None:
        expected_size = header_size
    elif header_size not in (None, expected_size):
        raise FormatError(
            f"RefPack header size {header_size} differs from archive size "
            f"{expected_size}"
        )

    if expected_size is not None:
        try: decoded = fast_decode(data, expected_size, position)
        except ValueError as error: raise FormatError(str(error)) from error
        if decoded is not None: return decoded
    output = bytearray()

    def literal(count: int) -> None:
        nonlocal position
        if position + count > len(data):
            raise FormatError("truncated RefPack literal")
        output.extend(data[position : position + count])
        position += count

    while position < len(data):
        control = data[position]
        position += 1
        if control < 0x80:
            if position >= len(data):
                raise FormatError("truncated two-byte RefPack command")
            b1 = data[position]
            position += 1
            literal(control & 0x03)
            distance = ((control & 0x60) << 3) + b1 + 1
            count = ((control >> 2) & 0x07) + 3
            _copy_backref(output, distance, count)
        elif control < 0xC0:
            if position + 2 > len(data):
                raise FormatError("truncated three-byte RefPack command")
            b1, b2 = data[position], data[position + 1]
            position += 2
            literal(b1 >> 6)
            distance = ((b1 & 0x3F) << 8) + b2 + 1
            _copy_backref(output, distance, (control & 0x3F) + 4)
        elif control < 0xE0:
            if position + 3 > len(data):
                raise FormatError("truncated four-byte RefPack command")
            b1, b2, b3 = data[position], data[position + 1], data[position + 2]
            position += 3
            literal(control & 0x03)
            distance = ((control & 0x10) << 12) + (b1 << 8) + b2 + 1
            count = ((control & 0x0C) << 6) + b3 + 5
            _copy_backref(output, distance, count)
        elif control < 0xFC:
            literal(((control & 0x1F) << 2) + 4)
        else:
            literal(control & 0x03)
            break

        if expected_size is not None and len(output) > expected_size:
            raise FormatError("RefPack output exceeds declared size")

    if expected_size is not None and len(output) != expected_size:
        raise FormatError(
            f"RefPack produced {len(output)} bytes; expected {expected_size}"
        )
    return bytes(output)
