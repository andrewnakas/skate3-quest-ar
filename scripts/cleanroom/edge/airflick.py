"""Flicks done in the air, after an ollie / a nollie, at several air times: what each game names, and bails."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
D_ = 0.707
PATHS = {"C>UL": [(0, 0), (-D_, D_)], "C>UR": [(0, 0), (D_, D_)], "C>U": [(0, 0), (0, 1)], "C>D": [(0, 0), (0, -1)],
         "D>C>UL": [(0, -1)] * 3 + [(0, 0), (-D_, D_)], "D>C>UR": [(0, -1)] * 3 + [(0, 0), (D_, D_)],
         "U>C>DL": [(0, 1)] * 3 + [(0, 0), (-D_, -D_)], "U>C>DR": [(0, 1)] * 3 + [(0, 0), (D_, -D_)],
         "D>L>U sweep": [(0, -1), (-D_, -D_), (-1, 0), (-D_, D_), (0, 1)], "D>R>U sweep": [(0, -1), (D_, -D_), (1, 0), (D_, D_), (0, 1)]}
POPS = {"ollie": "0 12 0 0 0 -1 0\n13 13 0 0 0 0 0\n14 19 0 0 0 1 0\n", "nollie": "0 12 0 0 0 1 0\n13 13 0 0 0 0 0\n14 19 0 0 0 -1 0\n"}
def names(rows, port_):
    if port_:
        out = []
        for r in rows:
            if r["trick_name"] and r["trick_name"] not in out: out.append(r["trick_name"])
        return out
    out = []
    for r in rows:
        for e in r["events"].split("|"):
            if e[:4] in ("EV0:", "EV1:") and e[4:] not in out and "Air" not in e: out.append(e[4:])
    return out
def bail(rows, port_):
    return any((r["wiping_out"] == "1") if port_ else r["mode"] == "Bail" for r in rows)
if __name__ == "__main__":
    pops = [a for a in sys.argv[1:] if a in POPS] or list(POPS)
    for pop in pops:
        for at in (24, 32, 42):
            for pn, pts in PATHS.items():
                act = POPS[pop] + "".join(f"{at+i} {at+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(pts))
                pr, sk = pair(act, 120, extra=("--host-map",))
                a, b = names(pr, True), names(sk, False)
                print(f"{pop:6s} @{at} {pn:11s} P {a}{' BAIL' if bail(pr, True) else ''}  |  S {b}{' BAIL' if bail(sk, False) else ''}")
