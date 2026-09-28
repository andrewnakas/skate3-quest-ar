from __future__ import annotations

import struct
from dataclasses import dataclass
from pathlib import Path

from .binary import FormatError


@dataclass(frozen=True)
class LanguageTable:
    source: str
    declared_count: int
    pool_offset: int
    strings: tuple[str, ...]


def parse_language_table(path: Path) -> LanguageTable:
    path = Path(path)
    data = path.read_bytes()
    if len(data) < 28:
        raise FormatError(f"{path}: language table is shorter than its header")
    declared_count = struct.unpack_from("<I", data, 8)[0]
    header_size = struct.unpack_from("<I", data, 12)[0]
    pool_offset = struct.unpack_from("<I", data, 16)[0]
    if header_size != 28:
        raise FormatError(f"{path}: unsupported language header size {header_size}")
    # The pool descriptor occupies eight bytes at pool_offset. The actual
    # null-separated byte strings follow it.
    strings_offset = pool_offset + 8
    if strings_offset > len(data):
        raise FormatError(f"{path}: string pool lies outside the file")
    raw_strings = data[strings_offset:].split(b"\0")
    # The UI uses byte-oriented font character maps, including custom glyphs
    # in positions that are undefined in Windows-1252. Latin-1 gives every
    # source byte a lossless, one-to-one Unicode codepoint for the manifest.
    strings = tuple(value.decode("latin-1") for value in raw_strings)
    expected = declared_count + 2
    if len(strings) != expected:
        raise FormatError(
            f"{path}: expected {expected} pool strings, found {len(strings)}"
        )
    return LanguageTable(str(path), declared_count, pool_offset, strings)


def pair_language_tables(labels_path: Path, values_path: Path) -> dict:
    labels = parse_language_table(labels_path)
    values = parse_language_table(values_path)
    if len(labels.strings) != len(values.strings):
        raise FormatError(
            "language label/value pools have different numbers of strings"
        )

    entries = []
    seen: dict[str, int] = {}
    duplicate_labels = []
    for index, (label, value) in enumerate(zip(labels.strings, values.strings)):
        if label:
            if label in seen:
                duplicate_labels.append(
                    {"label": label, "first_index": seen[label], "index": index}
                )
            else:
                seen[label] = index
        entries.append({"index": index, "label": label, "value": value})

    return {
        "format": "skate3-language-pair",
        "encoding": "iso-8859-1-byte-preserving",
        "labels_source": labels.source,
        "values_source": values.source,
        "declared_count": labels.declared_count,
        "pool_entries": len(entries),
        "entries": entries,
        "duplicate_labels": duplicate_labels,
    }
