"""Kickflip (or heelflip) flicked, then the right stick put somewhere D ticks later and held:
which positions/delays make the port's underflip, against sk8. RT held too or not.
Usage: kickunder.py [kick|heel] [rt]"""
import sys
from edge import pair
from fuzz import tricks
D = 0.707
kind = sys.argv[1] if len(sys.argv) > 1 else "kick"
rt = "rt" in sys.argv
F = (D, D) if kind == "kick" else (-D, D)
POS = {"down": (0, -1), "dleft": (-D, -D), "dright": (D, -D), "uleft": (-D, D), "uright": (D, D), "left": (-1, 0), "right": (1, 0), "up": (0, 1)}
def run(pos, d):
    s = "0 10 0 0 0 -1 0\n11 11 0 0 0 0 0\n" + f"12 12 0 0 {F[0]} {F[1]} 0\n"
    x, y = POS[pos]
    s += f"{12 + d} {12 + d + 24} 0 0 {x} {y} 0" + (" 0 1\n" if rt else "\n")
    pr, sk = pair(s, 90, extra=("--host-map",))
    a = [t for t in tricks(pr, True) if t != "OLLIE"]; b = tricks(sk, False)
    return a, b
if __name__ == "__main__":
    for pos in (sys.argv[3:] or POS):
        for d in (2, 5, 7, 9, 12, 16):
            a, b = run(pos, d)
            pu = any("UNDER" in x for x in a); su = any("Under" in x for x in b)
            print(f"{pos:6s} d={d:2d} port {a} sk8 {b} {'' if pu == su else '<-- differ'}")
