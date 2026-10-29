"""Shove-its followed at once by a second stick move: which the original reads as an underflip."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from airflick import *
D_ = 0.707
SHOVES = {"fs": [(0, -1)] * 12 + [(-D_, -D_), (-1, 0), (-D_, D_), (0, 1)], "bs": [(0, -1)] * 12 + [(D_, -D_), (1, 0), (D_, D_), (0, 1)]}
MOVES = {"D": (0, -1), "DL": (-D_, -D_), "DR": (D_, -D_), "L": (-1, 0), "R": (1, 0)}
if __name__ == "__main__":
    for sn, pts in SHOVES.items():
        for mn, m in MOVES.items():
            row = []
            for gap in (0, 2, 4, 7, 10):
                seq = pts + [(0, 0)] * gap + [m] * 6
                act = "".join(f"{i} {i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(seq))
                pr, sk = pair(act, len(seq) + 70, extra=("--host-map",))
                pn = [n for n in names(pr, True)]; snm = names(sk, False)
                row.append(f"g{gap}:{'U' if any('UNDER' in n for n in pn) else '-'}{'U' if any('Underflip' in n for n in snm) else '-'}")
            print(f"{sn} shove then {mn:2s}: " + " ".join(row))
