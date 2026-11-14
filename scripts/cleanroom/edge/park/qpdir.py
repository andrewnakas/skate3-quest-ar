"""Quarter pipe: does it come back down forward or fakie? The port's board way from its trucks
(front - back), sk8's from its deck, each against its travel and relative to the start; steer pressed on the face (see qpcarve)."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def pway(r):
    f = [float(r[f"truck_f_p{a}"]) - float(r[f"truck_b_p{a}"]) for a in "xyz"]
    v = [float(r[f"deck_v{a}"]) for a in "xyz"]
    return "fwd" if sum(a * b for a, b in zip(f, v)) >= 0 else "FAK"
def sway(r):
    qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    f = (2 * (qx * qz + qw * qy), 2 * (qy * qz - qw * qx), 1 - 2 * (qx * qx + qy * qy))  # local +z
    v = [float(r[f"deck_v{a}"]) for a in "xyz"]
    return "fwd" if sum(a * b for a, b in zip(f, v)) >= 0 else "FAK"
def rel(way, start): return way if start == "fwd" else {"fwd": "FAK", "FAK": "fwd"}[way]
def out(rows):
    ys = [float(r["deck_py"]) for r in rows]; ip = ys.index(max(ys))
    j = next((i for i in range(ip, len(rows)) if ys[i] < ys[0] + 0.03), len(rows) - 1)
    return min(j + 3, len(rows) - 1)
if __name__ == "__main__":
    hold = int(os.environ.get("HOLD", 40))
    for brake in (int(a) for a in (sys.argv[1:] or ("0", "25", "50"))):
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else ""
        base, _ = run("", 120, pre=pre)
        t = next(i for i, r in enumerate(base) if float(r["deck_py"]) > 0.15)
        res = []
        for s in (-1, -0.6, -0.3, 0, 0.3, 0.6, 1):
            pr, sk = run(f"{t} {t+hold} {s} 0 0 0 0" if s else "", 200, pre=pre)
            a, b = out(pr), out(sk)
            res.append(f"{s:+.1f} P {rel(pway(pr[a]), pway(pr[0]))} {sp(pr[a]):4.1f} S {rel(sway(sk[b]), sway(sk[0]))} {sp(sk[b]):4.1f}")
        print(f"brake {brake:2d}:  " + " | ".join(res))
