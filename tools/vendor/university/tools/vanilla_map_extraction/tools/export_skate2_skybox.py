#!/usr/bin/env python3
"""Extract Skate 2's retail sky and attach it to a SKATE v12-v14 map.

The source is ``miscboot.big``. Only ``DIST_skybox.rx2`` and its texture RX2
are read; proxy mountains, advertising signs, and unrelated textures in the
same resources are deliberately excluded. The destination package is copied
streamingly and receives a replaceable ``SKYB`` extension, so an 800+ MiB map
does not need to pass through Blender again.
"""

from __future__ import annotations

import argparse
import importlib
import io
import os
from pathlib import Path, PureWindowsPath
import shutil
import struct
import sys
import zlib

from extract_skate2_big4 import read_entries
from skate3_streams import decompress_refpack


MODEL_ENTRY = r"data\content\world\models\DIST_skybox.rx2"
TEXTURE_ENTRY = r"data\content\world\models\DIST_skybox_Textures.rx2"
SKY_TEXTURE_NAMES = (
    "sky_detail_0x0000727a03e3870a",
    "sky_gradient_0x0000727903e3870a",
    "sun_gradient_0x0000727803e3870a",
)
SKY_TEXTURE_INDICES = {
    SKY_TEXTURE_NAMES[0]: 0,
    SKY_TEXTURE_NAMES[1]: 3,
    SKY_TEXTURE_NAMES[2]: 4,
}
COPY_BYTES = 8 * 1024 * 1024


class PackageError(ValueError):
    """Raised when a package cannot be patched without guessing."""


class FileReader:
    def __init__(self, stream: io.BufferedReader) -> None:
        self.stream = stream

    @property
    def offset(self) -> int:
        return self.stream.tell()

    def take(self, size: int, label: str) -> bytes:
        data = self.stream.read(size)
        if size < 0 or len(data) != size:
            raise PackageError(f"{label} extends past the end of the package")
        return data

    def skip(self, size: int, label: str) -> None:
        if size < 0:
            raise PackageError(f"{label} has a negative size")
        self.stream.seek(size, os.SEEK_CUR)

    def u32(self, label: str) -> int:
        return struct.unpack("<I", self.take(4, label))[0]

    def u64(self, label: str) -> int:
        return struct.unpack("<Q", self.take(8, label))[0]

    def string(self, label: str) -> str:
        size = self.u32(f"{label} length")
        if size > 64 * 1024:
            raise PackageError(f"{label} exceeds the string limit")
        return self.take(size, label).decode("utf-8")

    def stored(self, label: str) -> None:
        method = self.u32(f"{label} storage method")
        stored_size = self.u32(f"{label} stored size")
        if method not in (0, 1, 2):
            raise PackageError(f"{label} uses unsupported storage method {method}")
        self.skip(stored_size, f"{label} payload")


def _read_big4_entry(archive: Path, name: str) -> bytes:
    normalized = str(PureWindowsPath(name)).casefold()
    matches = [
        entry
        for entry in read_entries(archive)
        if str(PureWindowsPath(entry.name)).casefold() == normalized
    ]
    if len(matches) != 1:
        raise ValueError(f"BIG4 contains {len(matches)} matches for {name!r}")
    entry = matches[0]
    with archive.open("rb") as stream:
        stream.seek(entry.offset)
        data = stream.read(entry.size)
    if len(data) != entry.size:
        raise ValueError(f"BIG4 entry {name!r} is truncated")
    return decompress_refpack(data)


def _load_utt(utt_root: Path):
    if not (utt_root / "mdl_parser" / "parser.py").is_file():
        raise ValueError(f"UTT model parser was not found under {utt_root}")
    sys.path.insert(0, str(utt_root))
    model_parser = importlib.import_module("mdl_parser.parser")
    texture_parser = importlib.import_module("rx2_parser")
    return model_parser, texture_parser


def _stored(data: bytes) -> bytes:
    compressed = zlib.compress(data, 9)
    if len(compressed) < len(data):
        return struct.pack("<II", 1, len(compressed)) + compressed
    return struct.pack("<II", 0, len(data)) + data


def _string(value: str) -> bytes:
    encoded = value.encode("utf-8")
    return struct.pack("<I", len(encoded)) + encoded


