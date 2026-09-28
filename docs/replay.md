# Instant replay

The game keeps the last 30 seconds of completed presentation poses in memory.
Select / View (the small left menu button) opens replay, paused at the newest
frame. The available duration grows from zero after starting the game.

| Control | Action |
| --- | --- |
| Select / View, or B | Exit replay |
| LT / RT | Scrub backward / forward; trigger pressure changes speed, up to 4× |
| D-pad left / right | Step one recorded frame |
| A | Play / pause; playing from the end starts at the beginning |
| Y | Toggle recorded camera / free camera |
| Left stick | Move free camera horizontally |
| Right stick | Look around |
| LB / RB | Move down / up |
| Left-stick click | Faster camera movement |

Keyboard: F6 opens/closes replay, arrows scrub, comma/period step frames,
Space plays/pauses, C toggles camera. Free camera uses WASD, Q/E for height,
right mouse drag to look, and left Shift for speed.

Replay pauses live gameplay. Exiting returns to that same live state, not the
scrubbed position. Release the replay controls before resuming gameplay. Escape
can still open the graphics menu. This is an in-memory viewer, not video export
or a saved replay format.

## Implementation and verification

`Replay` stores timestamped `presentation::Snapshot` values: root transform,
complete skin/board pose, camera transform, and FOV. It records completed fixed
ticks and uses the same bone-local interpolation as live presentation. Camera
cuts, teleports, and cadence changes are not interpolated across. Scrubbing never
advances animation graphs, physics, gesture recognition, or stance events.

The replay input system runs after platform polling in PreUpdate, outside the
gated fixed simulation sets. Replay inputs are discarded from gameplay history;
the gate remains on input until buttons, sticks, and triggers are released.

Run `cargo test -p skate-game --locked replay::tests` for buffer retention,
scrubbing, camera mode, discontinuity, input isolation, and simulation gating.
For a screenshot smoke check, set `SKATE_VERIFY_REPLAY=1` when using the existing
`--verify <absolute-path.png>` launch option.

The remaining trick hitch and nollie inward-heel/fakie behavior are not changed
by this feature. Replay allows inspecting their final displayed poses directly.
