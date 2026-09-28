from __future__ import annotations

import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import numpy as np

from .errors import PSGDataError, PSGFormatError
from .model import Bone, MaterialParameter, Mesh, PSGModel, VertexAttribute

MAGIC_PS3 = b"\x89RW4ps3"
MODEL_TYPE_PS3 = b"\x00\x00\x00\x10"
MAGIC_X360 = b"\x89RW4xb2"
MODEL_TYPE_X360 = b"\x00\x00\x00\x04"
MODEL_TYPE_X360_NHL = b"\x00\x00\x08\x00"

TOC_RECORD_SIZE = 24
TYPE_VERTEX = b"\x00\x02\x00\xEA"
TYPE_FACE = b"\x00\x02\x00\xEB"
TYPE_MESH_INFO = b"\x00\x02\x00\xE9"
TYPE_MATERIAL = b"\x00\xEB\x00\x05"
TYPE_BONES = b"\x00\xEB\x00\x01"


@dataclass(slots=True, frozen=True)
class _TOCRecord:
    index: int
    offset: int
    values: tuple[int, int, int, int, int]
    resource_type: bytes


@dataclass(slots=True, frozen=True)
class _BufferResource:
    offset: int
    size: int
    info_offset: int
    toc_index: int


@dataclass(slots=True, frozen=True)
class _MeshLayout:
    stride: int
    position: VertexAttribute
    uv1: VertexAttribute | None
    attributes: tuple[VertexAttribute, ...]


_DTYPE_MAP: dict[str, np.dtype] = {
    "float32": np.dtype(">f4"),
    "float16": np.dtype(">f2"),
    "int16": np.dtype(">i2"),
    "uint16": np.dtype(">u2"),
}


def is_psg_model(data: bytes | bytearray | memoryview) -> bool:
    view = memoryview(data)
    return (
        len(view) >= 0x74
        and bytes(view[:7]) == MAGIC_PS3
        and bytes(view[0x70:0x74]) == MODEL_TYPE_PS3
    )


def is_rx2_model(data: bytes | bytearray | memoryview) -> bool:
    view = memoryview(data)
    return (
        len(view) >= 0x5C
        and bytes(view[:7]) == MAGIC_X360
        and bytes(view[0x58:0x5C]) in (MODEL_TYPE_X360, MODEL_TYPE_X360_NHL)
    )


def load_psg(path: str | Path, *, strict: bool = False) -> PSGModel:
    source_path = Path(path)
    return parse_psg(source_path.read_bytes(), source_path=source_path, strict=strict)


def load_rx2(path: str | Path, *, strict: bool = False) -> PSGModel:
    source_path = Path(path)
    return parse_rx2(source_path.read_bytes(), source_path=source_path, strict=strict)


def load_model(path: str | Path, *, strict: bool = False) -> PSGModel:
    """Load a model PSG or RX2, auto-detecting the platform by magic."""
    source_path = Path(path)
    data = source_path.read_bytes()
    if is_rx2_model(data):
        return parse_rx2(data, source_path=source_path, strict=strict)
    return parse_psg(data, source_path=source_path, strict=strict)


def parse_psg(
    data: bytes | bytearray | memoryview,
    *,
    source_path: str | Path | None = None,
    strict: bool = False,
) -> PSGModel:
    return _parse_model(data, source_path=source_path, strict=strict, platform="ps3")


def parse_rx2(
    data: bytes | bytearray | memoryview,
    *,
    source_path: str | Path | None = None,
    strict: bool = False,
) -> PSGModel:
    return _parse_model(data, source_path=source_path, strict=strict, platform="xbx")


def parse_model(
    data: bytes | bytearray | memoryview,
    *,
    source_path: str | Path | None = None,
    strict: bool = False,
) -> PSGModel:
    """Parse a model PSG or RX2, auto-detecting the platform by magic."""
    if is_rx2_model(data):
        return parse_rx2(data, source_path=source_path, strict=strict)
    return parse_psg(data, source_path=source_path, strict=strict)


