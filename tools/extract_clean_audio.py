"""Extract the clean player sounds from the skate3-cleanroom web build.

    python3 tools/extract_clean_audio.py [out_dir]

Downloads the start of the published core.pack (SKPK1: u32 count, then per file
u32 path length, path, u64 size, bytes) and writes every assets/clean-audio/
entry to out_dir (default crates/skate-game/clean-audio). Those sounds are CC0
recordings or resynthesised from coarse descriptors (see the repo's README and
CREDITS.json); no retail audio.
"""
import os
import struct
import sys
import urllib.request

URL = "https://andrewnakas.github.io/skate3-cleanroom/core.pack"
PREFIX = "assets/clean-audio/"


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "crates/skate-game/clean-audio"
    req = urllib.request.Request(URL, headers={"Range": "bytes=0-6000000"})
    data = urllib.request.urlopen(req).read()
    if data[:5] != b"SKPK1":
        sys.exit("not an SKPK1 pack")
    count = struct.unpack_from("<I", data, 8)[0]
    at, written = 12, 0
    os.makedirs(out, exist_ok=True)
    for _ in range(count):
        n = struct.unpack_from("<I", data, at)[0]
        path = data[at + 4:at + 4 + n].decode()
        size = struct.unpack_from("<Q", data, at + 4 + n)[0]
        at += 12 + n
        if not path.startswith(PREFIX):
            break
        with open(os.path.join(out, path[len(PREFIX):]), "wb") as f:
            f.write(data[at:at + size])
        written += 1
        at += size
    print(f"{written} files -> {out}")


if __name__ == "__main__":
    main()
