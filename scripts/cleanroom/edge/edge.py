"""Edge-case probes: the port (s3trace, cleanpack) vs sk8 started at the port's state."""
import sys, os, math, csv, io, subprocess
sys.path.insert(0, os.path.expanduser("~/skate3-quest-ar/scripts/cleanroom"))
import feel
PACK = "/Volumes/FreeAgent GoFlex Drive/MacOffload/Documents/haloskate-ref/cleanpack/core.pack"
S3 = os.path.expanduser("~/cleanroom/skatecore-cpp/build/s3trace")
SK = os.path.expanduser("~/cleanroom/skatecore-cpp/build/sk8trace")
D = os.path.dirname(os.path.abspath(__file__))
def _rows(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    return list(csv.DictReader(io.StringIO(r.stdout)))
def pair(action, ticks, setup=None, sk_action=None, params=(), facing_at=0):
    """Run action (ticks relative to the start) in both. Returns (port rows from t0, sk8 rows)."""
    setup = setup or {}
    prefix, t0 = setup.get("prefix", feel.PREFIX), setup.get("t0", feel.T0)
    p = os.path.join(D, f"p{os.getpid()}.txt"); open(p, "w").write(prefix + (feel.shift(action, t0) if action.strip() else ""))
    rows = _rows([S3, PACK, "test-world", p, str(t0 + ticks)])
    r0, rf = rows[t0], rows[t0 + setup.get("facing_at", facing_at)]
    vx, vz = float(rf["deck_vx"]), float(rf["deck_vz"])
    place = ["--pos", f"{float(r0['deck_px']):.4f},0,{float(r0['deck_pz']):.4f}", "--facing", f"{math.atan2(vx, vz):.5f}"]
    for x0, y0, z0, x1, y1, z1 in setup.get("rails", []):
        place += ["--rail", f"{x0},{y0 - feel.FLOOR_Y:.4f},{z0},{x1},{y1 - feel.FLOOR_Y:.4f},{z1}"]
    speed = math.hypot(float(r0["deck_vx"]), float(r0["deck_vz"]))
    a = sk_action if sk_action is not None else action
    q = os.path.join(D, f"q{os.getpid()}.txt"); open(q, "w").write(feel.shift(a, 1) if a.strip() else "\n")
    cmd = [SK, q, str(ticks + 1), "--speed", f"{speed:.4f}", *place]
    for kv in params: cmd += ["--param", kv]
    return rows[t0:], _rows(cmd)[1:]
def hs(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
def yawp(r):
    qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    return math.degrees(math.atan2(2 * (qw * qy + qx * qz), 1 - 2 * (qy * qy + qx * qx)))
def vyaw(r): return math.degrees(math.atan2(float(r["deck_vx"]), float(r["deck_vz"])))
def show(pr, sk, lo=0, hi=None, step=5, floor=feel.FLOOR_Y):
    hi = hi or min(len(pr), len(sk))
    for i in range(lo, min(hi, len(pr), len(sk)), step):
        a, b = pr[i], sk[i]
        print(f"{i:4d} P v{hs(a):5.2f} y{float(a['deck_py'])-floor:5.2f} yaw{yawp(a):7.1f} vdir{vyaw(a):7.1f} {a['phys_state'][:11]:11s} {a['trick_name'][:14]:14s}"
              f"| S v{hs(b):5.2f} y{float(b['deck_py']):5.2f} yaw{math.degrees(float(b['yaw'])):7.1f} vdir{vyaw(b):7.1f} {b['mode'][:8]:8s} {b['events'][:40]}")
