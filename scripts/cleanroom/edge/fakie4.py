import sys
from edge import *
br = int(sys.argv[1]) if len(sys.argv) > 1 else 110
s = float(sys.argv[2]) if len(sys.argv) > 2 else -0.67
hold = int(sys.argv[3]) if len(sys.argv) > 3 else 3
act = f"0 {br} 0 0 0 0 0x2000\n" + feel.shift(feel.ollie(30, f"37 100 {s} 0 0 0 0\n"), br + 5)
p0 = br + 5 + 140
act += f"{p0} {p0+hold} 0 0 0 0 0x1000\n"
pr, sk = pair(act, p0 + 120)
for i in list(range(br, p0, 15)) + list(range(p0, p0 + 120, 5)):
    a, b = pr[i], sk[i]
    print(f"{i:4d} P v{hs(a):5.2f} yaw{yawp(a):7.1f} vdir{vyaw(a):7.1f} {a['motion_state'][15:64]:49s} | S v{hs(b):5.2f} yaw{math.degrees(float(b['yaw'])):7.1f} vdir{vyaw(b):7.1f} fk{b['fakie']} {b['mode']} {b['events'][:30]}")
