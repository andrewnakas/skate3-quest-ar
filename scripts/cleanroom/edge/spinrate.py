"""In-air spin rate (deg per tick, 3-tick means) by stick, port vs sk8."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spinland import *
def rates(ys):
    d = [((b - a + 180) % 360 - 180) for a, b in zip(ys, ys[1:])]
    return d
if __name__ == "__main__":
    for m in [float(a) for a in sys.argv[1:]] or [0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95, 1.0]:
        pr, sk = pair(ollie(30, f"37 100 {-m} 0 0 0 0\n"), 92)
        pa = rates([yawp(r) for r in pr]); sa = rates([math.degrees(float(r["yaw"])) for r in sk])
        to = next(i for i, r in enumerate(pr) if r["phys_state"] in ("KnownAir", "PhysicsAir"))
        print(f"m{m:4.2f} air@{to} P " + " ".join(f"{abs(sum(pa[i:i+3])/3):4.1f}" for i in range(36, 88, 3)))
        print(f"           S " + " ".join(f"{abs(sum(sa[i:i+3])/3):4.1f}" for i in range(36, 88, 3)))
