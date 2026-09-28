from __future__ import annotations

import struct


class FormatError(ValueError):
    """Raised when an input is malformed or outside supported bounds."""


class Reader:
    def __init__(self, data: bytes, label: str = "input"):
        self.data = data
        self.label = label

    def require(self, offset: int, size: int) -> None:
        if offset < 0 or size < 0 or offset > len(self.data) - size:
            raise FormatError(
                f"{self.label}: range {offset:#x}+{size:#x} is outside "
                f"{len(self.data):#x} bytes"
            )

    def bytes(self, offset: int, size: int) -> bytes:
        self.require(offset, size)
        return self.data[offset : offset + size]

    def u8(self, offset: int) -> int:
        self.require(offset, 1)
        return self.data[offset]

    def u16be(self, offset: int) -> int:
        self.require(offset, 2)
        return struct.unpack_from(">H", self.data, offset)[0]

    def u32be(self, offset: int) -> int:
        self.require(offset, 4)
        return struct.unpack_from(">I", self.data, offset)[0]

    def i32be(self, offset: int) -> int:
        self.require(offset, 4)
        return struct.unpack_from(">i", self.data, offset)[0]

    def u64be(self, offset: int) -> int:
        self.require(offset, 8)
        return struct.unpack_from(">Q", self.data, offset)[0]

    def f32be(self, offset: int) -> float:
        self.require(offset, 4)
        return struct.unpack_from(">f", self.data, offset)[0]

    def cstring(self, offset: int, limit: int | None = None) -> str:
        if offset <= 0 or offset >= len(self.data):
            return ""
        end_limit = len(self.data) if limit is None else min(len(self.data), offset + limit)
        end = self.data.find(b"\0", offset, end_limit)
        if end < 0:
            end = end_limit
        return self.data[offset:end].decode("utf-8", errors="replace")


def align(value: int, alignment: int) -> int:
    return (value + alignment - 1) & ~(alignment - 1)
