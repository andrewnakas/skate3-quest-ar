import sys
from edge import *
from rails import tl
S = feel.ON_RAIL
base = sys.argv[1]  # "5-0" or "nose"
mag = float(sys.argv[2])
by = -1 if base == "5-0" else 1
for deg in (range(90, 271, 15) if base == "5-0" else list(range(270, 360, 15)) + list(range(0, 91, 15))):
    x, y = round(mag*math.sin(math.radians(deg)), 3), round(mag*math.cos(math.radians(deg)), 3)
    act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n30 74 0 0 0 {by} 0\n75 105 0 0 {x} {y} 0"
    pr, sk = pair(act, 110, S)
    a, b = tl(pr, sk)
    pn = " ".join(t for t in a.split() if ":T:" in t and "OLLIE" not in t)
    sn = " ".join(f"{i}:" + e[4:].replace("Backside ", "") for i, r in enumerate(sk) for e in r["events"].split("|") if e.startswith("EV4:"))
    print(f"{deg:3d} ({x:5.2f},{y:5.2f}) port {pn:52s} | sk8 {sn}")
