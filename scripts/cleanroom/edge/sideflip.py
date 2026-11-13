"""Regular stance on the test world: a flick from a side straight to the diagonal above it (and
variants with a step in between), port vs sk8: a flip or that side's shove-it."""
from edge import pair
from fuzz import tricks
D = 0.707
PATHS = {
    "R-ur": [(1, 0), (D, D)], "L-ul": [(-1, 0), (-D, D)],
    "dR-R-ur": [(D, -D), (1, 0), (D, D)], "dL-L-ul": [(-D, -D), (-1, 0), (-D, D)],
    "R-R-ur": [(1, 0), (1, 0), (D, D)], "L-L-ul": [(-1, 0), (-1, 0), (-D, D)],
    "R-ur-ur": [(1, 0), (D, D), (D, D)], "L-ul-ul": [(-1, 0), (-D, D), (-D, D)],
    "R-ur-u": [(1, 0), (D, D), (0, 1)], "L-ul-u": [(-1, 0), (-D, D), (0, 1)],
    "R-.9ur": [(1, 0), (0.9, 0.44)], "L-.9ul": [(-1, 0), (-0.9, 0.44)],
}
for k, p in PATHS.items():
    s = "0 4 0 0 0 -1 0\n" + "".join(f"{5 + i} {5 + i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(p))
    pr, sk = pair(s, 70, extra=("--host-map",))
    a, b = tricks(pr, True), tricks(sk, False)
    print(f"{k:9s} port {a}  sk8 {b}")
