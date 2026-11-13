"""Frontside shove-it flicked (swept down-left, left, up-left, up), then the stick pulled down D
ticks after the flick's end: port's FS shove-it underflip window, against sk8. Optionally the stick
held up after the flick for H ticks first. Usage: fsunder.py [H]"""
import sys
from edge import pair
from fuzz import tricks
D = 0.707
H = int(sys.argv[1]) if len(sys.argv) > 1 else 0
if __name__ == "__main__":
    for path in ("sweep", "L-c"):
        for d in (2, 4, 6, 8, 10, 12, 14, 18, 25):
            if d <= H: continue
            s = "0 10 0 0 0 -1 0\n" + ("11 11 0 0 -0.707 -0.707 0\n12 12 0 0 -1 0 0\n13 13 0 0 -0.707 0.707 0\n14 14 0 0 0 1 0\n" if path == "sweep"
                                       else "11 11 0 0 -0.707 -0.707 0\n12 12 0 0 -1 0 0\n13 13 0 0 0 0 0\n14 14 0 0 0 0 0\n")
            if H: s += f"15 {14 + H} 0 0 0 1 0\n"
            s += f"{14 + d} {14 + d + 20} 0 0 0 -1 0\n"
            pr, sk = pair(s, 100, extra=("--host-map",))
            a = [t for t in tricks(pr, True) if t != "OLLIE"]; b = tricks(sk, False)
            pu = any("UNDER" in x for x in a); su = any("Under" in x for x in b)
            print(f"{path:5s} H={H} d={d:2d} port {a} sk8 {b} {'' if pu == su else '<-- differ'}")
