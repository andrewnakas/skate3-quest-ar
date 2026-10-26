from edge import *
for st in (-1.0, -0.6):
    for n in [0, 20, 40, 60, 75, 85]:
        t = n + 15
        act = (f"0 {n} 0 0 0 0 0x2000\n" if n else "") + f"{t} {t+75} {st} 0 0 0 0\n"
        pr, sk = pair(act, t + 76)
        f = lambda rows, yaw: round(((yaw(rows[t+70]) - yaw(rows[t+35]) + 540) % 360 - 180) / (35/60))
        print(f"steer {st} v {hs(pr[t]):4.2f}->{hs(pr[t+70]):4.2f} port {f(pr, yawp):5d} deg/s | sk8 v {hs(sk[t]):4.2f}->{hs(sk[t+70]):4.2f} {f(sk, lambda r: math.degrees(float(r['yaw']))):5d}")
