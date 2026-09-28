# Hybrid ledge climbing

Get off the board with **Y**, approach a reachable wall, and press **X** (the existing offboard jump button). The skater jumps, grabs the top, and stays hanging. Release X and press it again to climb up. Normal walking resumes on top. Holding the first press does not automatically climb.

The default course has a **green, 2.2 metre practice block beside the starting platform, opposite the half-pipe**, on the lower floor. Approach it from that floor, facing its wall, Jump towards it with a little run-up; gentle arm anticipation starts up to 2.6 metres from a valid edge and grows with proximity. Jump before running into the wall so the stock jump retains its forward momentum. Collision triangles drive the detection, so imported maps can use the same mechanic without named climbable objects.

## Current scope

- Grounded takeoff onto static, near-vertical walls with a flat top roughly 1.9–2.75 metres above the feet.
- Room for both hands, about 0.9 metres of supported depth, and clear standing headroom are required.
- The held board blends onto the back during climbing and returns to the carry pose. A dropped board continues simulating independently.
- Hanging uses the final reach pose. There is no separate breathing idle, shimmy, or drop button yet. A requested teleport cancels climbing.
- The skater rig has no separate finger bones, so these clips cannot close individual fingers around the edge.

## Animation and movement ownership

`physics/climbing` is a separate extension, not an invented stock state or `.abin` record. Its phases are Reach → Hang → Mantle → Settle. While attached it owns character movement and pauses the stock animation/character solve. It writes the same real skeleton bodies, auxiliary COM bodies, physical outputs, and render pose used by the existing presentation interpolation and gameplay camera.

Before catching, the normal action/motion graphs, gravity, collision solve and jump trajectory continue running. A presentation overlay starts during the run, with continuous distance-based gain capped at 45% on the ground. The jump builds towards full reach, with a 0.10-second exponential response smoothing changes in the target weight. Two-bone IK guides each wrist towards its own fixed palm contact; palm orientation follows the top surface. Axial roll is applied to the forearm instead of inverting the wrist; that rotation preserves the elbow-to-wrist line and the palm contact. If the player turns away, moves out of range or loses the approach, the overlay smoothly fades out without changing the airborne trajectory. The original rig has no finger joints, so this positions palms rather than curling fingers.

Both wrists must be inside the actual arm reach before attachment. The saved ledge stays fixed as the skater rises past the original wall probe. Catch blends the current jump pose into hanging, with extra settling time for a high catch. Local translation/rotation/scale blending smooths the pull-up and return to walking. Hand IK stays active through the supporting half of the pull-up, then releases smoothly. Standing clearance is checked again before pulling up.

`approach.rs` runs after native physical output and before presentation capture; it does not publish its arm overlay into the physics bodies. Once caught, the attached phase owns the full pose and body as before. The saved ground return pose is normalized to the support plane so a raised foot during the run cycle does not lower the walking root.

On completion, the existing offboard controller is placed on the upper surface. Its collision queries are refreshed from that surface and its physical targets are updated before the native pipeline resumes. This prevents stale ground hits or COM targets from creating a fall or bail.

## Assets and rebaking

Optional asset: `assets/private/custom/climbing.json`. Missing assets leave stock gameplay available; malformed or mismatched clips report a load error. This private asset is ignored by Git, like the existing stock animation banks.

The JSON contains version 1, two named clips (`reach`, `mantle`), FPS, joint names, parent indices, and local SQT samples. Sample order is scale XYZ, quaternion XYZW, translation XYZ. It uses native animation bone axes, not raw Blender matrix-basis channels. It does not modify the stock bank decoder or its SHA-bound cache format.

Custom clips and their authoring tools are not distributed. Export clips using the
schema above and the native bone axes from your prepared skater rig.

## Verification

Run `cargo test -p skate-game climbing` for the geometry checks. Set `SKATE3_ASSET_ROOT` to the absolute assets directory and add `-- --include-ignored` to run the real-asset playback tests too. Those tests dismount through the stock graph, grab, hold X without automatically climbing, press again, complete the climb, stand still, and walk on the upper surface; both held and dropped board cases are covered. A separate real-asset test turns away during the reach and verifies that it fades out, never attaches, and lands normally.

Optional `SKATE_CLIMB_CAPTURE` names a folder for pose snapshots from the tests. `runtime-export/preview_runtime.py`, outside the project, renders those captured runtime bones through the original skater skin for visual inspection.
