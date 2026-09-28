"""Decode the secondary Xenos texture coordinate used by retail lightmaps."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Protocol

import numpy


class VertexAttribute(Protocol):
    offset: int
    descriptor: bytes


@dataclass(frozen=True)
class LightmapUvDecode:
    values: numpy.ndarray
    format_code: int
    offset: int
    usage_index: int


@dataclass(frozen=True)
class DecalUvDecode:
    values: numpy.ndarray
    format_code: int
    offset: int
    usage_index: int


@dataclass(frozen=True)
class RetailWorldFrameDecode:
    normals: numpy.ndarray
    tangents: numpy.ndarray
    tangent_handedness: numpy.ndarray
    lightmap_format_code: int
    lightmap_offset: int
    tangent_format_code: int
    tangent_offset: int


def _texture_coordinates(
    attributes: Iterable[VertexAttribute],
) -> list[tuple[int, int, VertexAttribute]]:
    texcoords: list[tuple[int, int, VertexAttribute]] = []
    for declaration_index, attribute in enumerate(attributes):
        descriptor = bytes(attribute.descriptor)
        if len(descriptor) != 16:
            raise ValueError("Xenos vertex descriptors must contain 16 bytes")
        if descriptor[9] == 5:
            texcoords.append(
                (int(descriptor[10]), declaration_index, attribute)
            )
    texcoords.sort(key=lambda item: (item[0], item[1]))
    return texcoords


def _secondary_texcoord(
    attributes: Iterable[VertexAttribute],
) -> tuple[VertexAttribute, int] | None:
    texcoords = _texture_coordinates(attributes)
    if len(texcoords) < 2:
        return None
    usage_index, _declaration_index, attribute = texcoords[1]
    return attribute, usage_index


def decode_decal_uvs(
    data: bytes,
    *,
    vertex_buffer_offset: int,
    vertex_count: int,
    vertex_stride: int,
    attributes: Iterable[VertexAttribute],
) -> DecalUvDecode | None:
    """Decode zw from a half4/float4 primary TEXCOORD.

    decalenvironment shaders use this third UV pair for the art overlay while
    xy remains the base diffuse UV. Two-component declarations have no
    independent decal coordinates and return ``None``.
    """

    texcoords = _texture_coordinates(attributes)
    if not texcoords:
        return None
    usage_index, _declaration_index, attribute = texcoords[0]
    descriptor = bytes(attribute.descriptor)
    format_code = int.from_bytes(descriptor[4:8], "big")
    xenos_format = format_code & 0x3F
    if xenos_format == 32:
        component_bytes = 2
        dtype = numpy.dtype(">f2")
    elif xenos_format == 38:
        component_bytes = 4
        dtype = numpy.dtype(">f4")
    else:
        return None
    offset = int(attribute.offset) + component_bytes * 2
    required_end = (
        vertex_buffer_offset
        + max(0, vertex_count - 1) * vertex_stride
        + offset
        + component_bytes * 2
    )
    if required_end > len(data):
        raise ValueError(
            "retail decal UV data extends past the RX2 vertex buffer"
        )
    if vertex_count == 0:
        values = numpy.empty((0, 2), dtype=numpy.float32)
    else:
        values = numpy.ndarray(
            shape=(vertex_count, 2),
            dtype=dtype,
            buffer=data,
            offset=vertex_buffer_offset + offset,
            strides=(vertex_stride, component_bytes),
        ).astype(numpy.float32, copy=True)
    if not numpy.isfinite(values).all():
        raise ValueError("retail decal UVs contain non-finite values")
    return DecalUvDecode(
        values=numpy.ascontiguousarray(values, dtype=numpy.float32),
        format_code=format_code,
        offset=offset,
        usage_index=usage_index,
    )


def decode_lightmap_uvs(
    data: bytes,
    *,
    vertex_buffer_offset: int,
    vertex_count: int,
    vertex_stride: int,
    attributes: Iterable[VertexAttribute],
) -> LightmapUvDecode | None:
    """Decode the second TEXCOORD without changing its retail sign bits.

    Static world shaders use ``abs(uv)`` because the signs also carry tangent
    frame handedness. Keeping the signed values here lets the Blender importer
    apply the correct shader-family policy rather than destroying provenance
    during extraction.
    """

    selected = _secondary_texcoord(attributes)
    if selected is None:
        return None
    attribute, usage_index = selected
    descriptor = bytes(attribute.descriptor)
    format_code = int.from_bytes(descriptor[4:8], "big")
    xenos_format = format_code & 0x3F
    offset = int(attribute.offset)
    if vertex_count < 0 or vertex_stride <= 0 or offset < 0:
        raise ValueError("invalid retail vertex-buffer dimensions")

    if xenos_format in (25, 26):
        component_bytes = 2
        dtype = numpy.dtype(">i2")
    elif xenos_format in (31, 32):
        component_bytes = 2
        dtype = numpy.dtype(">f2")
    elif xenos_format in (37, 38):
        component_bytes = 4
        dtype = numpy.dtype(">f4")
    else:
        raise ValueError(
            f"unsupported lightmap UV Xenos format 0x{format_code:08X}"
        )

    required_end = (
        vertex_buffer_offset
        + max(0, vertex_count - 1) * vertex_stride
        + offset
        + component_bytes * 2
    )
    if required_end > len(data):
        raise ValueError(
            "retail lightmap UV data extends past the RX2 vertex buffer"
        )
    if vertex_count == 0:
        values = numpy.empty((0, 2), dtype=numpy.float32)
    else:
        values = numpy.ndarray(
            shape=(vertex_count, 2),
            dtype=dtype,
            buffer=data,
            offset=vertex_buffer_offset + offset,
            strides=(vertex_stride, component_bytes),
        ).astype(numpy.float32, copy=True)
    if xenos_format in (25, 26):
        values /= numpy.float32(32767.0)
    if not numpy.isfinite(values).all():
        raise ValueError("retail lightmap UVs contain non-finite values")
    return LightmapUvDecode(
        values=numpy.ascontiguousarray(values, dtype=numpy.float32),
        format_code=format_code,
        offset=offset,
        usage_index=usage_index,
    )


def decode_retail_world_frame(
    data: bytes,
    *,
    vertex_buffer_offset: int,
    vertex_count: int,
    vertex_stride: int,
    attributes: Iterable[VertexAttribute],
) -> RetailWorldFrameDecode | None:
    """Decode the exact static-world normal, tangent, and handedness.

    Skate 3's environment vertex layouts do not store a conventional normal
    attribute. Their signed SHORT4 secondary TEXCOORD stores the lightmap
    unwrap in ``xy`` and the normal's ``xy`` in ``zw``. The sign of unwrap
    ``y`` reconstructs normal ``z``; unwrap ``x`` is the tangent handedness.
    The usage-6 PACKED11_11_10N attribute is the authored tangent.
    """

    attributes = tuple(attributes)
    selected = _secondary_texcoord(attributes)
    if selected is None:
        return None
    lightmap_attribute, _usage_index = selected
    lightmap_descriptor = bytes(lightmap_attribute.descriptor)
    lightmap_format_code = int.from_bytes(
        lightmap_descriptor[4:8], "big"
    )
    if (lightmap_format_code & 0x3F) != 26:
        return None

    tangent_attribute = None
    for attribute in attributes:
        descriptor = bytes(attribute.descriptor)
        if len(descriptor) != 16:
            raise ValueError("Xenos vertex descriptors must contain 16 bytes")
        format_code = int.from_bytes(descriptor[4:8], "big")
        if descriptor[9] == 6 and (format_code & 0x3F) == 16:
            tangent_attribute = attribute
            break
    if tangent_attribute is None:
        return None

    lightmap_offset = int(lightmap_attribute.offset)
    tangent_offset = int(tangent_attribute.offset)
    if (
        vertex_count < 0
        or vertex_stride <= 0
        or lightmap_offset < 0
        or tangent_offset < 0
    ):
        raise ValueError("invalid retail vertex-buffer dimensions")
    required_end = (
        vertex_buffer_offset
        + max(0, vertex_count - 1) * vertex_stride
        + max(lightmap_offset + 8, tangent_offset + 4)
    )
    if required_end > len(data):
        raise ValueError(
            "retail world tangent-frame data extends past the RX2 "
            "vertex buffer"
        )

    if vertex_count == 0:
        short4 = numpy.empty((0, 4), dtype=numpy.int16)
        words = numpy.empty((0,), dtype=numpy.uint32)
    else:
        short4 = numpy.ndarray(
            shape=(vertex_count, 4),
            dtype=numpy.dtype(">i2"),
            buffer=data,
            offset=vertex_buffer_offset + lightmap_offset,
            strides=(vertex_stride, 2),
        ).astype(numpy.int32, copy=True)
        words = numpy.ndarray(
            shape=(vertex_count,),
            dtype=numpy.dtype(">u4"),
            buffer=data,
            offset=vertex_buffer_offset + tangent_offset,
            strides=(vertex_stride,),
        ).astype(numpy.uint32, copy=True)

    normals = numpy.empty((vertex_count, 3), dtype=numpy.float32)
    normals[:, :2] = short4[:, 2:4].astype(numpy.float32)
    normals[:, :2] /= numpy.float32(32767.0)
    normal_z_squared = (
        1.0
        - normals[:, 0] * normals[:, 0]
        - normals[:, 1] * normals[:, 1]
    )
    normals[:, 2] = numpy.sqrt(
        numpy.maximum(normal_z_squared, numpy.float32(0.0))
    )
    normals[:, 2] *= numpy.where(
        short4[:, 1] > 0,
        numpy.float32(1.0),
        numpy.float32(-1.0),
    )

    def signed_component(
        values: numpy.ndarray, shift: int, bits: int
    ) -> numpy.ndarray:
        mask = numpy.uint32((1 << bits) - 1)
        decoded = ((values >> numpy.uint32(shift)) & mask).astype(
            numpy.int32
        )
        sign_bit = 1 << (bits - 1)
        decoded[decoded >= sign_bit] -= 1 << bits
        return decoded

    tangents = numpy.empty((vertex_count, 3), dtype=numpy.float32)
    tangents[:, 0] = signed_component(words, 0, 11).astype(numpy.float32)
    tangents[:, 1] = signed_component(words, 11, 11).astype(numpy.float32)
    tangents[:, 2] = signed_component(words, 22, 10).astype(numpy.float32)
    tangents[:, 0:2] /= numpy.float32(1023.0)
    tangents[:, 2] /= numpy.float32(511.0)

    tangent_handedness = numpy.where(
        short4[:, 0] > 0,
        numpy.float32(1.0),
        numpy.float32(-1.0),
    )
    if not (
        numpy.isfinite(normals).all()
        and numpy.isfinite(tangents).all()
        and numpy.isfinite(tangent_handedness).all()
    ):
        raise ValueError("retail world tangent frame contains non-finite data")

    tangent_descriptor = bytes(tangent_attribute.descriptor)
    return RetailWorldFrameDecode(
        normals=numpy.ascontiguousarray(normals, dtype=numpy.float32),
        tangents=numpy.ascontiguousarray(tangents, dtype=numpy.float32),
        tangent_handedness=numpy.ascontiguousarray(
            tangent_handedness, dtype=numpy.float32
        ),
        lightmap_format_code=lightmap_format_code,
        lightmap_offset=lightmap_offset,
        tangent_format_code=int.from_bytes(
            tangent_descriptor[4:8], "big"
        ),
        tangent_offset=tangent_offset,
    )
