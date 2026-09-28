"""Decode retail RenderWare ClusteredMesh collision from Skate 3 RX2 assets."""

from __future__ import annotations

from dataclasses import dataclass
import math
from pathlib import Path
import struct
from typing import Iterable


RX2_TOC_RECORD_SIZE = 24
RX2_TYPE_CLUSTERED_MESH = 0x00080006
MESH_HEADER_SIZE = 96
CLUSTER_HEADER_SIZE = 16


@dataclass(frozen=True)
class CollisionTriangle:
    a: tuple[float, float, float]
    b: tuple[float, float, float]
    c: tuple[float, float, float]
    surface: int
    edge_codes: tuple[int, int, int] | None = None
    group_id: int | None = None
    unit_flags: int = 0


@dataclass(frozen=True)
class ClusteredMesh:
    bounds_min: tuple[float, float, float]
    bounds_max: tuple[float, float, float]
    triangles: tuple[CollisionTriangle, ...]
    triangle_cluster_indices: tuple[int, ...]
    cluster_count: int
    vertex_count: int
    unit_count: int
    compression_counts: tuple[tuple[int, int], ...]
    mesh_flags: int
    group_id_width: int
    surface_id_width: int


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _be_u16(data: bytes, offset: int) -> int:
    return struct.unpack_from(">H", data, offset)[0]


def _be_u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]


def _be_f32(data: bytes, offset: int) -> float:
    return struct.unpack_from(">f", data, offset)[0]


def _be_vec3(data: bytes, offset: int) -> tuple[float, float, float]:
    return struct.unpack_from(">3f", data, offset)


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _read_variable_id(data: bytes, offset: int, width: int) -> tuple[int, int]:
    _require(width in (1, 2), f"unsupported collision ID width {width}")
    end = offset + width
    _require(end <= len(data), "collision unit ID extends beyond its cluster")
    # RenderWare's unit IDs are the exception inside the otherwise
    # big-endian Xbox 360 blob. The native reader consumes the low byte first.
    return int.from_bytes(data[offset:end], "little"), end


def _decode_vertices(
    cluster: bytes,
    vertex_count: int,
    compression: int,
    granularity: float,
    vertex_data_end: int,
) -> list[tuple[float, float, float]]:
    vertices: list[tuple[float, float, float]] = []
    if compression == 1:
        _require(vertex_data_end >= 28, "compressed cluster omits its base")
        base = struct.unpack_from(">3i", cluster, CLUSTER_HEADER_SIZE)
        for index in range(vertex_count):
            offset = CLUSTER_HEADER_SIZE + 12 + index * 6
            _require(
                offset + 6 <= vertex_data_end,
                "16-bit compressed vertex extends into the unit stream",
            )
            delta = struct.unpack_from(">3h", cluster, offset)
            vertices.append(
                tuple(
                    _f32((base[axis] + delta[axis]) * granularity)
                    for axis in range(3)
                )
            )
    elif compression == 2:
        for index in range(vertex_count):
            offset = CLUSTER_HEADER_SIZE + index * 12
            _require(
                offset + 12 <= vertex_data_end,
                "32-bit compressed vertex extends into the unit stream",
            )
            value = struct.unpack_from(">3i", cluster, offset)
            vertices.append(
                tuple(_f32(component * granularity) for component in value)
            )
    elif compression == 0:
        for index in range(vertex_count):
            offset = CLUSTER_HEADER_SIZE + index * 16
            _require(
                offset + 16 <= vertex_data_end,
                "uncompressed vertex extends into the unit stream",
            )
            vertices.append(_be_vec3(cluster, offset))
    else:
        raise ValueError(
            f"unsupported RenderWare cluster compression {compression}"
        )
    _require(
        all(math.isfinite(component) for vertex in vertices for component in vertex),
        "collision cluster contains a non-finite vertex",
    )
    return vertices


def _triangulate_unit(
    unit_type: int,
    indices: list[int],
) -> list[tuple[int, int, int]]:
    if unit_type == 1:
        return [(indices[0], indices[1], indices[2])]
    if unit_type == 2:
        # This is the exact split used by rw::collision::ClusteredMesh:
        # first 0-1-2, then 3-2-1.
        return [
            (indices[0], indices[1], indices[2]),
            (indices[3], indices[2], indices[1]),
        ]
    raise ValueError(
        "RenderWare triangle-list units require a separately verified "
        "triangulation path"
    )


