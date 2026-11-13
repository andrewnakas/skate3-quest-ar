"""Push gain over (speed, hold length): brake n ticks, coast 70, then A held h ticks. Port vs sk8."""
from edge import *
def g(rows, t):
    v0 = hs(rows[t]); return v0, max(hs(rows[i]) for i in range(t, t + 75)) - v0
if __name__ == "__main__":
    HOLDS = (1, 2, 3, 4, 6, 8, 10, 14, 20)
    for n in (30, 60, 80, 100, 118, 124):
        t = n + 70
        P, S = [], []
        for h in HOLDS:
            pr, sk = pair(f"0 {n} 0 0 0 0 0x2000\n{t} {t+h-1} 0 0 0 0 0x1000\n", t + 80, extra=("--host-map",))
            (pv, pg), (sv, sg) = g(pr, t), g(sk, t)
            P.append(pg); S.append(sg)
        print(f"v {pv:4.2f}/{sv:4.2f} P " + " ".join(f"{x:4.2f}" for x in P))
        print(f"            S " + " ".join(f"{x:4.2f}" for x in S))
    print("holds", HOLDS)
