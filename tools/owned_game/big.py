from __future__ import annotations

import zlib
from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath

from .binary import FormatError, Reader, align
from .refpack import decompress as decompress_refpack


@dataclass(frozen=True)
class BigEntry:
    index: int
    path: str
    offset: int
    stored_size: int
    unpacked_size: int
    compression: int


def _decompress_chunkref(data: bytes, expected_size: int) -> bytes:
    reader = Reader(data, "chunkref")
    if reader.bytes(0, 8) != b"chunkref" or reader.u32be(8) != 2:
        raise FormatError("invalid chunkref header")
    total_size = reader.u32be(12)
    chunk_size = reader.u32be(16)
    chunk_count = reader.u32be(20)
    alignment = reader.u32be(24)
    if (
        total_size != expected_size
        or chunk_size == 0
        or chunk_count == 0
        or alignment == 0
        or alignment > 4096
        or alignment & (alignment - 1)
    ):
        raise FormatError("invalid chunkref dimensions")

    output = bytearray()
    cursor = 28
    for _ in range(chunk_count):
        payload = align(cursor + 8, alignment)
        descriptor = payload - 8
        packed_size = reader.u32be(descriptor)
        method = reader.u32be(descriptor + 4)
        packed = reader.bytes(payload, packed_size)
        expected_chunk = min(chunk_size, total_size - len(output))
        if method == 0:
            decoded = packed
        elif method == 2:
            decoded = decompress_refpack(packed, expected_chunk)
        elif method == 3:
            try:
                decoded = zlib.decompress(packed)
            except zlib.error as error:
                raise FormatError("chunkref zlib stream is invalid") from error
        elif method == 4:
            decoded = packed
        else:
            raise FormatError(f"unsupported chunkref method {method}")
        if len(decoded) != expected_chunk:
            raise FormatError(
                f"chunkref chunk decoded to {len(decoded)} bytes; "
                f"expected {expected_chunk}"
            )
        output.extend(decoded)
        cursor = payload + packed_size

    if len(output) != total_size:
        raise FormatError(
            f"chunkref produced {len(output)} bytes; expected {total_size}"
        )
    return bytes(output)


