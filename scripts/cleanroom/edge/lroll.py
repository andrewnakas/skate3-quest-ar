import sys
from edge import *
from rails import tl
S = feel.ON_RAIL
base = sys.argv[1]  # "50" or "5-0" or "nose"
by = {"50": 0, "5-0": -1, "nose": 1}[base]
for deg in range(0, 360, 45):
    x, y = round(math.sin(math.radians(deg)), 3), round(math.cos(math.radians(deg)), 3)
    act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n30 74 0 0 0 {by} 0\n75 105 {x} {y} 0 {by} 0"
    pr, sk = pair(act, 115, S)
    a, b = tl(pr, sk)
    pn = " ".join(t for t in a.split() if (":T:" in t and "OLLIE" not in t) or "Wipe" in t or "Air" in t)
    sn = " ".join(f"{i}:" + e[4:].replace("Backside ", "") for i, r in enumerate(sk) for e in r["events"].split("|") if e.startswith(("EV4:", "EV3:")))
    print(f"L{deg:3d} ({x:5.2f},{y:5.2f}) port {pn:60s} | sk8 {sn}")
