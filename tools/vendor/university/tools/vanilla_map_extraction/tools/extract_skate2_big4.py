"""List or extract Skate 2 BIG4 archives.

Skate 2's Xbox 360 archives use an EA BIG4 file table, but the legacy
``bigfile.exe`` bundled with UTT rejects this particular container variant.
The files themselves are stored verbatim, so only a small bounded table reader
is needed before the existing Skate stream extractor can take over.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import os
from pathlib import Path, PureWindowsPath


COPY_CHUNK_SIZE = 8 * 1024 * 1024


@dataclass(frozen=True)
class Big4Entry:
    offset: int
    size: int
    name: str


def _read_exact(stream, size: int) -> bytes:
    data = stream.read(size)
    if len(data) != size:
        raise ValueError("unexpected end of BIG4 archive")
    return data


def read_entries(archive_path: Path) -> list[Big4Entry]:
    archive_size = archive_path.stat().st_size
    with archive_path.open("rb") as stream:
        if _read_exact(stream, 4) != b"BIG4":
            raise ValueError(f"{archive_path} is not a BIG4 archive")

        stored_size_bytes = _read_exact(stream, 4)
        stored_size_be = int.from_bytes(stored_size_bytes, "big")
        stored_size_le = int.from_bytes(stored_size_bytes, "little")
        if archive_size not in (stored_size_be, stored_size_le):
            raise ValueError(
                "BIG4 stored archive size does not match the input file: "
                f"{stored_size_be} (BE), {stored_size_le} (LE), "
                f"actual {archive_size}"
            )

        entry_count = int.from_bytes(_read_exact(stream, 4), "big")
        header_size = int.from_bytes(_read_exact(stream, 4), "big")
        if not 16 <= header_size <= archive_size:
            raise ValueError(f"invalid BIG4 header size {header_size}")

        entries: list[Big4Entry] = []
        normalized_names: set[str] = set()
        for index in range(entry_count):
            offset = int.from_bytes(_read_exact(stream, 4), "big")
            size = int.from_bytes(_read_exact(stream, 4), "big")
            name_bytes = bytearray()
            while True:
                byte = _read_exact(stream, 1)
                if byte == b"\0":
                    break
                name_bytes.extend(byte)
                if len(name_bytes) > 4096:
                    raise ValueError(f"entry {index} has an invalid path")
            name = name_bytes.decode("ascii")
            normalized_name = str(PureWindowsPath(name)).casefold()
            if normalized_name in normalized_names:
                raise ValueError(
                    f"BIG4 contains a duplicate output path: {name!r}"
                )
            normalized_names.add(normalized_name)
            if offset < header_size or offset + size > archive_size:
                raise ValueError(
                    f"entry {index} is outside the archive: "
                    f"{name!r}, offset={offset}, size={size}"
                )
            entries.append(Big4Entry(offset=offset, size=size, name=name))

        if stream.tell() > header_size:
            raise ValueError(
                f"BIG4 file table ends at {stream.tell()}, "
                f"past declared header size {header_size}"
            )
    return entries


def _safe_output_path(output_root: Path, entry_name: str) -> Path:
    windows_path = PureWindowsPath(entry_name)
    if windows_path.is_absolute() or windows_path.drive:
        raise ValueError(f"unsafe absolute BIG4 entry path: {entry_name!r}")
    if any(part in ("", ".", "..") for part in windows_path.parts):
        raise ValueError(f"unsafe BIG4 entry path: {entry_name!r}")

    candidate = output_root.joinpath(*windows_path.parts).resolve()
    try:
        candidate.relative_to(output_root)
    except ValueError as error:
        raise ValueError(f"BIG4 entry escapes output root: {entry_name!r}") from error
    return candidate


def extract_archive(
    archive_path: Path,
    output_root: Path,
    entries: list[Big4Entry],
) -> None:
    output_root.mkdir(parents=True, exist_ok=True)
    output_root = output_root.resolve()
    with archive_path.open("rb") as source:
        for index, entry in enumerate(entries, start=1):
            destination = _safe_output_path(output_root, entry.name)
            if destination.is_file() and destination.stat().st_size == entry.size:
                print(f"[{index}/{len(entries)}] cached {entry.name}")
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            source.seek(entry.offset)
            remaining = entry.size
            temporary = destination.with_name(destination.name + ".tmp")
            with temporary.open("wb") as target:
                while remaining:
                    block = source.read(min(remaining, COPY_CHUNK_SIZE))
                    if not block:
                        raise ValueError(
                            f"unexpected end of archive while extracting {entry.name!r}"
                        )
                    target.write(block)
                    remaining -= len(block)
            os.replace(temporary, destination)
            print(f"[{index}/{len(entries)}] extracted {entry.name}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument(
        "--output",
        type=Path,
        help="Output directory. Omit to validate and list the archive.",
    )
    args = parser.parse_args()

    archive_path = args.archive.resolve()
    entries = read_entries(archive_path)
    total_size = sum(entry.size for entry in entries)
    print(
        f"{archive_path.name}: {len(entries)} entries, "
        f"{total_size:,} stored bytes"
    )
    if args.output is None:
        for entry in entries:
            print(f"{entry.offset:12d} {entry.size:12d} {entry.name}")
    else:
        extract_archive(archive_path, args.output, entries)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
