"""Rolling fakie a long while (an ollie 180 landed backwards, braked to a crawl so the port stays on
the test world): does the port ever shuffle round to switch by itself? The flick at the end says
(up-right: KICKFLIP fakie, HEELFLIP switch)."""
from switchflick import LEAD
from fuzz import pair, tricks
import math
if __name__ == "__main__":
    for t in (200, 300, 420):
        s = LEAD + "92 122 0 0 0 0 0x2000\n" + f"{t-5} {t-1} 0 0 0 -1 0\n{t} {t} 0 0 0 0 0\n{t+1} {t+1} 0 0 0.707 0.707 0\n"
        pr, sk = pair(s, t + 40, extra=("--host-map",))
        sw = [i for i, r in enumerate(pr) if r["anim_clip"] == "B_SWITCH"][:1]
        v = math.hypot(float(pr[t-6]["deck_vx"]), float(pr[t-6]["deck_vz"]))
        print(t, f"port v {v:.2f}", tricks(pr[t-5:], True), "B_SWITCH", sw, "| sk8", f"v {float(sk[t-6]['speed']):.2f}", tricks(sk[t-5:], False),
              "switch at", next((i for i, r in enumerate(sk) if r["stance"] == "1"), None))
