"""Brake (B) held N ticks from steady riding, and the same N ticks after an ollie landing:
speed lost at the end of the hold and 20 ticks after, port vs sk8."""
import math
from edge import pair
def sp(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
OLLIE = "0 10 0 0 0 -1 0\n11 11 0 0 0 0 0\n12 12 0 0 0 1 0\n"
if __name__ == "__main__":
    for lead, name in (("", "steady"), (OLLIE, "after ollie")):
        for start in ((10,) if not lead else (70, 85, 100, 120)):
            for n in (10, 20, 30, 60):
                pr, sk = pair(lead + f"{start} {start + n - 1} 0 0 0 0 0x2000\n", start + n + 22, extra=("--host-map",))
                e = start + n
                print(f"{name:11s} @{start:3d} x{n:2d}: port {sp(pr[start]):.2f} -> {sp(pr[e]):.2f} -> {sp(pr[e + 20]):.2f}"
                      f"   sk8 {float(sk[start]['speed']):.2f} -> {float(sk[e]['speed']):.2f} -> {float(sk[e + 20]['speed']):.2f}")
