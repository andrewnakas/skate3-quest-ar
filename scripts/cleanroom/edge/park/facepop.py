"""Ollies up the quarter pipe's face: the up speed and speed into the wall just before the pop and
a few ticks after, and the surface's up at the pop - port vs sk8 (to fit what a ramp-face pop adds)."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def up(r):
    qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    return 1 - 2 * (qx * qx + qz * qz)
def first_air(rows, port_):
    return next((i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if port_ else r["mode"] == "Air")), None)
which = sys.argv[1] if len(sys.argv) > 1 else "fast"
fn = run if which == "fast" else run2
for t in range(6, 40, 3):
    pr, sk = fn(f"{t-8} {t} 0 0 0 -1 0\n{t+1} {t+7} 0 0 0 1 0", 130)
    out = []
    for rows, p in ((pr, True), (sk, False)):
        a = first_air(rows, p)
        if a is None: out.append("no air"); continue
        vals = " ".join(f"{float(rows[a+k]['deck_vy']):5.2f}/{sp(rows[a+k]):4.2f}" for k in (-3, 0, 3, 6, 10))
        peak = max(float(r["deck_py"]) for r in rows[a:a+90])
        out.append(f"@{a:3d} up{up(rows[a-3]):5.2f} vy/v {vals} peak {peak:4.2f}")
    print(f"t{t:2d} P {out[0]}\n    S {out[1]}")
