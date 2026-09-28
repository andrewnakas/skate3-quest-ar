from __future__ import annotations

from dataclasses import asdict, dataclass
from pathlib import Path, PurePosixPath

from .binary import FormatError, Reader, align
from .refpack import decompress


@dataclass(frozen=True)
class BigEntry:
    index: int
    path: str
    offset: int
    stored_size: int
    unpacked_size: int
    compression: int


class BigArchive:
    """Reader for the EB BIG v3 archives used by Skate 3 front-end data."""

    def __init__(self, path: Path):
        self.path = Path(path)
        self.data = self.path.read_bytes()
        self.reader = Reader(self.data, str(self.path))
        self.entries = self._parse()

    def _parse(self) -> list[BigEntry]:
        r = self.reader
        r.require(0, 48)
        if r.u16be(0) != 0x4542:
            raise FormatError("not an EB BIG archive")
        version = r.u16be(2)
        if version != 3:
            raise FormatError(f"unsupported EB BIG version {version}")
        count = r.u32be(4)
        if count > 1_000_000:
            raise FormatError(f"unreasonable BIG entry count {count}")
        flags = r.u16be(8)
        shift = r.u8(10)
        if shift > 31:
            raise FormatError(f"invalid BIG alignment shift {shift}")
        index_size = r.u32be(12)
        names_size = r.u32be(16)
        name_record_size = r.u8(20)
        directory_record_size = r.u8(21)
        if name_record_size < 3 or directory_record_size < 2:
            raise FormatError("invalid BIG name record sizing")
        if index_size > len(self.data) or names_size > len(self.data) - index_size:
            raise FormatError("BIG index/name blocks exceed file")

        entry_size = 20 if flags & 1 else 16
        entries_start = 48
        entries_bytes = entry_size * count
        compression_start = entries_start + align(entries_bytes, 16)
        names_start = index_size
        directories_start = names_start + align(name_record_size * count, 16)
        r.require(entries_start, entries_bytes)
        r.require(compression_start, count)
        r.require(names_start, name_record_size * count)

        directories: list[str] = []
        directory_bytes_end = index_size + names_size
        if directory_bytes_end > len(self.data):
            raise FormatError("BIG directory block exceeds file")
        cursor = directories_start
        while cursor + directory_record_size <= directory_bytes_end:
            directories.append(
                r.bytes(cursor, directory_record_size)
                .split(b"\0", 1)[0]
                .decode("utf-8", errors="replace")
            )
            cursor += directory_record_size

        result: list[BigEntry] = []
        for index in range(count):
            entry = entries_start + index * entry_size
            file_offset = r.u32be(entry) << shift
            declared_stored = r.u32be(entry + 4)
            unpacked_size = r.u32be(entry + 8) or declared_stored
            # EB v3 stores zero in the packed-size field for raw entries.
            stored_size = declared_stored or unpacked_size
            compression = r.u8(compression_start + index)
            name_record = names_start + index * name_record_size
            directory_index = r.u16be(name_record)
            filename = (
                r.bytes(name_record + 2, name_record_size - 2)
                .split(b"\0", 1)[0]
                .decode("utf-8", errors="replace")
            )
            directory = (
                directories[directory_index]
                if directory_index < len(directories)
                else "."
            )
            archive_path = filename if directory in ("", ".") else f"{directory}/{filename}"
            r.require(file_offset, stored_size)
            result.append(
                BigEntry(
                    index,
                    archive_path.replace("\\", "/"),
                    file_offset,
                    stored_size,
                    unpacked_size,
                    compression,
                )
            )
        return result

    def manifest(self) -> list[dict]:
        return [asdict(entry) for entry in self.entries]

    def read(self, entry: BigEntry) -> bytes:
        packed = self.reader.bytes(entry.offset, entry.stored_size)
        if entry.compression == 0:
            if len(packed) != entry.unpacked_size:
                raise FormatError(
                    f"{entry.path}: uncompressed size metadata does not match"
                )
            return packed
        if entry.compression == 1:
            return decompress(packed, entry.unpacked_size)
        raise FormatError(
            f"{entry.path}: compression {entry.compression} is not yet supported "
            "(2=chunked RefPack, 3=chunked zlib, 4=Xbox LZX)"
        )

    @staticmethod
    def _safe_relative(path: str) -> Path:
        pure = PurePosixPath(path.replace("\\", "/"))
        if pure.is_absolute() or not pure.parts or any(
            part in ("", ".", "..") for part in pure.parts
        ):
            raise FormatError(f"unsafe archive path {path!r}")
        if ":" in pure.parts[0]:
            raise FormatError(f"unsafe archive path {path!r}")
        return Path(*pure.parts)

    def extract(self, output: Path, force: bool = False) -> list[Path]:
        output = Path(output).resolve()
        output.mkdir(parents=True, exist_ok=True)
        written = []
        for entry in self.entries:
            relative = self._safe_relative(entry.path)
            target = (output / relative).resolve()
            if output != target and output not in target.parents:
                raise FormatError(f"archive path escapes output: {entry.path!r}")
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists() and not force:
                raise FileExistsError(f"refusing to overwrite {target}")
            target.write_bytes(self.read(entry))
            written.append(target)
        return written