def _decode_cluster(
    cluster: bytes,
    granularity: float,
    group_id_width: int,
    surface_id_width: int,
) -> tuple[list[CollisionTriangle], int, int, int]:
    _require(
        len(cluster) >= CLUSTER_HEADER_SIZE,
        "collision cluster is smaller than its header",
    )
    unit_count = _be_u16(cluster, 0)
    unit_bytes = _be_u16(cluster, 2)
    vertex_blocks = _be_u16(cluster, 4)
    cluster_bytes = _be_u16(cluster, 8)
    vertex_count = cluster[10]
    compression = cluster[12]
    _require(
        cluster_bytes == len(cluster),
        "collision cluster size does not match its table span",
    )
    unit_offset = (vertex_blocks + 1) * 16
    unit_end = unit_offset + unit_bytes
    _require(
        CLUSTER_HEADER_SIZE <= unit_offset <= unit_end <= len(cluster),
        "collision cluster has invalid vertex/unit spans",
    )
    vertices = _decode_vertices(
        cluster,
        vertex_count,
        compression,
        granularity,
        unit_offset,
    )

    triangles: list[CollisionTriangle] = []
    cursor = unit_offset
    for _unit_index in range(unit_count):
        _require(cursor < unit_end, "collision unit stream ends prematurely")
        flags = cluster[cursor]
        cursor += 1
        unit_type = flags & 0x0F
        if unit_type == 1:
            output_triangle_count = 1
        elif unit_type == 2:
            output_triangle_count = 2
        elif unit_type == 3:
            _require(
                cursor < unit_end,
                "collision triangle-list unit omits its triangle count",
            )
            output_triangle_count = cluster[cursor]
            cursor += 1
        else:
            raise ValueError(f"unsupported collision unit type {unit_type}")

        index_count = output_triangle_count + 2
        indices_end = cursor + index_count
        _require(
            indices_end <= unit_end,
            "collision unit vertex indices extend beyond the unit stream",
        )
        indices = list(cluster[cursor:indices_end])
        cursor = indices_end
        _require(
            all(index < vertex_count for index in indices),
            "collision unit references a missing cluster vertex",
        )
        edge_codes: tuple[int, ...] | None = None
        if flags & 0x20:
            edge_codes = tuple(cluster[cursor : cursor + index_count])
            cursor += index_count
            _require(
                cursor <= unit_end,
                "collision unit edge codes extend beyond the unit stream",
            )
        group_id: int | None = None
        if flags & 0x40:
            group_id, cursor = _read_variable_id(
                cluster,
                cursor,
                group_id_width,
            )
        surface = 0
        if flags & 0x80:
            surface, cursor = _read_variable_id(
                cluster,
                cursor,
                surface_id_width,
            )
        for triangle_index, (a, b, c) in enumerate(
            _triangulate_unit(unit_type, indices)
        ):
            triangle_edge_codes: tuple[int, int, int] | None = None
            if edge_codes is not None:
                if unit_type == 1:
                    triangle_edge_codes = (
                        edge_codes[0],
                        edge_codes[1],
                        edge_codes[2],
                    )
                else:
                    # Quad units split as 0-1-2 and 3-2-1. The generated
                    # diagonal is smooth; boundary codes retain their native
                    # corner association. University uses triangle units, but
                    # decoding this correctly keeps the general RX2 reader
                    # lossless enough for other districts.
                    triangle_edge_codes = (
                        (
                            edge_codes[0],
                            edge_codes[1],
                            0x1A,
                        )
                        if triangle_index == 0
                        else (
                            edge_codes[3],
                            edge_codes[2],
                            0x1A,
                        )
                    )
            triangles.append(
                CollisionTriangle(
                    vertices[a],
                    vertices[b],
                    vertices[c],
                    surface,
                    triangle_edge_codes,
                    group_id,
                    flags,
                )
            )

    _require(cursor == unit_end, "collision unit stream has trailing bytes")
    return triangles, vertex_count, unit_count, compression


