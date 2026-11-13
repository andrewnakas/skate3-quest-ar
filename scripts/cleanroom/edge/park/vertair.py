"""Vert airs off the KenneyPark quarter pipe (lip ollies and plain roll-ups at several speeds): the
tick it leaves, the peak (tick, height) and where it comes back in (tick, height). Port vs sk8."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qpturn import *
def air(rows, port_):
    a = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir") if port_ else r["mode"] == "Air")]
    if not a: return None
    s, e = a[0], a[-1] + 1
    ys = [float(r["deck_py"]) for r in rows]
    pk = max(range(s, e), key=lambda i: ys[i])
    return f"off@{s:3d} y{ys[s]:4.2f}  peak@{pk:3d} y{ys[pk]:4.2f}  in@{e:3d} y{ys[min(e, len(ys)-1)]:4.2f}  air {e - s:2d}"
CASES = [(b, a) for b in (0, 15, 35) for a in ("22 30 0 0 0 -1 0\n31 37 0 0 0 1 0", "28 36 0 0 0 -1 0\n37 43 0 0 0 1 0", "")]
if __name__ == "__main__":
    for brake, act in CASES:
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else ""
        pr, sk = run(act, 200, pre=pre)
        print(f"brake {brake:2d} {'ollie@'+act.split()[0] if act else 'roll    '}  P {air(pr, True)}\n                   S {air(sk, False)}")
