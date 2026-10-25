from edge import *
from side import setup
from magnet import RX
for b, c in [(8,12),(8,14),(10,14),(10,16),(12,16),(12,18),(14,20),(9,15),(11,15),(13,19)]:
    e = f"{433-b-c} {433-c} 1 0 0 0 0\n{433-c+1} 433 -1 0 0 0 0\n"
    pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 110, setup(e))
    pn = next((r["trick_name"] for r in pr[40:] if r["trick_name"] and "OLLIE" not in r["trick_name"]), "-")
    sn = next((e2[4:] for r in sk for e2 in r["events"].split("|") if e2.startswith("EV4:")), "-")
    x17 = float(pr[19]["deck_px"]) - RX; vx = float(pr[19]["deck_vx"]); yw = yawp(pr[19])
    print(f"R{b}/L{c}: port x {x17:+.2f} vx {vx:+.2f} yaw {yw:6.1f} -> {pn:14s} | sk8 {sn}")
