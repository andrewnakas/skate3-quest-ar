#!/usr/bin/env python3
"""Clean-room audit for the public (no-import) tier of the Quest AR app.

Checks that the clean tier shares nothing with the retail game:
  files       no retail-format or private/ file in the APK or among the files the clean tier opened
  bytes       no 32-byte run of any clean blob (embedded sounds, generated meshes) occurs in retail data
  names       no retail identifier (bones, clips, graph ops, VLT names, gesture names) in clean sources
  values      no float literal with 3+ significant digits in clean sources equals a retail VLT float
  provenance  no porting markers (retail addresses, "port of", skate_data::) in clean sources
  credits     every embedded sound is credited as CC0 or original synthesis (none resynthesised)

Usage:
  scripts/cleanroom/audit.py --retail <assets dir of an imported/unpacked pack>
      [--apk target/release/apk/skate3ar.apk] [--opens <log with SKATE_OPEN lines>]
      [--blobs <dir from --dump-clean-assets>]

The retail tree is only read locally to compare against; nothing from it is written anywhere.
Exit status 1 on any failure.
"""
import argparse
import json
import os
import re
import struct
import sys
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
HOME = Path.home()

# Everything the public tier compiles and runs that is ours. Missing entries are skipped.
CLEAN_SOURCES = [
    HOME / "cleanroom/skatecore-cpp/clean",
    REPO / "crates/skate-lite-sys/src",
    REPO / "crates/skate-lite-sys/build.rs",
    REPO / "crates/skate-lite-sys/vendor",
    REPO / "crates/skate-game/src/clean",
    REPO / "crates/skate-game/src/xr/panel.rs",
]
CLEAN_BLOBS = [REPO / "crates/skate-game/clean-audio"]
SOURCE_EXT = {".c", ".cc", ".cpp", ".h", ".hpp", ".rs", ".py", ".wgsl", ".toml", ".json"}
RETAIL_EXT = {".abin", ".stategraph", ".pat", ".shk", ".vlt", ".bin", ".big"}
RETAIL_NAMES = {"skater-collections.json", "physics-skeletons.json", "input.cfg", "game.json"}

PROVENANCE = [
    (re.compile(r"\b(?:0x)?82[0-9A-Fa-f]{6}\b"), "retail address"),
    (re.compile(r"\bTU3\b"), "retail build tag"),
    (re.compile(r"\bport(?:ed)? (?:of|from)\b", re.I), "porting note"),
    (re.compile(r"\brecovered\b", re.I), "recovered value"),
    (re.compile(r"\bstock\b", re.I), "stock data reference"),
    (re.compile(r"skate_data::|skate_core::"), "faithful-port crate"),
]

failures = []


def fail(section, msg):
    failures.append(f"[{section}] {msg}")


def walk(paths, exts=None):
    for p in paths:
        if not p.exists():
            continue
        files = [p] if p.is_file() else sorted(x for x in p.rglob("*") if x.is_file())
        for f in files:
            if exts is None or f.suffix.lower() in exts:
                yield f


# ------------------------------------------------------------------ files

def is_retail_path(name):
    n = name.replace("\\", "/")
    base = n.rsplit("/", 1)[-1]
    return "private/" in n or Path(base).suffix.lower() in RETAIL_EXT or base in RETAIL_NAMES


def check_files(apk, opens):
    checked = 0
    if apk:
        with zipfile.ZipFile(apk) as z:
            for info in z.infolist():
                checked += 1
                if is_retail_path(info.filename) or info.filename.endswith("core.pack"):
                    fail("files", f"APK contains {info.filename}")
    if opens:
        for line in Path(opens).read_text(errors="replace").splitlines():
            m = re.search(r"SKATE_OPEN (.+)$", line)
            if m:
                checked += 1
                if is_retail_path(m.group(1).strip()):
                    fail("files", f"clean tier opened {m.group(1).strip()}")
    return checked


# ------------------------------------------------------------------ bytes

WINDOW = 32


