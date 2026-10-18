#!/usr/bin/env python3
"""Turn pose-estimated reference clips into median key-pose measurements per trick.

    python3 analyze_poses.py <reference dir>  > reference_poses.json

Camera-independent body measurements per frame (all lengths divided by the skater's leg length):
  ext        hip-to-ankle distance (mean of both legs): 1 = straight legs, low = crouched
  hip_h      hip height above the lower ankle
  tuck       mean ankle height relative to the hips (higher = feet pulled up)
  lift_diff  height difference between the two ankles
  width      ankle separation along the hip axis (stance width)
  trunk      trunk pitch from vertical, degrees (positive = bent forward)
  arm_h      mean wrist height relative to the shoulders
  arm_out    mean wrist distance out from the shoulders along the hip axis
  knee       mean knee flexion, degrees (0 = straight)
Pop-type tricks are cut at events (crouch minimum, pop = extension peak, tuck peak, landing
absorb) and the median of each event across every clip is reported; steady tricks (push, manual,
grinds, slides) report medians over all confident frames. Medians over many clips and skaters are
the targets for our own keyframed poses; no single performance is reproduced.
"""
import json
import sys
from pathlib import Path

import numpy as np

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else Path.home() / "cleanroom/skate-reference")
L_SH, R_SH, L_WR, R_WR, L_HIP, R_HIP, L_KN, R_KN, L_AN, R_AN = 11, 12, 15, 16, 23, 24, 25, 26, 27, 28
POP_TRICKS = {"ollie", "kickflip", "heelflip", "shuvit", "grab", "landing"}


def angle(a, b, c):
    u, v = a - b, c - b
    cos = np.sum(u * v, -1) / (np.linalg.norm(u, axis=-1) * np.linalg.norm(v, axis=-1) + 1e-9)
    return 180 - np.degrees(np.arccos(np.clip(cos, -1, 1)))


def features(world):
    hip = (world[:, L_HIP] + world[:, R_HIP]) / 2
    sh = (world[:, L_SH] + world[:, R_SH]) / 2
    leg = (np.linalg.norm(world[:, L_HIP] - world[:, L_KN], axis=-1) + np.linalg.norm(world[:, L_KN] - world[:, L_AN], axis=-1)
           + np.linalg.norm(world[:, R_HIP] - world[:, R_KN], axis=-1) + np.linalg.norm(world[:, R_KN] - world[:, R_AN], axis=-1)) / 2
    leg = np.median(leg)
    axis = world[:, L_HIP] - world[:, R_HIP]
    axis[:, 1] = 0
    axis /= np.linalg.norm(axis, axis=-1, keepdims=True) + 1e-9
    ext = (np.linalg.norm(world[:, L_HIP] - world[:, L_AN], axis=-1) + np.linalg.norm(world[:, R_HIP] - world[:, R_AN], axis=-1)) / 2 / leg
    ank_y = np.stack([world[:, L_AN, 1], world[:, R_AN, 1]], -1)
    hip_h = (hip[:, 1] - ank_y.min(-1)) / leg
    tuck = (ank_y.mean(-1) - hip[:, 1]) / leg + 1  # 0 = feet a leg below the hips
    lift_diff = np.abs(ank_y[:, 0] - ank_y[:, 1]) / leg
    width = np.abs(np.sum((world[:, L_AN] - world[:, R_AN]) * axis, -1)) / leg
    up = sh - hip
    trunk = np.degrees(np.arctan2(np.hypot(up[:, 0], up[:, 2]), up[:, 1]))
    wr = np.stack([world[:, L_WR], world[:, R_WR]], 1)
    shs = np.stack([world[:, L_SH], world[:, R_SH]], 1)
    arm_h = np.mean(wr[..., 1] - shs[..., 1], -1) / leg
    arm_out = np.mean(np.abs(np.sum((wr - shs) * axis[:, None], -1)), -1) / leg
    knee = (angle(world[:, L_HIP], world[:, L_KN], world[:, L_AN]) + angle(world[:, R_HIP], world[:, R_KN], world[:, R_AN])) / 2
    return {"ext": ext, "hip_h": hip_h, "tuck": tuck, "lift_diff": lift_diff, "width": width, "trunk": trunk,
            "arm_h": arm_h, "arm_out": arm_out, "knee": knee}


def smooth(x, k=5):
    if len(x) < k:
        return x
    return np.convolve(np.pad(x, k // 2, mode="edge"), np.ones(k) / k, mode="valid")


def confident(image, ok):
    vis = image[:, [L_HIP, R_HIP, L_KN, R_KN, L_AN, R_AN], 2].min(-1)
    return ok & (vis > 0.5)


def pop_events(f, good, fps):
    """Indices of (crouch, pop, tuck, land) for each ollie-like move found. Clips run at unknown
    speeds (slow motion is common), so several time scales are tried and overlaps dropped."""
    ext = smooth(np.where(good, f["ext"], np.nan))
    tuck = smooth(np.where(good, f["tuck"], np.nan))
    found = []
    for win in (3, 6, 12, 24, 48):
        i = win
        while i < len(ext) - win:
            seg = ext[i - win:i + win + 1]
            if np.isnan(ext[i]) or np.nanmax(seg) != ext[i] or ext[i] < 0.9:
                i += 1
                continue
            before = ext[max(0, i - 6 * win):i]
            after_t = tuck[i:i + 10 * win]
            if len(before) < 3 or np.all(np.isnan(before)) or len(after_t) < 3 or np.all(np.isnan(after_t)):
                i += 1
                continue
            crouch = max(0, i - 6 * win) + int(np.nanargmin(before))
            tk = i + int(np.nanargmax(after_t))
            after_e = ext[tk:tk + 8 * win]
            if len(after_e) < 3 or np.all(np.isnan(after_e)):
                i += 1
                continue
            land = tk + int(np.nanargmin(after_e))
            # a real pop: crouched before, legs pulled up after
            if ext[i] - ext[crouch] > 0.08 and tuck[tk] - tuck[i] > 0.08:
                if not any(a <= i <= d for a, _, _, d in found):
                    found.append((crouch, i, tk, land))
                i = land + win
            else:
                i += 1
    return sorted(found)


def summarise(rows):
    keys = rows[0].keys()
    return {k: round(float(np.median([r[k] for r in rows])), 3) for k in keys} | {"n": len(rows)}


def main():
    out = {}
    for trick_dir in sorted((ROOT / "poses").glob("*")):
        trick = trick_dir.name
        per_event = {"crouch": [], "pop": [], "tuck": [], "land": []}
        steady = []
        clips = 0
        for npz in sorted(trick_dir.glob("*.npz")):
            d = np.load(npz)
            if d["ok"].sum() < 10:
                continue
            clips += 1
            world = d["world"].astype(float)
            f = features(world)
            good = confident(d["image"], d["ok"])
            if trick in POP_TRICKS:
                for ev in pop_events(f, good, float(d["fps"])):
                    for name, idx in zip(per_event, ev):
                        per_event[name].append({k: float(v[idx]) for k, v in f.items()})
            else:
                for idx in np.nonzero(good)[0]:
                    steady.append({k: float(v[idx]) for k, v in f.items()})
        entry = {"clips": clips}
        if trick in POP_TRICKS:
            for name, rows in per_event.items():
                if rows:
                    entry[name] = summarise(rows)
        elif steady:
            entry["steady"] = summarise(steady)
        out[trick] = entry
    json.dump(out, sys.stdout, indent=1)
    print()


if __name__ == "__main__":
    main()
