"""The port's flick map in switch (after a push out of fakie shuffles the feet round) against the
same flicks ridden fakie (no push): which tricks each stick path gives in each stance.
Usage: switchflick.py [port|both]"""
import sys
from fuzz import pair, tricks
D = 0.707
LEAD = "0 25 0 0 0 -1 0\n26 26 0 0 0 0 0\n27 27 0 0 0 1 0\n31 68 -0.8 0 0 0 0\n"
PUSH = "88 100 0 0 0 0 0x1000\n"
PATHS = {
    "up": [(0, 0), (0, 1)], "upright": [(0, 0), (D, D)], "upleft": [(0, 0), (-D, D)],
    "sweepR": [(D, -D), (1, 0), (D, D), (0, 1)], "sweepL": [(-D, -D), (-1, 0), (-D, D), (0, 1)],
    "R": [(D, -D), (1, 0)], "L": [(-D, -D), (-1, 0)], "R-cen": [(D, -D), (1, 0), (0, 0)], "L-cen": [(-D, -D), (-1, 0), (0, 0)],
    "R-ur": [(1, 0), (D, D)], "L-ul": [(-1, 0), (-D, D)],
    "tre": [(-1, 0), (0, -1), (1, 0)], "laser": [(1, 0), (0, -1), (-D, D)],
    "varK": [(D, -D), (0, 0), (0, 1)], "varH": [(-D, -D), (0, 0), (0, 1)],
    "hard": [(-D, -D), (0, 0), (D, D)], "inward": [(D, -D), (0, 0), (-D, D)],
}
def run(path, push, t=136):
    s = LEAD + (PUSH if push else "") + f"{t - 5} {t - 1} 0 0 0 -1 0\n"
    for i, (x, y) in enumerate(path): s += f"{t + i} {t + i} 0 0 {x} {y} 0\n"
    pr, sk = pair(s, t + 60, extra=("--host-map",))
    last = lambda l: [x for x in l if x.upper() not in ("OLLIE", "OLLIE 180")][-1:] or l[-1:]
    return last(tricks(pr[t - 5:], True)), last(tricks(sk[t - 5:], False))
if __name__ == "__main__":
    for k, p in PATHS.items():
        (pf, sf), (ps, ss) = run(p, False), run(p, True)
        print(f"{k:8s} fakie port {pf} sk8 {sf}   switch port {ps} sk8 {ss}")
