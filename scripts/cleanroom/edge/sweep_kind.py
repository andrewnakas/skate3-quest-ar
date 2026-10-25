import sys
from edge import *
from rails import tl
S = feel.ON_RAIL
start = int(sys.argv[1]) if len(sys.argv) > 1 else 30   # when the stick goes there (catch ~60)
mag = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0
crouch = len(sys.argv) > 3 and sys.argv[3] == "c"
ly = -1 if crouch else 0
for deg in range(0, 360, int(sys.argv[4]) if len(sys.argv) > 4 else 15):
    x, y = round(mag*math.sin(math.radians(deg)), 3), round(mag*math.cos(math.radians(deg)), 3)
    act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n{start} 100 0 {ly} {x} {y} 0"
    pr, sk = pair(act, 110, S)
    a, b = tl(pr, sk)
    pn = " ".join(t for t in a.split() if ":T:" in t and "OLLIE" not in t)
    sn = " ".join(f"{i}:" + e[4:] for i, r in enumerate(sk) for e in r["events"].split("|") if e.startswith("EV4:"))
    print(f"{deg:3d} ({x:5.2f},{y:5.2f}) port {pn:40s} | sk8 {sn}")
