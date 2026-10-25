from edge import *
for n in [110, 60]:
    for h in [3, 10, 20, 30, 45]:
        t = n + 20
        act = f"0 {n} 0 0 0 0 0x2000\n{t} {t+3} 0 0 0 0 0x1000\n{t+70} {t+70+h} 0 0 0 0 0x1000\n"
        pr, sk = pair(act, t + 170)
        def g(rows):
            v1 = hs(rows[t + 65]); vs = [round(hs(rows[t + 70 + k]) - v1, 2) for k in (20, 30, 40, 60, 90)]
            return f"v {v1:4.2f} gain@20/30/40/60/90 {vs}"
        print(f"brake {n} hold {h:2d}: port {g(pr)} | sk8 {g(sk)}")
