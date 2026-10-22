#!/usr/bin/env python3
"""Every flick trick through both engines: the faithful port (s3trace) and the clean-room sk8
(sk8trace), from the same rolling start on flat ground with the same right-stick path.

For each gesture in sk8's trick table (crates/skate-lite-sys/vendor/src/flick.cpp) the stick path
is built the way sk8's demo paths are (a 30-tick crouch on the first token, then one tick per
sample), fed to both, and the table prints what each engine made of it: the trick it named, ticks
from the flick's end to takeoff and to landing, how many times the deck went over (flips), how far
it turned about its up axis (shove-it, in half turns), and whether it bailed.

Usage:
  scripts/cleanroom/trick_table.py --pack <core.pack> [--only NAME] [--param key=value ...]
"""
import argparse
import csv
import io
import math
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HOME = Path.home()
REPO = Path(__file__).resolve().parents[2]
FLICK = REPO / "crates/skate-lite-sys/vendor/src/flick.cpp"
sys.path.insert(0, str(Path(__file__).parent))
import feel  # noqa: E402  (PREFIX, T0, run)

TOKENS = {"TUp": 0, "TUpRight": 1, "TRight": 2, "TDownRight": 3, "TDown": 4, "TDownLeft": 5, "TLeft": 6,
          "TUpLeft": 7, "TCentre": 8}
CROUCH = 30
DWELL = 6  # ticks on each token (sk8's kSweepDwell + 1: held, not swept through)


def gesture_tricks():
    out = []
    for name, toks in re.findall(r'\{"([^"]+)", \{([^}]*)\}', FLICK.read_text()):
        toks = [TOKENS[t.strip()] for t in toks.split(",") if t.strip()]
        if toks and name not in [n for n, _ in out]:
            out.append((name, toks))
    return out


def path_for(tokens):
    """The stick samples (one per tick) as sk8's demo_path makes them."""
    pts, prev = [], None
    rim = lambda sector: (round(math.sin(sector * math.pi / 4), 4), round(math.cos(sector * math.pi / 4), 4))
    for tok in tokens:
        if tok == 8:
            pts += [(0, 0), (0, 0)]
            prev = None
            continue
        if prev is not None and min(abs(tok - prev) % 8, 8 - abs(tok - prev) % 8) >= 3:
            pass  # straight across the stick
        elif prev is not None:
            d = 1 if ((tok - prev + 8) % 8) <= 4 else -1
            pts += [rim(prev + d * k / 4) for k in (1, 2, 3)]
        else:
            pts.append((0, 0))
        pts += [rim(tok)] * DWELL
        prev = tok
    return pts


def script(pts, start):
    """Lines for the trace tools: the first rim sample held for the crouch, then a tick each,
    then the stick let go. Returns (text, tick the flick ends)."""
    lines, t, first = [], start, True
    for x, y in pts:
        n = CROUCH if first and (x, y) != (0, 0) else 1
        if (x, y) != (0, 0):
            first = False
        lines.append(f"{t} {t + n - 1} 0 0 {x} {y} 0")
        t += n
    return "\n".join(lines) + "\n", t


def quat_vec(q, v):
    x, y, z, w = q
    vx, vy, vz = v
    # q * v * q^-1
    tx, ty, tz = 2 * (y * vz - z * vy), 2 * (z * vx - x * vz), 2 * (x * vy - y * vx)
    return (vx + w * tx + (y * tz - z * ty), vy + w * ty + (z * tx - x * tz), vz + w * tz + (x * ty - y * tx))


