"""Riding head-on into park geometry from a standstill near the spawn: push twice, then coast into it.
Speed and height every 5 ticks around the hit, port vs sk8."""
import rest
from rest import *
def run(pos, facing, ticks=260):
    rest.SPAWN = (pos, facing)
    return rest_pair("0 5 0 0 0 0 0x1000\n", ticks)
if __name__ == "__main__":
    for name, pos, fac in (("+X z0 (box)", "13.5,0.1,0", "1.5708"), ("+X z-3 (steps)", "13.5,0.1,-3", "1.5708"),
                           ("-Z x12 (rail)", "12,0.1,-1.5", "3.14159")):
        pr, sk = run(pos, fac)
        print(name)
        print("  P " + " ".join(f"{hs3(pr[i]):4.2f}/{float(pr[i]['deck_py']):.2f}" for i in range(60, 260, 10)))
        print("  S " + " ".join(f"{hs3(sk[i]):4.2f}/{float(sk[i]['deck_py']):.2f}" for i in range(60, 260, 10)))
