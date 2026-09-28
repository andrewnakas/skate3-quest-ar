# Captured crouch and treflip experiment

The optional `assets/private/custom/crouch-treflip.json` replaces body samples in
the regular ollie crouch transition, ollie/360-shuvit anticipation loops, and the
default-style low/high 360-flip ground/air slots. Removing or renaming that file
restores stock playback on the next launch. Other styles and nollie remain stock.

The existing graph owns selection, time, blending, mirroring, attributes and
trajectory. Board and wheel samples are copied unchanged. Reparented hand/foot
targets are rebuilt relative to the stock board. This is a body-motion experiment,
not a new board animation or a complete replacement of every crouch variation.

## Capture and timing

Source: `Videos/2026-09-07 18-24-06.mp4`, 348 frames at 60 fps.
BlendCap's installed `save_mhr_data.py` processed every frame with hand inference;
347 detections succeeded. Only the final frame was missing, after the trick.
BVH generation uses the T-pose skeleton, body smoothing 0.10 s, root smoothing
0.10 s and vertical smoothing 0.025 s. Original source and capture are preserved.

Source frame numbers are one-based:

| Section | Source frames | Runtime slot timing |
| --- | --- | --- |
| Crouch down | 161–186 | 15 keys at 30 fps |
| Crouched idle | Small closed oscillation over 186–189 | 55/45 keys at 30 fps |
| Treflip ground/pop | 186–201 | 13 keys at 60 fps |
| Treflip air/catch | 201–235 | 33/28 keys at 60 fps |

The original video has no sustained crouched idle; that loop is synthesized from
the bottom pose. Foot IK follows stock contact locations in the grounded sections
and fades back at catch. The treflip uses the stock actor-relative pelvis path so
the physics jump is not applied twice. Clip exit blends toward the stock pose.
These event locations are estimates checked against the video and decoded stock
clips, not ground-truth motion capture. Controller feel still needs playtesting.

Working files and CLI scripts are in
`Documents/skate-animation-extraction/video-182406/`. The Blender preview contains
individual clip actions and a combined `PLAY THIS - Crouch, Idle, Treflip` action.
It uses the game's existing deformation skeleton; finger animation is not part
of that 35-bone runtime skeleton. The earlier directly skinned BlendCap file is
unchanged.

Asset SHA-256:
`e92bc0ebaf34f467be1da5b4cc1541ac23e7c729eac12cec899f7b641f731e98`.
The private asset is not checked into Git.

## Validation

`cargo test -p skate-game animation_pose::authored_clips --locked -- --ignored`
checks board/trajectory preservation, stock slot timing, closed idle body loops,
and rejection of incompatible skeletons/timing. These tests require private data.
Build with `Build.ps1`. A `--verify` startup capture was also completed; it is a
startup/render check, not a controller-driven trick playtest.
