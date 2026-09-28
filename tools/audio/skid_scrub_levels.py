"""Compare lateral wheel scrub against retail, from the skid family's own intensity word.

The wheel scrub is `Class_wheels_skid`, a *held* loop whose intensity is
`w10 = min(90, counter + trunc(90 * slip))`, where
`slip = clamp((|Motion80 . deckX| + 0.75) / 45, 0, 1)` (vault K1 45.0, K2 -0.75).

So `w10` inverts straight back to a lateral wheel speed, which makes it the one quantity that
can be compared between this engine and a retail capture without any shared tooling:

    |lateral| in [ (w10/90)*45 - 0.75 , ((w10+1)/90)*45 - 0.75 )   m/s

`w10 == 1` is the floor (K2 makes slip non-zero at rest), so "audible scrub" starts at 2.

Usage:
    python tools/audio/skid_scrub_levels.py logs/audio-trace-*.log

Retail reference traces are the probe sessions (gzipped); pass them the same way. Frames where
the revert counter is ramping inflate `w10` by up to 45, so the high tail is not purely slip --
the bands below 4 are the ones to compare.
"""

import gzip
import re
import sys
from collections import Counter

K1, K2 = 45.0, -0.75

# Ours: `UP <handle> Class_wheels_skid <handle> | w0 w1 ...`
OURS = re.compile(r"UP \S+ Class_wheels_skid \S+ \| ((?:[0-9A-F]{8} ){11,})")
# Retail probe: `skate3-audio-update: ... skid ... [w0 w1 ...]`
RETAIL = re.compile(r"\[((?:[0-9A-F]{8} ){10,})")


def lateral_range(w10: int) -> tuple[float, float]:
    lo = max(0.0, (w10 / 90.0) * K1 + K2)
    hi = max(0.0, ((w10 + 1) / 90.0) * K1 + K2)
    return lo, hi


def read(path: str) -> Counter:
    opener = gzip.open if path.endswith(".gz") else open
    counts: Counter = Counter()
    with opener(path, "rt", errors="replace") as handle:
        for line in handle:
            if "skid" not in line:
                continue
            retail = "skate3-audio-update" in line
            match = (RETAIL if retail else OURS).search(line)
            if not match:
                continue
            words = [int(w, 16) for w in match.group(1).split()]
            if len(words) > 10:
                counts[words[10]] += 1
    return counts


def report(path: str, counts: Counter) -> None:
    total = sum(counts.values())
    if not total:
        print(f"{path}: no skid update words found")
        return
    over = lambda n: 100.0 * sum(c for w, c in counts.items() if w >= n) / total
    print(f"\n{path}  ({total} held frames)")
    print("  w10  lateral m/s      frames   share")
    for w10 in sorted(counts):
        if w10 > 6 and counts[w10] < total * 0.001:
            continue
        lo, hi = lateral_range(w10)
        print(f"  {w10:>3}  {lo:4.2f}-{hi:4.2f}   {counts[w10]:>8}  {100.0*counts[w10]/total:5.2f}%")
    print(f"  scrub over 0.25 m/s (w10>=2): {over(2):5.2f}%")
    print(f"  scrub over 0.75 m/s (w10>=3): {over(3):5.2f}%")


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    for path in sys.argv[1:]:
        report(path, read(path))
    print(
        "\nretail reference (probe sessions play1/play2/play4):\n"
        "  over 0.25 m/s: 16.4% / ~10% / ~6%\n"
        "  over 0.75 m/s:  8.7% /   4% /   2%\n"
        "\nCOMPARE LIKE WITH LIKE. Those retail traces are busy play sessions. Two runs of this\n"
        "engine on 2026-09-21 gave 1.5%/0.4% (casual rolling) and 25.1%/12.0% (deliberate\n"
        "scrubbing) -- a 30x spread from playstyle alone, wider than any plausible physics\n"
        "deviation. One session proves nothing: match what the rider was doing, or condition the\n"
        "distribution on speed and turn input."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
