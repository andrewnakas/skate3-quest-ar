from edge import *
import sys
# brake for n ticks, coast, one push tap, measure the gain; then a second tap
for n in [0, 30, 60, 80, 100, 110, 118, 124]:
    t = n + 20
    act = (f"0 {n} 0 0 0 0 0x2000\n" if n else "") + f"{t} {t+3} 0 0 0 0 0x1000\n{t+70} {t+73} 0 0 0 0 0x1000\n"
    pr, sk = pair(act, t + 150)
    def g(rows):
        v0 = hs(rows[t]); v1 = hs(rows[t + 65]); v2 = hs(rows[t + 140])
        # first tick the speed rises
        c = next((i - t for i in range(t, t + 65) if hs(rows[i]) > v0 + 0.05), None)
        return f"v0 {v0:5.2f} +{v1-v0:5.2f} (at {c}) then +{v2-v1:5.2f}"
    print(f"brake {n:3d}: port {g(pr)} | sk8 {g(sk)}")
