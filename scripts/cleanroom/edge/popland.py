"""A second ollie flicked soon after landing the first: crouch starting D ticks after the landing for
C ticks, then the flick up. Does it pop, and when, in the port vs sk8? (the flick during the landing)"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fuzz import pair
FC = int(os.environ.get("FIRST_CROUCH", 10))
FIRST = f"0 {FC-1} 0 0 0 -1 0\n{FC} {FC} 0 0 0 0 0\n{FC+1} {FC+1} 0 0 0 1 0\n"
def airborne(rows, port_):
    return [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "GroundAnimation") if port_ else r["mode"] == "Air")]
def takeoffs(rows, port_):
    a = set(airborne(rows, port_)); return [i for i in sorted(a) if i - 1 not in a]
def lands(rows, port_):
    a = set(airborne(rows, port_)); return [i for i in sorted(a) if i + 1 not in a]
if __name__ == "__main__":
    pr, sk = pair(FIRST, 120, extra=("--host-map",))
    lp, ls = lands(pr, True)[0] + 1, lands(sk, False)[0] + 1
    print(f"first landing: port {lp} sk8 {ls}")
    for c in (int(x) for x in (sys.argv[1:] or ("5", "12"))):
        for d in range(0, 31, 3):
            t = lp + d
            s = FIRST + f"{t} {t+c-1} 0 0 0 -1 0\n{t+c} {t+c} 0 0 0 0 0\n{t+c+1} {t+c+1} 0 0 0 1 0\n"
            pr, sk = pair(s, t + c + 60, extra=("--host-map",))
            P = [x - (t + c + 1) for x in takeoffs(pr, True)[1:]]
            S = [x - (t + c + 1) for x in takeoffs(sk, False)[1:]]
            print(f"crouch {c:2d} from land+{d:2d}: port pops {P[:1] or '-'}  sk8 {S[:1] or '-'}  (ticks after the flick)")
