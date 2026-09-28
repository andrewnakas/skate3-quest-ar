"""Decode retail Pegasus tSplineData grind paths from Skate 3 RX2 assets."""

from __future__ import annotations

import math
from pathlib import Path
import re
import struct


RX2_TOC_RECORD_SIZE = 24
RX2_TYPE_SPLINE_DATA = 0x00EB0004
SPLINE_HEADER_SIZE = 16
SPLINE_RAIL_SIZE = 32
SPLINE_SEGMENT_SIZE = 144
SPLINE_SEGMENT_PAYLOAD_SIZE = 120
SPLINE_CONTINUITY_EPSILON = 1.0e-3
CELL_SIM_STREAM_PATTERN = re.compile(
    r"^cSim_(-?\d+)_(-?\d+)_high\.xsf$",
    re.IGNORECASE,
)
CELL_LOCAL_CLASSIFICATION_MARGIN = 40.0


def _vec3(data: bytes, offset: int) -> tuple[float, float, float]:
    return struct.unpack_from(">3f", data, offset)


def _add(
    *values: tuple[float, float, float],
) -> tuple[float, float, float]:
    return tuple(sum(value[axis] for value in values) for axis in range(3))


def _distance(
    left: tuple[float, float, float],
    right: tuple[float, float, float],
) -> float:
    return math.sqrt(
        sum((left[axis] - right[axis]) ** 2 for axis in range(3))
    )


def grind_cell_translation(
    stream_file: str,
) -> tuple[float, float, float] | None:
    """Return a simulation cell's runtime-space translation, if named."""

    match = CELL_SIM_STREAM_PATTERN.match(Path(stream_file).name)
    if match is None:
        return None
    return float(match.group(1)), 0.0, float(match.group(2))


def classify_grind_coordinate_frame(
    stream_file: str,
    native_segment_payloads: list[str] | tuple[str, ...],
    *,
    margin: float = CELL_LOCAL_CLASSIFICATION_MARGIN,
) -> str:
    """Classify one rail as world-space, cell-local, or ambiguous.

    Skate 2's BAM streams mix both coordinate conventions, including within
    one tSplineData asset. Classification therefore has to happen per rail.
    The rail midpoint is compared with the origin and the stream cell center;
    near-boundary cases remain untouched instead of being guessed.
    """

    translation = grind_cell_translation(stream_file)
    if translation is None or not native_segment_payloads:
        return "world_space"
    if not math.isfinite(margin) or margin < 0.0:
        raise ValueError("grind coordinate classification margin is invalid")

    midpoint_x = 0.0
    midpoint_z = 0.0
    endpoint_count = 0
    for payload_hex in native_segment_payloads:
        payload = bytes.fromhex(payload_hex)
        if len(payload) != SPLINE_SEGMENT_PAYLOAD_SIZE:
            raise ValueError(
                "retail grind segment payload must contain 120 bytes"
            )
        values = struct.unpack(">30f", payload)
        coefficient_a = values[0:3]
        coefficient_b = values[4:7]
        coefficient_c = values[8:11]
        coefficient_d = values[12:15]
        segment_end = _add(
            coefficient_d,
            coefficient_c,
            coefficient_b,
            coefficient_a,
        )
        midpoint_x += coefficient_d[0] + segment_end[0]
        midpoint_z += coefficient_d[2] + segment_end[2]
        endpoint_count += 2

    midpoint_x /= endpoint_count
    midpoint_z /= endpoint_count
    local_distance = math.hypot(midpoint_x, midpoint_z)
    world_distance = math.hypot(
        midpoint_x - translation[0],
        midpoint_z - translation[2],
    )
    if local_distance + margin < world_distance:
        return "cell_local"
    if world_distance + margin < local_distance:
        return "world_space"
    return "ambiguous"


def translate_native_segment_payload(
    payload_hex: str,
    translation: tuple[float, float, float],
) -> str:
    """Translate one exact native cubic and its retail bounds metadata."""

    payload = bytearray.fromhex(payload_hex)
    if len(payload) != SPLINE_SEGMENT_PAYLOAD_SIZE:
        raise ValueError("retail grind segment payload must contain 120 bytes")
    if not all(math.isfinite(component) for component in translation):
        raise ValueError("grind segment translation is not finite")
    # Word 12 is polynomial D (the segment origin); words 20 and 24 are the
    # native bounds vectors. A/B/C are derivatives and must remain unchanged.
    for vector_word in (12, 20, 24):
        for axis, component in enumerate(translation):
            offset = (vector_word + axis) * 4
            value = struct.unpack_from(">f", payload, offset)[0]
            struct.pack_into(">f", payload, offset, value + component)
    return payload.hex()