class BigArchive:
    """Streaming reader for the EB BIG v3 archives used by Skate 3."""

    def __init__(self, path: Path):
        self.path = Path(path).resolve()
        self.file_size = self.path.stat().st_size
        with self.path.open("rb") as stream:
            header = stream.read(48)
            if len(header) != 48:
                raise FormatError(f"{self.path}: truncated BIG header")
            header_reader = Reader(header, str(self.path))
            index_size = header_reader.u32be(12)
            names_size = header_reader.u32be(16)
            metadata_size = index_size + names_size
            if metadata_size > self.file_size:
                raise FormatError("BIG index/name blocks exceed file")
            stream.seek(0)
            self.metadata = stream.read(metadata_size)
        self.entries = self._parse()

    def _parse(self) -> list[BigEntry]:
        reader = Reader(self.metadata, str(self.path))
        if reader.u16be(0) != 0x4542:
            raise FormatError("not an EB BIG archive")
        version = reader.u16be(2)
        if version != 3:
            raise FormatError(f"unsupported EB BIG version {version}")
        count = reader.u32be(4)
        if count > 1_000_000:
            raise FormatError(f"unreasonable BIG entry count {count}")
        flags = reader.u16be(8)
        shift = reader.u8(10)
        if shift > 31:
            raise FormatError(f"invalid BIG alignment shift {shift}")
        index_size = reader.u32be(12)
        names_size = reader.u32be(16)
        name_record_size = reader.u8(20)
        directory_record_size = reader.u8(21)
        if name_record_size < 3 or directory_record_size < 2:
            raise FormatError("invalid BIG name record sizing")

        entry_size = 20 if flags & 1 else 16
        entries_start = 48
        entries_bytes = entry_size * count
        compression_start = entries_start + align(entries_bytes, 16)
        names_start = index_size
        directories_start = names_start + align(name_record_size * count, 16)
        reader.require(entries_start, entries_bytes)
        reader.require(compression_start, count)
        reader.require(names_start, name_record_size * count)

        directories: list[str] = []
        directory_bytes_end = index_size + names_size
        cursor = directories_start
        while cursor + directory_record_size <= directory_bytes_end:
            directories.append(
                reader.bytes(cursor, directory_record_size)
                .split(b"\0", 1)[0]
                .decode("utf-8", errors="replace")
            )
            cursor += directory_record_size

        result: list[BigEntry] = []
        for index in range(count):
            entry_offset = entries_start + index * entry_size
            file_offset = reader.u32be(entry_offset) << shift
            declared_stored = reader.u32be(entry_offset + 4)
            unpacked_size = reader.u32be(entry_offset + 8) or declared_stored
            stored_size = declared_stored or unpacked_size
            compression = reader.u8(compression_start + index)
            name_record = names_start + index * name_record_size
            directory_index = reader.u16be(name_record)
            filename = (
                reader.bytes(name_record + 2, name_record_size - 2)
                .split(b"\0", 1)[0]
                .decode("utf-8", errors="replace")
            )
            directory = (
                directories[directory_index]
                if directory_index < len(directories)
                else "."
            )
            archive_path = filename if directory in ("", ".") else f"{directory}/{filename}"
            if file_offset > self.file_size - stored_size:
                raise FormatError(f"{archive_path}: payload exceeds BIG archive")
            result.append(
                BigEntry(
                    index=index,
                    path=archive_path.replace("\\", "/"),
                    offset=file_offset,
                    stored_size=stored_size,
                    unpacked_size=unpacked_size,
                    compression=compression,
                )
            )
        return result

    def manifest(self) -> list[dict]:
        return [asdict(entry) for entry in self.entries]

    def read(self, entry: BigEntry) -> bytes:
        with self.path.open("rb") as stream:
            stream.seek(entry.offset)
            packed = stream.read(entry.stored_size)
        if len(packed) != entry.stored_size:
            raise FormatError(f"{entry.path}: truncated BIG payload")
        if entry.compression == 0:
            decoded = packed
        elif entry.compression == 1:
            decoded = decompress_refpack(packed, entry.unpacked_size)
        elif entry.compression in (2, 3, 4):
            decoded = _decompress_chunkref(packed, entry.unpacked_size)
        else:
            raise FormatError(
                f"{entry.path}: unsupported BIG compression {entry.compression}"
            )
        if len(decoded) != entry.unpacked_size:
            raise FormatError(
                f"{entry.path}: decoded to {len(decoded)} bytes; "
                f"expected {entry.unpacked_size}"
            )
        return decoded

    @staticmethod
    def safe_relative(path: str) -> Path:
        pure = PurePosixPath(path.replace("\\", "/"))
        if pure.is_absolute() or not pure.parts or any(
            part in ("", ".", "..") for part in pure.parts
        ):
            raise FormatError(f"unsafe archive path {path!r}")
        if ":" in pure.parts[0]:
            raise FormatError(f"unsafe archive path {path!r}")
        return Path(*pure.parts)

    def extract_entries(
        self,
        entries: list[BigEntry],
        output: Path,
        overwrite: bool = False,
    ) -> list[Path]:
        output = Path(output).resolve()
        output.mkdir(parents=True, exist_ok=True)
        written: list[Path] = []
        for entry in entries:
            relative = self.safe_relative(entry.path)
            target = (output / relative).resolve()
            if output != target and output not in target.parents:
                raise FormatError(f"archive path escapes output: {entry.path!r}")
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists() and not overwrite:
                raise FileExistsError(f"refusing to overwrite {target}")
            target.write_bytes(self.read(entry))
            written.append(target)
        return written
