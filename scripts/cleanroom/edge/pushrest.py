"""Push gain by starting speed, tapped vs held: get going (held push), brake down to a speed (or to a stop),
then one push. Port vs sk8, both from KenneyPark's spawn heading -Z."""
from rest import *
def gain(rows, t):
    v0 = hs3(rows[t]); v1 = max(hs3(rows[i]) for i in range(t, t + 70))
    c = next((i - t for i in range(t, t + 70) if hs3(rows[i]) > v0 + 0.05), None)
    return v0, v1 - v0, c
if __name__ == "__main__":
    for brake in (0, 10, 20, 30, 40, 60, 90):
        row = []
        for kind, hold in (("tap", 3), ("held", 30)):
            lead = "0 300 0 0 0 0 0x1000\n" if brake else ""  # (get to ~6.9 first)
            b0 = 120
            act = lead.replace("300", str(b0 - 1)) + (f"{b0} {b0+brake} 0 0 0 0 0x2000\n" if brake else "")
            t = b0 + brake + 40 if brake else 20
            act += f"{t} {t+hold} 0 0 0 0 0x1000\n"
            pr, sk = rest_pair(act, t + 80)
            (pv, pg, pc), (sv, sg, sc) = gain(pr, t), gain(sk, t)
            row.append(f"{kind} P v{pv:4.2f} +{pg:4.2f}@{pc} S v{sv:4.2f} +{sg:4.2f}@{sc}")
        print(f"brake {brake:3d}: " + "  |  ".join(row))
