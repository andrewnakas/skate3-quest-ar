"""Air time and pop height by crouch length, the port vs sk8, for each flick (from rolling on the flat)."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import popland as P
from fuzz import pair, FLICKS
if __name__ == "__main__":
    kinds = sys.argv[1:] or ["ollie", "kick", "fsshove", "shove"]
    for kind in kinds:
        for c in (1, 4, 10, 18, 25, 40):
            s = f"0 {c-1} 0 0 0 -1 0\n" + "".join(f"{c+i} {c+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(FLICKS[kind]))
            pr, sk = pair(s, c + 90, extra=("--host-map",))
            out = []
            for rows, port_ in ((pr, True), (sk, False)):
                t, l = P.takeoffs(rows, port_), P.lands(rows, port_)
                y0 = float(rows[0]["deck_py"])
                h = max(float(r["deck_py"]) for r in rows) - y0
                out.append(f"off@{t[0] - c if t else '-':>2} air {l[0] - t[0] + 1 if t and l else '-':>3} h {h:4.2f} land@{l[0] + 1 - c if l else '-'}")
            print(f"{kind:8s} crouch {c:2d}  P {out[0]}  S {out[1]}")
