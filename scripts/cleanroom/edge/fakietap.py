"""Push pressed for N ticks while rolling fakie (after an ollie 180 landed backwards): does the
port shuffle round to switch (B_SWITCH clip), and how much speed does it gain, against sk8."""
import math
from fuzz import pair
LEAD = "0 25 0 0 0 -1 0\n26 26 0 0 0 0 0\n27 27 0 0 0 1 0\n31 68 -0.8 0 0 0 0\n"
def sp(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
if __name__ == "__main__":
  for n in (1, 2, 3, 4, 5, 6, 8, 10, 12, 16):
      pr, sk = pair(LEAD + f"88 {88 + n - 1} 0 0 0 0 0x1000\n", 200, extra=("--host-map",))
      sw = any(r["anim_clip"] == "B_SWITCH" for r in pr[88:200])
      pclips = sorted(set(r["anim_clip"] for r in pr[88:200] if "PUSH" in r["anim_clip"]))
      print(f"A x{n:2d}: port switch={sw} {pclips} v {sp(pr[87]):.2f}->{sp(pr[199]):.2f}   "
            f"sk8 stance={sk[199]['stance']} fakie={sk[199]['fakie']} v {float(sk[87]['speed']):.2f}->{float(sk[199]['speed']):.2f}")
