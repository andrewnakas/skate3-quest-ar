import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def air_of(r, port_): return r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if port_ else r["mode"] == "Air"
def outcome(rows, port_):
    ys = [float(r["deck_py"]) for r in rows]
    peak = max(ys); ip = ys.index(peak)
    grind = next((i for i, r in enumerate(rows) if (r["phys_state"].startswith("Grind") if port_ else r["mode"] == "Grind")), None)
    bail = next((i for i, r in enumerate(rows) if ("Wipe" in r["phys_state"] or "Bail" in r["motion_state"] if port_ else r["mode"] == "Bail")), None)
    land = next((i for i in range(ip, len(rows)) if not air_of(rows[i], port_)), None)
    where = "-"
    if land is not None:
        where = f"x{float(rows[land]['deck_px']):5.2f} y{float(rows[land]['deck_py']):4.2f}"
    return f"peak {peak:4.2f}@{ip:3d} land@{land} {where} grind {grind} bail {bail}"
if __name__ == "__main__":
    for brake in (0, 25, 45):
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else ""
        for name, act in (("plain", ""), ("ollie@24", "16 24 0 0 0 -1 0\n25 31 0 0 0 1 0"),
                          ("ollie@30", "22 30 0 0 0 -1 0\n31 37 0 0 0 1 0"), ("ollie@36", "28 36 0 0 0 -1 0\n37 43 0 0 0 1 0")):
            pr, sk = run(act, 150, pre=pre)
            print(f"brake {brake:2d} v {sp(pr[0]):4.2f} {name:9s} port {outcome(pr, True)}\n{'':27s} sk8  {outcome(sk, False)}")
