from edge import *
for n in [0, 40, 70, 90, 100, 110, 118, 124, 130, 134]:
    t = n + 15
    act = (f"0 {n} 0 0 0 0 0x2000\n" if n else "") + f"{t} {t+50} -1 0 0 0 0\n"
    pr, sk = pair(act, t + 55)
    v = hs(pr[t])
    states = []
    for r in pr[t:t+50]:
        m = r["motion_state"].split("/")
        tag = "/".join(m[3:6])
        if not states or states[-1] != tag: states.append(tag)
    ypr = ((yawp(pr[t+40]) - yawp(pr[t]) + 540) % 360 - 180) / (40/60)
    ysk = ((math.degrees(float(sk[t+40]["yaw"])) - math.degrees(float(sk[t]["yaw"])) + 540) % 360 - 180) / (40/60)
    print(f"v {v:4.2f} port yaw/s {ypr:7.1f} {states[:4]} | sk8 v {hs(sk[t]):4.2f} yaw/s {ysk:7.1f}")
