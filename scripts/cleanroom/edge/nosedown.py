"""A nollie straight out of a nose manual: does the board leave at once (the nose already down) or after
the usual takeoff? By push (speed), manual length and the gap after it. The deck's first rise, ticks
after the flick down, the port vs sk8."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fuzz import pair
def rise(rows, t):
    y0 = float(rows[t - 1]["deck_py"])
    return next((i - t for i in range(t, min(t + 30, len(rows))) if float(rows[i]["deck_py"]) - y0 > 0.005), None)
if __name__ == "__main__":
    for push in (False, True):
        for man in (40, 55):
            for gap in (2, 4, 8):
                a = 40 if push else 10
                s = ("0 3 0 0 0 0 0x1000\n" if push else "") + f"{a} {a+man} 0 0 0 0.5 0\n"
                t = a + man + 1 + gap
                s += f"{t} {t+10} 0 0 0 -1 0\n"
                pr, sk = pair(s, t + 40, extra=("--host-map",))
                v = float(sk[t - 1]["speed"])
                print(f"push {int(push)} manual {man} gap {gap} (v {v:.1f}): port rises +{rise(pr, t)}  sk8 +{rise(sk, t)}   port name {sorted(set(r['trick_name'] for r in pr[t:t+20]) - {''})}")