def build_skyb_payload(
    model_data: bytes,
    texture_data: bytes,
    utt_root: Path,
    *,
    elevation: float = 0.0,
    sun_angular_scale: float = 0.035,
    exposure_multiplier: float = 1.0,
) -> tuple[bytes, dict[str, object]]:
    model_parser, texture_parser = _load_utt(utt_root)
    model = model_parser.parse_rx2(model_data)
    sky_meshes = [
        mesh
        for mesh in model.meshes
        if str(mesh.name).casefold().startswith("skybox")
    ]
    if len(sky_meshes) != 1:
        raise ValueError(
            f"expected one named skybox mesh; found {len(sky_meshes)}"
        )
    mesh = sky_meshes[0]
    if len(mesh.vertices) == 0 or len(mesh.faces) == 0:
        raise ValueError("retail skybox mesh is empty")
    if len(mesh.vertices) > 65535:
        raise ValueError("retail skybox exceeds the renderer vertex limit")

    texture_file = texture_parser.parse_rx2(texture_data)
    if len(texture_file.textures) != 8:
        raise ValueError(
            "DIST_skybox_Textures.rx2 no longer has the expected 8 textures"
        )
    selected = {
        name: texture_file.textures[index]
        for name, index in SKY_TEXTURE_INDICES.items()
    }
    expected_dimensions = {
        SKY_TEXTURE_NAMES[0]: (2048, 256),
        SKY_TEXTURE_NAMES[1]: (512, 64),
        SKY_TEXTURE_NAMES[2]: (512, 16),
    }
    for name, texture in selected.items():
        if (texture.width, texture.height) != expected_dimensions[name]:
            raise ValueError(
                f"{name} is {texture.width}x{texture.height}; expected "
                f"{expected_dimensions[name][0]}x{expected_dimensions[name][1]}"
            )

    vertices = bytearray()
    for position, uv in zip(mesh.vertices, mesh.uvs, strict=True):
        vertices.extend(
            struct.pack(
                "<5f",
                float(position[0]),
                float(position[1]),
                float(position[2]),
                float(uv[0]),
                float(uv[1]),
            )
        )
    indices = bytearray()
    for face in mesh.faces:
        indices.extend(
            struct.pack("<3I", int(face[0]), int(face[1]), int(face[2]))
        )

    payload = bytearray()
    payload.extend(struct.pack("<II", len(mesh.vertices), len(mesh.faces) * 3))
    payload.extend(
        struct.pack(
            "<7f",
            elevation,
            1.0,
            1.0,
            1.0,
            1.0,
            sun_angular_scale,
            exposure_multiplier,
        )
    )
    # Shader order is gradient, detail, sun. All remain linear UNORM because
    # the recovered retail shader performs its own x^2 transfer.
    for name in (SKY_TEXTURE_NAMES[1], SKY_TEXTURE_NAMES[0], SKY_TEXTURE_NAMES[2]):
        texture = selected[name]
        rgba = bytes(texture.rgba)
        payload.extend(_string(name))
        payload.extend(struct.pack("<III", texture.width, texture.height, 0))
        payload.extend(_stored(rgba))
    payload.extend(_stored(bytes(vertices)))
    payload.extend(_stored(bytes(indices)))
    return bytes(payload), {
        "mesh": str(mesh.name),
        "vertices": len(mesh.vertices),
        "triangles": len(mesh.faces),
        "textures": {
            name: [texture.width, texture.height]
            for name, texture in selected.items()
        },
    }


def _find_extension_offset(path: Path) -> int:
    with path.open("rb") as stream:
        reader = FileReader(stream)
        magic = reader.take(8, "magic")
        if magic not in (b"SKATE12\0", b"SKATE13\0", b"SKATE14\0"):
            raise PackageError(
                f"{path} is not a SKATE v12-v14 package ({magic!r})"
            )
        version = int(magic[5:7])
        if reader.u32("endian marker") != 0x12345678:
            raise PackageError("package endian marker is invalid")
        reader.string("map name")
        reader.skip(49 * 4, "map metadata")
        counts = struct.unpack("<9I", reader.take(36, "count table"))
        (
            material_count,
            texture_count,
            vertex_count,
            index_count,
            collision_count,
            rail_count,
            door_count,
            light_count,
            route_count,
        ) = counts

        for index in range(material_count):
            reader.string(f"material {index} name")
            reader.skip(76, f"material {index} fields")
            if version >= 13:
                reader.u32(f"material {index} depth layer")
            if reader.u32(f"material {index} retail enabled"):
                reader.skip(16, f"material {index} retail ids")
                reader.string(f"material {index} retail shader")
                reader.skip(8, f"material {index} retail family/flags")
                binding_count = reader.u32(f"material {index} binding count")
                for binding in range(binding_count):
                    reader.string(f"material {index} binding {binding}")
                    reader.skip(16, f"material {index} binding fields")
                parameter_count = reader.u32(
                    f"material {index} parameter count"
                )
                for parameter in range(parameter_count):
                    reader.string(f"material {index} parameter {parameter}")
                    value_count = reader.u32(
                        f"material {index} parameter {parameter} value count"
                    )
                    for value in range(value_count):
                        reader.string(
                            f"material {index} parameter {parameter} value {value}"
                        )
                reader.string(f"material {index} source metadata")

        for index in range(texture_count):
            reader.string(f"texture {index} name")
            reader.skip(12, f"texture {index} dimensions")
            reader.stored(f"texture {index}")
        reader.stored("visual vertices")
        reader.stored("visual indices")
        reader.stored("collision")

        for index in range(rail_count):
            reader.string(f"rail {index} name")
            reader.u32(f"rail {index} closed")
            representation = reader.u32(f"rail {index} representation")
            if representation == 0:
                reader.skip(
                    reader.u32(f"rail {index} point count") * 12,
                    f"rail {index} points",
                )
            elif representation == 1:
                reader.skip(24, f"rail {index} native header")
                reader.skip(
                    reader.u32(f"rail {index} segment count") * 120,
                    f"rail {index} segments",
                )
            else:
                raise PackageError(
                    f"rail {index} has unsupported representation "
                    f"{representation}"
                )

        vertex_stride = 56
        collision_stride = 48
        for index in range(door_count):
            reader.string(f"door {index} name")
            reader.skip(28 * 4 + 4, f"door {index} fields")
            door_vertices = reader.u32(f"door {index} vertex count")
            door_indices = reader.u32(f"door {index} index count")
            door_collision = reader.u32(f"door {index} collision count")
            reader.skip(
                door_vertices * vertex_stride
                + door_indices * 4
                + door_collision * collision_stride,
                f"door {index} geometry",
            )
        for index in range(light_count):
            reader.string(f"light {index} name")
            reader.skip(4 + 14 * 4, f"light {index} fields")
        for index in range(route_count):
            reader.string(f"route {index} name")
            reader.skip(16, f"route {index} fields")
            reader.skip(
                reader.u32(f"route {index} point count") * 12,
                f"route {index} points",
            )
        return reader.offset


