"""Compare a Rust playtest trace against the retail capture, as transfer functions.

`tools/audio_capture_verify.py` checks a recovered formula against retail's own audio state. This
answers the different question: **does this engine, playing the game, land in the same place retail
did?** It bins both sides by speed and prints them side by side, so a divergence shows up as a
column that does not track rather than as a single number that is hard to interpret.

    python tools/audio_compare.py <rust-trace.log> [retail-extract-dir]

The Rust trace comes from `SKATE_AUDIO_TRACE=<file>` (`player_audio/trace.rs`). The retail extract
defaults to `.local/captures/extract`, restored from the owner's drive
(`<drive>:\\skate3-retail-captures\\extract`).

The three tables are the ones `docs/player-audio-retail-drivers.md` §11 tabulates for retail:

* **turn `+204`** -- the fraction of frames it is zero. It gates the second grain player through
  `f1164 = clamp(|COM velocity| x 0.24, cap) x turn_204`, and retail's is zero in 87% of frames. If
  this engine's is non-zero most of the time, the offset B layer runs permanently and the rolling
  bed is the same grain recording played from two positions 0.1 apart -- a doubled, phasey bed that
  gets worse with speed.
* **the grain records against ground speed** -- gain A, gain B, their ratio, and the two positions.
  Retail's B player is silent at 40 km/h and above; its positions differ by exactly 0.100.
* **`SenseOfSpeed_wind` against COM speed** -- the intensity word w3 and the MixMap level w7.
"""

import math
import re
import struct
import sys
from collections import defaultdict

RETAIL_TURN_ZERO_FRACTION = 0.87

# docs/player-audio-retail-drivers.md §11, keyed by the 5 km/h bin.
RETAIL_GRAIN = {
    0: (0.0047, 0.0006, 0.002, 0.001),
    5: (0.0613, 0.0118, 0.022, 0.010),
    10: (0.1245, 0.0595, 0.121, 0.043),
    15: (0.0721, 0.0989, 0.140, 0.063),
    20: (0.1700, 0.0078, 0.407, 0.308),
    25: (0.0950, 0.0637, 0.340, 0.241),
    30: (0.1121, 0.0567, 0.394, 0.295),
    35: (0.0829, 0.0373, 0.515, 0.415),
    40: (0.1214, 0.0000, 0.634, 0.534),
    45: (0.0303, 0.0000, 0.596, 0.496),
}
RETAIL_WIND = {
    15: (51, 210),
    20: (154, 2681),
    25: (272, 678),
    30: (442, 975),
    35: (509, 1521),
    40: (624, 3300),
    45: (683, 683),
    50: (906, 8246),
    55: (964, 8091),
}

MIN_SAMPLES = 20


def db(value, full=1.0):
    return 20.0 * math.log10(max(value, 1e-9) / full)


def ratio_db(b, a):
    if b <= 0.0:
        return None
    return 20.0 * math.log10(b / max(a, 1e-9))


def bin_kmh(metres_per_second):
    return int(round(metres_per_second * 3.6 / 5.0) * 5)


def parse_rust(path):
    """AS, GR and SenseOfSpeed_wind UP lines, keyed by frame."""
    state = {}
    grain = defaultdict(list)
    wind = {}
    as_re = re.compile(r"v208=(-?[\d.]+) com212=(-?[\d.]+) turn204=(-?[\d.]+)")
    gr_re = re.compile(
        r"GR (\d+) a=(-?[\d.]+),(-?[\d.]+),(-?[\d.]+) b=(-?[\d.]+),(-?[\d.]+),(-?[\d.]+)"
    )
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            parts = line.split(" ", 3)
            if len(parts) < 3:
                continue
            frame, tag = parts[1], parts[2]
            if tag == "AS":
                m = as_re.search(line)
                if m:
                    state[int(frame)] = tuple(float(g) for g in m.groups())
            elif tag == "GR":
                m = gr_re.search(line)
                if m:
                    g = m.groups()
                    grain[int(frame)].append(
                        (int(g[0]), tuple(float(x) for x in g[1:4]), tuple(float(x) for x in g[4:7]))
                    )
            elif tag == "UP" and " SenseOfSpeed_wind " in line and "|" in line:
                words = line.split("|", 1)[1].split()
                if len(words) > 7:
                    wind[int(frame)] = (int(words[3], 16), int(words[7], 16))
    if not state:
        sys.exit(
            f"{path}: no AS lines with turn204. Re-stage the build -- the trace only carries "
            "turn204/com212 from 2026-09-23 onward."
        )
    return state, grain, wind


