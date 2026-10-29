"""Brute-force the port's revert input: an ollie (and a 180), then every button / trigger / stick
held around the landing; prints any trick or motion state naming a revert."""
import sys, os, subprocess, csv, io
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
from edge import _rows
BTN = {"A": 0x1000, "B": 0x2000, "X": 0x4000, "Y": 0x8000, "LB": 0x100, "RB": 0x200, "LS": 0x40, "RS": 0x80}
def port_only(action, ticks, t0=feel.T0):
    p = os.path.join(D, f"r{os.getpid()}.txt"); open(p, "w").write(feel.PREFIX + feel.shift(action, t0))
    return _rows([S3, PACK, "test-world", p, str(t0 + ticks)])[t0:]
def names(rows):
    seen = []
    for r in rows:
        for k in ("trick_name", "motion_state", "action_state"):
            v = r.get(k, "")
            if v and v not in seen and ("evert" in v or k == "trick_name"):
                seen.append(v)
    return seen
if __name__ == "__main__":
    base = {"ollie": "0 12 0 0 0 -1 0\n13 19 0 0 0 1 0\n", "180": "0 30 0 0 0 -1 0\n31 37 0 0 0 1 0\n37 100 -0.7 0 0 0 0\n"}
    land = {"ollie": 52, "180": 88}
    combos = []
    for b, v in BTN.items(): combos.append((b, f"0 0 0 0 {v}"))
    combos += [("LT", "0 0 0 0 0 1 0"), ("RT", "0 0 0 0 0 0 1"), ("LT+RT", "0 0 0 0 0 1 1"), ("LB+RB", f"0 0 0 0 {0x300}")]
    for nm, (lx, ly) in {"Lleft": (-1, 0), "Lright": (1, 0), "Lup": (0, 1), "Ldown": (0, -1)}.items(): combos.append((nm, f"{lx} {ly} 0 0 0"))
    for nm, (rx, ry) in {"Rleft": (-1, 0), "Rright": (1, 0), "Rup": (0, 1), "Rdown": (0, -1)}.items(): combos.append((nm, f"0 0 {rx} {ry} 0"))
    for kind in sys.argv[1:] or ["ollie", "180"]:
        L = land[kind]
        for when in (-12, -4, 0, 4):
            for nm, vals in combos:
                act = base[kind] + f"{L+when} {L+when+25} {vals}\n"
                rows = port_only(act, L + 80)
                n = names(rows)
                rv = [x for x in n if "evert" in x]
                print(f"{kind:5s} at{when:+3d} {nm:7s} {'REVERT ' + str(rv) if rv else ''} {[x for x in n if 'evert' not in x][:4]}")
