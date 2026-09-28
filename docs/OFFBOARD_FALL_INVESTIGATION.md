# Offboard fall investigation

User reproduction: `logs/game-20260906-224637.stderr.log`, default map,
Hardcore, BipedAir state 501. No automated or agent gameplay tests were run.

The four captured airborne ticks show drive weight 1 and no reported world
contacts. This does not exclude self contacts: the feedback publisher filters
same-actor contact reports. Bail reason 18 is requested at tick 4.

The crucial change is in the target, not merely a physical leg drifting away:

| Tick | Part 19 drive position (animation coordinates) | Physical position (world coordinates) |
| --- | --- | --- |
| 1 | 0.017, 0.475, -0.596 | 2.643, 0.444, 2.526 |
| 2 | -0.082, 0.411, -0.573 | 2.603, 0.361, 2.591 |
| 3 | -0.549, 0.294, -0.335 | 2.394, 0.215, 2.944 |
| 4 | 0.537, 1.619, -0.047 | 2.642, 0.408, 2.682 |

At tick 4 part 19 has 1.206 m pose error and part 21 has 0.803 m, exceeding
the existing squash check. Both feet have local IK targets and full external
blend. Their retained local translation deltas are effectively zero in XYZ.
The existing capture did not contain the unmodified animation pose, so it
cannot distinguish an animation target discontinuity from an IK-generated one.
The next capture includes original joint positions, pre-IK volume positions
and basis lengths, final foot target frames, and the current animation name.

Static comparisons found no basis to change the air trajectory, disable the
bail, or change the joint solver. Native foot IK uses a rigid transpose for
the external adjacent-volume frame (82BEE390), and native prelanding leg
extension really multiplies the accumulated height (82BAFD30); neither is an
established defect. MatchAirTime (82BBA070) seeks the current sequence to the
fraction of elapsed flight. Generated native instruction extracts are in
`logs/biped-*.txt`; authoritative TU3 sources remain in Skate3Research.

## Located defect

The next user capture, `logs/game-20260906-225525.stderr.log`, identifies
`SS_BR_JUMP_RUN_INTO`. Before IK, part 19 moves from animation Y=0.541
at tick 3 to Y=1.794 at tick 4 and Y=2.363 at tick 5. Original joint positions
already contain the stretch, while the mapped rotation bases have unit length.

The selection-space adapter incorrectly wrapped its selected raw child in a
second BindPose node. The main playback tree already had that wrapper. This
composed rest-pose translations and rotations twice as the jump blended in.

Native evidence: SelectionSpace::SetAttributes82D26CC0, call at82D26F88,
dispatches through IAnimatable+76. TU3 vtable8231E050+76 contains82E32328.
That three-instruction thunk passes the construction map and calls82D19648
directly; it does not call posture or AddBindPose. Main GetAnimTree82B980A0
instead invokes vtable208 (posture) and164 (AddBindPose82B98118) after the
raw builder. The adapter now preserves this distinction and leaves selected
children raw, including nested selection spaces. Bail thresholds and physics
are unchanged. Compilation is checked; gameplay validation belongs to the user.
