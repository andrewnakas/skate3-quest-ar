"""B (brake) held 30 ticks after an ollie landing: regular (no spin) or fakie (ollie 180, stick m),
clean or sketchy: speed lost, port vs sk8."""
import math
from switchflick import LEAD
from fuzz import pair
def sp(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
if __name__ == "__main__":
    for m in (0.0, 0.7, 0.75, 0.8):
        for b0 in (92, 120):
            L = LEAD.replace("-0.8", f"-{m}") if m else LEAD.replace("31 68 -0.8 0 0 0 0\n", "")
            pr, sk = pair(L + f"{b0} {b0 + 30} 0 0 0 0 0x2000\n", b0 + 40, extra=("--host-map",))
            ev = [r["events"] for r in sk if "EV1" in r["events"]][:1]
            print(f"m {m:.2f} brake@{b0}: port {sp(pr[b0]):.2f}->{sp(pr[b0 + 32]):.2f} {pr[b0]['anim_clip']:14s} "
                  f"| sk8 {float(sk[b0]['speed']):.2f}->{float(sk[b0 + 32]['speed']):.2f} fakie={sk[b0]['fakie']} {ev}")
