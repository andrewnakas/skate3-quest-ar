"""A manual pressed during / after a push: when does it start, the port vs sk8? (the port won't go into a
manual until its push is over). The stick to a nose manual (part-way up) from tick K after a push of N."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fuzz import pair
def start(rows, port_, a):
    return next((i for i in range(a, len(rows)) if ("Manual" in rows[i]["motion_state"] if port_ else rows[i]["mode"] == "Manual")), None)
if __name__ == "__main__":
    for n in (3, 12):
        out = []
        for k in (10, 30, 50, 60, 70, 80, 90):
            s = f"0 {n} 0 0 0 0 0x1000\n{k} {k+60} 0 0 0 {os.environ.get('Y', '0.5')} 0\n"
            pr, sk = pair(s, k + 62, extra=("--host-map",))
            out.append(f"K{k}: P{start(pr, True, k)} S{start(sk, False, k)}")
        print(f"push {n:2d}: " + "  ".join(out))
