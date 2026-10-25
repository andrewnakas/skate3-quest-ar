from edge import *
# brake for N ticks (B = 0x2000), then coast
for n in [0, 40, 70, 100]:
    act = (f"0 {n} 0 0 0 0 0x2000\n" if n else "") + f"{n+1} {n+2} 0 0 0 0 0\n"
    pr, sk = pair(act, n + 160)
    vs = lambda rows: [round(hs(rows[i]), 2) for i in range(n, n + 160, 20)]
    print(f"brake {n:3d}: port {vs(pr)}\n           sk8  {vs(sk)}")