def _parse_model(
    data: bytes | bytearray | memoryview,
    *,
    source_path: str | Path | None = None,
    strict: bool = False,
    platform: str = "ps3",
) -> PSGModel:
    raw = bytes(data)
    if platform == "xbx":
        if len(raw) < 0x5C:
            raise PSGFormatError("File is too small to contain an RX2 header")
        if raw[:7] != MAGIC_X360:
            raise PSGFormatError("Not an EA RenderWare4 Xbox 360 file")
        type_ok = raw[0x58:0x5C] in (MODEL_TYPE_X360, MODEL_TYPE_X360_NHL)
        type_offset = 0x58
        type_field = raw[0x58:0x5C]
    else:
        if len(raw) < 0x74:
            raise PSGFormatError("File is too small to contain a PSG header")
        if raw[:7] != MAGIC_PS3:
            raise PSGFormatError("Not an EA RenderWare4 PS3 file")
        type_ok = raw[0x70:0x74] == MODEL_TYPE_PS3
        type_offset = 0x70
        type_field = raw[0x70:0x74]
    if not type_ok:
        raise PSGFormatError(
            f"{'X360' if platform == 'xbx' else 'PS3'} file is not a model "
            f"(type {type_field.hex(' ') or '<missing>'})"
        )

    warnings: list[str] = []
    file_count = _read_i32(raw, 0x20, "file count")
    file_table = _read_i32(raw, 0x30, "file table offset")
    header_size = _read_i32(raw, 0x44, "header size")

    if not 0 < file_count <= 1_000_000:
        raise PSGDataError(f"Invalid PSG table record count: {file_count}")
    if file_table < 0:
        raise PSGDataError(f"Invalid PSG table offset: {file_table}")

    table_end = file_table + file_count * TOC_RECORD_SIZE
    if table_end > len(raw):
        raise PSGDataError(
            f"PSG table ends at 0x{table_end:X}, beyond file size 0x{len(raw):X}"
        )

    records = _read_toc(raw, file_table, file_count)
    vertex_resources: list[_BufferResource] = []
    face_resources: list[_BufferResource] = []
    mesh_info_offsets: list[int] = []
    material_offsets: list[int] = []
    bone_offsets: list[int] = []

    for record in records:
        resource_type = record.resource_type
        if resource_type in (TYPE_VERTEX, TYPE_FACE, TYPE_MATERIAL, TYPE_BONES):
            if record.offset < TOC_RECORD_SIZE:
                warnings.append(
                    f"TOC record {record.index} cannot access its preceding metadata block"
                )
                continue
            info = _unpack(raw, ">12i", record.offset - TOC_RECORD_SIZE, "TOC metadata")
        else:
            info = None

        next_paired_type = (
            records[record.index + 2].resource_type
            if record.index + 2 < len(records)
            else None
        )

        if resource_type == TYPE_VERTEX and info is not None:
            if next_paired_type == TYPE_FACE:
                vertex_resources.append(
                    _BufferResource(info[0], info[2], info[6], record.index)
                )
        elif resource_type == TYPE_FACE and info is not None:
            if next_paired_type != TYPE_FACE:
                face_resources.append(
                    _BufferResource(info[0], info[2], info[6], record.index)
                )
        elif resource_type == TYPE_MESH_INFO:
            mesh_info_offsets.append(record.values[0])
        elif resource_type == TYPE_MATERIAL and info is not None:
            material_offsets.append(info[6])
        elif resource_type == TYPE_BONES:
            bone_offsets.append(record.values[0])

    materials: list[MaterialParameter] = []
    mesh_names: list[str] = []
    diffuse_names: list[str] = []
    for material_offset in _deduplicate(material_offsets):
        try:
            parsed, names, diffuse = _parse_material_table(raw, material_offset)
            materials.extend(parsed)
            mesh_names.extend(names)
            diffuse_names.extend(diffuse)
        except PSGDataError as exc:
            if strict:
                raise
            warnings.append(f"Material table at 0x{material_offset:X}: {exc}")

    paired_count = min(
        len(vertex_resources), len(face_resources), len(mesh_info_offsets)
    )
    if paired_count == 0:
        raise PSGDataError(
            "No complete vertex/face/mesh-info set was found in the PSG table"
        )

    counts = (
        len(vertex_resources),
        len(face_resources),
        len(mesh_info_offsets),
    )
    if len(set(counts)) != 1:
        warnings.append(
            "Resource counts do not match: "
            f"{counts[0]} vertex, {counts[1]} face, {counts[2]} mesh-info; "
            f"using the first {paired_count} complete set(s)"
        )

    meshes: list[Mesh] = []
    for mesh_index in range(paired_count):
        vertex_resource = vertex_resources[mesh_index]
        face_resource = face_resources[mesh_index]
        mesh_info_offset = mesh_info_offsets[mesh_index]

        try:
            layout = _parse_mesh_layout(raw, mesh_info_offset, warnings, platform)
            vertices, uvs = _parse_vertices(
                raw,
                vertex_resource,
                header_size,
                layout,
                warnings,
                mesh_index,
            )
            faces = _parse_faces(
                raw,
                face_resource,
                header_size,
                len(vertices),
                warnings,
                mesh_index,
                platform,
            )
        except PSGDataError as exc:
            if strict:
                raise
            warnings.append(f"Mesh {mesh_index}: {exc}; mesh skipped")
            continue

        normals = _compute_vertex_normals(vertices, faces)

        meshes.append(
            Mesh(
                name=_name_for_mesh(mesh_names, mesh_index, paired_count),
                vertices=vertices,
                faces=faces,
                uvs=uvs,
                normals=normals,
                material_name=_material_for_mesh(
                    diffuse_names, mesh_index, paired_count
                ),
                vertex_stride=layout.stride,
                attributes=layout.attributes,
                source_offsets={
                    "mesh_info": mesh_info_offset,
                    "vertex_buffer": vertex_resource.offset + header_size,
                    "face_buffer": face_resource.offset + header_size,
                },
            )
        )

    if not meshes:
        detail = warnings[-1] if warnings else "unknown parsing error"
        raise PSGDataError(f"No renderable mesh could be parsed: {detail}")

    bones: list[Bone] = []
    for skeleton_index, bone_offset in enumerate(_deduplicate(bone_offsets)):
        try:
            bones.extend(_parse_bones(raw, bone_offset, skeleton_index, warnings))
        except PSGDataError as exc:
            if strict:
                raise
            warnings.append(f"Skeleton at 0x{bone_offset:X}: {exc}")

    names_location_offset = None
    names_location = None
    if len(raw) >= 0x234:
        names_location_offset = _read_i32(raw, 0x230, "names location offset")
        names_location = names_location_offset + 0x220

    return PSGModel(
        meshes=meshes,
        bones=bones,
        materials=materials,
        warnings=warnings,
        source_path=Path(source_path) if source_path is not None else None,
        metadata={
            "magic": MAGIC_X360 if platform == "xbx" else MAGIC_PS3,
            "type": raw[type_offset:type_offset + 4],
            "file_count": file_count,
            "file_table": file_table,
            "header_size": header_size,
            "names_location_offset": names_location_offset,
            "names_location": names_location,
            "platform": platform,
        },
    )


