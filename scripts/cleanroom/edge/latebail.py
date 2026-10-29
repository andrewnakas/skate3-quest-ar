"""Late flips (down, centre, up-right in the air) out of pops of every size at every moment: land or bail."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from airflick import *
if __name__ == "__main__":
    for crouch in (2, 6, 12, 30):
        row = []
        for off in range(8, 44, 4):
            at = crouch + 8 + off
            act = f"0 {crouch} 0 0 0 -1 0\n{crouch+1} {crouch+1} 0 0 0 0 0\n{crouch+2} {crouch+7} 0 0 0 1 0\n" + \
                  "".join(f"{at+i} {at+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(PATHS["D>C>UR"]))
            pr, sk = pair(act, at + 90, extra=("--host-map",))
            pl = len(names(pr, True)) > 1; sl = len(names(sk, False)) > 1
            pb = bail(pr, True); sb = bail(sk, False)
            f = lambda late, b: ("B" if b else ("L" if late else "-"))
            row.append(f"{off}:{f(pl,pb)}{f(sl,sb)}")
        print(f"crouch {crouch:2d}  " + " ".join(row))
