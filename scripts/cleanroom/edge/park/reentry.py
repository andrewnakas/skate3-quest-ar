"""Ramp-face ollies that come back down into the quarter pipe: speed at the bottom / out on the flat."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qpturn import *
if __name__ == "__main__":
    out = []
    for brake in (10, 15, 20, 25):
        for at in (22, 28):
            pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n"
            pr, sk = run(f"{at} {at+8} 0 0 0 -1 0\n{at+9} {at+15} 0 0 0 1 0", 150, pre=pre)
            out.append(f"b{brake}@{at} {sp(pr[149]):4.1f}/{sp(sk[149]):4.1f}")
    print("  ".join(out))
