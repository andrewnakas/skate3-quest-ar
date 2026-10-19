#!/usr/bin/env python3
"""Side-by-side feel comparison: the faithful port (imported-disc tier) vs the clean-room sk8 engine.

Runs the same input scenarios through both engines and prints the measured feel (push gain,
coasting, ollie height and hang time per crouch length, flip timing, carve and spin rates,
gravity in the air) with the ratio sk8/port. The goal is "feels similar": ratios near 1.0,
with every sk8 value still our own number (scripts/cleanroom/audit.py checks that).

The port only runs locally against your own converted pack; nothing from it is stored.

Usage:
  scripts/cleanroom/feel.py --pack <core.pack> [--s3trace PATH] [--sk8trace PATH]
      [--param key=value ...]   (try sk8 values without rebuilding)
"""
import argparse
import csv
import io
import math
import subprocess
import sys
import tempfile
from pathlib import Path

HOME = Path.home()
DT = 1 / 60

# The port's test world: from rest, pushes roll it off the first slope onto a long flat by tick ~240.
# Every action starts at T0 on that flat; sk8 starts on its flat floor at the port's speed at T0.
T0 = 300
PREFIX = "30 41 0 0 0 0 0x1000\n70 81 0 0 0 0 0x1000\n110 121 0 0 0 0 0x1000\n150 161 0 0 0 0 0x1000\n"

A = 0x1000


def ollie(hold, after=""):
    return f"0 {hold} 0 0 0 -1 0\n{hold + 1} {hold + 7} 0 0 0 1 0\n{after}"


def flick_path(points, start):
    """Lines placing successive stick points one tick apart from `start`."""
    out = []
    for i, (x, y) in enumerate(points):
        out.append(f"{start + i} {start + i} 0 0 {x} {y} 0")
    return "\n".join(out) + "\n"


KICKFLIP = [(0, -1)] * 30 + [(0, 0), (-0.707, 0.707), (-0.707, 0.707), (-0.707, 0.707)]

# name -> (action script relative to the action start, ticks to run, metric function name)
SCENARIOS = {
    "coast": ("", 180, "coast"),
    # A held from rolling: when the speed starts to come on, gain per second, cadence
    # (braked down first: both start above the top pushing speed)
    "push_hold": ("0 89 0 0 0 0 0x2000\n90 240 0 0 0 0 0x1000\n", 240, "push"),
    "push_tap": ("0 89 0 0 0 0 0x2000\n90 92 0 0 0 0 0x1000\n", 180, "push"),
    "ollie_tap": (ollie(4), 120, "air"),
    "ollie_half": (ollie(12), 120, "air"),
    "ollie_full": (ollie(30), 120, "air"),
    "kickflip": (flick_path(KICKFLIP, 0), 140, "air"),
    "carve_right": ("0 90 1 0 0 0 0\n", 100, "carve"),
    "spin_air": (ollie(30, "37 100 -1 0 0 0 0\n"), 130, "air"),
}


def shift(script, by):
    out = []
    for line in script.strip().splitlines():
        f = line.split()
        f[0], f[1] = str(int(f[0]) + by), str(int(f[1]) + by)
        out.append(" ".join(f))
    return "\n".join(out) + "\n"


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"{cmd[0]} failed: {r.stderr[-500:]}")
    return list(csv.DictReader(io.StringIO(r.stdout)))


