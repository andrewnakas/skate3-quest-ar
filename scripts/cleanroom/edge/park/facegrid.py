"""Ollies off the quarter pipe's face at each point up it, on both lines: air time and peak over the plain run."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
def air(rows, port_):
    a = [i for i, r in enumerate(rows) if (r['phys_state'] in ('KnownAir', 'PhysicsAir') if port_ else r['mode'] == 'Air')]
    if not a: return "no air"
    i0 = i1 = a[0]
    while i1 + 1 < len(rows) and (i1 + 1) in a: i1 += 1
    return f"air {i0}-{i1} ({i1-i0})"
which = sys.argv[1] if len(sys.argv) > 1 else "fast"
fn = run if which == "fast" else run2
base, bsk = fn("", 130)
for t in range(6, 42, 3):
    pr, sk = fn(f"{t-8} {t} 0 0 0 -1 0\n{t+1} {t+7} 0 0 0 1 0", 130)
    gain = max(float(x['deck_py']) for x in pr[t:t+100]) - max(float(x['deck_py']) for x in base[t:t+100])
    sgain = max(float(x['deck_py']) for x in sk[t:t+100]) - max(float(x['deck_py']) for x in bsk[t:t+100])
    print(f"{which} t {t:2d} | port {air(pr, True):18s} gain {gain:+.2f} | sk8 {air(sk, False):18s} gain {sgain:+.2f}")
