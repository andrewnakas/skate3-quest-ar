#!/usr/bin/env python3
"""Pose-estimate reference skateboarding clips (MediaPipe Pose Landmarker, heavy model).

    <venv>/bin/python extract_poses.py <reference dir>

For every videos/<trick>/<id>.(mp4|webm) under the reference dir, writes poses/<trick>/<id>.npz:
  world  (frames, 33, 3)  metric 3D joints, hip-centred, y up (MediaPipe world landmarks, y flipped)
  image  (frames, 33, 3)  normalised image x, y (down) and visibility
  ok     (frames,)        a person was found
  fps    frame rate
Only the most prominent person is tracked. The clips and these files are local reference data for
authoring our own animation; nothing from them ships.
"""
import sys
from pathlib import Path

import cv2
import mediapipe as mp
import numpy as np

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else Path.home() / "cleanroom/skate-reference")
MODEL = ROOT / "pose_landmarker_heavy.task"
MAX_SIDE = 960


def landmarker():
    o = mp.tasks.vision.PoseLandmarkerOptions(
        base_options=mp.tasks.BaseOptions(model_asset_path=str(MODEL)),
        running_mode=mp.tasks.vision.RunningMode.VIDEO,
        num_poses=1,
        min_pose_detection_confidence=0.5,
        min_tracking_confidence=0.5,
    )
    return mp.tasks.vision.PoseLandmarker.create_from_options(o)


def extract(video, out):
    cap = cv2.VideoCapture(str(video))
    fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    world, image, ok = [], [], []
    with landmarker() as lm:
        i = 0
        while True:
            good, frame = cap.read()
            if not good:
                break
            h, w = frame.shape[:2]
            s = MAX_SIDE / max(h, w)
            if s < 1:
                frame = cv2.resize(frame, (int(w * s), int(h * s)))
            rgb = mp.Image(image_format=mp.ImageFormat.SRGB, data=cv2.cvtColor(frame, cv2.COLOR_BGR2RGB))
            r = lm.detect_for_video(rgb, int(i * 1000 / fps))
            if r.pose_world_landmarks:
                wl = r.pose_world_landmarks[0]
                il = r.pose_landmarks[0]
                world.append([(p.x, -p.y, p.z) for p in wl])
                image.append([(p.x, p.y, p.visibility) for p in il])
                ok.append(True)
            else:
                world.append(np.zeros((33, 3)))
                image.append(np.zeros((33, 3)))
                ok.append(False)
            i += 1
    cap.release()
    out.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(out, world=np.array(world, np.float32), image=np.array(image, np.float32),
                        ok=np.array(ok), fps=fps)
    return i, sum(ok)


def main():
    for video in sorted((ROOT / "videos").glob("*/*")):
        if video.suffix not in (".mp4", ".webm", ".mkv"):
            continue
        out = ROOT / "poses" / video.parent.name / (video.stem + ".npz")
        if out.exists():
            continue
        n, found = extract(video, out)
        print(f"{video.parent.name}/{video.name}: {found}/{n} frames with a pose", flush=True)


if __name__ == "__main__":
    main()
