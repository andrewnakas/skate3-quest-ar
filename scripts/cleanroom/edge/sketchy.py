"""After a sketchy landing (an ollie 180 off flat): when a flick pops again, and when a push starts."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from airflick import *
BASE = "0 25 0 0 0 -1 0\n26 26 0 0 0 0 0\n27 27 0 0 0 1 0\n31 68 -1 0 0 0 0\n"
def land_of(rows, port_):
    air = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir") if port_ else r["mode"] == "Air")]
    return air[-1] + 1
if __name__ == "__main__":
    pr0, sk0 = pair(BASE, 200, extra=("--host-map",))
    Lp, Ls = land_of(pr0, True), land_of(sk0, False)
    print("lands P", Lp, pr0[Lp]["motion_state"][-20:], " S", Ls, [r["events"] for r in sk0 if "EV2" in r["events"]])
    for k in (12, 20, 30, 45, 60, 75, 90):
        t = Lp + k - 6
        act = BASE + f"{t} {t+5} 0 0 0 -1 0\n{t+6} {t+6} 0 0 0 0 0\n{t+7} {t+7} 0 0 0.707 0.707 0\n"
        pr, sk = pair(act, t + 60, extra=("--host-map",))
        print(f"flick at land+{k}: P {names(pr, True)[1:]}  S {names(sk, False)[1:]}")
    for k in (3, 6, 12):
        t = Lp + k
        act = BASE + f"{t} {t+3} 0 0 0 0 0x1000\n"
        pr, sk = pair(act, t + 60, extra=("--host-map",))
        ps = next((i for i in range(t, t + 60) if "Push" in pr[i]["motion_state"]), None)
        ss = next((i for i in range(t, t + 60) if "EV7" in sk[i]["events"]), None)
        print(f"push at land+{k}: P push state from {ps - Lp if ps else None}  S push event at {ss - Lp if ss else None} (from landing)")
