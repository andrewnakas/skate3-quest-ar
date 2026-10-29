"""A flip with RB held on through it, onto the rail: the port dark-catches into a darkslide."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
S = feel.ON_RAIL
def tl(rows, port_):
    out, prev = [], None
    for i, r in enumerate(rows):
        s_ = r["phys_state"] if port_ else f"{r['mode']}:{r.get('grind_kind','')}"
        if s_ != prev: out.append(f"{i}:{s_[:16]}"); prev = s_
        if port_ and r["trick_name"] and (not out or r["trick_name"] not in out[-1]): out.append(f"{i}:T:{r['trick_name']}")
        if not port_ and r["events"]: out.append(f"{i}:{r['events'][:40]}")
    return " ".join(out)
if __name__ == "__main__":
    for nm, flick in (("kick", "-0.707 0.707"), ("heel", "0.707 0.707")):
        for rb in ((12, 80), (16, 60), (20, 45)):
            act = f"0 8 0 0 0 -1 0\n9 9 0 0 0 0 0\n10 13 0 0 {flick} 0\n{rb[0]} {rb[1]} 0 0 0 0 0x200\n"
            pr, sk = pair(act, 160, S, extra=("--host-map",))
            print(f"== {nm} RB {rb}\n  P {tl(pr, True)}\n  S {tl(sk, False)}")
