"""Every control, alone and in the common combinations, held while rolling on flat ground: what the
original does vs what sk8 does (app button layout). Prints speed after, turn, height reached, states."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
B = {"A": 0x1000, "B": 0x2000, "X": 0x4000, "Y": 0x8000, "LB": 0x100, "RB": 0x200, "LS": 0x40, "RS": 0x80, "BACK": 0x20, "START": 0x10}
def line(b=0, lx=0, ly=0, rx=0, ry=0, lt=0, rt=0, t0=0, t1=90):
    return f"{t0} {t1} {lx} {ly} {rx} {ry} {b} {lt} {rt}\n"
CASES = {
    "nothing": line(),
    **{n: line(v) for n, v in B.items()},
    "LT": line(lt=1), "RT": line(rt=1), "LT+RT": line(lt=1, rt=1),
    "RB+steerL": line(B["RB"], lx=-1), "RB+steerR": line(B["RB"], lx=1),
    "B+steerL": line(B["B"], lx=-1), "X+steerL": line(B["X"], lx=-1),
    "LB+steerL": line(B["LB"], lx=-1),
    "lstick back": line(ly=-1), "lstick fwd": line(ly=1),
    "lstick back-left": line(lx=-0.707, ly=-0.707), "lstick back-right": line(lx=0.707, ly=-0.707),
    "rstick down hold": line(ry=-1), "rstick up hold": line(ry=1),
    "rstick down 0.5": line(ry=-0.5), "rstick up 0.5": line(ry=0.5),
    "rstick left hold": line(rx=-1), "rstick right hold": line(rx=1),
    "LS+rstick flick": line(B["LS"], ry=-1, t1=12) + line(B["LS"], ry=1, t0=13, t1=20),
    "A tap x3": line(B["A"], t1=3) + line(B["A"], t0=40, t1=43) + line(B["A"], t0=80, t1=83),
}
def summ(rows, port_):
    v0 = hs(rows[0]); v1 = hs(rows[-1])
    y0 = float(rows[0]["deck_py"]); peak = max(float(r["deck_py"]) for r in rows) - y0
    a = (vyaw(rows[-1]) - vyaw(rows[0]) + 180) % 360 - 180 if v1 > 0.3 else float("nan")
    st = []
    for r in rows:
        s_ = (r["phys_state"] + "/" + r["motion_state"].split("/")[-2] if port_ else r["mode"])
        if not st or st[-1] != s_: st.append(s_)
    names = sorted((set((r["trick_name"] if port_ else "") for r in rows) | set(e[4:] for r in rows if not port_ for e in r["events"].split("|") if e[:4] in ("EV1:", "EV0:"))) - {""})
    return f"v {v0:4.1f}->{v1:4.1f} turn {a:6.0f} up {peak:4.2f} {names} {' > '.join(st[:5])[:70]}"
if __name__ == "__main__":
    only = sys.argv[1:]
    for n, act in CASES.items():
        if only and n not in only: continue
        pr, sk = pair(act, 130, extra=("--host-map",))
        print(f"{n:18s} P {summ(pr, True)}\n{'':18s} S {summ(sk, False)}")
