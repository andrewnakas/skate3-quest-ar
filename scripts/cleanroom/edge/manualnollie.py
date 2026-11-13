"""Out of a nose manual (stick part-up held M ticks), the stick pulled down: the port's nollie leaves
at once; its air (ticks) and peak by how long the manual was held, against sk8."""
from edge import pair
import feel
F = feel.FLOOR_Y
def air(rows, p):
    a = [i for i, r in enumerate(rows) if (r["phys_state"] in ("KnownAir", "PhysicsAir", "GroundAnimation") if p else r["mode"] == "Air")]
    return (a[0], a[-1] + 1) if a else (None, None)
if __name__ == "__main__":
    for man in (12, 16, 20, 25, 30, 40, 60, 90):
        s = f"0 {man} 0 0 0 0.5 0\n{man + 4} {man + 10} 0 0 0 -1 0\n"
        pr, sk = pair(s, man + 100, extra=("--host-map",))
        (pa, pb), (sa, sb) = air(pr, True), air(sk, False)
        pk = max(float(r["deck_py"]) - F for r in pr); spk = max(float(r["deck_py"]) for r in sk)
        print(f"manual {man:2d}: port leaves +{pa - man - 4} air {pb - pa} peak {pk:.2f} {pr[man + 4]['anim_clip']:18s}  sk8 leaves +{sa - man - 4} air {sb - sa} peak {spk:.2f} {sk[man+2]['mode']}")
