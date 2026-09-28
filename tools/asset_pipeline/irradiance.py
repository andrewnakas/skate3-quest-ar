"""Export retail RWOBJECTTYPE_IRRADIANCEDATA (0xEB0024) without resampling."""
import json
import math
import struct
from pathlib import Path


def decode(raw):
    if raw[:7] != b'\x89RW4xb2':
        raise ValueError('Not an Xbox RX2 resource')
    count, table = struct.unpack_from('>I', raw, 32)[0], struct.unpack_from('>I', raw, 48)[0]
    groups = []
    for i in range(count):
        offset, _, size, _, _, kind = struct.unpack_from('>6I', raw, table + i*24)
        if kind != 0xEB0024:
            continue
        n, start = struct.unpack_from('>2I', raw, offset)
        if start < 8 or start+n*160 > size or offset+size > len(raw):
            raise ValueError('Invalid irradiance record extent')
        records = []
        for j in range(n):
            values = struct.unpack_from('>39fI', raw, offset+start+j*160)
            if not all(math.isfinite(x) for x in values[:39]):
                raise ValueError('Non-finite irradiance sample')
            records.append(struct.pack('<39fI', *values))
        if records:
            groups.append(records)
    return groups


def write(manifest_path, output):
    manifest = json.loads(manifest_path.read_text())
    groups = []
    for asset in manifest['other_presentation_assets']:
        groups.extend(decode((manifest_path.parent / asset['rx2']).read_bytes()))
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open('wb') as f:
        f.write(b'SHP1' + struct.pack('<I', len(groups)))
        for records in groups:
            f.write(struct.pack('<I', len(records)))
            f.write(b''.join(records))
    return sum(map(len, groups))


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print('Exported', write(args.manifest, args.output), 'irradiance samples')
