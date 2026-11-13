"""One fuzz seed in detail: the script, then per tick the port's state beside sk8's.
Usage: seed.py SEED [from_tick] [to_tick]"""
import sys, random
from fuzz import gen, pair
seed = int(sys.argv[1]); a0 = int(sys.argv[2]) if len(sys.argv) > 2 else 0; a1 = int(sys.argv[3]) if len(sys.argv) > 3 else 160
act = gen(random.Random(seed))
print(act)
pr, sk = pair(act, 160, extra=("--host-map",))
for t in range(a0, min(a1, len(pr), len(sk))):
    p, s = pr[t], sk[t]
    print(f"{t:3d} | {p['phys_state'][:10]:10s} {p['motion_state'][:12]:12s} {p['action_state'][:14]:14s} {p['trick_name'][:20]:20s} "
          f"| {s['mode'][:8]:8s} fk={s.get('fakie','')} yaw={float(s.get('yaw',0) or 0):6.2f} {s['events'][:40]}")
