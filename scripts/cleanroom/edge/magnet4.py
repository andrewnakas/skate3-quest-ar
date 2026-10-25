from edge import *
from magnet import RX
import sys
def run(extra, label):
    S = dict(feel.ON_RAIL); S["prefix"] = feel.RAIL_PREFIX + extra
    pr, sk = pair("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0", 70, S)
    def row(rows, port):
        c = next((i for i, r in enumerate(rows) if (r["phys_state"].startswith("Grind") if port else r["mode"] == "Grind")), None)
        return " ".join(f"{i}:{float(rows[i]['deck_px'])-RX:+.2f}/{float(rows[i]['deck_vx']):+.2f}" for i in (17, 25, 40, 52, 58)) + f" catch {c}"
    print(f"{label:28s} P {row(pr,True)}\n{'':28s} S {row(sk,False)}")
if __name__ == "__main__":
    for a, b, c in [(1, 6, 6), (1, 8, 8), (1, 10, 10), (1, 12, 12), (-1, 6, 6), (-1, 8, 8), (-1, 10, 10), (-1, 12, 12), (1, 14, 14), (-1, 14, 14)]:
        # steer one way for b ticks, then back the other for c ticks, ending at 433
        e = f"{433-b-c} {433-c} {a} 0 0 0 0\n{433-c+1} 433 {-a} 0 0 0 0\n"
        run(e, f"steer {a:+d} {b}/{c}")
