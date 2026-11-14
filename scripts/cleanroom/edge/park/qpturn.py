"""Slow up the quarter pipe at angles: does it roll back fakie or carve round, and how fast out."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def byaw(r, port_):
    if not port_: return math.degrees(float(r["yaw"]))
    qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    return math.degrees(math.atan2(2 * (qw * qy + qx * qz), 1 - 2 * (qy * qy + qx * qx)))
def vdir(r): return math.degrees(math.atan2(float(r["deck_vx"]), float(r["deck_vz"])))
def d(a, b): return (a - b + 180) % 360 - 180
def summary(rows, port_):
    ys = [float(r["deck_py"]) for r in rows]; ip = ys.index(max(ys))
    j = next((i for i in range(ip, len(rows)) if ys[i] < ys[0] + 0.03), len(rows) - 1)
    r0, r1 = rows[0], rows[min(j + 3, len(rows) - 1)]
    turn = d(vdir(r1), vdir(r0))
    rel = d(byaw(r1, port_), vdir(r1))  # board vs travel at exit: ~0 forward, ~180 fakie
    return f"peak {max(ys):4.2f} out@{j:3d} v {sp(r1):4.2f} turn {turn:6.1f} board {'FAKIE' if abs(rel) > 90 else 'fwd  '}({rel:6.1f})"
if __name__ == "__main__":
    brake = int(sys.argv[1]) if len(sys.argv) > 1 else 45
    for s in [float(x) for x in sys.argv[2:]] or (-0.8, -0.6, -0.45, 0, 0.45, 0.6, 0.8):
        # (the brake ends well before the steer: just off it the foot coming back up blocks steering)
        pre = f"{T0-90} {T0-90+brake} 0 0 0 0 0x2000\n" + (f"{T0-20} {T0-8} {s} 0 0 0 0\n" if s else "")
        pr, sk = run("", 240, pre=pre)
        ang = d(vdir(pr[30]), 90)
        print(f"steer {s:5.2f} v0 {sp(pr[0]):4.2f} in{ang:6.1f}  P {summary(pr, True)}\n{'':27s}S {summary(sk, False)}")
