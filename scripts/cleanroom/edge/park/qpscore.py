"""One score for quarter-pipe parity: mean |log(sk8/port)| of exit speeds over plain roll-ups,
angled lines and lip ollies (lower is better)."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qpturn import *
def exit_v(rows):
    ys = [float(r["deck_py"]) for r in rows]; ip = ys.index(max(ys))
    j = next((i for i in range(ip, len(rows)) if ys[i] < ys[0] + 0.03), len(rows) - 1)
    return sp(rows[min(j + 3, len(rows) - 1)])
cases = []
for brake, s in ((45, 0), (60, 0), (20, -0.6), (20, 0), (20, 0.6)):
    pre = f"{T0-75} {T0-75+brake} 0 0 0 0 0x2000\n" + (f"{T0-28} {T0-16} {s} 0 0 0 0\n" if s else "")
    cases.append(("run", pre, ""))
for brake in (0, 15, 35, 45):
    for act in ("22 30 0 0 0 -1 0\n31 37 0 0 0 1 0", "28 36 0 0 0 -1 0\n37 43 0 0 0 1 0"):
        cases.append(("run", f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else "", act))
errs = []
out = []
for kind, pre, act in cases:
    pr, sk = run(act, 240, pre=pre)
    a, b = exit_v(pr), exit_v(sk)
    errs.append(abs(math.log(max(b, 0.1) / max(a, 0.1)))); out.append(f"{a:.1f}/{b:.1f}")
pr, sk = run2("", 260); a, b = exit_v(pr), exit_v(sk)
errs.append(abs(math.log(b / a))); out.append(f"{a:.1f}/{b:.1f}")
print(f"score {sum(errs)/len(errs):.3f}  " + " ".join(out))
