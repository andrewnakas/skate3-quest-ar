"""Extract raw Xenos microcode from Skate 2's RefPack FPO/VPO objects."""

from __future__ import annotations

import argparse
from pathlib import Path
import struct

from skate3_streams import decompress_refpack


OBJECT_MAGICS = {
    ".fpo": 0x102A1100,
    ".vpo": 0x102A1101,
}
LITERAL_CONSTANT_BYTES = 64


def extract_shader(source: Path, output: Path) -> None:
    decoded = decompress_refpack(source.read_bytes())
    if len(decoded) < 12:
        raise ValueError(f"{source} has a truncated shader object")
    magic, microcode_offset, microcode_size = struct.unpack_from(
        ">III", decoded
    )
    expected_magic = OBJECT_MAGICS.get(source.suffix.lower())
    if expected_magic is None:
        raise ValueError(f"{source} is not an FPO or VPO shader object")
    if magic != expected_magic:
        raise ValueError(
            f"{source} has unexpected shader magic 0x{magic:08X}"
        )
    end = microcode_offset + microcode_size
    if (
        microcode_offset < 12
        or microcode_size < LITERAL_CONSTANT_BYTES + 8
        or microcode_size % 4
        or end != len(decoded)
    ):
        raise ValueError(
            f"{source} has invalid microcode range "
            f"{microcode_offset}:{end} of {len(decoded)}"
        )
    # The serialized Xbox shader object prefixes the actual control-flow
    # stream with four literal float4 constants. Xenia's Shader analyzer
    # consumes the raw control-flow and ALU/fetch stream only.
    ucode_start = microcode_offset + LITERAL_CONSTANT_BYTES
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(decoded[ucode_start:end])
    print(
        f"{source.name}: {len(source.read_bytes())} stored -> "
        f"{len(decoded)} object -> "
        f"{microcode_size - LITERAL_CONSTANT_BYTES} ucode bytes -> {output}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("shaders", nargs="+", type=Path)
    args = parser.parse_args()
    output_root = args.output.resolve()
    for source in args.shaders:
        source = source.resolve()
        prefix = "ps_" if source.suffix.lower() == ".fpo" else "vs_"
        extract_shader(
            source,
            output_root / f"{prefix}{source.stem}.bin",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
