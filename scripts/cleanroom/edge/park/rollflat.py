"""Straight roll-ups: speed 8 ticks after it's back down on the flat, port vs sk8, and the mean |log| error."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rollback import *
def back(rows):
    ys = [float(r["deck_py"]) for r in rows]; ip = ys.index(max(ys))
    j = next((i for i in range(ip, len(rows)) if ys[i] < ys[0] + 0.03), len(rows) - 1)
    return v3(rows[min(j + 8, len(rows) - 1)])
if __name__ == "__main__":
    e, out = [], []
    for brake in (20, 30, 40, 45, 50, 55, 60, 65):
        pr, sk = run("", 260, pre=f"{T0-75} {T0-75+brake} 0 0 0 0 0x2000\n")
        a, b = back(pr), back(sk); e.append(abs(math.log(b / a))); out.append(f"{a:.1f}/{b:.1f}")
    print(f"err {sum(e)/len(e):.3f}  " + " ".join(out))
