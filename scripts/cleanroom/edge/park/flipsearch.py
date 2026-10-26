import sys, math, itertools; sys.path.insert(0,'.')
from park import *
def pitch_total(rows):
    tot = 0; prev = None
    for r in rows:
        qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
        up_y = 1 - 2 * (qx*qx + qz*qz); fy = 2 * (qy*qz - qw*qx)
        a = math.atan2(fy, up_y)
        if prev is not None: tot += (a - prev + math.pi) % (2*math.pi) - math.pi
        prev = a
    return math.degrees(tot)
base = "22 30 0 0 0 -1 0\n31 37 0 0 0 1 0\n"
BTN = {"none": 0, "LB": 0x100, "RB": 0x200, "LB+RB": 0x300, "Y": 0x8000, "X": 0x4000, "A": 0x1000, "B": 0x2000, "LS": 0x40, "RS": 0x80}
hits = []
for (bn, bit), ly, trig, rs in itertools.product(BTN.items(), (1, -1), ((0, 0), (1, 1)), ((0, 0), (0, 1), (0, -1))):
    extra = f"40 100 0 {ly} {rs[0]} {rs[1]} {hex(bit)} {trig[0]} {trig[1]}\n"
    pr, sk = run(base + extra, 130)
    names = sorted(set(r["trick_name"] for r in pr if r["trick_name"]))
    pt = pitch_total(pr[36:120])
    flag = any("FLIP" in n and "KICK" not in n and "HEEL" not in n for n in names) or abs(pt) > 200
    if flag:
        hits.append((bn, ly, trig, rs, names, pt)); print("HIT", bn, ly, trig, rs, names, round(pt))
print("done", len(hits))