def _read_toc(data: bytes, offset: int, count: int) -> list[_TOCRecord]:
    records: list[_TOCRecord] = []
    for index in range(count):
        record_offset = offset + index * TOC_RECORD_SIZE
        values = _unpack(data, ">5i", record_offset, f"TOC record {index}")
        records.append(
            _TOCRecord(
                index=index,
                offset=record_offset,
                values=values,
                resource_type=data[record_offset + 20 : record_offset + 24],
            )
        )
    return records


def _parse_material_table(
    data: bytes, offset: int
) -> tuple[list[MaterialParameter], list[str], list[str]]:
    header = _unpack(data, ">8i", offset, "material header")
    material_count = header[1]
    header_size = header[3]
    params_size = header[4]

    if material_count < 0 or material_count > 100_000:
        raise PSGDataError(f"invalid material count {material_count}")
    if material_count == 0:
        return [], [], []
    if header_size < 0 or params_size < header_size:
        raise PSGDataError("invalid material table sizes")

    payload_size = params_size - header_size
    if payload_size % material_count:
        raise PSGDataError(
            f"parameter area {payload_size} is not divisible by {material_count}"
        )
    block_size = payload_size // material_count
    if block_size not in (24, 32):
        raise PSGDataError(f"unsupported material parameter size {block_size}")

    parameters: list[MaterialParameter] = []
    mesh_names: list[str] = []
    diffuse_names: list[str] = []
    cursor = offset + header_size

    for index in range(material_count):
        if block_size == 32:
            values = _unpack(data, ">8i", cursor, f"material parameter {index}")
            type_offset = values[0]
            value_offset = values[6]
        else:
            values = _unpack(data, ">6i", cursor, f"material parameter {index}")
            type_offset = values[0]
            value_offset = values[1]
        cursor += block_size

        kind = _read_cstring(data, offset + type_offset)
        # RX2 32-byte constant channels store their float in the first GUID
        # word (+0x10), not at the string pointer (+0x18). Native scene capture
        # uses the same field for these named scalar channels.
        if block_size == 32 and kind in {
            "detailNormalUVScale", "macroOverlayUVScale", "macroOverlayOpacity",
            "uAnimationSpeed", "vAnimationSpeed",
        }:
            value = repr(struct.unpack_from(">f", data, cursor - block_size + 0x10)[0])
        else:
            value = _read_cstring(data, offset + value_offset)
        parameters.append(MaterialParameter(kind=kind, value=value))
        if kind == "Name":
            mesh_names.append(value)
        elif kind == "diffuse":
            diffuse_names.append(value)

    return parameters, mesh_names, diffuse_names


