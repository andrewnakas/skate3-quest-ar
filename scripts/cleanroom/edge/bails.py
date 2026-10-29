"""Which landings bail: flips (quick, held for more turns) and spins out of pops of every height,
the port vs sk8. Prints LAND / BAIL per engine, and the flips/turns done."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
def flick(points, start):
    return "".join(f"{start+i} {start+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(points))
K = (-0.707, 0.707)
def trick(kind, crouch, hold):
    if kind == "kick": pts = [(0, -1)] * crouch + [(0, 0), K, K, K] + [K] * hold
    elif kind == "heel": pts = [(0, -1)] * crouch + [(0, 0), (0.707, 0.707), (0.707, 0.707), (0.707, 0.707)] + [(0.707, 0.707)] * hold
    elif kind == "tre": pts = [(-1, 0)] * crouch + [(-0.707, -0.707), (0, -1), (0.707, -0.707), (1, 0), (0.707, 0.707)] + [(0.707, 0.707)] * hold
    return flick(pts, 0)
def bailed(rows, port_):
    for i, r in enumerate(rows):
        if (r.get("wiping_out") == "1") if port_ else r["mode"] == "Bail": return i
    return None
if __name__ == "__main__":
    for kind in sys.argv[1:] or ["kick", "heel", "tre"]:
        for crouch in (2, 6, 12, 30):
            for hold in (0, 10, 25, 45, 80):
                pr, sk = pair(trick(kind, crouch, hold), crouch + hold + 120)
                bp, bs = bailed(pr, True), bailed(sk, False)
                tp = [r["trick_name"] for r in pr if r["trick_name"]]
                ts = [e[4:] for r in sk for e in r["events"].split("|") if e.startswith("EV1:")]
                mark = "" if (bp is None) == (bs is None) else "  <-- DIFF"
                print(f"{kind:4s} crouch {crouch:2d} hold {hold:2d}  port {'BAIL@'+str(bp) if bp is not None else 'land':8s} {tp[-1:] }  sk8 {'BAIL@'+str(bs) if bs is not None else 'land':8s} {ts[-1:]}{mark}")