def series(rows, port):
    """(tick, y, horizontal speed, vy, yaw, airborne, events) per row."""
    out = []
    for r in rows:
        vx, vy, vz = float(r["deck_vx"]), float(r["deck_vy"]), float(r["deck_vz"])
        if port:
            qx, qy, qz, qw = (float(r[k]) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
            yaw = math.atan2(2 * (qw * qy + qx * qz), 1 - 2 * (qy * qy + qx * qx))
            air = r["phys_state"] in ("KnownAir", "GroundAnimation") or "InAir" in r["motion_state"]
        else:
            yaw = float(r["yaw"])
            air = r["mode"] == "Air"
        out.append((int(r["tick"]), float(r["deck_py"]), math.hypot(vx, vz), vy, yaw, air, r.get("events", "")))
    return out


def m_coast(s):
    v0, v1 = s[0][2], s[-1][2]
    return {"speed_start": v0, "decel_mps2": (v0 - v1) / ((len(s) - 1) * DT)}


def m_air(s):
    ground = s[0][1]
    air = [x for x in s if x[5]]
    if not air:
        return {"airborne": 0}
    peak = max(x[1] for x in s) - ground
    t_air = len(air) * DT
    # gravity from the vy slope in the middle of the flight
    mid = air[len(air) // 4: 3 * len(air) // 4]
    g = -(mid[-1][3] - mid[0][3]) / ((mid[-1][0] - mid[0][0]) * DT) if len(mid) > 2 else float("nan")
    yaw_turn = 0.0
    for a, b in zip(s, s[1:]):
        d = (b[4] - a[4] + math.pi) % (2 * math.pi) - math.pi
        yaw_turn += d
    return {
        "peak_m": peak,
        "air_s": t_air,
        "takeoff_vy": max(x[3] for x in s),
        "gravity": g,
        "spin_deg": abs(math.degrees(yaw_turn)),
        "speed_after": s[-1][2],
    }


def m_push(s):
    s = s[90:]
    v0 = s[0][2]
    rise = [b[2] - a[2] for a, b in zip(s, s[1:])]
    start = next((i for i, d in enumerate(rise) if d > 0.004), len(rise))
    # strokes = separate runs of speeding up
    strokes, on = 0, False
    for d in rise:
        if d > 0.004 and not on:
            strokes += 1
        on = d > 0.004
    return {"speed_start": v0, "lag_s": start * DT, "gain_mps": max(x[2] for x in s) - v0,
            "strokes": strokes, "gain_per_s": (s[-1][2] - v0) / ((len(s) - 1) * DT)}


def m_carve(s):
    turn = 0.0
    for a, b in zip(s[:60], s[1:61]):
        turn += (b[4] - a[4] + math.pi) % (2 * math.pi) - math.pi
    return {"yaw_rate_dps": abs(math.degrees(turn)) / (60 * DT), "speed_after": s[-1][2]}


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
    print(f"{'scenario':<12} {'metric':<14} {'port':>9} {'sk8':>9} {'ratio':>7}")
    for name, (action, ticks, metric) in SCENARIOS.items():
        if a.only and a.only != name:
            continue
        port_script = tmp / f"{name}.port.txt"
        port_script.write_text(PREFIX + (shift(action, T0) if action.strip() else ""))
        port = series(run([str(a.s3trace), str(a.pack), "test-world", str(port_script), str(T0 + ticks)]), True)
        port = port[T0:]
        speed = port[0][2]
        sk8_script = tmp / f"{name}.sk8.txt"
        sk8_script.write_text(shift(action, 1) if action.strip() else "")
        sk8 = series(run([str(a.sk8trace), str(sk8_script), str(ticks + 1), "--speed", f"{speed:.4f}", *extra]), False)
        sk8 = sk8[1:]
        f = globals()[f"m_{metric}"]
        mp, ms = f(port), f(sk8)
        if name != "spin_air":
            mp.pop("spin_deg", None)  # only drift outside the spin scenario
        mp.pop("takeoff_vy", None)  # the port's deck is lifted by animation, not ballistic
        for k in mp:
            pv, sv = mp[k], ms.get(k, float("nan"))
            ratio = sv / pv if pv not in (0, 0.0) and not math.isnan(pv) else float("nan")
            flag = "" if math.isnan(ratio) or 0.85 <= ratio <= 1.15 else "  <--"
            print(f"{name:<12} {k:<14} {pv:>9.3f} {sv:>9.3f} {ratio:>7.2f}{flag}")


if __name__ == "__main__":
    main()
