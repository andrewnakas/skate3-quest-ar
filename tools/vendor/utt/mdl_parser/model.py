from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import numpy as np


@dataclass(slots=True, frozen=True)
class MaterialParameter:
    kind: str
    value: str


@dataclass(slots=True, frozen=True)
class VertexAttribute:
    semantic: str
    offset: int
    data_type: str
    components: int
    descriptor: bytes


@dataclass(slots=True, frozen=True)
class Bone:
    name: str
    matrix: np.ndarray
    skeleton_index: int = 0


@dataclass(slots=True)
class Mesh:
    name: str
    vertices: np.ndarray
    faces: np.ndarray
    uvs: np.ndarray | None = None
    normals: np.ndarray | None = None
    material_name: str | None = None
    vertex_stride: int = 0
    attributes: tuple[VertexAttribute, ...] = ()
    source_offsets: dict[str, int] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.vertices = np.ascontiguousarray(self.vertices, dtype=np.float32)
        self.faces = np.ascontiguousarray(self.faces, dtype=np.uint32)
        if self.uvs is not None:
            self.uvs = np.ascontiguousarray(self.uvs, dtype=np.float32)

        if self.vertices.ndim != 2 or self.vertices.shape[1] != 3:
            raise ValueError("vertices must have shape (N, 3)")
        if self.faces.ndim != 2 or self.faces.shape[1] != 3:
            raise ValueError("faces must have shape (N, 3)")
        if self.uvs is not None and (self.uvs.ndim != 2 or self.uvs.shape[1] != 2):
            raise ValueError("uvs must have shape (N, 2)")

    @property
    def vertex_count(self) -> int:
        return int(self.vertices.shape[0])

    @property
    def triangle_count(self) -> int:
        return int(self.faces.shape[0])

    @property
    def bounds(self) -> tuple[np.ndarray, np.ndarray]:
        if self.vertex_count == 0:
            zero = np.zeros(3, dtype=np.float32)
            return zero.copy(), zero.copy()
        return self.vertices.min(axis=0), self.vertices.max(axis=0)


@dataclass(slots=True)
class PSGModel:
    meshes: list[Mesh]
    bones: list[Bone] = field(default_factory=list)
    materials: list[MaterialParameter] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    source_path: Path | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    @property
    def vertex_count(self) -> int:
        return sum(mesh.vertex_count for mesh in self.meshes)

    @property
    def triangle_count(self) -> int:
        return sum(mesh.triangle_count for mesh in self.meshes)

    @property
    def bounds(self) -> tuple[np.ndarray, np.ndarray]:
        populated = [mesh for mesh in self.meshes if mesh.vertex_count]
        if not populated:
            zero = np.zeros(3, dtype=np.float32)
            return zero.copy(), zero.copy()

        minimums = np.stack([mesh.bounds[0] for mesh in populated], axis=0)
        maximums = np.stack([mesh.bounds[1] for mesh in populated], axis=0)
        return minimums.min(axis=0), maximums.max(axis=0)

    @property
    def center(self) -> np.ndarray:
        minimum, maximum = self.bounds
        return (minimum + maximum) * 0.5

    @property
    def radius(self) -> float:
        minimum, maximum = self.bounds
        return float(np.linalg.norm(maximum - minimum) * 0.5)

    def summary(self) -> str:
        source = self.source_path.name if self.source_path else "<memory>"
        return (
            f"{source}: {len(self.meshes)} mesh(es), "
            f"{self.vertex_count:,} vertices, {self.triangle_count:,} triangles, "
            f"{len(self.bones)} bone(s)"
        )
