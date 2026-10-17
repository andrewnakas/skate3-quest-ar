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

## 2. The data: `core.pack`

Both the Quest app and skatecore-cpp read one file, `core.pack` (90 MB,
534 files). It is published by skate3-cleanroom:
`https://andrewnakas.github.io/skate3-cleanroom/core.pack`, described by `pack.json`.

Format `SKPK1` (little-endian): magic `SKPK1` padded to 8 bytes, `u32` file
count, then per file `u32` path length, UTF-8 path, `u64` size, bytes. Paths
look like a converted installation: `assets/private/game.json`,
`assets/private/skater.glb`, `assets/private/stock/data/anim/*.abin`,
`assets/private/stock/data/state/...`, `assets/clean-audio/*.wav`, `maps/...`.
Readers: `crates/skate-game/src/clean_pack.rs` (Rust, unpacks to disk) and
`src/s3/data/vfs.*` in skatecore-cpp (in memory).

What is in it, and how clean it is (full detail in the skate3-cleanroom README):

| Content | Treatment |
|---|---|
| Skater model and its textures | **Regenerated** (new geometry lattice, new triangulation, re-quantised skin weights; textures rebuilt from coarse colour grids + noise). Taint-scanned: 0 shared 32-byte runs with retail. |
| Sounds (27 + CC0 recordings) | **Regenerated** or CC0 recordings (credits in `assets/clean-audio/CREDITS.json`). |
| Animations (`.abin`), state graphs, physics/VLT tuning, joystick gesture patterns, camera configs, skeleton | **Kept as facts** from the retail game (the skating *is* these numbers). |
| Music, speech, everything else on the disc | Not shipped. |

**Policy: the public no-import build must be 100% clean-room and original.**
The kept-facts rows above (animations, state graphs, tuning, the skeleton and
player colliders, and with them the retail proportions) are not acceptable in
anything shipped publicly with bundled data. They are being replaced with
original work for 0.03; until then public Quest builds bundle nothing and
players import their own disc. Plan the Spectacles version around the fully
clean pack, not this one. The project is unofficial and not affiliated with EA.

You only need a subset for a skater in AR: `assets/private/` minus
`native-skies`, `native-props`, `hud`, `session-marker`, and none of `maps/` or
`mods/` (about 64 MB; most of it is the 46 MB of animation and graph data in
`stock/`).
Development Quest builds bundle the whole pack for simplicity.

## 3. Recommended Spectacles architecture

```
 Spectacles host (C++)                              skatecore-cpp (C++17 library)
 ─────────────────────                              ─────────────────────────────
 world mesh / planes ──► collision quads+tris ─────► GamePhysics::set_real_surfaces*   (port, ~130 lines)
 placed ramps/rails  ──► piece geometry ──────────► (pieces.rs → C++, pure math, ~330 lines)
 ledges, rails       ──► polylines ───────────────► GamePhysics::set_grind_rails*      (port, ~60 lines)
 input (hands/phone) ──► XInput-style Pad ────────► driver::Sim::step(Pad)  60 Hz fixed tick
                         render ◄── Presentation: root + bone matrices (skater.glb skin)
                         audio  ◄── Presentation: board_speed, wheel_surface, trick_identifier, ...
 head pose ─────────────► spatial listener; skater position = emitter
 ```
 `*` exists in the Rust Quest app only; see 3.2.

### 3.1 What skatecore-cpp gives you today

- `s3::game::driver::Sim` (`src/s3/game/driver.hpp`): `Sim::load(vfs, "assets",
  map, difficulty)`, then one `step(Pad)` per 1/60 s tick, then
  `presentation()` for rendering and audio. `Pad` is XInput units (buttons
  bitmask, 0-255 triggers, ±32767 sticks), so any input source works if you
  map it onto a virtual Xbox pad. That is exactly what the Quest app does with
  Touch controllers (`crates/skate-game/src/xr/touch.rs`).
- `Presentation`: skeleton root transform, global bone matrices in animation
  space (skin `skater.glb` with them), camera (ignore in AR), audio inputs,
  trick names and scores.