def report_turn(state):
    turns = [t for _, _, t in state.values()]
    zero = sum(1 for t in turns if abs(t) < 1e-6) / len(turns)
    print(f"\nturn +204 over {len(turns)} frames")
    print(f"  this engine  zero in {zero * 100:5.1f}% of frames   range {min(turns):+.3f}..{max(turns):+.3f}")
    print(f"  retail       zero in {RETAIL_TURN_ZERO_FRACTION * 100:5.1f}% of frames   range -1.000..+1.000")
    if zero < 0.5:
        print(
            "  -> DIVERGES. The second grain player is gated on this; retail's is off during\n"
            "     ordinary straight rolling. A turn that is rarely zero keeps the offset B layer\n"
            "     running and is the mechanism behind a doubled, echoey bed at speed."
        )
    else:
        print("  -> tracks retail; the B player is not being held open by the turn input.")


def report_grain(state, grain):
    rows = defaultdict(list)
    for frame, records in grain.items():
        if frame not in state:
            continue
        speed = state[frame][0]
        for truck, a, b in records:
            if truck == 0:
                rows[bin_kmh(speed)].append((a, b))
    if not rows:
        print("\nno GR lines joined to AS frames; skipping the grain table")
        return
    print("\ngrain players vs ground speed (truck 0)   [retail in brackets]")
    print(" km/h |         gain A |         gain B |      B/A |   pos A |   pos B |     n")
    for k in sorted(rows):
        r = rows[k]
        if len(r) < MIN_SAMPLES:
            continue
        ga = sum(a[0] for a, _ in r) / len(r)
        gb = sum(b[0] for _, b in r) / len(r)
        pa = sum(a[2] for a, _ in r) / len(r)
        pb = sum(b[2] for _, b in r) / len(r)
        ref = RETAIL_GRAIN.get(k)
        rb = ratio_db(gb, ga)
        ratio = f"{rb:+6.1f}" if rb is not None else " silent"
        if ref:
            print(
                f"{k:5d} | {ga:6.4f} [{ref[0]:6.4f}] | {gb:6.4f} [{ref[1]:6.4f}] | {ratio} |"
                f" {pa:5.3f} [{ref[2]:5.3f}] | {pb:5.3f} [{ref[3]:5.3f}] | {len(r):5d}"
            )
        else:
            print(
                f"{k:5d} | {ga:6.4f} [   -- ] | {gb:6.4f} [   -- ] | {ratio} |"
                f" {pa:5.3f} [  -- ] | {pb:5.3f} [  -- ] | {len(r):5d}"
            )
        if abs((pa - pb) - 0.1) > 0.002 and pa > 0.11:
            print(f"        !! position A - position B = {pa - pb:.4f}, retail's is exactly 0.100")
    fast = [k for k in rows if k >= 40 and len(rows[k]) >= MIN_SAMPLES]
    for k in fast:
        gb = sum(b[0] for _, b in rows[k]) / len(rows[k])
        if gb > 0.001:
            print(
                f"        !! gain B is {gb:.4f} at {k} km/h; retail's is exactly 0 at 40 km/h and above"
            )


def report_wind(state, wind):
    rows = defaultdict(list)
    for frame, (w3, w7) in wind.items():
        if frame in state:
            rows[bin_kmh(state[frame][1])].append((w3, w7))
    if not rows:
        print("\nno SenseOfSpeed_wind updates in the trace (it posts above 15 km/h) -- skipping")
        return
    print("\nSenseOfSpeed_wind vs COM speed   [retail in brackets]")
    print(" km/h |   w3 intensity |        w7 level |    dBFS |     n")
    for k in sorted(rows):
        r = rows[k]
        if len(r) < MIN_SAMPLES:
            continue
        w3 = sum(a for a, _ in r) / len(r)
        w7 = sum(b for _, b in r) / len(r)
        ref = RETAIL_WIND.get(k)
        cell3 = f"{w3:6.0f} [{ref[0]:5d}]" if ref else f"{w3:6.0f} [  -- ]"
        cell7 = f"{w7:7.0f} [{ref[1]:5d}]" if ref else f"{w7:7.0f} [  -- ]"
        print(f"{k:5d} | {cell3} | {cell7} | {db(w7, 32767.0):7.1f} | {len(r):5d}")


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    state, grain, wind = parse_rust(sys.argv[1])
    frames = sorted(state)
    print(f"{sys.argv[1]}: {len(frames)} AS frames, {len(grain)} GR frames, {len(wind)} wind updates")
    speeds = [v for v, _, _ in state.values()]
    print(f"ground speed: max {max(speeds) * 3.6:.1f} km/h, mean {sum(speeds) / len(speeds) * 3.6:.1f} km/h")
    report_turn(state)
    report_grain(state, grain)
    report_wind(state, wind)


if __name__ == "__main__":
    main()
