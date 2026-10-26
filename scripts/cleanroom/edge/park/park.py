"""KenneyPark: the port ridden on the real map, sk8 started at the port's state on the same mesh.
The mesh (map geometry from your own pack) is dumped to a temp dir on first use, never stored."""
import csv, io, math, os, subprocess, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
PACKDIR = "/Volumes/FreeAgent GoFlex Drive/MacOffload/Documents/haloskate-ref/cleanpack"
PACK, MAP = os.path.join(PACKDIR, "core.pack"), os.path.join(PACKDIR, "KenneyPark.skate")
BUILD = os.path.expanduser("~/cleanroom/skatecore-cpp/build")
S3, SK = os.path.join(BUILD, "s3trace"), os.path.join(BUILD, "sk8trace")
D = tempfile.mkdtemp(prefix="sk8park")
MESH = os.path.join(tempfile.gettempdir(), "sk8park_kenney.txt")
if not os.path.exists(MESH) or os.path.getsize(MESH) == 0:
    with open(MESH, "w") as f:
        subprocess.run([os.path.join(BUILD, "mapdump"), MAP], stdout=f, stderr=subprocess.DEVNULL)
# the route into the quarter pipe (pushes, a turn round), then actions from T0
LINE = "30 41 0 0 0 0 0x1000\n70 81 0 0 0 0 0x1000\n90 170 -1 0 0 0 0\n" + "".join(f"{t} {t+11} 0 0 0 0 0x1000\n" for t in (180, 215, 250, 285))
T0 = 390
def shift(script, by):
    return "".join(f"{int(a)+by} {int(b)+by} {' '.join(r)}\n" for a, b, *r in (l.split() for l in script.strip().splitlines() if l.strip()))
def port(script, ticks):
    p = os.path.join(D, "kp.txt"); open(p, "w").write(script)
    return list(csv.DictReader(io.StringIO(subprocess.run([S3, PACK, MAP, p, str(ticks)], capture_output=True, text=True).stdout)))
def sk8_from(prow, script, ticks, params=()):
    vx, vz = (float(prow[k]) for k in ("deck_vx", "deck_vz"))
    p = os.path.join(D, "kps.txt"); open(p, "w").write(shift(script, 1) or "\n")
    cmd = [SK, p, str(ticks + 1), "--mesh", MESH, "--speed", f"{math.hypot(vx, vz):.4f}",
           "--pos", f"{float(prow['deck_px']):.4f},{float(prow['deck_py']):.4f},{float(prow['deck_pz']):.4f}",
           "--facing", f"{math.atan2(vx, vz):.5f}"]
    for kv in params: cmd += ["--param", kv]
    return list(csv.DictReader(io.StringIO(subprocess.run(cmd, capture_output=True, text=True).stdout)))[1:]
def run(extra="", ticks=150, params=(), pre=""):
    """The port down the line (plus `pre`, absolute ticks) and `extra` from T0; sk8 from the port's T0 state."""
    pr = port(LINE + pre + (shift(extra, T0) if extra.strip() else ""), T0 + ticks)
    return pr[T0:], sk8_from(pr[T0], extra, ticks, params)
def sp(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
# the second line: up the same quarter pipe at an angle, slower (T2)
LINE2 = "30 41 0 0 0 0 0x1000\n70 81 0 0 0 0 0x1000\n90 170 -1 0 0 0 0\n180 191 0 0 0 0 0x1000\n215 226 0 0 0 0 0x1000\n"
T2 = 411
def run2(extra="", ticks=200, params=()):
    pr = port(LINE2 + (shift(extra, T2) if extra.strip() else ""), T2 + ticks)
    return pr[T2:], sk8_from(pr[T2], extra, ticks, params)
