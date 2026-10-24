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
HEELFLIP = [(0, -1)] * 30 + [(0, 0), (0.707, 0.707), (0.707, 0.707), (0.707, 0.707)]
# the flick's end kept there through the air (held: more turns)
KICKFLIP_HELD = KICKFLIP + [(-0.707, 0.707)] * 45

# The port's test world has rails: the low one runs along z at x -7, 0.45 m over the floor. The rail
# scenarios steer round on the long flat and come back down it (sidestep, U-turn, straighten), then
# start at tick 434 there; sk8 starts at the port's spot and speed on its floor with the same rail.
FLOOR_Y = -1.535  # the port's floor height (sk8's floor is y 0)
RAIL_PREFIX = PREFIX + "245 263 -1 0 0 0 0\n264 286 1 0 0 0 0\n290 412 1 0 0 0 0\n413 425 -1 0 0 0 0\n"
LOW_RAIL = (-7.0, FLOOR_Y + 0.45, 7.0, -7.0, FLOOR_Y + 0.45, 15.0)
# (the port is still easing out of the turn at t0: sk8 takes its heading a few ticks on, settled)
ON_RAIL = {"prefix": RAIL_PREFIX, "t0": 434, "rails": [LOW_RAIL], "facing_at": 10}

# name -> (action script relative to the action start, ticks to run, metric function name[, setup])
SCENARIOS = {
    "coast": ("", 150, "coast"),  # (the port rolls off the world past ~170)
    # A held from rolling: when the speed starts to come on, gain per second, cadence
    # (braked down first: both start above the top pushing speed)
    "push_hold": ("0 89 0 0 0 0 0x2000\n90 240 0 0 0 0 0x1000\n", 240, "push"),
    "push_tap": ("0 89 0 0 0 0 0x2000\n90 92 0 0 0 0 0x1000\n", 180, "push"),
    "ollie_tap": (ollie(4), 120, "air"),
    "ollie_half": (ollie(12), 120, "air"),
    "pop_down50": ("0 12 0 0 0 -0.50 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_down60": ("0 12 0 0 0 -0.60 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_down70": ("0 12 0 0 0 -0.70 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_down80": ("0 12 0 0 0 -0.80 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_down90": ("0 12 0 0 0 -0.90 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_tap34": ("0 5 0 0 0 -0.34 0\n6 12 0 0 0 1 0\n", 120, "air"),
    "pop_tap40": ("0 5 0 0 0 -0.40 0\n6 12 0 0 0 1 0\n", 120, "air"),
    "pop_tap60": ("0 5 0 0 0 -0.60 0\n6 12 0 0 0 1 0\n", 120, "air"),
    "pop_tap80": ("0 5 0 0 0 -0.80 0\n6 12 0 0 0 1 0\n", 120, "air"),
    "pop_tap90": ("0 5 0 0 0 -0.90 0\n6 12 0 0 0 1 0\n", 120, "air"),
    "pop_deep88": ("0 12 0 0 0 -0.88 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_deep89": ("0 12 0 0 0 -0.89 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "pop_deep95": ("0 12 0 0 0 -0.95 0\n13 19 0 0 0 1 0\n", 120, "air"),
    "flip_part6": (flick_path([(0, -1)] * 12 + [(0, 0)] + [(-0.424, 0.424)] * 7, 0), 120, "air"),
    "flip_part7": (flick_path([(0, -1)] * 12 + [(0, 0)] + [(-0.495, 0.495)] * 7, 0), 120, "air"),
    "flip_part8": (flick_path([(0, -1)] * 12 + [(0, 0)] + [(-0.566, 0.566)] * 7, 0), 120, "air"),
    "flipk_c2": (flick_path([(0, -1)] * 2 + [(0, 0), (-0.707, 0.707), (-0.707, 0.707), (-0.707, 0.707)], 0), 120, "flip"),
    "fliph_c2": (flick_path([(0, -1)] * 2 + [(0, 0)] + [(-0.707, 0.707)] * 48, 0), 120, "flip"),
    "flipk_c6": (flick_path([(0, -1)] * 6 + [(0, 0), (-0.707, 0.707), (-0.707, 0.707), (-0.707, 0.707)], 0), 120, "flip"),
    "fliph_c6": (flick_path([(0, -1)] * 6 + [(0, 0)] + [(-0.707, 0.707)] * 48, 0), 120, "flip"),
    "flipk_c12": (flick_path([(0, -1)] * 12 + [(0, 0), (-0.707, 0.707), (-0.707, 0.707), (-0.707, 0.707)], 0), 120, "flip"),
    "fliph_c12": (flick_path([(0, -1)] * 12 + [(0, 0)] + [(-0.707, 0.707)] * 48, 0), 120, "flip"),
    "heldA_c35": (flick_path([(0, 0)] + [(0, -1)] * 35 + [(0.707, 0.707)] * 110, 0), 150, "flip"),
    "heldB_tiny": (flick_path([(0, -1)] * 2 + [(0, 0)] + [(0.707, 0.707)] * 110, 0), 150, "flip"),
    "manpose_35": ("0 90 0 0 0 -0.35 0\n", 100, "pose"),
    "manpose_55": ("0 90 0 0 0 -0.55 0\n", 100, "pose"),
    "manpose_85": ("0 90 0 0 0 -0.85 0\n", 100, "pose"),
    "manpose_nose80": ("0 90 0 0 0 0.8 0\n", 100, "pose"),
    "manpose_step": ("0 40 0 0 0 -0.4 0\n41 100 0 0 0 -0.8 0\n", 110, "pose"),
    "grpose_m055": ("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n16 160 0 0 0 -0.55 0\n", 160, "pose", ON_RAIL),
    "grpose_m07": ("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n16 160 0 0 0 -0.7 0\n", 160, "pose", ON_RAIL),
    "grpose_m1": ("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n16 160 0 0 0 -1 0\n", 160, "pose", ON_RAIL),
    "grpose_07": ("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n16 160 0 0 0 0.7 0\n", 160, "pose", ON_RAIL),
    "grpose_1": ("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n16 160 0 0 0 1 0\n", 160, "pose", ON_RAIL),
    "pop_up61": ("0 12 0 0 0 -1 0\n13 19 0 0 0 0.61 0\n", 120, "air"),
    "pop_up50": ("0 12 0 0 0 -1 0\n13 19 0 0 0 0.50 0\n", 120, "air"),
    "pop_up60": ("0 12 0 0 0 -1 0\n13 19 0 0 0 0.60 0\n", 120, "air"),
    "pop_up70": ("0 12 0 0 0 -1 0\n13 19 0 0 0 0.70 0\n", 120, "air"),
    "pop_up80": ("0 12 0 0 0 -1 0\n13 19 0 0 0 0.80 0\n", 120, "air"),
    "pop_up90": ("0 12 0 0 0 -1 0\n13 19 0 0 0 0.90 0\n", 120, "air"),
    "ollie_full": (ollie(30), 120, "air"),
    "kickflip": (flick_path(KICKFLIP, 0), 140, "air"),
    "flip_kick": (flick_path(KICKFLIP, 0), 140, "flip"),
    "flip_heel": (flick_path(HEELFLIP, 0), 140, "flip"),
    "flip_held": (flick_path(KICKFLIP_HELD, 0), 160, "flip"),
    "carve_right": ("0 90 1 0 0 0 0\n", 100, "carve"),
    # part-way sticks: the pad's dead zone and scaling (half a stick is not half a turn)
    "carve_half": ("0 90 0.5 0 0 0 0\n", 100, "carve"),
    "carve_70": ("0 90 0.7 0 0 0 0\n", 100, "carve"),
    "carve_tiny": ("0 90 0.2 0 0 0 0\n", 100, "carve"),
    "carve_35": ("0 90 0.35 0 0 0 0\n", 100, "carve"),
    "carve_40": ("0 90 0.40 0 0 0 0\n", 100, "carve"),
    "carve_45": ("0 90 0.45 0 0 0 0\n", 100, "carve"),
    "carve_55": ("0 90 0.55 0 0 0 0\n", 100, "carve"),
    "carve_60": ("0 90 0.60 0 0 0 0\n", 100, "carve"),
    "carve_65": ("0 90 0.65 0 0 0 0\n", 100, "carve"),
    "carve_75": ("0 90 0.75 0 0 0 0\n", 100, "carve"),
    "carve_80": ("0 90 0.80 0 0 0 0\n", 100, "carve"),
    "carve_85": ("0 90 0.85 0 0 0 0\n", 100, "carve"),
    "carve_90": ("0 90 0.90 0 0 0 0\n", 100, "carve"),
    "carve_95": ("0 90 0.95 0 0 0 0\n", 100, "carve"),
    "spin_40": (ollie(30, "37 100 -0.40 0 0 0 0\n"), 130, "air"),
    "spin_50": (ollie(30, "37 100 -0.50 0 0 0 0\n"), 130, "air"),
    "spin_70": (ollie(30, "37 100 -0.70 0 0 0 0\n"), 130, "air"),
    "spin_80": (ollie(30, "37 100 -0.80 0 0 0 0\n"), 130, "air"),
    "spin_90": (ollie(30, "37 100 -0.90 0 0 0 0\n"), 130, "air"),
    "spin_air": (ollie(30, "37 100 -1 0 0 0 0\n"), 130, "air"),
    "spin_half": (ollie(30, "37 100 -0.6 0 0 0 0\n"), 130, "air"),
    # the left stick held back to a diagonal: a powerslide from full speed (speed kept at 0.25/0.5/1/2 s)
    "powerslide": ("0 120 0.7 -0.7 0 0 0\n", 122, "slide"),
    # the right stick part-way down / up and held still: a manual / nose manual, no balancing
    # (the port's last ~2.6 s before it rolls off the world's end; time held and the slow-down)
    "manual_hold": ("0 150 0 0 0 -0.5 0\n", 155, "manual"),
    "manual_nose": ("0 150 0 0 0 0.5 0\n", 155, "manual"),
    # an ollie onto the low rail: a 50-50 down its length, rolling off the end
    "grind_low": ("0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n", 160, "grind", ON_RAIL),
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
        qx, qy, qz, qw = (float(r.get(k) or 0) for k in ("deck_qx", "deck_qy", "deck_qz", "deck_qw"))
        board_up_y = 1 - 2 * (qx * qx + qz * qz)  # the deck's up axis, its height part
        grind = r["phys_state"].startswith("Grind") if port else r["mode"] == "Grind"
        if not port:
            air = air or r["mode"] == "Grind"
        manual = "Manual" in r["motion_state"] if port else r["mode"] == "Manual"
        # the deck's nose up (+) or down, degrees: its forward axis's height part
        nose_y = 2 * (qy * qz - qw * qx)
        pitch = math.degrees(math.asin(max(-1.0, min(1.0, nose_y))))
        out.append((int(r["tick"]), float(r["deck_py"]), math.hypot(vx, vz), vy, yaw, air, r.get("events", ""),
                    board_up_y, grind, manual, pitch))
    return out


def m_pose(s):
    """The deck's pitch through a grind or manual: at its start and 10 / 20 / 40 / 60 ticks in
    (the stick picks how far it tips, and it eases there)."""
    on = [i for i, x in enumerate(s) if x[8] or x[9]]
    if not on:
        return {"held": 0}
    i0 = on[0]
    out = {"ticks_on": len(on)}
    for k in (0, 10, 20, 40, 60):
        if i0 + k < len(s) and (s[i0 + k][8] or s[i0 + k][9]):
            out[f"pitch_{k}"] = abs(s[i0 + k][10])
    return out


def m_coast(s):
    # averaged over the first / last 20 ticks: the port's deck speed jitters a little
    v0 = sum(x[2] for x in s[:20]) / 20
    v1 = sum(x[2] for x in s[-20:]) / 20
    return {"speed_start": s[0][2], "decel_mps2": (v0 - v1) / ((len(s) - 20) * DT)}


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
    """From the push press (tick 90, after braking): when the first stroke's speed comes on, what
    the first two strokes give, and the time between them."""
    s = s[90:]
    v0 = s[0][2]
    bursts, i = [], 1
    while i < len(s):
        if s[i][2] - s[i - 1][2] > 0.02:
            j = i
            while j + 1 < len(s) and s[j + 1][2] - s[j][2] > 0.005:
                j += 1
            if bursts and i - bursts[-1][3] < 12:  # the same stroke biting on
                bursts[-1] = (bursts[-1][0], bursts[-1][1], s[j][2], j)
            else:
                bursts.append((i, s[i - 1][2], s[j][2], j))
            i = j + 1
        else:
            i += 1
    out = {"speed_start": v0, "first_s": bursts[0][0] * DT if bursts else float("nan")}
    for k, (t, a, b, _) in enumerate(bursts[:2]):
        out[f"gain{k + 1}"] = b - a
    if len(bursts) > 1:
        out["stroke_s"] = (bursts[1][0] - bursts[0][0]) * DT
    return out


def m_flip(s):
    """Ticks from the flick's end (tick 33 of the path) to takeoff, to the deck upside down, to it
    level again, and how many times it went over."""
    end = 33
    air = next((i for i, x in enumerate(s) if x[5]), None)
    flips, down, first_down, last_up = 0, False, None, None
    for i, x in enumerate(s):
        if not down and x[7] < -0.3:
            down = True
            first_down = first_down if first_down is not None else i
        elif down and x[7] > 0.9:
            down = False
            flips += 1
            last_up = i
    return {"takeoff_ticks": (air - end) if air is not None else float("nan"),
            "upside_down": (first_down - end) if first_down is not None else float("nan"),
            "level_again": (last_up - end) if last_up is not None else float("nan"),
            "flips": flips}


def m_grind(s):
    """Speed kept catching the rail, the slow-down along it, time on it, and the speed kept rolling
    off its end and landing (on the ground 20 ticks later)."""
    on = [i for i, x in enumerate(s) if x[8]]
    if not on:
        return {"grinding": 0}
    a, b = on[0], on[-1]
    land = next((i for i in range(b + 1, len(s)) if not s[i][5] and not s[i][8]), None)
    after = s[min(land + 20, len(s) - 1)][2] if land is not None else float("nan")
    up = next((i for i, x in enumerate(s) if x[5]), a)
    return {"catch_ticks": a - up,  # from leaving the ground to on the rail
            "catch_keep": s[a + 2][2] / s[a - 2][2],
            "rail_decel": (s[a + 3][2] - s[b - 3][2]) / ((b - a - 6) * DT),
            "rail_s": (b - a + 1) * DT,
            "off_keep": after / s[b][2]}


def m_manual(s):
    on = [i for i, x in enumerate(s) if x[9]]
    if not on:
        return {"manual_s": 0}
    a, b = on[0], on[-1]
    return {"engage_ticks": a, "manual_s": (b - a + 1) * DT,
            "decel_mps2": (s[a][2] - s[b][2]) / (max(b - a, 1) * DT)}


def m_slide(s):
    v0 = s[0][2]
    return {f"keep_{t}": s[t][2] / v0 for t in (15, 30, 60, 120)}


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
    for name, (action, ticks, metric, *setup) in SCENARIOS.items():
        if a.only and not __import__("re").fullmatch(a.only, name):
            continue
        setup = setup[0] if setup else {}
        prefix, t0 = setup.get("prefix", PREFIX), setup.get("t0", T0)
        port_script = tmp / f"{name}.port.txt"
        port_script.write_text(prefix + (shift(action, t0) if action.strip() else ""))
        rows = run([str(a.s3trace), str(a.pack), "test-world", str(port_script), str(t0 + ticks)])
        port = series(rows, True)[t0:]
        speed = port[0][2]
        place = []
        if setup.get("rails"):
            r0, rf = rows[t0], rows[t0 + setup.get("facing_at", 0)]
            vx, vz = float(rf["deck_vx"]), float(rf["deck_vz"])
            place = ["--pos", f"{float(r0['deck_px']):.4f},0,{float(r0['deck_pz']):.4f}",
                     "--facing", f"{math.atan2(vx, vz):.5f}"]
            for rail in setup["rails"]:
                x0, y0, z0, x1, y1, z1 = rail
                place += ["--rail", f"{x0},{y0 - FLOOR_Y:.4f},{z0},{x1},{y1 - FLOOR_Y:.4f},{z1}"]
        sk8_script = tmp / f"{name}.sk8.txt"
        sk8_script.write_text(shift(action, 1) if action.strip() else "")
        sk8 = series(run([str(a.sk8trace), str(sk8_script), str(ticks + 1), "--speed", f"{speed:.4f}", *place,
                          *extra]), False)
        sk8 = sk8[1:]
        f = globals()[f"m_{metric}"]
        mp, ms = f(port), f(sk8)
        if not name.startswith("spin_"):
            mp.pop("spin_deg", None)  # only drift outside the spin scenario
        mp.pop("takeoff_vy", None)  # the port's deck is lifted by animation, not ballistic
        for k in mp:
            pv, sv = mp[k], ms.get(k, float("nan"))
            ratio = sv / pv if pv not in (0, 0.0) and not math.isnan(pv) else float("nan")
            flag = "" if math.isnan(ratio) or 0.85 <= ratio <= 1.15 else "  <--"
            print(f"{name:<12} {k:<14} {pv:>9.3f} {sv:>9.3f} {ratio:>7.2f}{flag}")


if __name__ == "__main__":
    main()
