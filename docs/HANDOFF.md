# Handoff: Skate 3 AR (Quest 3) → Snap Spectacles

For Matthew Hallberg, from Andrew Nakas. October 2026.

This is a working AR skateboarding toy on Quest 3: the Skate 3 skating
simulation (physics, flick-it tricks, grinds, bails, scoring) runs in
passthrough, on your real floor and furniture, plus ramps you place with the
controllers. The goal of the handoff is the same experience on Spectacles.

Everything here is three repos and one data pack:

| Repo | What it is | Language | Status |
|---|---|---|---|
| [skate3-quest-ar](https://github.com/andrewnakas/skate3-quest-ar) (this one) | The Quest 3 AR app: the full engine + OpenXR passthrough, room understanding, park builder, spatial audio | Rust + Bevy 0.18 | v0.0.1-alpha released, plays on device |
| [skatecore-cpp](https://github.com/andrewnakas/skatecore-cpp) | The same skate simulation ported to **C++17**, tick-for-tick bit-identical to the Rust reference, plus a tiny standalone "lite" engine | C++17, Emscripten for web | Live web demo, 1001 ported tests pass |
| [skate3-cleanroom](https://github.com/andrewnakas/skate3-cleanroom) | The **clean-room asset pack** (`core.pack`) and the WebGPU build of the engine | Python tools + wasm site | Live at andrewnakas.github.io/skate3-cleanroom |

Upstream engine (Rust): [SK8-ENGINE/skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine), GPL-3.0.

**Spectacles can run C++ (per Matthew), so the recommended path is skatecore-cpp** (section 3).
It is the same simulation as the Quest app, with no Bevy, no Rust toolchain and
no GPU code: the host draws, the library simulates.

---

## 1. Try it first

- **Quest 3:** install the APK from the
  [releases page](https://github.com/andrewnakas/skate3-quest-ar/releases)
  (`adb install -r skate3ar-*.apk`, developer mode on). Public builds need data
  converted from your own disc (`scripts/quest/push-assets.sh`); a development
  build can bundle `core.pack` instead (`scripts/quest/fetch-clean-pack.sh`). Controls are in
  [scripts/quest/README.md](../scripts/quest/README.md).
- **Browser (C++ runtime):** https://andrewnakas.github.io/skatecore-cpp/
  (Space to push, arrows to flick: down then up is an ollie). This is the exact
  code you would put on Spectacles, compiled to WebAssembly.
- **Browser (Rust engine):** https://andrewnakas.github.io/skate3-cleanroom/

## 2. The engine and data: use the clean-room `sk8` engine

**Policy: the public no-import build must be 100% clean-room and original.**
Since 0.03 the Quest app's default tier meets it, and the Spectacles version
should be built the same way.

- **Engine:** `sk8`, in skatecore-cpp `clean/` (C++17, C API `clean/include/sk8.h`,
  GPL-3.0 like the rest of skatecore-cpp). It was written from scratch, with
  its own physics, its own flick recogniser (stick paths read as rim and centre
  tokens), its own 16 tricks and its own tuning table with menu descriptors.
  It reads **no files**. The host supplies a segment raycast and grind-rail
  polylines, steps it at 1/60 s, and reads back a state (position, deck and
  stance rotations, mode, crouch, lean, flip progress, grabs, score) and
  events (pop, trick, land, bail, grind, bank, push). Tests: `make sk8-test`.
- **Skater:** built in code: a figure of simple segments with its own
  proportions, posed from the state with two-bone IK feet on the deck, plus
  a procedural tumble for bails (`crates/skate-game/src/clean/skater.rs`; port
  the maths, it is about 300 lines).
- **Sounds:** `crates/skate-game/clean-audio/`. These are 22 CC0 recordings and
  9 sounds synthesised by `scripts/cleanroom/synth_sounds.py`, with credits in
  `CREDITS.json`.
- **Rust host for reference:** `crates/skate-lite-sys` (bindings, built with
  `cc`) and `crates/skate-game/src/clean/` (collision grid, stepping with time
  scale, menus).
- **Proof:** `scripts/cleanroom/audit.py --retail <your converted assets>` checks
  the clean sources, sounds and APK for retail files, shared 32-byte runs,
  retail identifiers, float literals equal to retail tuning values, porting
  markers and uncredited sounds.

The old `core.pack` (skate3-cleanroom, `SKPK1` format, readers
`crates/skate-game/src/clean_pack.rs` and skatecore-cpp `src/s3/data/vfs.*`)
is **not** clean. Its model, textures and some sounds are derived from
retail sources, and its animations, state graphs, VLT tuning, gesture
patterns, camera configs and skeleton are kept as retail facts. It drives the
faithful port (`src/s3`, which itself carries retail addresses). Only use it
in an import-your-own-disc tier, never bundled. The project is unofficial and
not affiliated with EA.

## 3. Recommended Spectacles architecture

```
 Spectacles host (C++)                              skatecore-cpp clean/ (sk8, C API)
 ─────────────────────                              ─────────────────────────────────
 world mesh / planes ──► triangles (grid) ─────────► sk8_raycast_fn callback (your collision)
 placed ramps/rails  ──► piece geometry ──────────► (pieces.rs → C++, pure math, ~330 lines)
 ledges, rails       ──► polylines ───────────────► sk8_world_add_rail
 input (hands/phone) ──► sk8_input (2 sticks + bits) ► sk8_rider_step, 60 Hz fixed step
                         render ◄── sk8_state: position, board_rot, rider_rot, crouch, lean, flip, grab
                         audio  ◄── sk8_event: pop / land / grind / bail / push / bank
 head pose ─────────────► spatial listener; skater position = emitter
```

### 3.1 What skatecore-cpp gives you today

- **`sk8` (clean/, use this):** `sk8_world_new(raycast, user)`,
  `sk8_world_add_rail`, `sk8_rider_new`, `sk8_rider_place(pos, facing, speed)`,
  `sk8_rider_step(rider, world, &input)` once per 1/60 s, `sk8_rider_state`,
  `sk8_rider_poll_event`. Tuning: `sk8_param_count/at/find` and
  `sk8_rider_set_param`, which is what the Quest SK8 menu uses. Practice:
  `SK8_FLAG_NEVER_BAIL`, and `SK8_FLAG_NO_RESPAWN` for host-handled markers.
  Tutorials: `sk8_trick_name`, `sk8_trick_demo_path`, `sk8_flick_classify`.
  Build: compile `clean/src/*.cpp` as C++17. No data, no other dependencies.
- **Faithful port (`s3::game::driver::Sim`, import tier only):** it needs the
  retail-derived `core.pack` described in §2, so it is not for a public build.
  `Sim::load(vfs, ...)`, `step(Pad)`, `presentation()`; parity tooling in
  `tools/parity.sh`.
- The older `include/skate_api.h` "lite" engine is also data-free, but it
  ports retail pieces (its gesture recogniser, trick table and some formulas),
  so it is superseded by `sk8` for clean builds.

### 3.2 What to port from the Quest app (the AR layer)

The AR layer is small and mostly pure math. With `sk8` there is nothing to
add to the engine: collision is your raycast and rails are polylines. File
references are in this repo:

| Piece | Rust source | Notes |
|---|---|---|
| Engine-neutral AR world | `crates/skate-game/src/xr/link.rs` | `ArWorld` (room quads, mesh triangles, ledge edges, piece surfaces, rails, game metres) plus `TeleportSkater` and `SkaterLink`. Both tiers' adapters consume it. |
| Collision for `sk8` | `crates/skate-game/src/clean/collide.rs` (~150 lines, tested) | Uniform XZ grid over triangles answering segment casts; surface ids for sounds. |
| Stepping, time scale, markers | `clean/sim.rs`, `clean/menu.rs` | Fixed 1/60 s steps from frame time × game speed, pause and frame step, teleports. |
| Room → collision | `xr/real_world.rs` | Planes/boxes → one-sided quads (walls double-sided), furniture tops become grindable ledges, scanned mesh only where no plane covers it, and the placed pieces. Rebuilds only when the room or layout changes. |
| Planes from the room mesh | `xr/mesh_planes.rs` (pure, 5 tests) | Spectacles has its own world mesh and plane APIs; this is the fallback/reference for fitting rideable flat patches. |
| Park pieces | `xr/build/pieces.rs` (pure, 7 tests) | Kicker, quarter pipe, half pipe, rail, ledge, fun box, platform, bank, stairs + handrail, spine as quads + grind polylines. Winding rule: quads face out of the solid (course collision is one-sided). |
| Placement UX | `xr/build/mod.rs` | Ray-cast ghost, snap yaw to table edges (`snap.rs`), move/turn/resize/copy/undo, starter park, layout save (`layout.rs`). |
| World scale | `xr/rig.rs` + `build/mod.rs` `finish_rescale` | See 3.3. |
| Park scale | `xr/build/mod.rs` `Placed::world_transform(root, park_scale)` | Every placed piece is scaled about its base (0.25-4x) while the skater keeps its size; after a change the skater is dropped onto whatever is now under it. |
| World-space menu panel | `xr/panel.rs` | UI camera → image → unlit quad in tracking space (there is no flat HUD in XR). |
| Spatial audio | `xr/spatial_audio.rs`, `player_audio_clean.rs` | Listener at the head, every sound emitted at the skater, distances in real metres. |
| Clean sound driver | `player_audio_clean.rs` (`install_sk8`, `observe_sk8`) | Loops (rolling per surface, grinds, powerslide) faded by board speed; one-shots from `sk8` events (pop, flip+catch, land, push, bail, bank chime). |

### 3.3 Conventions you need

- **Units:** the simulation is in metres at skater scale, +Y up, fixed 60 Hz.
  skatecore-cpp: board nose +Z at heading 0, right-handed.
- **Miniature scale `k`:** physics never changes. The AR app scales the *view*
  instead: tracking (real) space maps to the game world with
  `world = W + R_yaw * (k * tracking)`. The default is `k = 12` (a 1.8 m skater
  is 15 cm), and the left stick sets anywhere from 1 (life size) to 60. When
  `k` changes, the skater is teleported to the same real spot
  (`TeleportSkater`) and the collision is rebuilt. On Spectacles
  you can drop this: put a scale on the root node that holds the skater, and
  feed collision in game units (real metres × `k`).
- **Collision winding:** quad triangles `(0,2,1)` and `(0,3,2)`, normal
  `(q2 - q0) × (q1 - q0)` facing out. Real-world quads in `real_world.rs` use
  the course floor's winding (unit test `quad_winding_matches_the_course_floor`).
- **Rebuilds are expensive.** On Quest a full collision rebuild is tens of ms.
  Rebuild on change (room revision, layout revision, scale) and never mid-grind.

### 3.4 Questions to settle on the Spectacles side

1. How native code is hosted (a native plugin, a C++ component, or wasm), and
   the CPU budget per frame. On Quest the sim is not the bottleneck; on
   Spectacles measure `Sim::step` first.
2. Input: there is no gamepad. Options: map hand gestures / a phone controller
   onto the virtual pad (push, steer, and a right-stick flick for tricks), or
   a simplified gesture layer that writes the same `Pad`.
3. World understanding API: world mesh vs detected planes, update rate, and
   whether a hit-test is available for placement.
4. Shipping the pack (90 MB, or the ~64 MB skater subset): bundled vs
   downloaded on first run.

## 4. The Quest app in brief (this repo)

```
crates/skate-quest/       Android entry (NativeActivity), permissions, APK metadata
crates/skate-game/src/xr/ OpenXR passthrough layer:
  mod.rs          plugin wiring, Adreno workarounds (no indirect draws, no eye occlusion culling)
  actions.rs      Touch controller actions;   touch.rs  Touch → virtual Xbox pad
  rig.rs          tracking root ↔ game world (scale k, STAGE-anchored)
  planes.rs       Meta Scene API (planes, boxes, room mesh), live re-query every 15 s
  mesh_planes.rs  fit flat patches in the room mesh;   plane_view.rs  overlay
  real_world.rs   room + pieces → collision and grind rails
  build/          park builder (pieces, placement, snap, layout save)
  spawn.rs        point-to-spawn the skater
  spatial_audio.rs  headset listener, real-metre falloff
crates/skate-game/src/clean_pack.rs   SKPK1 unpacker (first launch)
crates/skate-game/src/player_audio_clean.rs  clean sound driver (spatial in AR)
```

Build (macOS): `scripts/quest/build-apk.sh --install`. It fetches the OpenXR
loader and `core.pack` the first time. Logs: `adb logcat -s skate3`. Useful
lines: `SKATE_ANDROID unpacked the clean pack`, `SKATE_CLEAN_AUDIO loaded`,
`AR collision:`, `PLANES`, `BUILD`.

Hard-won fixes worth knowing if you ever touch the Quest build: loading must
not block the Android UI thread (ANR), the Adreno driver hangs on Bevy's GPU
indirect path (`NoIndirectDrawing` on the eye cameras), and Scene spaces need
`LOCATABLE` enabled before they report poses.

## 5. Known issues

- Real furniture doesn't hide virtual pieces behind it (no depth occlusion
  yet).
- `XR_EXT_plane_detection` is not offered by the Quest runtime, so planes come
  from Space Setup plus our own mesh fitting.
- skatecore-cpp: StartPark's spawn falls through the floor (also true of the
  Rust snapshot); irrelevant in AR.
- Licensing: the engine and this repo are GPL-3.0. skatecore-cpp is a port of
  that GPL code and its README says "License: undecided", so settle that
  before shipping (GPL-3.0 is the consistent choice).

## 6. Contacts and history

Andrew Nakas (andrewnakas on GitHub) owns all three repos. Session notes and
design decisions are in `docs/` here, `STATUS.md` + `docs/PORT_PLAN.md` in
skatecore-cpp, and `STATUS.md` in skate3-cleanroom.