- Determinism: `-ffp-contract=off`, bit-identical traces vs Rust
  (`tools/parity.sh`, `docs/PARITY.md`). Keep that flag on any new toolchain.
- Build: `make s3-test` runs the 1001 ported tests natively (needs the pack);
  `tools/build_web.sh site` builds the wasm demo (~690 translation units,
  ~10 min clean at `-j4`, about 1 GB RAM per job). The output is 4.0 MB of wasm.
- Web host reference: `web/s3/host.cpp` (loading the pack into a `Vfs`, map
  geometry out, sim stepping) and `web/s3/main.js` (three.js skinning, camera,
  clean-audio driver, HUD). Port the JS skinning/audio logic to the Spectacles
  side.

**Alternative, the "lite" engine** (`include/skate_api.h`, `src/*.cpp`, 145
lines of C API): a small procedural skater that needs **no data pack**, only
a raycast callback into your collision, and derives grind rails from any
triangle soup. It is far less faithful (one rigid body, authored flick
patterns) but tiny, and a fast way to get something skating on Spectacles on
day one.

### 3.2 What to port from the Quest app (the AR layer)

The C++ port is a snapshot of the engine before the AR work. The AR layer is
small and mostly pure math. File references are in this repo:

| Piece | Rust source | Notes |
|---|---|---|
| Real surfaces as collision | `crates/skate-game/src/physics.rs` `set_real_surfaces`, `physics/ground.rs` `course_on_real_world`, `PlacedSurfaces` (~190 lines changed vs the snapshot) | Replaces the map collision with room quads, mesh triangles and ledge edges; rebuilds the off-board grab scene. |
| Runtime grind rails | `physics.rs` `set_grind_rails` | Builds a `grind_world::StaticProvider::authored` from polylines and re-binds the air trajectory. Never swap while grinding. |
| Room → collision | `xr/real_world.rs` | Planes/boxes → one-sided quads (walls double-sided), furniture tops become grindable ledges, scanned mesh only where no plane covers it, and the placed pieces. Rebuilds only when the room or layout changes. |
| Planes from the room mesh | `xr/mesh_planes.rs` (pure, 5 tests) | Spectacles has its own world mesh and plane APIs; this is the fallback/reference for fitting rideable flat patches. |
| Park pieces | `xr/build/pieces.rs` (pure, 7 tests) | Kicker, quarter pipe, half pipe, rail, ledge, fun box, platform, bank, stairs + handrail, spine as quads + grind polylines. Winding rule: quads face out of the solid (course collision is one-sided). |
| Placement UX | `xr/build/mod.rs` | Ray-cast ghost, snap yaw to table edges (`snap.rs`), move/turn/resize/copy/undo, starter park, layout save (`layout.rs`). |
| World scale | `xr/rig.rs` + `build/mod.rs` `finish_rescale` | See 3.3. |
| Spatial audio | `xr/spatial_audio.rs`, `player_audio_clean.rs` | Listener at the head, every sound emitted at the skater, distances in real metres. |
| Clean sound driver | `player_audio_clean.rs` | Loops (rolling per surface, grinds, powerslide) faded by board speed; one-shots on state edges (pop, flip+catch, land, push, footstep, bail). ~300 lines; reads the same fields `Presentation` exposes. |

### 3.3 Conventions you need

- **Units:** the simulation is in metres at skater scale, +Y up, fixed 60 Hz.
  skatecore-cpp: board nose +Z at heading 0, right-handed.
- **Miniature scale `k`:** physics never changes. The AR app scales the *view*
  instead: tracking (real) space maps to the game world with
  `world = W + R_yaw * (k * tracking)`. The default is `k = 12` (a 1.8 m skater
  is 15 cm), and the left stick sets anywhere from 1 (life size) to 60. When
  `k` changes, the skater is teleported to the same real spot
  (`SkaterRuntime::travel_to`) and the collision is rebuilt. On Spectacles
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
