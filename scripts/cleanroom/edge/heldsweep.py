"""An in-air flick swept straight out of the stick still held from the pop (no centre between):
does it flip late? Nollie / ollie, the pop stick held to several ticks, then the sweep; and the
same with one centre tick first."""
from airflick import pair, names, bail, D_
SW = {"D>L>U": [(-D_, -D_), (-1, 0), (-D_, D_), (0, 1)], "D>R>U": [(D_, -D_), (1, 0), (D_, D_), (0, 1)]}
POP = {"nollie": ("0 12 0 0 0 1 0\n13 13 0 0 0 0 0\n", -1), "ollie": ("0 12 0 0 0 -1 0\n13 13 0 0 0 0 0\n", 1)}
if __name__ == "__main__":
    for pop, (lead, y0) in POP.items():
        for centre in (False, True):
            for hold in (8, 12, 16, 22, 30):
                end = 14 + hold
                act = lead + f"14 {end-1} 0 0 0 {y0} 0\n"
                pts = ([(0, 0)] if centre else []) + [(x, y * y0 * -1) for x, y in SW["D>L>U"]]
                act += "".join(f"{end+i} {end+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(pts))
                pr, sk = pair(act, 130, extra=("--host-map",))
                print(f"{pop:6s} {'centre' if centre else 'held  '} hold {hold:2d}: P {names(pr, True)}{' BAIL' if bail(pr, True) else ''} | S {names(sk, False)}{' BAIL' if bail(sk, False) else ''}")
