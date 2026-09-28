"""Check recovered packet formulas against retail: capture audio state -> redelivered words."""
import math
import struct
import sys
from collections import defaultdict

EX = sys.argv[1] if len(sys.argv) > 1 else r".local/captures/extract"

states = {}
for line in open(EX + r"\state.tsv"):
    p = line.rstrip("\n").split("\t")
    states[int(p[0])] = [int(x, 16) for x in p[2:]]


def f(w, off):
    return struct.unpack(">f", struct.pack(">I", w[(off - 192) // 4]))[0]


def b(w, off):
    word = w[(off - 192) // 4]
    return (word >> (8 * (3 - (off - 192) % 4))) & 0xFF


def i32(w, off):
    v = w[(off - 192) // 4]
    return v - (1 << 32) if v & 0x80000000 else v


def f32(x):
    return struct.unpack(">f", struct.pack(">f", x))[0]


def trunc(x):
    return int(math.trunc(x))


def clamp(x, lo, hi):
    return lo if x < lo else hi if x > hi else x


def updates(obj):
    for line in open(EX + rf"\updates\{obj}.tsv"):
        p = line.split("\t")
        frame = int(p[0])
        words = [int(x, 16) for x in p[2].split("|")[1].split()]
        yield frame, p[2].split()[0], words


def check(name, obj, word, formula, lag_options=(0, 1, -1), sample=None):
    for lag in lag_options:
        total = ok = 0
        bad = []
        for frame, node, words in updates(obj):
            st = states.get(frame - lag)
            if st is None:
                continue
            expected = formula(st)
            if expected is None:
                continue
            total += 1
            if words[word] == expected:
                ok += 1
            elif len(bad) < 4:
                bad.append((frame, node, words[word], expected))
        if total:
            print(f"{name:28s} lag {lag:+d}: {ok}/{total} exact  bad={bad}")


v = lambda st: f(st, 208)

check("rolling w3 speed", "Class_rolling", 3,
      lambda st: clamp(trunc(clamp(v(st) / 70.0 * 3.6, 0.0, 1.0) * 10000.0), 0, 10000))
check("rolling w7 (340||339)", "Class_rolling", 7, lambda st: int(bool(b(st, 340) or b(st, 339))))
check("grind w7 speed", "Class_grind", 7,
      lambda st: min(trunc(clamp(3.6 * (v(st) - 0.5) / 45.0, 0.0, 1.0) * 10000.0), 9000))
check("SoS rattle w3", "SenseOfSpeed_rattle", 3,
      lambda st: trunc(clamp((v(st) * 3.6 - 30.0) / 50.0, 0.0, 1.0) * 1000.0))
check("SoS wind w3 (212)", "SenseOfSpeed_wind", 3,
      lambda st: trunc(clamp((f(st, 212) * 3.6 - 15.0) / 40.0, 0.0, 1.0) * 1000.0))
check("skid w7", "Class_wheels_skid", 7, lambda st: trunc(clamp(v(st) * 0.08, 0.0, 1.0) * 10000.0))
check("squeaks w7", "Class_Squeaks", 7, lambda st: trunc(clamp(v(st) * 0.08, 0.0, 1.0) * 1000.0))
check("seams w8", "Class_Seams", 8, lambda st: trunc(clamp((v(st) - 0.5) * 0.08, 0.0, 1.0) * 10000.0))
check("treatment w7 air ms", "Class_Treatment", 7, lambda st: clamp(trunc(f(st, 236) * 1000.0), 0, 10000))
check("treatment w8 until ms", "Class_Treatment", 8, lambda st: clamp(trunc(f(st, 240) * 1000.0), 0, 10000))
check("treatment w9 height", "Class_Treatment", 9,
      lambda st: trunc(clamp(f(st, 260) * 166.667, 0.0, 1000.0)))
check("treatment w10 timescale", "Class_Treatment", 10, lambda st: clamp(trunc(f(st, 220) * 500.0), 0, 1000))
check("treatment w14 !paused", "Class_Treatment", 14, lambda st: int(b(st, 224) == 0))


def rate(a, d, t):
    x = min(trunc(abs(a) / d * 1000.0), 1000)
    return x if x >= t else 0


check("flips w7 rate z", "Class_Flips", 7, lambda st: rate(f(st, 488), 10.2, 603))
check("flips w8 rate y", "Class_Flips", 8, lambda st: rate(f(st, 484), 4.4, 703))
check("flips w9 rate x", "Class_Flips", 9, lambda st: rate(f(st, 480), 3.0, 297))
check("squeaks w9 lateral", "Class_Squeaks", 9,
      lambda st: (lambda x: 0 if x < 50 else min(x, 1000))(trunc(abs(f(st, 488)) / 1.5 * 1000.0)))
check("foot_drag w7", "Class_foot_drag", 7,
      lambda st: trunc(clamp((v(st) - 0.5) / 50.0 * 3.6, 0.0, 1.0) * 10000.0))
check("board_slide w3", "c_board_slide", 3,
      lambda st: trunc(clamp((v(st) - 0.5) / 15.0 * 3.6, 0.0, 1.0) * 10000.0))
check("body_slide w3", "c_body_slide", 3, lambda st: trunc(f(st, 212) / 4.5 * 1000.0))
check("cloth_falls w3", "c_cloth_falls", 3,
      lambda st: trunc(max(f(st, 328), f(st, 672)) / 5.0 * 1000.0))