def _parse_mesh_layout(
    data: bytes, offset: int, warnings: list[str], platform: str = "ps3"
) -> _MeshLayout:
    info = _unpack(data, ">iiHHi", offset, "mesh layout header")
    if platform == "xbx":
        descriptor_count = info[2]
        descriptor_size = 16
        stride_byte_after = True
    else:
        descriptor_count = info[3]
        descriptor_size = 8
        stride_byte_after = False
    if not 0 < descriptor_count <= 256:
        raise PSGDataError(f"invalid vertex descriptor count {descriptor_count}")

    descriptors = [
        _slice(data, offset + 16 + index * descriptor_size, descriptor_size,
               f"vertex descriptor {index}")
        for index in range(descriptor_count)
    ]
    if stride_byte_after:
        stride = _slice(data, offset + 16 + descriptor_count * descriptor_size, 1,
                        "vertex stride")[0]
        if stride == 0:
            stride = 32
    else:
        stride = descriptors[-1][5]
        if stride == 0:
            stride = max((descriptor[5] for descriptor in descriptors), default=0)
    if not 0 < stride <= 4096:
        raise PSGDataError(f"invalid vertex stride {stride}")

    attributes: list[VertexAttribute] = []
    position: VertexAttribute | None = None
    uv1: VertexAttribute | None = None

    for descriptor in descriptors:
        attribute = _classify_descriptor(descriptor, platform)
        if attribute is None:
            attributes.append(
                VertexAttribute(
                    semantic="unknown",
                    offset=int.from_bytes(
                        descriptor[2:4] if platform == "ps3" else descriptor[0:4],
                        "big",
                    ),
                    data_type="raw",
                    components=0,
                    descriptor=descriptor,
                )
            )
            continue

        attributes.append(attribute)
        if attribute.semantic == "position" and position is None:
            position = attribute
        elif attribute.semantic == "uv1" and uv1 is None:
            uv1 = attribute

    if position is None:
        raw_descriptors = ", ".join(item.hex() for item in descriptors)
        raise PSGDataError(f"unsupported position descriptor(s): {raw_descriptors}")

    position = _validated_attribute(position, stride, warnings, "position")
    if uv1 is not None:
        try:
            uv1 = _validated_attribute(uv1, stride, warnings, "UV")
        except PSGDataError as exc:
            warnings.append(f"Mesh layout at 0x{offset:X}: {exc}; UVs ignored")
            uv1 = None

    return _MeshLayout(
        stride=stride,
        position=position,
        uv1=uv1,
        attributes=tuple(attributes),
    )


