"""From a standstill at KenneyPark's spawn: the port (s3trace on your map) vs sk8 (sk8trace on the same
park's mesh, same spot, heading -Z, speed 0). rest_pair(action, ticks) -> (port rows, sk8 rows), tick 0 aligned."""
import sys, os, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
from edge import _rows, D
MAP = os.path.join(os.path.dirname(PACK), "KenneyPark.skate")
MESH = os.path.expanduser("~/skate-compare/kenney_mesh.txt")
SPAWN = ("13.5,0.1,-1.5", "3.14159")
def rest_pair(action, ticks, extra=()):
    if not os.path.exists(MESH):
        os.makedirs(os.path.dirname(MESH), exist_ok=True)
        open(MESH, "w").write(subprocess.run([os.path.expanduser("~/cleanroom/skatecore-cpp/build/mapdump"), MAP], capture_output=True, text=True).stdout)
    p = os.path.join(D, f"rp{os.getpid()}.txt"); open(p, "w").write(action if action.strip() else "\n")
    pr = _rows([S3, PACK, MAP, p, str(ticks)])
    sk = _rows([SK, p, str(ticks), "--mesh", MESH, "--pos", SPAWN[0], "--facing", SPAWN[1], *extra, "--host-map"])
    return pr, sk
def hs3(r): return math.hypot(float(r["deck_vx"]), float(r["deck_vz"]))
if __name__ == "__main__":
    for name, act in {"tap": "0 3 0 0 0 0 0x1000\n", "tap@20": "20 23 0 0 0 0 0x1000\n", "hold": "0 300 0 0 0 0 0x1000\n",
                      "2 taps": "0 3 0 0 0 0 0x1000\n40 43 0 0 0 0 0x1000\n"}.items():
        pr, sk = rest_pair(act, 160)
        print(f"{name:7s} P " + " ".join(f"{hs3(pr[i]):4.2f}" for i in range(0, 160, 8)))
        print(f"{'':7s} S " + " ".join(f"{hs3(sk[i]):4.2f}" for i in range(0, 160, 8)))
