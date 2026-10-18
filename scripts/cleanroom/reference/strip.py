#!/usr/bin/env python3
"""Frame strip of a reference clip around its detected pop, with the pose skeleton drawn.

    <venv>/bin/python strip.py <video> <poses.npz> <out.png> [event index] [frames]
"""
import sys

import cv2
import numpy as np

sys.path.insert(0, __file__.rsplit("/", 1)[0])
import analyze_poses as A  # noqa: E402

BONES = [(11, 12), (11, 23), (12, 24), (23, 24), (23, 25), (25, 27), (24, 26), (26, 28), (11, 13), (13, 15),
         (12, 14), (14, 16), (27, 31), (28, 32)]

video, npz, out = sys.argv[1:4]
which = int(sys.argv[4]) if len(sys.argv) > 4 else 0
count = int(sys.argv[5]) if len(sys.argv) > 5 else 8
d = np.load(npz)
f = A.features(d["world"].astype(float))
events = A.pop_events(f, A.confident(d["image"], d["ok"]), float(d["fps"]))
if not events:
    sys.exit("no pop found")
crouch, pop, tuck, land = events[min(which, len(events) - 1)]
span = max(land - crouch, 8)
idx = np.linspace(crouch - span * 0.2, land + span * 0.3, count).astype(int).clip(0, len(d["ok"]) - 1)
cap = cv2.VideoCapture(video)
tiles = []
for i in idx:
    cap.set(cv2.CAP_PROP_POS_FRAMES, int(i))
    ok, frame = cap.read()
    if not ok:
        continue
    h, w = frame.shape[:2]
    if d["ok"][i]:
        pts = d["image"][i]
        for a, b in BONES:
            pa = (int(pts[a][0] * w), int(pts[a][1] * h))
            pb = (int(pts[b][0] * w), int(pts[b][1] * h))
            cv2.line(frame, pa, pb, (0, 255, 255), 2)
    tag = "crouch" if i <= crouch else "pop" if i <= pop else "air" if i <= tuck else "land" if i <= land else "after"
    cv2.putText(frame, f"{i} {tag}", (10, 30), cv2.FONT_HERSHEY_SIMPLEX, 1, (0, 0, 255), 2)
    tiles.append(cv2.resize(frame, (320, int(320 * h / w))))
rows = [np.hstack(tiles[k:k + 4]) for k in range(0, len(tiles) - len(tiles) % 4, 4)]
cv2.imwrite(out, np.vstack(rows))
print(f"events {len(events)}; strip of event {which}: crouch {crouch} pop {pop} tuck {tuck} land {land}")