def _read_extensions(path: Path, offset: int) -> list[tuple[bytes, bytes]]:
    records: list[tuple[bytes, bytes]] = []
    with path.open("rb") as stream:
        stream.seek(offset)
        reader = FileReader(stream)
        count = reader.u32("extension count")
        if count > 1024:
            raise PackageError("extension count is implausible")
        for index in range(count):
            start = reader.offset
            tag = reader.take(4, f"extension {index} tag")
            reader.u32(f"extension {index} schema")
            reader.u32(f"extension {index} decoded size")
            reader.stored(f"extension {index}")
            end = reader.offset
            stream.seek(start)
            records.append((tag, stream.read(end - start)))
            stream.seek(end)
        stream.seek(0, os.SEEK_END)
        if reader.offset != stream.tell():
            raise PackageError("package has trailing bytes after extensions")
    return records


def attach_skyb(source: Path, destination: Path, payload: bytes) -> None:
    source = source.resolve()
    destination = destination.resolve()
    if source == destination:
        raise ValueError("destination must differ from source")
    offset = _find_extension_offset(source)
    records = [
        record
        for tag, record in _read_extensions(source, offset)
        if tag != b"SKYB"
    ]
    encoded = (
        b"SKYB"
        + struct.pack("<II", 1, len(payload))
        + _stored(payload)
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    with source.open("rb") as input_stream, temporary.open("wb") as output:
        remaining = offset
        while remaining:
            block = input_stream.read(min(remaining, COPY_BYTES))
            if not block:
                raise PackageError("source ended before its extension table")
            output.write(block)
            remaining -= len(block)
        output.write(struct.pack("<I", len(records) + 1))
        for record in records:
            output.write(record)
        output.write(encoded)
    os.replace(temporary, destination)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--utt-root", type=Path, required=True)
    parser.add_argument("--source-map", type=Path, required=True)
    parser.add_argument("--output-map", type=Path, required=True)
    parser.add_argument("--collision-sidecar", type=Path)
    parser.add_argument("--output-sidecar", type=Path)
    args = parser.parse_args()

    model_data = _read_big4_entry(args.archive.resolve(), MODEL_ENTRY)
    texture_data = _read_big4_entry(args.archive.resolve(), TEXTURE_ENTRY)
    payload, report = build_skyb_payload(
        model_data, texture_data, args.utt_root.resolve()
    )
    attach_skyb(
        args.source_map.resolve(), args.output_map.resolve(), payload
    )
    if args.collision_sidecar or args.output_sidecar:
        if not args.collision_sidecar or not args.output_sidecar:
            parser.error(
                "--collision-sidecar and --output-sidecar must be used together"
            )
        args.output_sidecar.resolve().parent.mkdir(
            parents=True, exist_ok=True
        )
        shutil.copy2(
            args.collision_sidecar.resolve(),
            args.output_sidecar.resolve(),
        )
    print(
        "Skate 2 SKYB attached:",
        args.output_map.resolve(),
        f"mesh={report['mesh']}",
        f"vertices={report['vertices']}",
        f"triangles={report['triangles']}",
        f"payload={len(payload):,}",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
