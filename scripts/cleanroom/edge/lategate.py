"""The earliest a late flick is taken in the air: the first tick (after the pop's flick) a late flick's
sweep can START and still count, the port vs sk8, out of an ollie, a nollie and a nose-manual nollie."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fuzz import pair, tricks
D = 0.707
SWEEPS = {"fsshove": [(-D, -D), (-1, 0), (-D, D), (0, 1)], "kick": [(0, -1), (0, 0), (-D, D)], "heel": [(0, -1), (0, 0), (D, D)]}
LEADS = {"ollie": ("0 9 0 0 0 -1 0\n10 10 0 0 0 0 0\n11 11 0 0 0 1 0\n", 11),
         "nollie": ("0 9 0 0 0 1 0\n10 10 0 0 0 0 0\n11 11 0 0 0 -1 0\n", 11),
         "noseman nollie": ("0 57 0 0 0 0.5 0\n62 72 0 0 0 -1 0\n", 62)}
def late(rows, port_):
    return any(("LATE" in n) if port_ else n.startswith("Late") for n in tricks(rows, port_))
if __name__ == "__main__":
    for ln, (lead, t0) in LEADS.items():
        for sn, sw in SWEEPS.items():
            first = {}
            for port_ in (True, False):
                for k in range(4, 24):
                    s = lead + "".join(f"{t0+k+i} {t0+k+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(sw))
                    a, b = pair(s, t0 + k + 70, extra=("--host-map",))
                    if late(a if port_ else b, port_):
                        first[port_] = k; break
            print(f"{ln:15s} {sn:8s} first late sweep start (ticks after the pop flick): port {first.get(True)} sk8 {first.get(False)}")
