from edge import *
from magnet import RX
import sys
for s, a, b in [(-0.6,420,433), (-1,420,433), (0.4,420,433), (-0.8,420,433)]:
    S = dict(feel.ON_RAIL); S["prefix"] = feel.RAIL_PREFIX + f"{a} {b} {s} 0 0 0 0\n"
    pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 70, S)
    print(f"== steer {s}")
    for i in range(10, 62, 3):
        r = pr[i]
        print(f"  {i:2d} x {float(r['deck_px'])-RX:6.3f} vx {float(r['deck_vx']):6.2f} h {float(r['deck_py'])-(feel.FLOOR_Y+0.45):6.2f} vy {float(r['deck_vy']):5.2f} z {float(r['deck_pz']):6.2f} vz {float(r['deck_vz']):5.2f} yaw {yawp(r):6.1f} {r['phys_state'][:10]}")
