"""Turn rate (deg/s, 5-tick means) with the left stick hard over, braked to each speed first."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
def rates(ys):
    return [((ys[i+5] - ys[i] + 540) % 360 - 180) * 12 for i in range(0, len(ys) - 5, 5)]
if __name__ == "__main__":
    mag = float(sys.argv[1]) if len(sys.argv) > 1 else -1
    for n in [0, 40, 70, 90, 110, 124, 134]:
        t = n + 15
        act = (f"0 {n} 0 0 0 0 0x2000\n" if n else "") + f"{t} {t+70} {mag} 0 0 0 0\n"
        pr, sk = pair(act, t + 75, extra=("--host-map",))
        py = [yawp(r) for r in pr[t:t+70]]; sy = [math.degrees(float(r["yaw"])) for r in sk[t:t+70]]
        print(f"v{hs(pr[t]):4.1f} P " + " ".join(f"{abs(x):4.0f}" for x in rates(py)))
        print(f"v{hs(sk[t]):4.1f} S " + " ".join(f"{abs(x):4.0f}" for x in rates(sy)))
