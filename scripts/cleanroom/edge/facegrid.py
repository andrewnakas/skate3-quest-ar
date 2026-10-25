import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qp import run as run1
from qp2 import run2
def rot(q, v):
    x,y,z,w=q; vx,vy,vz=v
    ix= w*vx + y*vz - z*vy; iy= w*vy + z*vx - x*vz; iz= w*vz + x*vy - y*vx; iw=-x*vx - y*vy - z*vz
    return (ix*w + iw*-x + iy*-z - iz*-y, iy*w + iw*-y + iz*-x - ix*-z, iz*w + iw*-z + ix*-y - iy*-x)
def q(r): return tuple(float(r[k]) for k in ("deck_qx","deck_qy","deck_qz","deck_qw"))
def air(rows, port, after=0):
    a = [i for i, r in enumerate(rows) if i >= after and (r['phys_state'] in ('KnownAir','PhysicsAir') if port else r['mode'] == 'Air')]
    if not a: return "no air"
    i0 = a[0]; i1 = a[0]
    while i1 + 1 < len(rows) and (i1 + 1) in a: i1 += 1
    return f"air {i0}-{i1} ({i1-i0})"
which = sys.argv[1]
fn = run1 if which == "fast" else run2
base, bsk = fn("", 130)
for t in range(6, 42, 3):
    act = f"{t-8} {t} 0 0 0 -1 0\n{t+1} {t+7} 0 0 0 1 0"
    pr, sk = fn(act, 130)
    r = base[t + 4]
    u = rot(q(r), (0, 1, 0)); v = math.sqrt(sum(float(r[k])**2 for k in ("deck_vx", "deck_vy", "deck_vz")))
    # peak gain over the no-pop run, first 100 ticks
    gain = max(float(x['deck_py']) for x in pr[t:t+100]) - max(float(x['deck_py']) for x in base[t:t+100])
    sgain = max(float(x['deck_py']) for x in sk[t:t+100]) - max(float(x['deck_py']) for x in bsk[t:t+100])
    print(f"{which} t {t:2d} up_y {u[1]:.2f} v {v:4.1f} y {float(r['deck_py']):.2f} | port {air(pr, True):28s} gain {gain:+.2f} | sk8 {air(sk, False):28s} gain {sgain:+.2f}")