def _classify_descriptor(descriptor: bytes, platform: str = "ps3") -> VertexAttribute | None:
    if platform == "xbx":
        return _classify_descriptor_x360(descriptor)
    prefix = descriptor[:2]
    suffix = descriptor[6:8]
    raw_offset = int.from_bytes(descriptor[2:4], "big")

    if prefix == b"\x02\x03" and suffix == b"\x00\x01":
        return VertexAttribute("position", raw_offset, "float32", 3, descriptor)
    if prefix in (b"\x03\x03", b"\x03\x04") and suffix == b"\x00\x01":
        return VertexAttribute("position", raw_offset, "float16", 3, descriptor)
    if descriptor == b"\x00\x1A\x23\xA6\x00\x00\x00\x01":
        return VertexAttribute("position", 0, "uint16", 3, descriptor)
    if prefix == b"\x01\x04" and suffix == b"\x00\x01":
        return VertexAttribute("position", raw_offset, "int16", 3, descriptor)

    if descriptor == b"\x00\x2C\x23\xA5\x00\x05\x00\x06":
        return VertexAttribute("uv1", raw_offset, "float32", 2, descriptor)
    if prefix in (b"\x03\x04", b"\x03\x02") and suffix == b"\x08\x01":
        return VertexAttribute("uv1", raw_offset, "float16", 2, descriptor)
    if prefix == b"\x01\x02" and suffix == b"\x08\x01":
        return VertexAttribute("uv1", raw_offset, "int16", 2, descriptor)
    if descriptor == b"\x00\x2C\x20\x59\x00\x05\x00\x06":
        return VertexAttribute("uv1", raw_offset, "uint16", 2, descriptor)

    if prefix == b"\x01\x02" and suffix == b"\x09\x01":
        return VertexAttribute("uv2", raw_offset, "int16", 2, descriptor)
    if descriptor == b"\x00\x2C\x23\x5F\x00\x05\x02\x08":
        return VertexAttribute("uv3", raw_offset, "float16", 2, descriptor)
    return None


def _classify_descriptor_x360(descriptor: bytes) -> VertexAttribute | None:
    """Classify a 16-byte X360 vertex descriptor record.

    Layout: ``offset:i`` (bytes 0-3), ``type:8s`` (bytes 4-11), ``unknown:i``
    (bytes 12-15). Positions always live at offset 0 in the buffer (the
    plugin binds them there); UVs use the descriptor's offset field.
    """
    kind = descriptor[4:12]
    offset = int.from_bytes(descriptor[0:4], "big")

    if kind == b"\x00\x2A\x23\xB9\x00\x00\x00\x01":
        return VertexAttribute("position", 0, "float32", 3, descriptor)
    if kind == b"\x00\x1A\x23\x60\x00\x00\x00\x01":
        return VertexAttribute("position", 0, "float16", 3, descriptor)
    if kind == b"\x00\x1A\x23\xA6\x00\x00\x00\x01":
        return VertexAttribute("position", 0, "uint16", 3, descriptor)
    if kind == b"\x00\x1A\x21\x5A\x00\x00\x00\x01":
        return VertexAttribute("position", 0, "int16", 3, descriptor)

    if kind == b"\x00\x2C\x23\xA5\x00\x05\x00\x06":
        return VertexAttribute("uv1", offset, "float32", 2, descriptor)
    if kind in (b"\x00\x1A\x23\x60\x00\x05\x00\x06",
                b"\x00\x2C\x23\x5F\x00\x05\x00\x06"):
        return VertexAttribute("uv1", offset, "float16", 2, descriptor)
    if kind == b"\x00\x2C\x21\x59\x00\x05\x00\x06":
        return VertexAttribute("uv1", offset, "int16", 2, descriptor)
    if kind == b"\x00\x2C\x20\x59\x00\x05\x00\x06":
        return VertexAttribute("uv1", offset, "uint16", 2, descriptor)

    if kind == b"\x00\x2C\x21\x59\x00\x05\x01\x07":
        return VertexAttribute("uv2", offset, "int16", 2, descriptor)
    if kind == b"\x00\x2C\x23\x5F\x00\x05\x02\x08":
        return VertexAttribute("uv3", offset, "float16", 2, descriptor)
    return None


def _validated_attribute(
    attribute: VertexAttribute,
    stride: int,
    warnings: list[str],
    label: str,
) -> VertexAttribute:
    dtype = _DTYPE_MAP[attribute.data_type]
    required = dtype.itemsize * attribute.components
    if 0 <= attribute.offset and attribute.offset + required <= stride:
        return attribute

    if required <= stride:
        warnings.append(
            f"{label} descriptor offset 0x{attribute.offset:X} exceeds stride "
            f"{stride}; using offset 0"
        )
        return VertexAttribute(
            semantic=attribute.semantic,
            offset=0,
            data_type=attribute.data_type,
            components=attribute.components,
            descriptor=attribute.descriptor,
        )
    raise PSGDataError(
        f"{label} attribute needs {required} bytes but vertex stride is {stride}"
    )


