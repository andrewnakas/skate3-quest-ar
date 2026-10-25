import sys
from edge import *
from rails import tl
from side import setup
S = setup(f"{433-13-19} {433-19} 1 0 0 0 0\n{433-19+1} 433 -1 0 0 0 0\n")
mag = float(sys.argv[1]) if len(sys.argv) > 1 else 1.0
for deg in range(0, 360, 30):
    x, y = round(mag*math.sin(math.radians(deg)), 3), round(mag*math.cos(math.radians(deg)), 3)
    act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n30 100 0 0 {x} {y} 0"
    pr, sk = pair(act, 110, S)
    a, b = tl(pr, sk)
    pn = " ".join(t for t in a.split() if ":T:" in t and "OLLIE" not in t)
    sn = " ".join(f"{i}:" + e[4:] for i, r in enumerate(sk) for e in r["events"].split("|") if e.startswith("EV4:"))
    print(f"{deg:3d} ({x:5.2f},{y:5.2f}) port {pn:40s} | sk8 {sn}")
