# Remote-player visual interpolation

Use `scripts/launch-multiplayer-test.ps1` with your prepared `-AssetRoot` for two connected local clients. Add `-Players 10` or `-TwoControllers` as needed. See [multiplayer-test.md](multiplayer-test.md) for build instructions.

Both peers must use this build: timestamped state uses protocol magic `SK8NET04`. The previous v2 executable cannot join it. Steam remains optional and has its matching helper beside this build.

## Head correction and localhost delay follow-up

The default physical skeleton contains NECK and NECK1 but not the visual HEAD joint. The original remote fallback evaluated only the raw idle animation, omitting the RIG_TPOSE reference added by the normal animation graph. HEAD consequently inherited a missing local neck-to-head offset. The fix adds HEAD to the transmitted visual anchors (31 total) and composes RIG_TPOSE into the idle fallback for remaining untransmitted bones.

The same launcher now selects the separate v4 build; restart both test clients with it. Existing v3 windows are not terminated or overwritten.

Loopback-only sessions now capture/forward nearby pose updates at a target 20 Hz and use a 60–150 ms buffer range. The steady synthetic target was 62.5 ms and final visual age was 98.2 ms, including sampling phase and simulated 6 ms delivery. This is not a guarantee of live latency. Steam/LAN retain the more conservative timing profile and 10 Hz nearby pose target. The extra head anchor adds at most 12 bytes per compact full pose (120 bytes/s at 10 Hz).

Playback now resets excessive accumulated delay after a sender stall as well as after a suspended receiving render loop, avoiding a long period of slow catch-up. Continuous focused and unfocused Winit updates were already enabled; no Steam, controller or driver settings were changed. Background rendering or CPU scheduling can still lower the actual update rate.

Seventeen nongame tests passed, including new head-offset/bind-composition and localhost sender-stall recovery regressions. The observed live v3 session had median target buffers of 160/157 ms and a maximum of 1,312 ms on A; its maximum application ping was 985 ms. These indicate local scheduling/processing delays but do not isolate their cause. The fixed game was built without being launched; verify head placement and responsiveness in the next user test.

## Original interpolation changes (online defaults)

- Body/root and skeletal pose histories are independent. A physical update no longer adds an old pose with a new arrival time, which previously produced alternating holds and rapid blends.
- Every physical/pose snapshot carries an eight-byte source capture timestamp. The host preserves it while forwarding and delta encoding. Idle detection ignores timestamp-only changes.
- Each remote actor has one monotonic playback clock shared by its two histories. It estimates delivered cadence and arrival jitter, then adjusts playback speed gently to maintain a buffer. It starts with a 150 ms target, grows under jitter, and supports longer intervals for distant actors. This is buffering in addition to transmission latency, not a ping reduction.
- The renderer samples both histories each display frame. Root position uses limited cubic Hermite interpolation with tangents estimated from root samples at their own capture times. Limiting avoids overshoot at stops. Root and skeletal rotations retain quaternion SLERP; skeletal local positions blend linearly.
- All newly decoded samples are consumed, including packets delivered in one batch. Histories and resolved pose caches are bounded to 64 entries. Duplicate/older samples do not create new pose keys.
- Pose capture now runs after the frame's animation update instead of taking the previous render pose in the fixed physics schedule. Network targets remain 20 Hz physics and 10 Hz pose nearby.
- A large root discontinuity resets both visual histories. A long suspended render loop resets playback to a fresh buffer. Missing updates hold the last supported sample rather than extrapolating through geometry. This layer does not delay or otherwise change the local player's controls or the existing collision solver.
- HUD/logs show `Visual interpolation: ... ms target buffer | stalls ...`, with cumulative buffer-starvation episodes across visible actors. `MULTIPLAYER_INTERPOLATION` is logged once per second.

## Verification

Fifteen nongame tests passed, including production packet/session tests, actual ten-session loopback UDP, timestamp preservation across a host with a different local clock, timestamp-independent idle detection, bounded histories, duplicate rejection, root-curve endpoint/overshoot tests, pause/reset handling and interpolation at 60/120/144 FPS.

The 29-second playback simulation uses 20 Hz movement and 10 Hz pose:

| Conditions | Final target buffer | Buffer starvation episodes |
|---|---:|---:|
| Steady 40 ms delivery | 150 ms | 0 at each tested FPS |
| 5% independent loss, 30–90 ms delivery jitter | 190.9 ms | 4 at each tested FPS |

The playhead remained monotonic and per-frame advancement stayed within the configured 8% catch-up speed. A few holds remain possible when loss exceeds the available buffer; arbitrary outages cannot be interpolated away.

The ten-peer production network audit also passed. Busy guest upload was 23.11 kB/s versus 22.87 kB/s before timestamps (about 240 bytes/s extra). Crowded host upload remained at approximately 1.025 MB/s including controls. No send-rate increase was introduced.

The game and Steam helper were built and staged without launching either. User testing is still needed for visual quality, fast flips, walking, bails, respawns and real online conditions. Sparse snapshots cannot recover a full spin absent from their endpoint rotations; this pass fixes timing and buffering, not animation-event reconstruction.

## References informing the design

- [Glenn Fiedler: Snapshot Interpolation](https://github.com/mas-bandwidth/gafferongames/blob/main/content/post/snapshot_interpolation.md) — buffered interpolation, Hermite curves, quaternion SLERP and extrapolation limitations.
- [Mirror's snapshot interpolation implementation](https://github.com/MirrorNetworking/Mirror/blob/master/Assets/Mirror/Core/SnapshotInterpolation/SnapshotInterpolation.cs) — source timelines, jitter-based buffer adjustment and gentle catch-up/slowdown.

Rebuild with `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/build-multiplayer-test.ps1`.
