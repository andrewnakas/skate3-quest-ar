from __future__ import annotations

from .binary import FormatError


def _copy_backref(output: bytearray, distance: int, count: int) -> None:
    if distance <= 0 or distance > len(output):
        raise FormatError(
            f"RefPack back-reference distance {distance} exceeds "
            f"{len(output)} output bytes"
        )
    for _ in range(count):
        output.append(output[-distance])


def decompress(data: bytes, expected_size: int | None = None) -> bytes:
    """Decode an EA RefPack stream, including its usual 0x10FB/0x90FB header."""
    if len(data) < 2:
        raise FormatError("RefPack stream is too short")

    pos = 0
    header_size = None
    if data[1] == 0xFB and data[0] in (0x10, 0x90):
        if data[0] & 0x80:
            if len(data) < 6:
                raise FormatError("truncated extended RefPack header")
            header_size = int.from_bytes(data[2:6], "big")
            pos = 6
        else:
            if len(data) < 5:
                raise FormatError("truncated RefPack header")
            header_size = int.from_bytes(data[2:5], "big")
            pos = 5
    if expected_size is None:
        expected_size = header_size
    elif header_size not in (None, expected_size):
        raise FormatError(
            f"RefPack header size {header_size} differs from archive size "
            f"{expected_size}"
        )

    output = bytearray()

    def literal(count: int) -> None:
        nonlocal pos
        if pos + count > len(data):
            raise FormatError("truncated RefPack literal")
        output.extend(data[pos : pos + count])
        pos += count

    while pos < len(data):
        control = data[pos]
        pos += 1
        if control < 0x80:
            if pos >= len(data):
                raise FormatError("truncated two-byte RefPack command")
            b1 = data[pos]
            pos += 1
            literal(control & 0x03)
            distance = ((control & 0x60) << 3) + b1 + 1
            count = ((control >> 2) & 0x07) + 3
            _copy_backref(output, distance, count)
        elif control < 0xC0:
            if pos + 2 > len(data):
                raise FormatError("truncated three-byte RefPack command")
            b1, b2 = data[pos], data[pos + 1]
            pos += 2
            literal(b1 >> 6)
            distance = ((b1 & 0x3F) << 8) + b2 + 1
            _copy_backref(output, distance, (control & 0x3F) + 4)
        elif control < 0xE0:
            if pos + 3 > len(data):
                raise FormatError("truncated four-byte RefPack command")
            b1, b2, b3 = data[pos], data[pos + 1], data[pos + 2]
            pos += 3
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
