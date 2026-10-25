import sys
from edge import *
from rails import tl
S = feel.ON_RAIL
start = int(sys.argv[1]) if len(sys.argv) > 1 else 72
for deg in range(0, 360, 45):
    x, y = round(math.sin(math.radians(deg)), 3), round(math.cos(math.radians(deg)), 3)
    act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n55 64 0 0 0 -1 0\n65 71 0 0 0 1 0\n{start} 115 0 0 {x} {y} 0"
    pr, sk = pair(act, 125, S)
    a, b = tl(pr, sk)
    pn = " ".join(t for t in a.split() if (":T:" in t and int(t.split(":")[0]) > 66) or "Wipe" in t)
    sn = " ".join(f"{i}:" + e[4:].replace("Backside ", "") for i, r in enumerate(sk) if i > 66 for e in r["events"].split("|") if e.startswith(("EV4:", "EV3:", "EV0:")))
    print(f"{deg:3d} ({x:5.2f},{y:5.2f}) port {pn:50s} | sk8 {sn}")
