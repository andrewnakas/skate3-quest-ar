"""Straight roll-ups (no tricks) at several speeds: entry speed, peak height, speed back out on the flat."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qpturn import *
v3 = lambda r: math.sqrt(sum(float(r[k]) ** 2 for k in ("deck_vx", "deck_vy", "deck_vz")))
if __name__ == "__main__":
    for brake in [int(a) for a in sys.argv[1:]] or (10, 20, 30, 40, 45, 50, 55, 60, 65):
        pr, sk = run("", 260, pre=f"{T0-75} {T0-75+brake} 0 0 0 0 0x2000\n")
        out = []
        for rows in (pr, sk):
            ys = [float(r["deck_py"]) for r in rows]
            out.append(f"in {v3(rows[0]):4.2f} peak {max(ys) - ys[0]:4.2f} out {v3(rows[-1]):4.2f}")
        print(f"brake {brake:2d}  P {out[0]}  | S {out[1]}")
