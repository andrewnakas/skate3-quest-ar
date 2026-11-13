"""In the air (an ollie off flat), the stick pulled down K ticks then flicked: what the port makes of an
ollie's gesture (down, centre, up) and its neighbours, against sk8; and whether it bails."""
from edge import pair
from fuzz import tricks
BASE = "0 10 0 0 0 -1 0\n11 11 0 0 0 0 0\n12 12 0 0 0 1 0\n"
D = 0.707
ENDS = {"c-up": [(0, 0), (0, 1)], "up": [(0, 1)], "c-ur": [(0, 0), (D, D)], "c-ul": [(0, 0), (-D, D)]}
if __name__ == "__main__":
    for at in (22, 30):
        for k in (1, 4, 8, 14):
            for e, path in ENDS.items():
                s = BASE + f"{at} {at + k - 1} 0 0 0 -1 0\n" + "".join(f"{at + k + i} {at + k + i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(path))
                pr, sk = pair(s, 110, extra=("--host-map",))
                a = [t for t in tricks(pr, True) if t != "OLLIE"]; b = [t for t in tricks(sk, False) if t != "Ollie"]
                pb = any(r["wiping_out"] == "1" for r in pr); sb = any(r["mode"] == "Bail" for r in sk)
                print(f"@{at} down x{k:2d} {e:5s}: port {a}{' BAIL' if pb else ''}   sk8 {b}{' BAIL' if sb else ''}")
