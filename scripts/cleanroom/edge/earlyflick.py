"""The earliest in-air flick each game takes, per pop and flick kind: the tick the flick ends,
counted from the first air tick (22 for these pops)."""
from airflick import pair, names, PATHS, POPS, D_
KINDS = {"D>C>UR": PATHS["D>C>UR"], "D>C>UL": PATHS["D>C>UL"], "U>C>DR": PATHS["U>C>DR"], "U>C>DL": PATHS["U>C>DL"],
         "D>L>U": PATHS["D>L>U sweep"], "D>R>U": PATHS["D>R>U sweep"]}
if __name__ == "__main__":
    for pop in POPS:
        for k, pts in KINDS.items():
            got = {}
            for end in range(26, 40):
                at = end - len(pts) + 1
                act = POPS[pop] + "".join(f"{at+i} {at+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(pts))
                pr, sk = pair(act, 110, extra=("--host-map",))
                for g, rows, p in (("P", pr, True), ("S", sk, False)):
                    if g not in got and len(names(rows, p)) > 1: got[g] = end - 22
            print(f"{pop:6s} {k:7s} port {got.get('P')}  sk8 {got.get('S')}")
