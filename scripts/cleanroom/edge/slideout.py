"""Off the end of a slide (board / lip) with the stick held to different ticks: ride away or bail."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rails import *
if __name__ == "__main__":
    for nm, (x, y) in {"board(1,0)": (1, 0), "lip(-1,0)": (-1, 0), "crook": (0.8, 0.8)}.items():
        for until in (80, 95, 100, 106, 140):
            act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n30 {until} 0 0 {x} {y} 0"
            pr, sk = pair(act, 170, S)
            pend = [r["phys_state"] for r in pr[100:170]]
            pbail = any("Wipe" in s_ or "Biped" in s_ for s_ in pend)
            sbail = any(r["mode"] in ("Bail", "Walk") for r in sk[100:170])
            print(f"{nm:10s} held to {until}: port {'OFF/BAIL' if pbail else 'rides'}  sk8 {'OFF/BAIL' if sbail else 'rides'}")
