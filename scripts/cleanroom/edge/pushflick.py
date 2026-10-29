"""An ollie flick at each moment of a push stroke (tapped / held): what each game pops."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
def names(rows, port_):
    if port_: return sorted(set(r["trick_name"] for r in rows) - {""})
    return sorted(set(e[4:] for r in rows for e in r["events"].split("|") if e[:4] == "EV0:"))
if __name__ == "__main__":
    for push in ("0 3 0 0 0 0 0x1000", "0 80 0 0 0 0 0x1000"):
        for at in range(6, 90, 6):
            act = f"{push}\n{at-4} {at} 0 0 0 -1 0\n{at+1} {at+1} 0 0 0 0 0\n{at+2} {at+3} 0 0 0 1 0\n"
            pr, sk = pair(act, at + 60, extra=("--host-map",))
            ps = pr[at]["motion_state"].split("/")[-3:]
            print(f"{'tap ' if push.startswith('0 3') else 'held'} flick@{at:2d} port {names(pr, True)} ({'/'.join(ps)[:30]})  sk8 {names(sk, False)}")
