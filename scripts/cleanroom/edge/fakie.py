import sys
from edge import *
def spin_act(s): return feel.ollie(30, f"37 100 {s} 0 0 0 0\n")
if __name__ == "__main__":
    for s in [-0.6, -0.65, -0.7, -0.75]:
        pr, sk = pair(spin_act(s), 140)
        yp0, ys0 = yawp(pr[0]), math.degrees(float(sk[0]["yaw"]))
        yp, ys = yawp(pr[139]), math.degrees(float(sk[139]["yaw"]))
        print(f"s {s}: port yaw {yp0:.0f}->{yp:.0f} vdir {vyaw(pr[139]):.0f} state {pr[139]['phys_state']} fakie? {pr[139].get('motion_state','')[:60]} | sk8 yaw {ys0:.0f}->{ys:.0f} vdir {vyaw(sk[139]):.0f} fakie {sk[139]['fakie']} {sk[139]['mode']}")
