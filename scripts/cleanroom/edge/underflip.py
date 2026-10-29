"""How short of done a flip can land: held flips out of pops of every height - the board's tilt
from level at touchdown and whether it bailed, the port vs sk8."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bails import *
def upy(r):
    qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    return 1 - 2 * (qx * qx + qz * qz)
def touchdown(rows, port_):
    air = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir") if port_ else r["mode"] == "Air")]
    return air[-1] + 1 if air else None
if __name__ == "__main__":
    for kind in ("kick",):
        for hold in (0, 25):
            for crouch in range(1, 13):
                pr, sk = pair(trick(kind, crouch, hold), crouch + hold + 120)
                out = []
                for rows, p in ((pr, True), (sk, False)):
                    L = touchdown(rows, p); b = bailed(rows, p)
                    out.append(f"land@{L} tilt{math.degrees(math.acos(max(-1, min(1, upy(rows[L-1]))))):5.0f} {'BAIL' if b is not None else 'ok  '}")
                print(f"hold {hold:2d} crouch {crouch:2d}  P {out[0]}   S {out[1]}")
