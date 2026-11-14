"""Carving on the quarter pipe face: the steer pressed once the port is on the face (y > 0.15) and held
N ticks, at a few approach speeds (brake ticks). Peak height, when it comes back out, exit speed, turn.
(The port's deck yaw is 180 off its travel on KenneyPark: its FAKIE column means forward.)"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
from qpturn import summary
if __name__ == "__main__":
    hold = int(os.environ.get("HOLD", 40))
    for brake in (int(a) for a in (sys.argv[1:] or ("0", "25", "50"))):
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else ""
        base, _ = run("", 120, pre=pre)
        t = next(i for i, r in enumerate(base) if float(r["deck_py"]) > 0.15)
        for s in (-1, -0.6, -0.3, 0.3, 0.6, 1):
            pr, sk = run(f"{t} {t+hold} {s} 0 0 0 0", 200, pre=pre)
            print(f"brake {brake:2d} @{t} steer {s:5.2f} v0 {sp(pr[0]):4.2f}  P {summary(pr, True)}\n{'':32s}S {summary(sk, False)}")
