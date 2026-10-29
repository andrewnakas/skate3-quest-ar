"""A held from various speeds (braked down first): where pushing stops adding speed, port vs sk8."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
if __name__ == "__main__":
    for brake in (0, 10, 20, 30, 45, 60, 90):
        act = (f"0 {brake} 0 0 0 0 0x2000\n" if brake else "") + f"{brake+1} {brake+400} 0 0 0 0 0x1000\n"
        pr, sk = pair(act, brake + 400, extra=("--host-map",))
        b = brake + 1
        out = []
        for rows in (pr, sk):
            vs = [hs(rows[min(b + k, len(rows) - 1)]) for k in (0, 60, 120, 240, 399)]
            out.append(" ".join(f"{v:5.2f}" for v in vs))
        print(f"brake {brake:3d}  P {out[0]}  | S {out[1]}")
