"""After a landing the port's brake doesn't bite for a while: B held from just after touchdown,
the tick its decel starts (from the landing) - clean ollie, clean 180, sketchy 180; port vs sk8."""
import math
from switchflick import LEAD
from fuzz import pair
from edge import pair as epair
def sp(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
def land(rows, port):
    air = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if port else r["mode"] == "Air")]
    return air[-1] + 1
def bite(rows, port, a):
    v = [sp(r) if port else float(r["speed"]) for r in rows]
    return next((i - a for i in range(a + 1, len(v) - 1) if v[i] - v[i + 1] > 0.02), None)
OLLIE = "0 10 0 0 0 -1 0\n11 11 0 0 0 0 0\n12 12 0 0 0 1 0\n"
if __name__ == "__main__":
    cases = [("clean ollie", OLLIE, epair), ("ollie 180 m.7", LEAD.replace("-0.8", "-0.7"), pair),
             ("ollie 180 m.75", LEAD.replace("-0.8", "-0.75"), pair), ("sketchy 180 m.8", LEAD, pair)]
    for name, lead, pf in cases:
        pr0, sk0 = pf(lead, 160, extra=("--host-map",))
        lp, ls = land(pr0, True), land(sk0, False)
        for d in (2, 20, 40):
            pr, sk = pf(lead + f"{lp + d} {lp + 160} 0 0 0 0 0x2000\n", lp + 120, extra=("--host-map",))
            print(f"{name:16s} land {lp}/{ls} {pr0[lp]['anim_clip']:14s} brake from +{d:2d}: port bites at +{bite(pr, True, lp)}  sk8 at +{bite(sk, False, ls)}")
