#!/usr/bin/env python3
"""Landing loudness against landing class, for retail and for this engine.

This is the measurement that settled "every landing sounds equally loud". Both sides emit an
``OUT`` line per audio block with the six native bus peaks and an RMS, so the same analysis runs
on either, and both are tagged with a frame number in column 2 that the state lines share.

Retail (recomp capture, ``SKATE3_AUDIO_CAPTURE=<path>`` on skate3.exe, music **off**):
    the air time comes from the ``Class_Treatment`` *update* packet, word 7, which is
    ``fctiwz(air x 1000)`` clamped to 10000 -- i.e. milliseconds. A landing is a falling edge of
    that word. The class is then retail's own bucketing of ``air x 0.5`` at 0.31 and 0.50, which
    is 0.62 s and 1.02 s of air.

This engine (``SKATE_AUDIO_TRACE=<path>``):
    the ``AS`` line carries ``air332``, ``land448`` (the per-wheel buckets) and ``landed464``.
    A landing is a falling edge of ``air332``; the class is the largest ``land448`` over the
    wheels that are down, which is exactly what ``sub_824BA630`` computes.

    python tools/audio/landing_levels.py logs/audio-trace-*.log
    python tools/audio/landing_levels.py --retail .local/captures/retail-levels-*.log

Reference numbers, retail, binned by its own class thresholds (35 landings, 16 ms to 1.2 s):

    class 0: n=24  mean -4.4 dBFS    class 1: +0.4 (+4.8)    class 2: +2.0 (+6.4)

Beware two traps this script exists to avoid:
  * A clipped peak reads as exactly 1.0 whatever is underneath it, so a stack that is 7 dB into
    the ceiling looks identical to one that is 0.1 dB into it. Always read ``clipped blocks`` and
    the per-class ``max`` before believing a mean.
  * Sampling only a narrow band of air times hides the class step entirely -- 15 landings that
    happened to span 350-783 ms once "proved" retail keeps landing gain flat. It does not.
"""

import argparse
import math
import re
import sys
from collections import defaultdict

OUT = re.compile(r"^(\d+) (\d+) OUT ([\d.\-eE ]+)\| rms ([\d.eE-]+)")
ENGINE_AS = re.compile(
    r"^(\d+) (\d+) AS .*?air332=(\d).*?land448=\[([0-9, ]+)\] landed464=\[([a-z, ]+)\]"
)
RETAIL_UP = re.compile(r"^(\d+) (\d+) UP \S+ Class_Treatment ")


def db(v):
    return 20.0 * math.log10(v) if v > 0 else -99.0


def retail_class(air_ms):
    """`bridge_wheel_landing`: factor = air x 0.5, bucketed at the vault's 0.31 and 0.50."""
    factor = (air_ms / 1000.0) * 0.5
    return 2 if factor >= 0.50 else (1 if factor >= 0.31 else 0)


def parse(path, retail):
    outs, landings, prev = [], [], 0
    with open(path, errors="ignore") as handle:
        for line in handle:
            match = OUT.match(line)
            if match:
                peak = max(float(x) for x in match.group(3).split())
                outs.append((int(match.group(2)), peak, float(match.group(4))))
                continue
            if retail:
                if not RETAIL_UP.match(line) or "|" not in line:
                    continue
                words = line.split("|")[1].split()
                if len(words) < 8:
                    continue
                air = int(words[7], 16)
                if prev > 0 and air == 0:
                    landings.append((int(line.split()[1]), retail_class(prev)))
                prev = air
            else:
                match = ENGINE_AS.match(line)
                if not match:
                    continue
                air = int(match.group(3))
                if prev == 1 and air == 0:
                    buckets = [int(x) for x in match.group(4).split(",")]
                    down = [x.strip() == "true" for x in match.group(5).split(",")]
                    cls = max((buckets[i] for i in range(4) if down[i]), default=None)
                    landings.append((int(match.group(2)), cls))
                prev = air
    return outs, landings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace")
    parser.add_argument("--retail", action="store_true", help="a recomp capture, not an engine trace")
    parser.add_argument("--window", type=int, default=30, help="frames after the landing to search")
    args = parser.parse_args()

    outs, landings = parse(args.trace, args.retail)
    if not outs:
        sys.exit(f"{args.trace}: no OUT lines -- wrong file, or tracing was off")
    outs.sort()

    clipped = sum(1 for _, peak, _ in outs if peak >= 0.999)
    bed = sorted(peak for _, peak, _ in outs)[len(outs) // 2]
    print(f"{args.trace}")
    print(f"  {len(landings)} landings, {len(outs)} output blocks, {clipped} at full scale")
    print(f"  bed (median block peak) {db(bed):+.1f} dBFS")
    if clipped:
        print("  NOTE: blocks are clipping, so any per-class mean below is a floor, not a level.")

    by_class = defaultdict(list)
    for frame, cls in landings:
        window = [peak for f, peak, _ in outs if frame - 2 <= f <= frame + args.window]
        if window:
            by_class[cls].append(db(max(window)))
    if not by_class:
        sys.exit("  no landing lined up with an output block -- do the frame columns agree?")

    print()
    base = None
    for cls in sorted(by_class, key=lambda c: (c is None, c)):
        values = by_class[cls]
        mean = sum(values) / len(values)
        if cls == 0:
            base = mean
        print(
            f"  class {cls}: n={len(values):3d}  mean {mean:+6.1f} dBFS"
            f"   min {min(values):+6.1f}  max {max(values):+6.1f}"
        )
    if base is not None:
        print("\n  relative to class 0        (retail: 0.0 / +4.8 / +6.4 dB)")
        for cls in sorted(by_class, key=lambda c: (c is None, c)):
            values = by_class[cls]
            print(f"    class {cls}: {sum(values) / len(values) - base:+.1f} dB")
        print(f"\n  retail's class 0 sits at -4.4 dBFS; this trace's is {base:+.1f}")


if __name__ == "__main__":
    main()
