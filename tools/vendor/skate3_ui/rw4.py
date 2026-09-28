from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path, PurePath

from .binary import FormatError, Reader
from .xenos import decode_rgba, parse_fetch_constant, write_png

TOC_TYPE = 0x00EB000B


@dataclass
class Texture:
    index: int
    name: str
    guid: int
    data: bytes
    platform: str
    metadata: dict


def _safe_name(name: str, index: int) -> str:
    leaf = PurePath(name.replace("\\", "/")).name
    filtered = "".join(c if c.isalnum() or c in "._-" else "_" for c in leaf)
    return filtered or f"texture_{index:04d}"


def read_textures(path: Path) -> tuple[dict, list[Texture]]:
    path = Path(path)
    raw = path.read_bytes()
    r = Reader(raw, str(path))
    r.require(0, 68)
    if r.bytes(0, 4) != b"\x89RW4":
        raise FormatError("not a RenderWare 4 arena")
    platform_magic = r.bytes(4, 4)
    if platform_magic == b"ps3\0":
        platform = "PS3"
    elif platform_magic == b"xb2\0":
        platform = "XBOX360"
    else:
        raise FormatError(f"unknown RW4 platform magic {platform_magic!r}")

    arena_offset = 32
    entry_count = r.u32be(arena_offset)
    dictionary_offset = r.u32be(arena_offset + 16)
    resource_header_size = r.u32be(arena_offset + 36)
    if entry_count > 1_000_000:
        raise FormatError("unreasonable RW4 dictionary count")
    r.require(dictionary_offset, entry_count * 24)
    dictionary = []
    for index in range(entry_count):
        offset = dictionary_offset + index * 24
        dictionary.append(
            {
                "pointer": r.u32be(offset),
                "relocation": r.u32be(offset + 4),
                "size": r.u32be(offset + 8),
                "alignment": r.u32be(offset + 12),
                "type_index": r.u32be(offset + 16),
                "type_id": r.u32be(offset + 20),
            }
        )

    toc = next((item for item in dictionary if item["type_id"] == TOC_TYPE), None)
    if not toc:
        raise FormatError("RW4 arena has no table of contents")
    toc_offset = toc["pointer"]
    r.require(toc_offset, 20)
    item_count = r.u32be(toc_offset)
    item_array = r.u32be(toc_offset + 4)
    if item_count > 1_000_000:
        raise FormatError("unreasonable RW4 TOC count")
    r.require(toc_offset + item_array, item_count * 24)

    textures = []
    for item_index in range(item_count):
        item = toc_offset + item_array + item_index * 24
        name_offset = r.u32be(item)
        name = r.cstring(toc_offset + name_offset)
        guid = r.u64be(item + 8)
        dictionary_index = r.u32be(item + 20) - 1
        if dictionary_index < 0 or dictionary_index + 1 >= len(dictionary):
            continue
        payload = dictionary[dictionary_index]
        info = dictionary[dictionary_index + 1]
        data_offset = payload["pointer"] + resource_header_size
        texture_data = r.bytes(data_offset, payload["size"])
        metadata = {"raw_size": len(texture_data)}
        if platform == "PS3":
            r.require(info["pointer"], 40)
            metadata.update(
                {
                    "format": r.u8(info["pointer"]),
                    "mipmaps": r.u8(info["pointer"] + 1),
                    "remap": r.u32be(info["pointer"] + 4),
                    "width": r.u16be(info["pointer"] + 8),
                    "height": r.u16be(info["pointer"] + 10),
                    "texture_offset": r.u32be(info["pointer"] + 20),
                    "store_type": r.u32be(info["pointer"] + 28),
                }
            )
        else:
            r.require(info["pointer"] + 28, 24)
            fetch_data = r.bytes(info["pointer"] + 28, 24)
            metadata["fetch_constant_hex"] = fetch_data.hex()
            metadata.update(parse_fetch_constant(fetch_data).manifest())
        textures.append(
            Texture(item_index, name, guid, texture_data, platform, metadata)
        )

    manifest = {
        "format": "renderware4-arena",
        "source": str(path),
        "platform": platform,
        "arena_id": r.u32be(28),
        "dictionary_entries": entry_count,
    }
    return manifest, textures


def extract_textures(
    path: Path, output: Path, force: bool = False, decode_png: bool = False
) -> dict:
    manifest, textures = read_textures(path)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    entries = []
    for texture in textures:
        name = _safe_name(texture.name, texture.index)
        target = output / f"{texture.index:04d}_{name}.texture.bin"
        if target.exists() and not force:
            raise FileExistsError(f"refusing to overwrite {target}")
        target.write_bytes(texture.data)
        entry = {
            "index": texture.index,
            "name": texture.name,
            "guid": f"{texture.guid:016x}",
            "raw_file": target.name,
            **texture.metadata,
        }
        if decode_png and texture.platform == "XBOX360":
            preview = output / f"{texture.index:04d}_{name}.png"
            if preview.exists() and not force:
                raise FileExistsError(f"refusing to overwrite {preview}")
            try:
                fetch = parse_fetch_constant(
                    bytes.fromhex(texture.metadata["fetch_constant_hex"])
                )
                rgba = decode_rgba(texture.data, fetch)
                rgba_target = output / f"{texture.index:04d}_{name}.rgba"
                if rgba_target.exists() and not force:
                    raise FileExistsError(f"refusing to overwrite {rgba_target}")
                rgba_target.write_bytes(rgba)
                write_png(preview, fetch.width, fetch.height, rgba)
                entry["rgba_file"] = rgba_target.name
                entry["preview_file"] = preview.name
            except (FormatError, ValueError) as error:
                entry["preview_error"] = str(error)
        entries.append(entry)
    manifest["textures"] = entries
    return manifest
