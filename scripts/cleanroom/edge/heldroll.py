"""Held kickflips (double) by pop height: the board's accumulated roll about its length, every 3 ticks,
from the pop to touchdown. Port vs sk8."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from underflip import *
def roll_series(rows):
    out, acc, prev = [], 0.0, None
    for r in rows:
        qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
        # board up vector and its forward (nose, +Z local)
        ux, uy, uz = 2*(qx*qy - qw*qz), 1 - 2*(qx*qx + qz*qz), 2*(qy*qz + qw*qx)
        fx, fy, fz = 2*(qx*qz + qw*qy), 2*(qy*qz - qw*qx), 1 - 2*(qx*qx + qy*qy)
        # roll = angle of up around forward, measured against world up projected
        wx, wy, wz = -fy*fx, 1 - fy*fy, -fy*fz  # world up minus its forward part
        n = math.sqrt(wx*wx + wy*wy + wz*wz) or 1
        wx, wy, wz = wx/n, wy/n, wz/n
        c = ux*wx + uy*wy + uz*wz
        s = (fy*uz - fz*uy)*wx + (fz*ux - fx*uz)*wy + (fx*uy - fy*ux)*wz
        a = math.degrees(math.atan2(s, c))
        if prev is not None: acc += (a - prev + 180) % 360 - 180
        prev = a; out.append(acc)
    return out
if __name__ == "__main__":
    for crouch in [int(a) for a in sys.argv[1:]] or (1, 4, 6, 9, 12):
        pr, sk = pair(trick("kick", crouch, 25), crouch + 25 + 120)
        for name, rows, p in (("P", pr, True), ("S", sk, False)):
            L = touchdown(rows, p); rs = roll_series(rows)
            a0 = next(i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if p else r["mode"] == "Air"))
            print(f"c{crouch:2d} {name} pop@{a0} land@{L}: " + " ".join(f"{abs(rs[i]):4.0f}" for i in range(a0, L + 1, 3)))
