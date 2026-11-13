"""Spins landed off the line (ollie, the left stick held through): the board's offset from the path
at touchdown, folded to the nearest line (d), which line it rolls away on, and the speed kept."""
from spinswing import *
if __name__ == "__main__":
    for m in [float(a) for a in sys.argv[1:]] or (0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.88, 0.9, 0.92, 0.94, 1.0):
        pr, sk = pair(ollie(30, f"37 100 {-m} 0 0 0 0\n"), 150)
        out = []
        for rows, p in ((pr, True), (sk, False)):
            L = land_i(rows, p)
            y = yawp(rows[L]) if p else math.degrees(float(rows[L]["yaw"]))
            off = (y - vyaw(rows[L - 3]) + 180) % 360 - 180
            v0 = hs(sk[L - 3]) if True else 0
            out.append(f"off{off:5.0f} keep {hs(rows[L+45]) / v0:4.2f}")
        print(f"m{m:4.2f} P {out[0]} | S {out[1]}  sk8 fakie {sk[-1]['fakie']}")
