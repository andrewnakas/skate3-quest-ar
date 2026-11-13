"""Late flips out of a nollie (stick up held C ticks, then down) at each moment of the air: land (L)
or bail (B), port vs sk8. Also out of a nose manual's nollie (manual held, then the stick down)."""
from airflick import pair, names, bail, PATHS
if __name__ == "__main__":
    for lead, label in ((lambda c: f"0 {c} 0 0 0 1 0\n{c+1} {c+1} 0 0 0 0 0\n{c+2} {c+7} 0 0 0 -1 0\n", "nollie"),
                        (lambda c: f"0 {c + 40} 0 0 0 0.5 0\n{c + 41} {c + 47} 0 0 0 -1 0\n", "nose manual")):
        for crouch in (2, 6, 12, 30):
            row = []
            base = crouch + 8 if label == "nollie" else crouch + 49
            for off in range(4, 44, 4):
                at = base + off
                act = lead(crouch) + "".join(f"{at+i} {at+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(PATHS["D>C>UR"]))
                pr, sk = pair(act, at + 90, extra=("--host-map",))
                pl = len(names(pr, True)) > 1; sl = len(names(sk, False)) > 1
                f = lambda late, b: ("B" if b else ("L" if late else "-"))
                row.append(f"{off}:{f(pl, bail(pr, True))}{f(sl, bail(sk, False))}")
            print(f"{label:11s} c{crouch:2d}  " + " ".join(row))
