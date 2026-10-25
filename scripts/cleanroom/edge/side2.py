from edge import *
from side import setup
from magnet import RX
for a, b, c, d in [(1,4,4,1),(1,5,5,1),(1,6,6,1),(1,7,7,1),(1,8,9,1),(1,6,10,1),(0.6,8,8,1),(1,3,12,1)]:
    # steer right b ticks, then left c ticks, ending at 433 (comes across from +x heading -x)
    e = f"{433-b-c} {433-c} {a} 0 0 0 0\n{433-c+1} 433 {-d} 0 0 0 0\n"
    pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 110, setup(e))
    pn = next((r["trick_name"] for r in pr[40:] if r["trick_name"] and "OLLIE" not in r["trick_name"]), "-")
    sn = next((e2[4:] for r in sk for e2 in r["events"].split("|") if e2.startswith("EV4:")), "-")
    x17 = float(pr[17]["deck_px"]) - RX; vx = float(pr[17]["deck_vx"]); yw = yawp(pr[17])
    print(f"R{b}/L{c}: port x {x17:+.2f} vx {vx:+.2f} yaw {yw:6.1f} -> {pn:14s} | sk8 {sn}")
