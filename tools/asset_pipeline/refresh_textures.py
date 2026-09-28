"""Refresh SKATE14 texture payloads from a newly decoded RX2 manifest cache.

Material, geometry, collision and metadata bytes remain unchanged. Publication
is atomic and never overwrites the input map. No renderer is launched.
"""
import hashlib
import json
import os
import struct
import tempfile
import zlib
from pathlib import Path
import numpy as np


def refresh(manifest, source, output):
    manifest, source, output = map(Path, (manifest, source, output))
    if source.resolve() == output.resolve():
        raise ValueError('Refresh requires a separate output map')
    data = json.loads(manifest.read_text())
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with source.open('rb') as src:
            def read(n):
                value = src.read(n)
                if len(value) != n:
                    raise ValueError('Truncated SKATE14 package')
                return value
            def uint():
                return struct.unpack('<I', read(4))[0]
            def text():
                return read(uint()).decode('utf-8')
            if read(8) != b'SKATE14\0' or uint() != 0x12345678:
                raise ValueError('Texture refresh requires SKATE14')
            text()
            read(49 * 4)
            counts = [uint() for _ in range(9)]
            for _ in range(counts[0]):
                text()
                read(4 + 7*4 + 8 + 4 + 16 + 4 + 16)
                if uint():
                    read(16)
                    text()
                    read(8)
                    for _ in range(uint()):
                        text()
                        read(16)
                    for _ in range(uint()):
                        text()
                        for _ in range(uint()):
                            text()
                    text()
            prefix_end = src.tell()
            src.seek(0)
            prefix = read(prefix_end)
            changed = []
            with tempfile.NamedTemporaryFile(dir=output.parent, suffix='.skate.tmp', delete=False) as dst:
                temporary = Path(dst.name)
                dst.write(prefix)
                for _ in range(counts[1]):
                    start = src.tell()
                    name = text()
                    width, height, _space = uint(), uint(), uint()
                    data_start = src.tell()
                    method, size = uint(), uint()
                    packed = read(size)
                    if method not in (0, 1):
                        raise ValueError('Unsupported texture compression')
                    old = zlib.decompress(packed) if method else packed
                    end = src.tell()
                    entry = data['textures'][name]
                    if (width, height) != (entry['width'], entry['height']):
                        raise ValueError('Texture dimensions changed: ' + name)
                    cache = (manifest.parent / entry['rgba']).read_bytes()
                    if len(cache) != width*height*4 or len(old) != len(cache):
                        raise ValueError('Texture byte count mismatch: ' + name)
                    array = np.frombuffer(cache, np.uint8).reshape(height, width, 4)
                    new = (array if entry.get('cube_faces') == 6 else array[::-1]).tobytes()
                    src.seek(start)
                    dst.write(read(data_start-start))
                    src.seek(end)
                    if new != old:
                        replacement = zlib.compress(new, 1)
                        dst.write(struct.pack('<II', 1, len(replacement)))
                        dst.write(replacement)
                        changed.append(dict(name=name, width=width, height=height, format=entry.get('format'),
                            changed_pixels=int(np.any(np.frombuffer(old, np.uint8).reshape(-1, 4) !=
                                np.frombuffer(new, np.uint8).reshape(-1, 4), axis=1).sum())))
                    else:
                        dst.write(struct.pack('<II', method, size))
                        dst.write(packed)
                old_tail_offset, new_tail_offset = src.tell(), dst.tell()
                tail_hash = hashlib.sha256()
                while chunk := src.read(1024*1024):
                    tail_hash.update(chunk)
                    dst.write(chunk)
            os.replace(temporary, output)
            temporary = None
            return dict(changed=changed, count=len(changed), textures=counts[1],
                prefix_sha256=hashlib.sha256(prefix).hexdigest(), tail_sha256=tail_hash.hexdigest(),
                old_tail_offset=old_tail_offset, new_tail_offset=new_tail_offset)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