def window_set(data):
    """Hashes of every WINDOW-byte window (all offsets) that is not low-entropy."""
    out = set()
    for i in range(0, len(data) - WINDOW + 1):
        w = data[i:i + WINDOW]
        if len(set(w)) >= 10:
            out.add(hash(w))
    return out


def check_bytes(retail, blob_dirs, apk):
    clean = {}
    for f in walk(blob_dirs):
        clean[str(f)] = f.read_bytes()
    if apk:
        with zipfile.ZipFile(apk) as z:
            for info in z.infolist():
                if info.filename.startswith("assets/"):
                    clean["apk:" + info.filename] = z.read(info.filename)
    index = {}
    for name, data in clean.items():
        for h in window_set(data):
            index.setdefault(h, name)
    hits = {}
    scanned = 0
    for f in walk([retail / "private"]):  # only retail-derived data, not the pack's own extras
        data = f.read_bytes()
        scanned += len(data)
        for i in range(0, len(data) - WINDOW + 1, 4):
            h = hash(data[i:i + WINDOW])
            if h in index:
                w = data[i:i + WINDOW]
                src = index[h]
                if w in clean[src] and len(set(w)) >= 10:
                    hits.setdefault((src, str(f)), 0)
                    hits[(src, str(f))] += 1
    for (src, ret), n in sorted(hits.items()):
        fail("bytes", f"{src} shares {n} run(s) with {ret}")
    return len(clean), scanned


# ------------------------------------------------------------------ names

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]{4,}")


def identifier_like(s):
    if len(s) < 5 or s.startswith("Hash_"):
        return False
    if "_" in s.strip("_"):
        return True
    if re.search(r"[a-z][A-Z]", s):  # CamelCase with an inner capital
        return True
    return s.isupper() and len(s) >= 5 and s.isalpha()


ALLOW = {
    # generic words and our own vocabulary that happen to look like identifiers
    "PUSH", "BRAKE", "SLIDE", "MANUAL", "RIDE", "GRIND", "BAIL", "TRICK", "LAND", "RESPAWN",
    # controller hardware: OpenXR Touch fields and Bevy's gamepad API
    "LEFT_STICK", "RIGHT_STICK",
}


def ascii_strings(data, minimum=5):
    return re.findall(rb"[A-Za-z_][A-Za-z0-9_]{%d,}" % (minimum - 1), data)


def retail_identifiers(retail):
    names = set()
    stock = retail / "private/stock"
    for f in walk([stock / "data"]):
        if f.suffix.lower() in {".abin", ".stategraph", ".pat", ".cfg", ".shk"}:
            for s in ascii_strings(f.read_bytes()):
                names.add(s.decode())
    sk = stock / "physics-skeletons.json"
    if sk.exists():
        for skel in json.loads(sk.read_text())["skeletons"]:
            names.add(skel["name"])
            for b in skel["bones"]:
                names.add(b["name"])
    col = stock / "skater-collections.json"
    if col.exists():
        for c in json.loads(col.read_text())["collections"]:
            for k in (c["class"], c["key"], *c["fields"].keys()):
                names.add(k)
    return {n for n in names if identifier_like(n) and n.upper() not in ALLOW}


def check_names(retail):
    names = retail_identifiers(retail)
    files = 0
    for f in walk(CLEAN_SOURCES, SOURCE_EXT):
        files += 1
        text = f.read_text(errors="replace")
        for m in IDENT.finditer(text):
            if m.group(0) in names:
                line = text.count("\n", 0, m.start()) + 1
                fail("names", f"{f}:{line} uses retail identifier {m.group(0)}")
    return len(names), files


# ------------------------------------------------------------------ values

FLOAT_LIT = re.compile(r"(?<![\w.])(\d+\.\d+)(?:e[+-]?\d+)?f?(?![\w.])")
FLOAT_TYPES = ("Float", "Vector3", "Vector4", "Vector2", "Graph", "Point", "Matrix", "Quat")


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def sig_digits(lit):
    digits = lit.replace(".", "").lstrip("0").rstrip("0")
    return len(digits)


