"""Find the port's sector edges round the up diagonals: a sweep from down stops a moment at angle
X on the way, then goes on to the diagonal. X inside the diagonal's sector: a sweep (shove-it);
outside it: a flick into it (flip)."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from flickjump import *
if __name__ == "__main__":
    for end, xs, mid in ((45, range(66, 100, 3), 135), (315, range(240, 290, 4), 225)):
        for hold in (1, 2):
            row = []
            for x in xs:
                seq = [rim(180)] * 12 + [rim(mid)] + [rim(x)] * hold + [rim(end)] * 3
                pn, sn = one(seq)
                row.append(f"{x}:{(pn or ['-'])[0][:6]}/{(sn or ['-'])[0][:6]}")
            print(f"end {end} hold {hold}: " + " ".join(row))
