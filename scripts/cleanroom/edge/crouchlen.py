"""Crouch-arc tricks (the crouch held off to a side, then swept round): by how long the crouch was
held, the port vs sk8."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bails import *
from flicksweep import names_port, names_sk8, rim
def arc(start, seq, crouch):
    pts = [rim(start)] * crouch + [rim(a) for a in seq] + [rim(seq[-1])] * 3
    return "".join(f"{i} {i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(pts))
cases = {"tre (L, round under to UR)": (270, [225, 180, 135, 90, 45]),
         "360 shove (DL round right)": (225, [180, 135, 90]),
         "FS shove (DR round left)": (135, [180, 225, 270]),
         "varial kick (L to D then U)": (270, [225, 180, 0]),
         "R crouch round to L": (90, [135, 180, 225, 270])}
if __name__ == "__main__":
    for name, (st, seq) in cases.items():
        row = []
        for crouch in (1, 2, 4, 8, 12):
            pr, sk = pair(arc(st, seq, crouch), crouch + 100)
            a, b = names_port(pr), names_sk8(sk)
            row.append(f"c{crouch}:{(a or ['-'])[-1][:9]}/{(b or ['-'])[-1][:9]}")
        print(f"{name:28s}", " ".join(row))