def _parse_vertices(
    data: bytes,
    resource: _BufferResource,
    header_size: int,
    layout: _MeshLayout,
    warnings: list[str],
    mesh_index: int,
) -> tuple[np.ndarray, np.ndarray | None]:
    if resource.size <= 0:
        raise PSGDataError(f"invalid vertex buffer size {resource.size}")
    buffer_offset = resource.offset + header_size
    _slice(data, buffer_offset, resource.size, "vertex buffer")

    vertex_count, remainder = divmod(resource.size, layout.stride)
    if vertex_count <= 0:
        raise PSGDataError("vertex buffer contains no complete vertices")
    if remainder:
        warnings.append(
            f"Mesh {mesh_index}: vertex buffer has {remainder} trailing byte(s)"
        )

    vertices = _decode_interleaved(
        data, buffer_offset, vertex_count, layout.stride, layout.position
    )
    if not np.isfinite(vertices).all():
        invalid = int(np.size(vertices) - np.isfinite(vertices).sum())
        warnings.append(
            f"Mesh {mesh_index}: replaced {invalid} non-finite position value(s) with zero"
        )
        vertices = np.nan_to_num(vertices, copy=False)

    uvs = None
    if layout.uv1 is not None:
        uvs = _decode_interleaved(
            data, buffer_offset, vertex_count, layout.stride, layout.uv1
        )
        if not np.isfinite(uvs).all():
            uvs = np.nan_to_num(uvs, copy=False)

    return vertices, uvs


def _parse_faces(
    data: bytes,
    resource: _BufferResource,
    header_size: int,
    vertex_count: int,
    warnings: list[str],
    mesh_index: int,
    platform: str = "ps3",
) -> np.ndarray:
    if resource.size < 0:
        raise PSGDataError(f"invalid face buffer size {resource.size}")

    count_offset = resource.info_offset + (32 if platform == "xbx" else 8)
    index_count = _read_i32(data, count_offset, "face index count")
    if index_count < 0:
        raise PSGDataError(f"invalid face index count {index_count}")

    buffer_offset = resource.offset + header_size
    _slice(data, buffer_offset, resource.size, "face buffer")
    available = resource.size // 2
    if index_count > available:
        warnings.append(
            f"Mesh {mesh_index}: header requests {index_count} indices but buffer "
            f"contains {available}; truncating"
        )
        index_count = available
    if index_count % 3:
        trimmed = index_count % 3
        warnings.append(
            f"Mesh {mesh_index}: dropping {trimmed} index value(s) not forming a triangle"
        )
        index_count -= trimmed
    if index_count == 0:
        return np.empty((0, 3), dtype=np.uint32)

    indices = np.frombuffer(
        data, dtype=np.dtype(">u2"), count=index_count, offset=buffer_offset
    ).astype(np.uint32, copy=True)
    faces = indices.reshape(-1, 3)
    valid = np.all(faces < vertex_count, axis=1)
    invalid_count = int((~valid).sum())
    if invalid_count:
        warnings.append(
            f"Mesh {mesh_index}: dropped {invalid_count} triangle(s) with out-of-range indices"
        )
        faces = faces[valid]
    return np.ascontiguousarray(faces, dtype=np.uint32)


def _decode_interleaved(
    data: bytes,
    buffer_offset: int,
    count: int,
    stride: int,
    attribute: VertexAttribute,
) -> np.ndarray:
    dtype = _DTYPE_MAP[attribute.data_type]
    required_end = (
        buffer_offset
        + (count - 1) * stride
        + attribute.offset
        + attribute.components * dtype.itemsize
    )
    if required_end > len(data):
        raise PSGDataError(
            f"{attribute.semantic} data ends at 0x{required_end:X}, beyond the file"
        )

    array = np.ndarray(
        shape=(count, attribute.components),
        dtype=dtype,
        buffer=data,
        offset=buffer_offset + attribute.offset,
        strides=(stride, dtype.itemsize),
    )
    values = array.astype(np.float32, copy=True)
    # Noesis normalizes integer vertex attributes (SHORT -> /32768,
    # USHORT -> /65535) the same way as position buffers.
    if attribute.data_type == "int16":
        values /= np.float32(32768.0)
    elif attribute.data_type == "uint16":
        values /= np.float32(65535.0)
    return values


