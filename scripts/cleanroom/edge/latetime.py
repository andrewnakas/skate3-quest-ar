"""Late kickflips at every moment of ollies / nollies of a few sizes: ticks from the flick to the
landing, and whether each game bails - the shortest that still lands in each."""
from airflick import pair, names, bail, PATHS
def landing(rows, p):
    a = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if p else r["mode"] == "Air")]
    return a[-1] + 1 if a else None
LEADS = {"ollie c2": "0 2 0 0 0 -1 0\n3 3 0 0 0 0 0\n4 9 0 0 0 1 0\n",
         "ollie c12": "0 12 0 0 0 -1 0\n13 13 0 0 0 0 0\n14 19 0 0 0 1 0\n",
         "nose man nollie": "0 30 0 0 0 0.5 0\n35 41 0 0 0 -1 0\n",
         "nose man long": "0 50 0 0 0 0.5 0\n55 61 0 0 0 -1 0\n"}
if __name__ == "__main__":
    for name, lead in LEADS.items():
        out = []
        for at in range(20, 110, 3):
            act = lead + "".join(f"{at+i} {at+i} 0 0 {x} {y} 0\n" for i, (x, y) in enumerate(PATHS["D>C>UR"]))
            pr, sk = pair(act, at + 90, extra=("--host-map",))
            pn, sn = names(pr, True), names(sk, False)
            if len(pn) < 2 and len(sn) < 2:
                continue
            fl = at + len(PATHS["D>C>UR"]) - 1
            lp, ls = landing(pr, True), landing(sk, False)
            out.append(f"{fl:3d}:p{'-' if len(pn) < 2 else ('B' if bail(pr, True) else 'L')}{lp - fl if lp else '?'}/s{'-' if len(sn) < 2 else ('B' if bail(sk, False) else 'L')}{ls - fl if ls else '?'}")
        print(f"{name:16s} " + " ".join(out))
