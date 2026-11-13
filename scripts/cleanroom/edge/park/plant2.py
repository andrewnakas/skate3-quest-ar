"""Handplant side on the angled slow line (run2) and the fast line steered either way: the port vs sk8."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
from plant import tl
if __name__ == "__main__":
    for st in (0, -0.7, 0.7):
        act = f"0 160 {st} 0 0 0 0x4200"
        pr, sk = run2(act, 200, sk_extra=("--host-map",))
        print(f"== line2 steer {st}\n  P {tl(pr, True)}\n  S {tl(sk, False)}")
    for st in (-0.7, 0.7):
        pr, sk = run(f"0 120 {st} 0 0 0 0x4200", 170, pre=f"{T0-55} {T0-30} 0 0 0 0 0x2000\n", sk_extra=("--host-map",))
        print(f"== line1 steer {st}\n  P {tl(pr, True)}\n  S {tl(sk, False)}")
