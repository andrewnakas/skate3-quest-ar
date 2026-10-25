from edge import *
from magnet import caught, RX
import sys
for s, a, b in [(0,0,0), (0.6,420,433), (1,420,433), (1,410,433), (1,400,433), (-0.6,420,433), (-1,420,433), (-1,410,433), (-1,400,433)]:
    S = dict(feel.ON_RAIL)
    S["prefix"] = feel.RAIL_PREFIX + (f"{a} {b} {s} 0 0 0 0\n" if s else "")
    pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 110, S)
    out = []
    for rows, port in ((pr, True), (sk, False)):
        c = caught(rows, port)
        lat = [round(float(rows[i]["deck_px"]) - RX, 2) for i in (0, 17, 35, 50, 58)]
        vx = [round(float(rows[i]["deck_vx"]), 2) for i in (17, 35, 50)]
        out.append(f"x {lat} vx {vx} catch {c}")
    print(f"steer {s:4.1f} {a}-{b}  port {out[0]}\n                 sk8  {out[1]}")
