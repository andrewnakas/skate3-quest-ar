"""After a spin lands off the line (port): board yaw vs path, speed, per tick from touchdown."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spinland import *
def fold(d):
    d = (d + 180) % 360 - 180
    if d > 90: d -= 180
    if d < -90: d += 180
    return d
def trace(rows, L, port_, n=30):
    out = []
    ref = None
    for k in range(-2, n):
        r = rows[L + k]
        y = yawp(r) if port_ else math.degrees(float(r["yaw"]))
        out.append((k, y, vyaw(r), fold(y - vyaw(r)), hs(r)))
    return out
if __name__ == "__main__":
    for m in [float(a) for a in sys.argv[1:]] or [0.5, 0.55, 0.6, 0.65, 0.75, 0.8, 0.85, 0.9, 0.95, 1.0]:
        pr, sk = pair(ollie(30, f"37 100 {-m} 0 0 0 0\n"), 140)
        L = land_i(pr, True)
        t = trace(pr, L, True)
        print(f"m{m} land {L} v0 {hs(pr[0]):.2f}")
        print("  off  " + " ".join(f"{x[3]:5.0f}" for x in t[::2]))
        print("  by   " + " ".join(f"{x[1]-t[2][1]:5.0f}" for x in t[::2]))
        print("  vd   " + " ".join(f"{x[2]-t[2][2]:5.0f}" for x in t[::2]))
        print("  v    " + " ".join(f"{x[4]:5.2f}" for x in t[::2]))
def both(m):
    pr, sk = pair(ollie(30, f"37 100 {-m} 0 0 0 0\n"), 140)
    for rows, p in ((pr, True), (sk, False)):
        L = land_i(rows, p); t = trace(rows, L, p)
        print(f"m{m} {'P' if p else 'S'} off " + " ".join(f"{x[3]:4.0f}" for x in t[::2]))
        print(f"        by  " + " ".join(f"{x[1]-t[2][1]:4.0f}" for x in t[::2]))
        print(f"        v   " + " ".join(f"{x[4]:4.1f}" for x in t[::2]))