def decode_grind_splines(data: bytes) -> list[dict[str, object]]:
    """Return every native grind spline in one simulation RX2 resource.

    Retail spline segments are cubic polynomials:

    ``position(t) = D + C*t + B*t^2 + A*t^3``

    The first 120 bytes of each 144-byte segment contain the four coefficient
    vectors plus retail auxiliary data. The final 24 bytes are relocated
    runtime links and are deliberately not copied into the portable manifest.
    """

    if len(data) < 0x34 or data[:7] != b"\x89RW4xb2":
        raise ValueError("asset is not an Xbox 360 RW4 RX2 resource")
    file_count = struct.unpack_from(">I", data, 0x20)[0]
    file_table = struct.unpack_from(">I", data, 0x30)[0]
    if file_table + file_count * RX2_TOC_RECORD_SIZE > len(data):
        raise ValueError("RX2 section table extends beyond the file")

    result: list[dict[str, object]] = []
    for record_index in range(file_count):
        record_offset = file_table + record_index * RX2_TOC_RECORD_SIZE
        section_offset, _, section_size, _, _ = struct.unpack_from(
            ">5I",
            data,
            record_offset,
        )
        section_type = struct.unpack_from(">I", data, record_offset + 20)[0]
        if section_type != RX2_TYPE_SPLINE_DATA:
            continue
        section_end = section_offset + section_size
        if (
            section_offset + SPLINE_HEADER_SIZE > len(data)
            or section_end > len(data)
        ):
            raise ValueError("RX2 spline section extends beyond the file")

        # TU3 GrindData::Add (82C1EEF0) reads lhz +2, including its loop
        # bound at 82C1EFBC. The first halfword is not part of the count:
        # Skate School has header 0001 0002, two rails, table end 0x50.
        header_halfword, rail_count, segment_count, rail_table, segment_table = (
            struct.unpack_from(">HH3I", data, section_offset)
        )
        expected_segment_table = (
            SPLINE_HEADER_SIZE + rail_count * SPLINE_RAIL_SIZE
        )
        expected_size = (
            expected_segment_table + segment_count * SPLINE_SEGMENT_SIZE
        )
        if (
            rail_table != SPLINE_HEADER_SIZE
            or segment_table != expected_segment_table
            or section_size != expected_size
        ):
            raise ValueError(
                "retail tSplineData has an inconsistent table layout"
            )

        covered_segments: set[int] = set()
        for rail_index in range(rail_count):
            rail_offset = section_offset + rail_table + (
                rail_index * SPLINE_RAIL_SIZE
            )
            (
                spline_id,
                type_signature,
                flags,
                first_segment,
                last_segment,
                trailing_word,
            ) = struct.unpack_from(">QQ4I", data, rail_offset)
            if (
                first_segment < segment_table
                or last_segment < first_segment
                or (first_segment - segment_table) % SPLINE_SEGMENT_SIZE
                or (last_segment - first_segment) % SPLINE_SEGMENT_SIZE
                or last_segment + SPLINE_SEGMENT_SIZE > section_size
            ):
                raise ValueError(
                    f"retail grind rail {rail_index} has invalid segment links"
                )

            payloads: list[str] = []
            first_start: tuple[float, float, float] | None = None
            previous_end: tuple[float, float, float] | None = None
            for relative_segment in range(
                first_segment,
                last_segment + 1,
                SPLINE_SEGMENT_SIZE,
            ):
                segment_index = (
                    relative_segment - segment_table
                ) // SPLINE_SEGMENT_SIZE
                if segment_index in covered_segments:
                    raise ValueError(
                        "retail tSplineData rails share a segment record"
                    )
                covered_segments.add(segment_index)
                segment_offset = section_offset + relative_segment
                native_values = struct.unpack_from(
                    ">30f",
                    data,
                    segment_offset,
                )
                if not all(math.isfinite(value) for value in native_values):
                    raise ValueError(
                        f"retail grind rail {rail_index} contains "
                        "non-finite native data"
                    )
                coefficient_a = _vec3(data, segment_offset)
                coefficient_b = _vec3(data, segment_offset + 16)
                coefficient_c = _vec3(data, segment_offset + 32)
                coefficient_d = _vec3(data, segment_offset + 48)
                segment_end = _add(
                    coefficient_d,
                    coefficient_c,
                    coefficient_b,
                    coefficient_a,
                )
                if first_start is None:
                    first_start = coefficient_d
                if (
                    previous_end is not None
                    and _distance(previous_end, coefficient_d)
                    > SPLINE_CONTINUITY_EPSILON
                ):
                    raise ValueError(
                        f"retail grind rail {rail_index} is discontinuous"
                    )
                previous_end = segment_end
                payloads.append(
                    data[
                        segment_offset : (
                            segment_offset + SPLINE_SEGMENT_PAYLOAD_SIZE
                        )
                    ].hex()
                )

            assert first_start is not None and previous_end is not None
            result.append(
                {
                    "section_index": record_index,
                    "section_offset": section_offset,
                    "section_header_halfword": header_halfword,
                    "rail_index": rail_index,
                    "spline_id": f"0x{spline_id:016X}",
                    "type_signature": f"0x{type_signature:016X}",
                    "flags": flags,
                    "trailing_word": trailing_word,
                    "closed": (
                        _distance(first_start, previous_end)
                        <= SPLINE_CONTINUITY_EPSILON
                    ),
                    "segment_count": len(payloads),
                    "native_segment_payloads": payloads,
                }
            )

        if len(covered_segments) != segment_count:
            raise ValueError(
                "retail tSplineData contains unreferenced segment records"
            )
    return result
