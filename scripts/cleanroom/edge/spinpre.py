"""Spin stick taken up before / around the pop: total rotation by landing, port vs sk8.
Usage: spinpre.py [mag]"""
import sys, math
from spinland import ollie, land_i, off
from edge import pair
m = float(sys.argv[1]) if len(sys.argv) > 1 else 0.8
for s in (20, 24, 27, 29, 31, 33, 35, 37, 40):
    pr, sk = pair(ollie(30, f"{s} 100 {-m} 0 0 0 0\n"), 110)
    lp, ls = land_i(pr, True), land_i(sk, False)
    rot = lambda rows, i, p: "  none" if i is None or i >= len(rows) else f"{off(rows[i], p) % 360:6.1f}"
    print(f"steer from {s:2d}: port land {lp} rot {rot(pr, lp, True)}   sk8 land {ls} rot {rot(sk, ls, False)}")
