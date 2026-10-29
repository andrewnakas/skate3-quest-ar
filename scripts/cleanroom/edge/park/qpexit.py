"""Up the quarter pipe and back down: speed and height every few ticks (exit speed after vert airs)."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def st(r, port_):
    return (r["phys_state"][:10] if port_ else r["mode"][:10])
if __name__ == "__main__":
    for brake in [int(a) for a in sys.argv[1:]] or [0, 25]:
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else ""
        for name, act in (("plain", ""), ("ollie@30", "22 30 0 0 0 -1 0\n31 37 0 0 0 1 0")):
            pr, sk = run(act, 220, pre=pre)
            print(f"brake {brake} {name}")
            for i in range(0, 220, 8):
                a, b = pr[i], sk[i]
                print(f"  {i:3d} P v{sp(a):5.2f} vy{float(a['deck_vy']):6.2f} y{float(a['deck_py']):5.2f} x{float(a['deck_px']):6.2f} {st(a,1):10s} | S v{sp(b):5.2f} vy{float(b['deck_vy']):6.2f} y{float(b['deck_py']):5.2f} x{float(b['deck_px']):6.2f} {st(b,0):10s} {b['events'][:30]}")
