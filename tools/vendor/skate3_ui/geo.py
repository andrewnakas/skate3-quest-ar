from __future__ import annotations

from pathlib import Path

from .binary import FormatError, Reader

RENDER_TYPES = {
    0: "line",
    1: "solid",
    2: "texture_clamped",
    3: "texture_wrapped",
}


def parse_geo(path: Path) -> dict:
    data = Path(path).read_bytes()
    r = Reader(data, str(path))
    # Retail bundles use an eight-byte header as the canonical empty GEO.
    if len(data) == 8 and r.u32be(4) == 0:
        return {"format": "skate3-geo", "source": str(path), "shapes": []}
    r.require(0, 12)
    count = r.u32be(4)
    if count > 4096:
        raise FormatError(f"unreasonable GEO shape count {count}")
    r.require(8, count * 4)
    shapes = []
    for index in range(count):
        shape_offset = r.u32be(8 + index * 4)
        if not shape_offset:
            continue
        r.require(shape_offset, 8)
        shape_id = r.u32be(shape_offset)
        unit_count = r.u32be(shape_offset + 4)
        if unit_count > 65536:
            raise FormatError(f"shape {shape_id}: unreasonable unit count")
        r.require(shape_offset + 8, unit_count * 4)
        units = []
        all_points: list[list[float]] = []
        for unit_index in range(unit_count):
            unit_offset = r.u32be(shape_offset + 8 + unit_index * 4)
            r.require(unit_offset, 0x34)
            render_type = r.u32be(unit_offset)
            color = [r.f32be(unit_offset + 4 + channel * 4) for channel in range(4)]
            texture_id = r.u32be(unit_offset + 0x14)
            uv_matrix = [
                r.f32be(unit_offset + 0x18 + component * 4)
                for component in range(6)
            ]
            primitive_count = r.u32be(unit_offset + 0x30)
            if primitive_count > 1_000_000:
                raise FormatError(f"shape {shape_id}: unreasonable primitive count")
            r.require(unit_offset + 0x34, primitive_count * 0x18)
            triangles = []
            for primitive in range(primitive_count):
                base = unit_offset + 0x34 + primitive * 0x18
                triangle = [
                    [r.f32be(base + vertex * 8), r.f32be(base + vertex * 8 + 4)]
                    for vertex in range(3)
                ]
                triangles.append(triangle)
                all_points.extend(triangle)
            units.append(
                {
                    "render_type": render_type,
                    "render_type_name": RENDER_TYPES.get(
                        render_type, f"unknown_{render_type}"
                    ),
                    "color": color,
                    "texture_id": texture_id,
                    "uv_matrix": uv_matrix,
                    "triangles": triangles,
                }
            )
        bounds = None
        if all_points:
            xs = [point[0] for point in all_points]
            ys = [point[1] for point in all_points]
            bounds = [min(xs), min(ys), max(xs), max(ys)]
        shapes.append({"id": shape_id, "bounds": bounds, "units": units})
    return {"format": "skate3-geo", "source": str(path), "shapes": shapes}
