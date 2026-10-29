"""Landing a spin off the line: speed after landing by how far the board is off the path."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
def ollie(hold, after=""): return f"0 {hold} 0 0 0 -1 0\n{hold+1} {hold+7} 0 0 0 1 0\n{after}"
def land_i(rows, port_):
    air = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir") if port_ else r["mode"] == "Air")]
    return air[-1] + 1 if air else None
def off(r, port_):
    y = yawp(r) if port_ else math.degrees(float(r["yaw"]))
    d = (y - vyaw(r) + 180) % 360 - 180
    return d
if __name__ == "__main__":
    mags = [float(a) for a in sys.argv[1:]] or [0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]
    for m in mags:
        pr, sk = pair(ollie(30, f"37 100 {-m} 0 0 0 0\n"), 160)
        out = []
        for rows, p in ((pr, True), (sk, False)):
            L = land_i(rows, p)
            if L is None: out.append("no land"); continue
            sp = [hs(rows[min(L + k, len(rows) - 1)]) for k in (-1, 0, 5, 10, 20, 40, 80)]
            out.append(f"land@{L} off{off(rows[L], p):6.1f} " + " ".join(f"{v:4.2f}" for v in sp) + f" {rows[L+3].get('motion_state', rows[L+3].get('mode'))[:22]}")
        print(f"m{m:4.2f} P {out[0]}\n      S {out[1]}")
