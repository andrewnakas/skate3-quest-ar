"""Handplants on the quarter pipe: RB (and X / triggers) held riding up it, the port vs sk8 (app layout)."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def tl(rows, port_):
    out, prev = [], None
    for i, r in enumerate(rows):
        s_ = r["phys_state"] if port_ else r["mode"]
        if s_ != prev: out.append(f"{i}:{s_[:14]}"); prev = s_
        if port_ and r["trick_name"] and r["trick_name"] not in " ".join(out[-2:]): out.append(f"{i}:T:{r['trick_name']}")
        if not port_ and r["events"]: out.append(f"{i}:{r['events'][:34]}")
    return " ".join(out)
if __name__ == "__main__":
    for brake in (25, 40):
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n"
        for nm, act in (("RB", "0 120 0 0 0 0 0x200"), ("RB+X", "0 120 0 0 0 0 0x4200"), ("RB+LT", "0 120 0 0 0 0 0x200 1 0"), ("RB+RT", "0 120 0 0 0 0 0x200 0 1")):
            pr, sk = run(act, 170, pre=pre, sk_extra=("--host-map",))
            print(f"== brake {brake} {nm}\n  P {tl(pr, True)}\n  S {tl(sk, False)}")