def retail_floats(retail):
    col = retail / "private/stock/skater-collections.json"
    out = set()
    if not col.exists():
        return out
    for c in json.loads(col.read_text())["collections"]:
        for field in c["fields"].values():
            if not any(t in field["type"] for t in FLOAT_TYPES):
                continue
            raw = bytes.fromhex(field["data"])
            for i in range(0, len(raw) - 3, 4):
                v = struct.unpack(">f", raw[i:i + 4])[0]
                if v == v and abs(v) < 1e7:
                    out.add(v)
    return out


def check_values(retail):
    values = retail_floats(retail)
    checked = 0
    for f in walk(CLEAN_SOURCES, {".c", ".cc", ".cpp", ".h", ".hpp", ".rs"}):
        text = f.read_text(errors="replace")
        for m in FLOAT_LIT.finditer(text):
            lit = m.group(1)
            if sig_digits(lit) < 3:
                continue  # too generic to say anything (0.5, 1.2, 25.0 ...)
            checked += 1
            v = f32(float(lit))
            if v in values or -v in values:
                line = text.count("\n", 0, m.start()) + 1
                fail("values", f"{f}:{line} literal {lit} equals a retail VLT float")
    return len(values), checked


# ------------------------------------------------------------------ provenance

def check_provenance():
    files = 0
    for f in walk(CLEAN_SOURCES, SOURCE_EXT):
        files += 1
        for n, line in enumerate(f.read_text(errors="replace").splitlines(), 1):
            for rx, what in PROVENANCE:
                if rx.search(line):
                    fail("provenance", f"{f}:{n} {what}: {line.strip()[:100]}")
    return files


def check_credits():
    audio = REPO / "crates/skate-game/clean-audio"
    credits = {c["sound"]: c for c in json.loads((audio / "CREDITS.json").read_text())}
    wavs = sorted(f.stem for f in audio.glob("*.wav"))
    for w in wavs:
        c = credits.get(w)
        if c is None:
            fail("credits", f"{w}.wav has no credit (resynthesised sounds must be replaced)")
        elif not ("CC0" in c["license"] or c["license"].startswith("Original work")):
            fail("credits", f"{w}.wav license is {c['license']!r}")
    return len(wavs)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--retail", type=Path, default=os.environ.get("SKATE_RETAIL_ASSETS"),
                    help="retail assets dir (contains private/stock)")
    ap.add_argument("--apk", type=Path)
    ap.add_argument("--opens", type=Path, help="log containing SKATE_OPEN lines from a clean run")
    ap.add_argument("--blobs", type=Path, action="append", default=[], help="extra clean blob dirs")
    ap.add_argument("--skip", action="append", default=[], help="skip a section (bytes is slow)")
    a = ap.parse_args()
    if not a.retail or not (Path(a.retail) / "private/stock").exists():
        sys.exit("need --retail <assets dir with private/stock> (or SKATE_RETAIL_ASSETS)")
    retail = Path(a.retail)

    print("clean sources:")
    for p in CLEAN_SOURCES:
        print(f"  {'ok ' if p.exists() else '-- '} {p}")
    if "files" not in a.skip:
        print(f"files: {check_files(a.apk, a.opens)} entries checked")
    if "provenance" not in a.skip:
        print(f"provenance: {check_provenance()} source files")
    if "names" not in a.skip:
        n, files = check_names(retail)
        print(f"names: {n} retail identifiers vs {files} source files")
    if "values" not in a.skip:
        n, checked = check_values(retail)
        print(f"values: {n} retail floats vs {checked} literals")
    if "credits" not in a.skip:
        print(f"credits: {check_credits()} sounds")
    if "bytes" not in a.skip:
        blobs, scanned = check_bytes(retail, CLEAN_BLOBS + a.blobs, a.apk)
        print(f"bytes: {blobs} clean blobs vs {scanned / 1e6:.1f} MB retail")
    if failures:
        print(f"\nFAIL: {len(failures)} finding(s)")
        for f in failures[:400]:
            print("  " + f)
        if len(failures) > 400:
            print(f"  ... {len(failures) - 400} more")
        sys.exit(1)
    print("\nPASS: nothing shared with the retail data")


if __name__ == "__main__":
    main()
