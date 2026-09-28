"""Read Skate 3 XST/SFIL stream assets without altering the source archives."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import struct


ATOC_HEADER_SIZE = 24
ATOC_RECORD_SIZE = 64
SFIL_HEADER_SIZE = 128
SFIL_RECORD_ALIGNMENT = 128
SFIL_ASSET_HEADER_SIZE = 128
SFIL_SECTION_TABLE_OFFSET = 128
SFIL_SECTION_DATA_OFFSET = 208

ASSET_TYPE_SIMULATION = 0x00000001
ASSET_TYPE_MODEL = 0x00000004
ASSET_TYPE_TEXTURE = 0x00001000


class StreamFormatError(ValueError):
    pass


@dataclass(frozen=True, slots=True)
class AssetRecord:
    asset_id: int
    processor_id: int
    total_size: int
    header_size: int
    alignment: int
    gpu_size: int
    asset_type: int
    raw_record: bytes


@dataclass(frozen=True, slots=True)
class StreamAsset:
    record: AssetRecord
    source_path: Path
    source_offset: int
    stored_size: int
    data: bytes


def align_up(value: int, alignment: int) -> int:
    return (value + alignment - 1) & ~(alignment - 1)


def read_atoc(path: str | Path) -> list[AssetRecord]:
    source = Path(path)
    data = source.read_bytes()
    if len(data) < ATOC_HEADER_SIZE or data[:4] != b"ATOC":
        raise StreamFormatError(f"{source} is not an ATOC table")
    version = struct.unpack_from(">I", data, 4)[0]
    if version != 3:
        raise StreamFormatError(f"{source} uses unsupported ATOC version {version}")
    count = struct.unpack_from(">I", data, 16)[0]
    expected = ATOC_HEADER_SIZE + count * ATOC_RECORD_SIZE
    if expected != len(data):
        raise StreamFormatError(
            f"{source} has {len(data)} bytes; expected {expected} for {count} records"
        )

    records: list[AssetRecord] = []
    for index in range(count):
        start = ATOC_HEADER_SIZE + index * ATOC_RECORD_SIZE
        raw = data[start : start + ATOC_RECORD_SIZE]
        records.append(
            AssetRecord(
                asset_id=struct.unpack_from(">Q", raw, 0)[0],
                processor_id=struct.unpack_from(">I", raw, 8)[0],
                total_size=struct.unpack_from(">I", raw, 12)[0],
                header_size=struct.unpack_from(">I", raw, 16)[0],
                alignment=struct.unpack_from(">I", raw, 20)[0],
                gpu_size=struct.unpack_from(">I", raw, 32)[0],
                asset_type=struct.unpack_from(">I", raw, 36)[0],
                raw_record=raw,
            )
        )
    return records


def decompress_refpack(source: bytes) -> bytes:
    if len(source) < 6 or source[:2] != b"\x10\xFB":
        raise StreamFormatError("RefPack payload has no 0x10FB header")

    expected_size = int.from_bytes(source[2:5], "big")
    try:
        from tools.asset_pipeline.fast_refpack import decode as fast_decode
    except ImportError:
        fast_decode = None
    if fast_decode is not None:
        try: decoded = fast_decode(source, expected_size, 5, True)
        except ValueError as error: raise StreamFormatError(str(error)) from error
        if decoded is not None: return decoded
    cursor = 5
    output = bytearray()

    def copy_literals(count: int) -> None:
        nonlocal cursor
        end = cursor + count
        if end > len(source):
            raise StreamFormatError("RefPack literal run exceeds its payload")
        output.extend(source[cursor:end])
        cursor = end

    def copy_reference(offset: int, count: int) -> None:
        if offset <= 0 or offset > len(output):
            raise StreamFormatError(
                f"RefPack back-reference {offset} exceeds {len(output)} output bytes"
            )
        for _ in range(count):
            output.append(output[-offset])

    while len(output) < expected_size:
        if cursor >= len(source):
            raise StreamFormatError("RefPack payload ended before its output was complete")
        control = source[cursor]
        cursor += 1

        if control < 0x80:
            if cursor >= len(source):
                raise StreamFormatError("truncated two-byte RefPack command")
            next_byte = source[cursor]
            cursor += 1
            copy_literals(control & 0x03)
            copy_reference(
                ((control & 0x60) << 3) + next_byte + 1,
                ((control & 0x1C) >> 2) + 3,
            )
        elif control < 0xC0:
            if cursor + 2 > len(source):
                raise StreamFormatError("truncated three-byte RefPack command")
            first, second = source[cursor : cursor + 2]
            cursor += 2
            copy_literals((first >> 6) & 0x03)
            copy_reference(
                ((first & 0x3F) << 8) + second + 1,
                (control & 0x3F) + 4,
            )
        elif control < 0xE0:
            if cursor + 3 > len(source):
                raise StreamFormatError("truncated four-byte RefPack command")
            first, second, third = source[cursor : cursor + 3]
            cursor += 3
            copy_literals(control & 0x03)
            copy_reference(
                ((control & 0x10) << 12) + (first << 8) + second + 1,
                ((control & 0x0C) << 6) + third + 5,
            )
        elif control < 0xFC:
            copy_literals(((control & 0x1F) << 2) + 4)
        else:
            copy_literals(control & 0x03)
            break

    if len(output) != expected_size:
        raise StreamFormatError(
            f"RefPack produced {len(output)} bytes; expected {expected_size}"
        )
    return bytes(output)


def _decode_section(
    source: bytes,
    cursor: int,
    uncompressed_size: int,
    stored_size: int,
    compression: int,
) -> tuple[bytes, int]:
    if uncompressed_size == 0 and stored_size == 0:
        return b"", cursor
    end = cursor + stored_size
    if end > len(source):
        raise StreamFormatError("SFIL section exceeds its source file")
    payload = source[cursor:end]
    if compression == 0:
        decoded = payload
    elif compression == 1:
        decoded = decompress_refpack(payload)
    else:
        raise StreamFormatError(f"unsupported SFIL compression method {compression}")
    if len(decoded) != uncompressed_size:
        raise StreamFormatError(
            f"SFIL section decoded to {len(decoded)} bytes; "
            f"expected {uncompressed_size}"
        )
    return decoded, end


def read_sfil(
    path: str | Path,
    records: list[AssetRecord],
    *,
    require_all_records: bool = True,
    record_index: dict[int, AssetRecord] | None = None,
) -> list[StreamAsset]:
    source_path = Path(path)
    source = source_path.read_bytes()
    if len(source) < SFIL_HEADER_SIZE or source[:4] != b"SFIL":
        raise StreamFormatError(f"{source_path} is not an SFIL stream")

    by_id = record_index if record_index is not None else {record.asset_id: record for record in records}
    source_view = memoryview(source)
    first_asset_offset = struct.unpack_from(">I", source, 16)[0]
    if first_asset_offset < SFIL_HEADER_SIZE or first_asset_offset > len(source):
        raise StreamFormatError(
            f"{source_path} has invalid first asset offset 0x{first_asset_offset:X}"
        )

    assets: list[StreamAsset] = []
    cursor = first_asset_offset
    seen_ids: set[int] = set()

    while cursor < len(source):
        # bytes[cursor:] copied the entire remaining stream for every asset.
        # A view preserves the same padding check without quadratic copying.
        if not any(source_view[cursor:]):
            break
        if cursor + SFIL_SECTION_DATA_OFFSET > len(source):
            if any(source_view[cursor:]):
                raise StreamFormatError(f"truncated SFIL asset at 0x{cursor:X}")
            break

        asset_id = struct.unpack_from(">Q", source, cursor)[0]
        stored_size = struct.unpack_from(">I", source, cursor + 8)[0]
        asset_header_size = struct.unpack_from(">I", source, cursor + 12)[0]
        asset_stride = struct.unpack_from(">I", source, cursor + 16)[0]
        record = by_id.get(asset_id)
        if record is None:
            raise StreamFormatError(
                f"SFIL asset 0x{asset_id:016X} is absent from its ATOC"
            )
        if asset_id in seen_ids:
            raise StreamFormatError(f"duplicate SFIL asset 0x{asset_id:016X}")

        if stored_size == record.total_size and asset_header_size >= SFIL_HEADER_SIZE:
            data_start = cursor + asset_header_size
            data_end = data_start + stored_size
            if data_end > len(source):
                raise StreamFormatError(
                    f"raw SFIL asset 0x{asset_id:016X} exceeds its source file"
                )
            decoded = source[data_start:data_end]
        else:
            section_values = struct.unpack_from(
                ">11I", source, cursor + SFIL_SECTION_TABLE_OFFSET
            )
            cpu_size, cpu_stored, cpu_compression = section_values[:3]
            gpu_size, gpu_stored, gpu_compression = section_values[8:11]
            if stored_size != 0x50 + cpu_stored + gpu_stored:
                raise StreamFormatError(
                    f"SFIL asset 0x{asset_id:016X} has inconsistent stored size"
                )

            data_cursor = cursor + SFIL_SECTION_DATA_OFFSET
            cpu, data_cursor = _decode_section(
                source, data_cursor, cpu_size, cpu_stored, cpu_compression
            )
            gpu, data_cursor = _decode_section(
                source, data_cursor, gpu_size, gpu_stored, gpu_compression
            )
            decoded = cpu + gpu
        if len(decoded) != record.total_size:
            raise StreamFormatError(
                f"asset 0x{asset_id:016X} decoded to {len(decoded)} bytes; "
                f"ATOC declares {record.total_size}"
            )

        assets.append(
            StreamAsset(
                record=record,
                source_path=source_path,
                source_offset=cursor,
                stored_size=stored_size,
                data=decoded,
            )
        )
        seen_ids.add(asset_id)
        minimum_stride = asset_header_size + stored_size
        if (
            asset_stride < minimum_stride
            or asset_stride % first_asset_offset != 0
            or cursor + asset_stride > len(source)
        ):
            raise StreamFormatError(
                f"SFIL asset 0x{asset_id:016X} in {source_path} "
                f"has invalid stride 0x{asset_stride:X}"
            )
        cursor += asset_stride

    missing = set(by_id) - seen_ids if require_all_records else set()
    if require_all_records and missing:
        missing_text = ", ".join(f"0x{asset_id:016X}" for asset_id in sorted(missing))
        raise StreamFormatError(f"ATOC assets missing from SFIL: {missing_text}")
    return assets


def load_global_stream(
    stream_directory: str | Path,
    stream_name: str,
    district_name: str = "DIST_MegaPark",
) -> list[StreamAsset]:
    root = Path(stream_directory)
    records = read_atoc(root / f"{district_name}_{stream_name}.xst")
    return read_sfil(root / f"c{stream_name}_Global.xsf", records)


def load_district_stream(
    stream_directory: str | Path,
    stream_name: str,
    district_name: str,
) -> list[StreamAsset]:
    """Load one ATOC whose assets are distributed over global and cell SFILs."""

    root = Path(stream_directory)
    records = read_atoc(root / f"{district_name}_{stream_name}.xst")
    stream_files = sorted(root.glob(f"c{stream_name}_*.xsf"))
    if not records and not stream_files:
        return []
    if not stream_files:
        raise StreamFormatError(
            f"{root} contains no c{stream_name}_*.xsf stream files"
        )

    assets_by_id: dict[int, StreamAsset] = {}
    record_index = {record.asset_id: record for record in records}
    for stream_file in stream_files:
        for asset in read_sfil(
            stream_file,
            records,
            require_all_records=False,
            record_index=record_index,
        ):
            asset_id = asset.record.asset_id
            previous = assets_by_id.get(asset_id)
            if previous is not None:
                if previous.record != asset.record or previous.data != asset.data:
                    raise StreamFormatError(
                        f"asset 0x{asset_id:016X} has conflicting SFIL copies"
                    )
                continue
            assets_by_id[asset_id] = asset

    expected_ids = {record.asset_id for record in records}
    missing = expected_ids - set(assets_by_id)
    if missing:
        missing_text = ", ".join(f"0x{asset_id:016X}" for asset_id in sorted(missing))
        raise StreamFormatError(
            f"ATOC assets missing from district SFILs: {missing_text}"
        )
    return [assets_by_id[record.asset_id] for record in records]
