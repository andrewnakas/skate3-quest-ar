"""Where the port tells a sweep (shove-it) from a flick (flip): the size of the last step onto the
end point, and of the step before it."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from flicksweep import *
from edge import _rows
def one(pts):
    body = script(pts, 0)
    p = os.path.join(D, "fj.txt"); open(p, "w").write(feel.PREFIX + feel.shift(body, feel.T0))
    pr = _rows([S3, PACK, "test-world", p, str(feel.T0 + len(pts) + 110)])
    q = os.path.join(D, "gj.txt"); open(q, "w").write(feel.shift(body, 1))
    sk = _rows([SK, q, str(len(pts) + 111), "--speed", f"{hs(pr[feel.T0]):.4f}"])
    return names_port(pr[feel.T0:]), names_sk8(sk)
def seqpts(seq, hold=2):
    return [rim(seq[0])] * 12 + [rim(a) for a in seq[1:]] + [rim(seq[-1])] * hold
if __name__ == "__main__":
    for end, side in ((45, 1), (315, -1)):
        for last in (25, 30, 35, 40, 45, 50, 60):
            for before in (30, 45):
                x = end + side * last; y = x + side * before
                seq = [180]
                a = 180
                while (side > 0 and a - before > y) or (side < 0 and a + before < y):  # down to y in steps of `before`
                    a = a - side * before if side > 0 else a + before
                    break
                seq = [180, (y + 180) / 2 if abs(y - 180) > 50 else 180, y, x, end]
                seq = [s % 360 for s in seq]
                print(f"end {end:3d} last {last:2d} before {before:2d} {[round(s) for s in seq]}", *one(seqpts(seq)))