def _parse_bones(
    data: bytes,
    offset: int,
    skeleton_index: int,
    warnings: list[str],
) -> list[Bone]:
    header = _unpack(data, ">12i4H2i", offset, "bone header")
    bone_count = header[14]
    names_relative_offset = header[11]
    if bone_count > 20_000:
        raise PSGDataError(f"invalid bone count {bone_count}")

    matrix_offset = offset + struct.calcsize(">12i4H2i")
    matrices_end = matrix_offset + bone_count * 64
    if matrices_end > len(data):
        raise PSGDataError("bone matrices extend beyond the file")

    matrices: list[np.ndarray] = []
    for index in range(bone_count):
        values = _unpack(data, ">16f", matrix_offset + index * 64, "bone matrix")
        matrices.append(np.asarray(values, dtype=np.float32).reshape(4, 4))

    names_offset = offset + names_relative_offset
    if not 0 <= names_offset < len(data):
        warnings.append(
            f"Skeleton {skeleton_index}: bone-name table is outside the file; using generated names"
        )
        names = [f"bone_{index}" for index in range(bone_count)]
    else:
        names = []
        cursor = names_offset
        for index in range(bone_count):
            try:
                name, cursor = _read_cstring_with_end(data, cursor)
            except PSGDataError:
                name = f"bone_{index}"
            names.append(name or f"bone_{index}")

    return [
        Bone(name=name, matrix=matrix, skeleton_index=skeleton_index)
        for name, matrix in zip(names, matrices)
    ]


def _name_for_mesh(names: list[str], index: int, mesh_count: int) -> str:
    if len(names) == mesh_count and names[index]:
        return names[index]
    if index < len(names) and names[index]:
        return names[index]
    return f"mesh_{index}"


def _material_for_mesh(
    diffuse_names: list[str], index: int, mesh_count: int
) -> str | None:
    if len(diffuse_names) == mesh_count:
        return diffuse_names[index] or None
    if index < len(diffuse_names):
        return diffuse_names[index] or None
    return None


def _deduplicate(values: Iterable[int]) -> list[int]:
    return list(dict.fromkeys(values))


def _read_i32(data: bytes, offset: int, label: str) -> int:
    return _unpack(data, ">i", offset, label)[0]


def _unpack(data: bytes, fmt: str, offset: int, label: str) -> tuple:
    if offset < 0:
        raise PSGDataError(f"{label} has a negative offset {offset}")
    size = struct.calcsize(fmt)
    if offset + size > len(data):
        raise PSGDataError(
            f"{label} at 0x{offset:X} needs {size} bytes, file has {len(data)}"
        )
    return struct.unpack_from(fmt, data, offset)


def _slice(data: bytes, offset: int, size: int, label: str) -> bytes:
    if offset < 0 or size < 0 or offset + size > len(data):
        raise PSGDataError(
            f"{label} range 0x{offset:X}..0x{offset + size:X} is outside the file"
        )
    return data[offset : offset + size]


def _read_cstring(data: bytes, offset: int) -> str:
    value, _ = _read_cstring_with_end(data, offset)
    return value


def _compute_vertex_normals(vertices: np.ndarray, faces: np.ndarray) -> np.ndarray | None:
    if len(faces) == 0:
        return None
    normals = np.zeros_like(vertices)
    face_positions = vertices[faces]
    face_normals = np.cross(
        face_positions[:, 1] - face_positions[:, 0],
        face_positions[:, 2] - face_positions[:, 0],
    )
    lengths = np.linalg.norm(face_normals, axis=1)
    valid = lengths > 1e-12
    face_normals[valid] /= lengths[valid, None]
    for i in range(3):
        np.add.at(normals, faces[:, i], face_normals)
    vertex_lengths = np.linalg.norm(normals, axis=1)
    valid_vertices = vertex_lengths > 1e-12
    normals[valid_vertices] /= vertex_lengths[valid_vertices, None]
    return normals


def _read_cstring_with_end(data: bytes, offset: int) -> tuple[str, int]:
    if not 0 <= offset < len(data):
        raise PSGDataError(f"string offset 0x{offset:X} is outside the file")
    end = data.find(b"\x00", offset, min(len(data), offset + 65_536))
    if end < 0:
        raise PSGDataError(f"unterminated string at 0x{offset:X}")
    return data[offset:end].decode("utf-8", errors="replace"), end + 1