def measure(rows, port, flick_end):
    air, events = [], []
    for r in rows:
        if port:
            a = r["phys_state"] in ("KnownAir", "GroundAnimation", "PhysicsAir") or "InAir" in r["motion_state"]
        else:
            a = r["mode"] == "Air"
        air.append(a)
        events.append(r.get("events", ""))
    take = next((i for i, a in enumerate(air) if a and i >= flick_end - 2), None)
    if take is None:
        return {"takeoff": None}
    land = next((i for i in range(take, len(air)) if not air[i]), None)
    q = lambda r: tuple(float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
    flips, down, yaw_turn, prev_yaw, level = 0, False, 0.0, None, None
    # (up to the landing: after it sk8 snaps a shove-it's half turn back, the board being the same
    # either way round)
    for i in range(take - 3, land if land else len(rows)):
        if i >= len(rows):
            break
        qq = q(rows[i])
        up = quat_vec(qq, (0, 1, 0))
        nose = quat_vec(qq, (0, 0, 1))
        if not down and up[1] < -0.3:
            down = True
        elif down and up[1] > 0.9:
            down = False
            flips += 1
            level = i
        if math.hypot(nose[0], nose[2]) > 0.5:
            yaw = math.atan2(nose[0], nose[2])
            if prev_yaw is not None:
                yaw_turn += (yaw - prev_yaw + math.pi) % (2 * math.pi) - math.pi
            prev_yaw = yaw
    text = " ".join(events)
    if port:
        names = sorted(set(re.findall(r"TRICK:([A-Z0-9_]+)", text)))
        bail = "WIPEOUT" in text or any(r.get("wiping_out") == "1" for r in rows)
    else:
        names = sorted(set(re.findall(r"EV1:([A-Za-z0-9 -]+)", text)))
        bail = "EV3:" in text
    return {"takeoff": take - flick_end, "land": (land - flick_end) if land else None,
            "level": (level - flick_end) if level else None, "flips": flips,
            "shove": round(abs(math.degrees(yaw_turn)) / 180, 1), "bail": bail, "names": names}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pack", required=True, type=Path)
    ap.add_argument("--s3trace", type=Path, default=HOME / "cleanroom/skatecore-cpp/build/s3trace")
    ap.add_argument("--sk8trace", type=Path, default=HOME / "cleanroom/skatecore-cpp/build/sk8trace")
    ap.add_argument("--param", action="append", default=[])
    ap.add_argument("--only")
    a = ap.parse_args()
    extra = [x for p in a.param for x in ("--param", p)]
    tmp = Path(tempfile.mkdtemp())
    print(f"{'trick':<26} {'engine':<5} {'takeoff':>7} {'level':>6} {'land':>5} {'flips':>5} {'shove':>5} bail  named")
    for name, toks in gesture_tricks():
        if a.only and a.only != name:
            continue
        pts = path_for(toks)
        body, end_rel = script(pts, 0)
        n = end_rel + 110
        port_script = tmp / "p.txt"
        shifted, end_p = script(pts, feel.T0)
        port_script.write_text(feel.PREFIX + shifted)
        prows = feel.run([str(a.s3trace), str(a.pack), "test-world", str(port_script), str(feel.T0 + n)])
        speed = math.hypot(float(prows[feel.T0]["deck_vx"]), float(prows[feel.T0]["deck_vz"]))
        mp = measure(prows, True, end_p)
        sk8_script = tmp / "k.txt"
        body1, end_k = script(pts, 1)
        sk8_script.write_text(body1)
        krows = feel.run([str(a.sk8trace), str(sk8_script), str(n + 1), "--speed", f"{speed:.4f}", *extra])
        mk = measure(krows, False, end_k)
        for eng, m in (("port", mp), ("sk8", mk)):
            if m.get("takeoff") is None:
                print(f"{name:<26} {eng:<5} {'-':>7}  (no takeoff)")
                continue
            fmt = lambda v: "-" if v is None else str(v)
            print(f"{name:<26} {eng:<5} {fmt(m['takeoff']):>7} {fmt(m['level']):>6} {fmt(m['land']):>5} "
                  f"{m['flips']:>5} {m['shove']:>5} {'BAIL' if m['bail'] else '    '}  {', '.join(m['names'])}")


if __name__ == "__main__":
    main()