def decode_clustered_mesh(data: bytes) -> ClusteredMesh:
    """Decode one big-endian serialized rw::collision::ClusteredMesh."""

    _require(len(data) >= MESH_HEADER_SIZE, "collision mesh is too small")
    bounds_min = _be_vec3(data, 0)
    bounds_max = _be_vec3(data, 16)
    expected_triangles = _be_u32(data, 40)
    kd_tree = _be_u32(data, 48)
    cluster_table = _be_u32(data, 52)
    granularity = _be_f32(data, 56)
    mesh_flags = _be_u16(data, 60)
    group_id_width = data[62]
    surface_id_width = data[63]
    cluster_count = _be_u32(data, 64)
    mesh_bytes = _be_u32(data, 80)
    _require(
        all(
            math.isfinite(component)
            for point in (bounds_min, bounds_max)
            for component in point
        ),
        "collision mesh has non-finite bounds",
    )
    _require(
        all(low <= high for low, high in zip(bounds_min, bounds_max)),
        "collision mesh bounds are inverted",
    )
    _require(
        math.isfinite(granularity) and granularity > 0.0,
        "collision mesh granularity is invalid",
    )
    _require(
        mesh_bytes <= len(data) and mesh_bytes >= MESH_HEADER_SIZE,
        "collision mesh byte count is invalid",
    )
    _require(
        kd_tree >= MESH_HEADER_SIZE
        and kd_tree + 48 <= cluster_table,
        "collision KD-tree header is invalid",
    )
    _require(
        cluster_table >= MESH_HEADER_SIZE
        and cluster_table + cluster_count * 4 <= mesh_bytes,
        "collision cluster table is invalid",
    )

    cluster_offsets = [
        _be_u32(data, cluster_table + index * 4)
        for index in range(cluster_count)
    ]
    _require(
        cluster_offsets == sorted(cluster_offsets)
        and len(set(cluster_offsets)) == len(cluster_offsets),
        "collision cluster offsets are not strictly increasing",
    )
    triangles: list[CollisionTriangle] = []
    triangle_cluster_indices: list[int] = []
    vertex_count = 0
    unit_count = 0
    compression_counts: dict[int, int] = {}
    for cluster_index, cluster_offset in enumerate(cluster_offsets):
        cluster_end = (
            cluster_offsets[cluster_index + 1]
            if cluster_index + 1 < len(cluster_offsets)
            else mesh_bytes
        )
        _require(
            cluster_offset >= cluster_table + cluster_count * 4
            and cluster_offset + CLUSTER_HEADER_SIZE <= cluster_end,
            "collision cluster table points outside the mesh",
        )
        declared_size = _be_u16(data, cluster_offset + 8)
        _require(
            cluster_offset + declared_size <= cluster_end,
            "collision cluster overlaps the following cluster",
        )
        try:
            decoded, vertices, units, compression = _decode_cluster(
                data[cluster_offset : cluster_offset + declared_size],
                granularity,
                group_id_width,
                surface_id_width,
            )
        except ValueError as error:
            raise ValueError(
                f"collision cluster {cluster_index} at "
                f"0x{cluster_offset:X}: {error}"
            ) from error
        triangles.extend(decoded)
        triangle_cluster_indices.extend([cluster_index] * len(decoded))
        vertex_count += vertices
        unit_count += units
        compression_counts[compression] = (
            compression_counts.get(compression, 0) + 1
        )

    _require(
        len(triangles) == expected_triangles,
        "collision mesh triangle count does not match decoded clusters",
    )
    return ClusteredMesh(
        bounds_min=bounds_min,
        bounds_max=bounds_max,
        triangles=tuple(triangles),
        triangle_cluster_indices=tuple(triangle_cluster_indices),
        cluster_count=cluster_count,
        vertex_count=vertex_count,
        unit_count=unit_count,
        compression_counts=tuple(sorted(compression_counts.items())),
        mesh_flags=mesh_flags,
        group_id_width=group_id_width,
        surface_id_width=surface_id_width,
    )


def decode_rx2_clustered_meshes(data: bytes) -> list[ClusteredMesh]:
    """Decode every ClusteredMesh section in one Xbox 360 RW4 RX2."""

    _require(
        len(data) >= 0x34 and data[:7] == b"\x89RW4xb2",
        "asset is not an Xbox 360 RW4 RX2 resource",
    )
    file_count = _be_u32(data, 0x20)
    file_table = _be_u32(data, 0x30)
    _require(
        file_table + file_count * RX2_TOC_RECORD_SIZE <= len(data),
        "RX2 section table extends beyond the file",
    )
    result: list[ClusteredMesh] = []
    for record_index in range(file_count):
        record_offset = file_table + record_index * RX2_TOC_RECORD_SIZE
        section_offset = _be_u32(data, record_offset)
        section_size = _be_u32(data, record_offset + 8)
        section_type = _be_u32(data, record_offset + 20)
        if section_type != RX2_TYPE_CLUSTERED_MESH:
            continue
        _require(
            section_offset + section_size <= len(data),
            "RX2 collision section extends beyond the file",
        )
        result.append(
            decode_clustered_mesh(
                data[section_offset : section_offset + section_size]
            )
        )
    return result


def decode_rx2_files(paths: Iterable[Path]) -> list[ClusteredMesh]:
    result: list[ClusteredMesh] = []
    for path in paths:
        result.extend(decode_rx2_clustered_meshes(path.read_bytes()))
    return result
