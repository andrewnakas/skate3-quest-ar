"""Natural right-stick flicks from a crouch, through both engines: straight across the stick or
swept round the rim (either way) to each end point, at a quick one sample per tick. Prints what
each engine named, and a tally of the ones that differ."""
import sys, os, math, subprocess, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
from edge import _rows
def rim(deg, r=1.0):  # 0 = up, 90 = right
    return (round(r * math.sin(math.radians(deg)), 3), round(r * math.cos(math.radians(deg)), 3))
def path(kind, end, crouch=12, step=1):
    pts = [rim(180)] * crouch
    if kind == "line":
        for k in (1, 2, 3):
            a, b = rim(180), rim(end)
            f = k / 4
            pts += [(round(a[0] + (b[0] - a[0]) * f, 3), round(a[1] + (b[1] - a[1]) * f, 3))] * step
    else:
        d = ((end - 180) % 360) if kind == "cw" else -((180 - end) % 360)
        n = max(2, int(abs(d) / 30))
        for k in range(1, n):
            pts += [rim(180 + d * k / n)] * step
    pts += [rim(end)] * 3
    return pts
def script(pts, t0):
    return "".join(f"{t0+i} {t0+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(pts))
def names_port(rows):
    out = []
    for r in rows:
        n = r["trick_name"]
        if n and n not in out: out.append(n)
    return out
def names_sk8(rows):
    out = []
    for r in rows:
        for e in r["events"].split("|"):
            if e.startswith("EV1:") and e[4:] not in out: out.append(e[4:])
    return out
if __name__ == "__main__":
    kinds = sys.argv[1:] or ["line", "cw", "ccw"]
    diff = 0; tot = 0
    for kind in kinds:
        for end in range(0, 360, 45 if kind != "line" else 45):
            if kind != "line" and end == 180: continue
            pts = path(kind, end)
            body = script(pts, 0)
            p = os.path.join(D, f"f{os.getpid()}.txt"); open(p, "w").write(feel.PREFIX + feel.shift(body, feel.T0))
            pr = _rows([S3, PACK, "test-world", p, str(feel.T0 + len(pts) + 110)])
            speed = hs(pr[feel.T0])
            q = os.path.join(D, f"g{os.getpid()}.txt"); open(q, "w").write(feel.shift(body, 1))
            sk = _rows([SK, q, str(len(pts) + 111), "--speed", f"{speed:.4f}"])
            a, b = names_port(pr[feel.T0:]), names_sk8(sk)
            def norm(l):
                out = []
                for x in l:
                    x = re.sub(r"[^a-z0-9]", "", x.lower()).replace("frontside", "fs").replace("backside", "bs").replace("shoveit", "shuvit").replace("pop", "")
                    out.append(x)
                return out
            same = norm(a)[-1:] == norm(b)[-1:] if a or b else True
            tot += 1; diff += not same
            print(f"{kind:4s} to {end:3d}  port {a}  sk8 {b} {'' if same else '  <-- DIFF'}")
    print(f"{diff}/{tot} differ")
