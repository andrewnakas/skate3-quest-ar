# In-process map transitions

Escape → Map selects an installed package; Load map changes the world in the
current process and resumes at the package spawn/heading. Loading shows its
current stage and suspends gameplay/menu actions until publication completes.
A failed preparation leaves the previous world available through Resume.

Successful changes save `settings/default-map.json`. Existing string pointers
remain compatible; JSON `null` explicitly selects the procedural test world.
Explicit `--map` and `--test-world` still override this saved default at launch.
`--start-paused` opens the menu before any gameplay ticks.

## Ownership and integration

- `map_transition.rs`: worker lifecycle, fresh simulation construction, atomic
  ECS replacement, error handling and compact `CurrentMap` metadata. No process
  spawn or restart path remains in the map selector.
- `map_render.rs`: `PreparedScene`, live-allocator asset reservations,
  `SceneCommands`, `MapEntity` and explicit `MapAssets` retirement. CPU map
  decoding, collision/query construction, mesh batching, textures and sky
  preparation happen on the worker. Only publication touches live ECS/assets.
- `world.rs`, `skate_world.rs`, `retail_render.rs`: initial startup and transitions
  share the scene builder. Rendering calculations are unchanged. All world/sky
  meshes, materials, images and lights have map ownership.
- `skater_animation.rs`, `physics/skater.rs`: immutable animation banks and
  `Arc<PoseEvaluator>` survive replacements, while graph/controller/skeleton
  state is constructed afresh. Existing character entities and bone bindings
  remain alive.
- `camera.rs`: camera runtime/history resets, and native HDR/tone components are
  installed or removed without replacing the presentation target or graphics
  configuration. Scene publication restores environment defaults first.
- `graphics_menu.rs`, `config.rs`, `map_library.rs`: asynchronous request UI,
  pause, current difficulty and default-map persistence.
- `physics.rs`, `grind_world.rs`, `app.rs`, `main.rs`: schedule/cadence, grind
  geometry and plugin wiring. Board collision/contact/query/grind state, input
  history, replay and presentation snapshots are replaced together.

Shared-file integration notes:

1. Renderer work splitting `retail_sky.rs` must retain the `SceneCommands` and
   generic `AssetSink` interfaces. Its additional world-material argument belongs
   in `PreparedScene::prepare` after world construction. If sky setup mutates
   materials, add a mutable iteration operation to both asset sink implementations.
2. Customiser state stored in its own resource survives map changes. The
   `WorldChanged { generation }` message and `MapTransitionSet` allow reapplying
   selection in PreUpdate after commit, while the loading overlay remains up.
   This branch does not add a row referencing the separate customiser module.
3. Trick changes should preserve the `SkaterRuntime::load_for_world` wrapper and
   shared animation source; its normal constructor still initializes all fields.

## Validation recorded on 2026-09-08

Release build passed with Bevy 0.18.1 and local patches:

```text
cargo build --release --locked --target x86_64-pc-windows-msvc -p skate-game --no-default-features
```

Used the existing shared target directory and static CRT target flags. The
executable was immediately copied into this task's own output directory.

Seven focused checks passed: repeated scene/asset retirement, abandoned scene
preparation, native/procedural camera components, saved-default round trips,
overlapping requests, render/lightmap construction and retained graphics controls.

A CPU-only installed-content lifecycle check completed six replacements:
BlackBoxPark → test world → University → test world → BlackBoxPark → test world.
University constructed 1,133,649 collision triangles, 4,866 meshes including sky,
and 2,057 images. Every return to the procedural scene restored four meshes,
zero map images and zero native materials. Old map entity IDs were invalidated;
character identity and animation Arc identities survived; contacts, ticks,
camera/replay/presentation history reset. Difficulty and push preferences survived.
A missing package failed without changing the current world. No simulation tick
was advanced by this check, and it did not write installed settings or maps.

A paused startup capture exited successfully with zero input polls, zero physics
ticks and zero animation ticks; the pause menu and 3,324 stock clips initialized.
This ran before the later instruction restricting runtime checks to the user.
Subsequent work used only compilation/static checks and prepared user launchers.

## Limits of the evidence

Repeated rendered/gameplay switches have not been exercised. GPU resource
retirement, driver memory behavior and playability after transitions require user
testing. ECS publication and GPU uploads/pipeline creation can still cause a
short hitch; the large CPU preparation phase stays off the UI thread. Retaining
the previous map until preparation succeeds temporarily needs memory for both
worlds. The loading overlay remains for several publication frames; this is not
a claim that every shader pipeline has finished compiling. Native sky availability
matches this branch's baseline renderer and must be integrated with the separate
authored-sky work. No map/character content is committed or bundled.

## Loading overlap follow-up

After the user confirmed the initial implementation works, the loader was changed
to prepare rendering concurrently with collision/skater/camera construction.
One additional scoped worker reads the same immutable decoded map; it does not
copy the package or cache old worlds. Both builders must finish before commit,
and the render worker is joined even if simulation construction fails. The
existing publication delay, validation and rollback behavior remain in place.

`MAP_LOAD_TIMING` in stderr reports file read/decode, validation, simulation,
render preparation and aggregate preparation milliseconds. Simulation and render
durations overlap, so they must not be added to estimate elapsed time.
`MAP_PUBLISH_TIMING` reports main-world publication CPU time, excluding subsequent
GPU work and the menu's publication-frame delay.

The follow-up is compile-checked only; no runtime checks or benchmarks were run.
Its actual speed improvement is unmeasured. Independent work can overlap, but
CPU/memory contention and the fraction spent reading/decoding or uploading to the
GPU determine the benefit on a particular map and machine. The earlier runtime
validation above describes the original sequential preparation build.

## Parallel package decoding follow-up

The user's timing logs showed Downtown spending 3,041 ms in combined read/parse,
compared with 1,304 ms render preparation and 490 ms simulation construction.
A standalone `skate-data` example isolated actual file reads at roughly 60 ms.
This tool only parses packages; it does not initialize Bevy, physics or gameplay.

The reader now scans texture descriptors and borrows their compressed payloads,
then decodes them with up to four workers by default. Dynamic assignment handles
unequal texture sizes; original indices determine the final texture order.
The workers finish before subsequent map processing or any world publication.
Maps with less than 1 MiB of decoded textures stay serial. Existing storage
methods, output-size bounds, DEFLATE input-consumption checks and format validation
are retained. Decompression buffers reserve a bounded initial capacity to reduce
reallocation. No content conversion, on-disk cache or persistent map cache is added.

Three-pass data measurements on the installed packages (milliseconds):

| Map | Previous parse times | Parallel parse times |
| --- | --- | --- |
| Downtown | 2934, 2929, 2777 | 1440, 1418, 1481 |
| Industrial | 1526, 1520, 1504 | 744, 679, 682 |

Separate full-field serial/parallel comparisons passed for Downtown, Industrial
and University. Two decoder tests passed for mixed raw/zlib/zstd storage, output
order at 2/4/8 workers, corrupt payloads, wrong sizes, unknown methods and DEFLATE
trailing bytes. All four existing format integration tests passed, including
documented versions and malformed/truncated input. These are data checks only.

`MAP_READ_TIMING` now separates `disk_ms` and `parse_ms`.
`MAP_DECODE_TIMING` divides parsing into textures, geometry and extensions, and
reports actual texture worker count. The existing total preparation/publication
timings remain available. End-to-end game loading improvement remains for the
user to test; the approximately halved decode time is not a claim of halved total
loading time. Game code was compiled; the new game build was not launched.
