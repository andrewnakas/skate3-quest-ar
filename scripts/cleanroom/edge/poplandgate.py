"""When after a landing does a flick pop again? The first popping flick, in ticks after the first tick
back on the ground, for flick shapes with and without the stick passing the centre after the crouch."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import popland as P
from fuzz import pair
SHAPES = {"ollie": ["0 1"], "ollie+centre": ["0 0", "0 1"], "shove": ["0.707 -0.707", "1 0", "0.707 0.707", "0 1"],
          "kick+centre": ["0 0", "-0.707 0.707"]}
def first_pop(land, port_, fl, crouch=5):
    for e in range(6, 16):  # the last flick input e ticks after landing
        t0 = land + e - len(fl) - crouch + 1
        s = P.FIRST + f"{t0} {t0+crouch-1} 0 0 0 -1 0\n" + "".join(f"{t0+crouch+i} {t0+crouch+i} 0 0 {x} 0\n" for i, x in enumerate(fl))
        a, b = pair(s, land + e + 50, extra=("--host-map",))
        if len(P.takeoffs(a if port_ else b, port_)) > 1:
            return e
if __name__ == "__main__":
    pr, sk = pair(P.FIRST, 120, extra=("--host-map",))
    lp, ls = P.lands(pr, True)[0] + 1, P.lands(sk, False)[0] + 1
    for k, fl in SHAPES.items():
        print(f"{k:13s} port {first_pop(lp, True, fl)}  sk8 {first_pop(ls, False, fl)}")
