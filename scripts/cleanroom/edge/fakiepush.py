"""A push from fakie (the rider shuffles round to switch, then pushes), then an ollie or a heelflip
flicked D ticks after the push button: which delays give a No Comply in the port and in sk8.
Route: the fuzz seed 970 lead-in (an ollie 180 landed fakie at ~84), A held 88-100."""
import sys
from fuzz import pair, tricks
LEAD = "0 25 0 0 0 -1 0\n26 26 0 0 0 0 0\n27 27 0 0 0 1 0\n31 68 -0.8 0 0 0 0\n88 100 0 0 0 0 0x1000\n"
FLICK = {"ollie": [(0, 1)], "heel": [(0.707, 0.707), (0, 1)]}
def run(d, kind, crouch=5):
    t = 88 + d; s = LEAD + f"{t - crouch} {t - 1} 0 0 0 -1 0\n"
    for i, (x, y) in enumerate(FLICK[kind]): s += f"{t + i} {t + i} 0 0 {x} {y} 0\n"
    pr, sk = pair(s, t + 70, extra=("--host-map",))
    return tricks(pr, True), tricks(sk, False)
if __name__ == "__main__":
    kinds = sys.argv[1:] or ["ollie", "heel"]
    for kind in kinds:
        for d in range(8, 64, 4):
            a, b = run(d, kind)
            a, b = [x for x in a if x != "OLLIE" or kind == "ollie"][-1:], b[-1:]
            nc_p = any("COMPLY" in x.upper() for x in a); nc_s = any("comply" in x.lower() for x in b)
            print(f"{kind:5s} d={d:2d}  port {a}  sk8 {b}  {'' if nc_p == nc_s else '<-- differ'}")
