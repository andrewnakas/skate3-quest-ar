"""Steering on the quarter pipe face: the travel's angle in the face's own plane (0 = straight up the
fall line, + = toward -z here), the port vs sk8, the steer pressed at tick ON for HOLD ticks."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def face_angle(r):
    qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    n = (2 * (qx * qy - qw * qz), 1 - 2 * (qx * qx + qz * qz), 2 * (qy * qz + qw * qx))
    v = [float(r[k]) for k in ("deck_vx", "deck_vy", "deck_vz")]
    u = [0 - n[1] * n[0], 1 - n[1] * n[1], 0 - n[1] * n[2]]
    lu = math.sqrt(sum(x * x for x in u))
    if lu < 0.15: return None  # (flat: no fall line)
    u = [x / lu for x in u]
    a = [n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0]]
    return math.degrees(math.atan2(sum(p * q for p, q in zip(v, a)), sum(p * q for p, q in zip(v, u))))
def fmt(a): return "  -  " if a is None else f"{a:5.0f}"
if __name__ == "__main__":
    on, hold = int(os.environ.get("ON", 34)), int(os.environ.get("HOLD", 40))
    pre = f"{T0-55} {T0-30} 0 0 0 0 0x2000\n"
    for st in [float(x) for x in sys.argv[1:]] or (0, 0.3, 0.6, 1, -0.3, -0.6, -1):
        pr, sk = run(f"{on} {on+hold} {st} 0 0 0 0", 110, pre=pre)
        print(f"steer {st:5.2f}\n  P " + " ".join(fmt(face_angle(pr[i])) for i in range(on, on + 60, 4)) +
              "\n  S " + " ".join(fmt(face_angle(sk[i])) for i in range(on, on + 60, 4)))
