import sys, math; sys.path.insert(0,'.')
from park import *
def pitch_series(rows, a, b):
    out=[]; prev=None; tot=0
    for r in rows[a:b]:
        qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
        up_y = 1 - 2 * (qx*qx + qz*qz); fy = 2 * (qy*qz - qw*qx)
        ang = math.atan2(fy, up_y)
        if prev is not None: tot += (ang - prev + math.pi) % (2*math.pi) - math.pi
        prev = ang; out.append(round(math.degrees(tot)))
    return out
base = "22 30 0 0 0 -1 0\n31 37 0 0 0 1 0\n"
if __name__ == "__main__":
    for name, extra in (("grab only", "40 110 0 0 0 0 0 0 1\n"), ("grab + left down", "40 110 0 -1 0 0 0 0 1\n"),
                        ("grab + left up", "40 110 0 1 0 0 0 0 1\n"), ("grab(LT) + left down", "40 110 0 -1 0 0 0 1 0\n"),
                        ("grab, then left down", "40 110 0 0 0 0 0 0 1\n46 110 0 -1 0 0 0 0 1\n"),
                        ("grab + left down + spin", "40 110 -0.7 -0.7 0 0 0 0 1\n")):
        pr, sk = run(base + extra, 140)
        names = [r["trick_name"] for r in pr if r["trick_name"]]
        seen = []
        for n in names:
            if not seen or seen[-1] != n: seen.append(n)
        land = next((i for i in range(45, 140) if pr[i]["phys_state"] not in ("KnownAir", "PhysicsAir", "GroundAnimation")), None)
        print(f"{name:26s} port {seen} land {land} {pr[land]['motion_state'][8:50] if land else ''}\n   pitch {pitch_series(pr, 38, 130)[::6]}\n   sk8   {pitch_series(sk, 38, 130)[::6]}")
