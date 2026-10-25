from edge import *
from rails import tl
import sys
def setup(extra):
    S = dict(feel.ON_RAIL); S["prefix"] = feel.RAIL_PREFIX + extra; return S
SIDES = {
    "from -x (14/14)": "405 419 -1 0 0 0 0\n420 433 1 0 0 0 0\n",
    "from +x (6/6)": "422 427 -1 0 0 0 0\n428 433 1 0 0 0 0\n",
    "from +x (sidestep +1 6/6)": "422 427 1 0 0 0 0\n428 433 -1 0 0 0 0\n",
    "straight": "",
}
if __name__ == "__main__":
    for name, extra in SIDES.items():
        pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 110, setup(extra))
        a, b = tl(pr, sk)
        print(f"== {name}\n  port {a}\n  sk8  {b}")
