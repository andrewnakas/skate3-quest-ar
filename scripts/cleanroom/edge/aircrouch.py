"""A crouch begun in the air before landing (or just after), then a flick on the ground: does it pop?"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from airflick import *
if __name__ == "__main__":
    base = "0 12 0 0 0 -1 0\n13 13 0 0 0 0 0\n14 19 0 0 0 1 0\n"
    pr0, _ = pair(base, 100, extra=("--host-map",))
    land = next(i for i in range(25, 100) if pr0[i]["phys_state"].startswith("Physics") and pr0[i]["phys_state"] != "PhysicsAir")
    print("port lands at", land)
    for start in range(land - 10, land + 12, 2):
        for hold in (10, 20):
            end = start + hold
            act = base + f"{start} {end} 0 0 0 -1 0\n{end+1} {end+1} 0 0 0 0 0\n{end+2} {end+2} 0 0 0.707 0.707 0\n"
            pr, sk = pair(act, end + 60, extra=("--host-map",))
            print(f"crouch {start-land:+3d}..{end-land:+3d} from landing: P {names(pr, True)[1:]}  S {names(sk, False)[1:]}")
