"""A grab (RT + stick up) on an ollie: how much later the port lands than without, by when the grab
starts (ticks after the pop flick) and how long it's held; sk8 alongside."""
from edge import pair
BASE = "0 10 0 0 0 -1 0\n11 11 0 0 0 0 0\n12 12 0 0 0 1 0\n"
def land(rows, port):
    air = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if port else r["mode"] == "Air")]
    return air[-1] + 1 if air else None
def peak(rows, port):
    return max(float(r["deck_py"]) for r in rows)
if __name__ == "__main__":
    pr, sk = pair(BASE, 110, extra=("--host-map",))
    p0, s0, pk0, sk0 = land(pr, True), land(sk, False), peak(pr, True), peak(sk, False)
    print(f"plain: port lands {p0}  sk8 {s0}")
    for start in (14, 19, 25, 32, 40, 48):
        for dur in (4, 10, 24):
            pr, sk = pair(BASE + f"{start} {start + dur - 1} 0 0 0 1 0 0 1\n", 110, extra=("--host-map",))
            print(f"grab @{start:2d} x{dur:2d}: port +{land(pr, True) - p0} peak +{peak(pr, True) - pk0:.2f}   sk8 +{land(sk, False) - s0} peak +{peak(sk, False) - sk0:.2f}")
