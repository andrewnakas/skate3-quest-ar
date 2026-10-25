from edge import *
from side import setup
from magnet import RX
cases = {"straight": "", "-0.6": "420 433 -0.6 0 0 0 0\n", "-0.8": "420 433 -0.8 0 0 0 0\n",
         "L14/R14": "405 419 -1 0 0 0 0\n420 433 1 0 0 0 0\n", "L6/R6": "422 427 -1 0 0 0 0\n428 433 1 0 0 0 0\n",
         "R4/L4": "425 429 1 0 0 0 0\n430 433 -1 0 0 0 0\n"}
for b, c in [(12,18),(14,20),(13,19)]:
    cases[f"R{b}/L{c}"] = f"{433-b-c} {433-c} 1 0 0 0 0\n{433-c+1} 433 -1 0 0 0 0\n"
for name, e in cases.items():
    pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 110, setup(e))
    c = next((i for i, r in enumerate(pr) if r["phys_state"].startswith("Grind")), None)
    if c is None: print(name, "no catch"); continue
    pn = next((r["trick_name"] for r in pr[c:] if r["trick_name"] and "OLLIE" not in r["trick_name"]), "-")
    r = pr[c - 2]
    sn = next((e2[4:] for rr in sk for e2 in rr["events"].split("|") if e2.startswith("EV4:")), "-")
    print(f"{name:10s} catch {c} yaw {yawp(r):7.1f} vdir {vyaw(r):7.1f} vx {float(r['deck_vx']):+.2f} x {float(r['deck_px'])-RX:+.2f} yaw@17 {yawp(pr[17]):7.1f} -> {pn:10s} | sk8 {sn}")
