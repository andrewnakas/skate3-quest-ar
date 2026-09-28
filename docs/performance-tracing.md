# Advanced performance capture

## Normal launch: tracing is off

Double-clicking `skate3rust.exe`, or running it without `--trace`, starts normal
play. It creates no performance capture file or export thread and installs no
per-frame trace sampling systems or GPU profiling plugin. Ordinary event logging
and crash reporting remain available. Trace support is compiled into the executable,
but its recording spans are disabled by default.

Tracing is an opt-in CLI diagnostic, not a graphics or gameplay setting. The private
`TRACE-*.bat` launchers enable it by supplying the flags below; launching the same
executable directly does not inherit those flags. F9/F10 capture controls are only
installed for a tracing launch. After a diagnostic session, relaunch without the
flags for normal play; completing a capture does not uninstall its instrumentation.

## Enable a capture through the CLI

Windows releases built from this revision include the recorder. No Rust, engine rebuild,
profiler installation, network listener or menu setting is needed to **record**.
From a terminal in the installed release directory:

```bat
skate3rust.exe --trace university-cpu.json --trace-wait --trace-seconds 30
```

Load University normally, settle at the area you want to measure, press **F9**, and
skate for 30 seconds. **F10** stops and exports early. Continue playing or close
normally; the capture does not terminate gameplay. Use a new output filename for
each launch: existing files are never overwritten. The destination directory must
already exist and be writable. Relative filenames resolve from the working directory.

For GPU pass diagnostics, repeat the same route and settings in a separate launch:

```bat
skate3rust.exe --trace university-gpu.json --trace-wait --trace-seconds 30 --trace-gpu
```

For startup/loading, omit `--trace-wait`. For unattended delayed recording use
`--trace-delay 60` instead of `--trace-wait`. Delay is measured from recorder
initialization, not map readiness. Duration defaults to 30 seconds (range 1–600);
delay range is 0–600 seconds. There is one capture per process. F9 cannot restart
a completed recording. Trace modifiers without `--trace` are rejected. Existing
`--assets` and `--map` options can select prepared data as usual.

CPU slices shorter than **25 microseconds** are omitted by default to keep high-FPS
captures useful within the file-size bound. Counters and metadata are not filtered.
Use `--trace-min-us 0` for an unfiltered, usually much shorter capture, or a value
up to 1000 for a coarser timeline. `capture_complete` reports `min_span_us` and
`filtered_short_spans`, separately from queue losses. Slice counts and summed slice
durations exclude those short scopes; do not treat the remainder as all CPU work.

Open the completed JSON using **Open trace file** in [Perfetto](https://ui.perfetto.dev/).
Select a CPU slice to see its duration; search for `physics::advance`,
`fixed_collision_and_solve`, `fixed_animation_graphs`, `prepare_mesh_bind_groups`,
`prepare_windows`, or `present_frames`. Expand counter tracks for frame intervals
and render pass diagnostics. Load after recording to avoid viewer GPU contention.
Nothing in the recorder uploads captures. Share the JSON file explicitly when needed.

## What the measurements mean

- CPU thread slices use Bevy 0.18.1's built-in ECS/system, schedule and render
  instrumentation plus startup/map/physics subspans. They measure **wall time inside
  a scope**, including waits and descheduling, not exclusively CPU execution time.
  Worker threads have anonymous numeric IDs. Nested slices must not be summed as
  independent work. Re-entered spans emit separate intervals. Scopes crossing a
  capture boundary can be absent; partial frames should be excluded from analysis.
- `frame_interval_ms` measures successive main-app `First` samples. It includes
  pacing and scheduling effects; it is not a monitor presentation/scanout measurement.
  `fixed_schedule_iterations` counts fixed schedule loops, not successful physics
  steps. `fixed_period_ms` reports the existing native-derived timestep. The main
  sample duration stops at the trace system in `Last`; unordered `Last` systems,
  including the FPS pacer, may fall outside it. Use their CPU spans separately.
- Render preparation, command encoding, queue submission and `present_frames`
  **are CPU intervals**. A long `prepare_windows` can reflect waiting for the GPU
  or presentation rather than expensive mesh preparation. Main and render apps
  are pipelined, so their work need not correspond to the same displayed frame.
- With `--trace-gpu`, `render/.../elapsed_gpu` counters are **measured GPU timestamp
  durations in milliseconds**, when the device supports them. `elapsed_cpu` is
  CPU pass encoding time. Other render counters are pipeline invocation statistics.
  Results arrive asynchronously and are sampled only when a new diagnostic arrives.
  They are plotted at CPU receipt time, **not on a calibrated GPU timeline**; do not
  infer exact GPU start times or associate them blindly with that main frame.
  Anonymous series IDs distinguish views; only recognized pass names are retained.
- Bevy's instrumented passes are covered, with added diagnostics for the custom
  retail exposure meter and tone pass. This is not complete GPU-frame coverage:
  other custom passes may be uninstrumented, and nested pass durations can overlap.
  Never add all counters to claim total GPU frame time. Missing GPU counters mean
  unavailable measurements, not zero GPU cost.
- Asset counters show main-world asset entries, not VRAM usage. Map meshes/images
  use `RENDER_WORLD` ownership and can leave those main-world collections after
  upload. Treat counts as lifecycle clues, not resident memory or draw-call counts.
- Metadata includes the existing build ID/revision, engine versions, map fingerprint,
  initial map counts, timestep, numeric adapter IDs/backend, window dimensions,
  camera target sizes/MSAA and graphics settings. Map names, source filenames,
  local asset paths, player names, environment dumps, CLI arguments and log payloads
  are omitted. The map fingerprint identifies content without publishing its path.

## Bounds, overhead and failure behavior

The writer runs on a separate thread with an 8,192-event queue. Full queues drop
events instead of waiting for disk; `capture_complete.args.dropped_events` reports
loss. A nonzero count makes the timeline incomplete. The event body is limited to
128 MiB, plus a small JSON header/footer, and ends early with `size_limit` if reached.
The time limit also exports while gameplay continues. A normal return, startup
error or Rust unwind drops the guard and finishes the file. Forced process termination,
native crashes, power loss or disk errors can leave incomplete JSON; this is not a
crash-safe journal. The crash supervisor retains its existing independent behavior.

CPU recording costs timestamps, span bookkeeping, JSON values and queue operations;
export costs CPU and disk bandwidth. GPU queries/readback and pipeline statistics
add overhead. This implementation has **not** had its overhead benchmarked in-game.
Compare CPU-only and GPU-enabled runs, and treat heavily instrumented results as
diagnostic evidence rather than baseline FPS. GPU instrumentation remains installed
for the entire `--trace-gpu` launch, including while waiting and after export. CPU
span subscription also remains installed after export, although event creation stops.
Relaunch without tracing for normal play. With no `--trace`, no recorder, writer,
sampling systems or GPU diagnostics are installed; the log subscriber rejects spans
before recording. Built-in trace callsites remain compiled but disabled.

The application retains its existing Vulkan device configuration. GPU timestamp
and pipeline statistics support depends on adapter features and drivers; the trace
does not force unsupported features or change device limits. Bevy 0.18.1 supports
these diagnostics on Vulkan/DX12, and internally bounds timestamp/query capacity.
CPU capture works when GPU measurements are unavailable. No textures, shaders,
resolution, culling policy, density, animation cadence or simulation timestep are
changed by the launch flags. Do not combine this workflow with the older
`SKATE_PERF_REPORT`/camera-sweep benchmark environment options: that is a separate
benchmark mode with different controls and collection semantics.

## Developer support and tooling choice

The package always enables Bevy's `trace` and `debug` features; `debug` retains ECS
diagnostic names and does **not** select Cargo's debug profile or change rendering.
The normal static release build
(`cargo build --release --locked -p skate-game --bin skate3rust --no-default-features`)
therefore supports the CLI. No separate `trace_chrome` or `trace_tracy` build feature
is necessary. The custom subscriber retains event logging and the existing panic
hook while filtering timeline and log layers independently; `RUST_LOG` controls
logs and cannot silently remove CPU trace systems.

Bevy's stock Chrome backend is convenient but its default formatter includes span
fields and lacks our capture controls/privacy bounds. Tracy offers live CPU/GPU
correlation and richer analysis, but requires matching capture/viewer tooling and
an alternative instrumented build. It is a developer escalation, not a requirement
for distributed capture. RenderDoc is useful for inspecting actual draws, resources,
pipeline state and overdraw on Vulkan; replay/capture overhead means its numbers
should not be substituted for an ordinary frame-time profile. Capture only after
the user launches the game. wgpu API tracing is primarily reproduction/replay of
GPU API traffic, not a replacement for CPU or hardware duration profiling, and may
contain asset contents. It is intentionally not enabled by these flags.

Versioned official references: [Bevy 0.18.1 profiling guide](https://github.com/bevyengine/bevy/blob/v0.18.1/docs/profiling.md),
[Bevy 0.18.1 render diagnostics](https://docs.rs/bevy_render/0.18.1/bevy_render/diagnostic/struct.RenderDiagnosticsPlugin.html),
[wgpu 27 source](https://github.com/gfx-rs/wgpu/tree/v27.0.1), and
[RenderDoc documentation](https://renderdoc.org/docs/getting_started/quick_start.html).
The pinned local Bevy/wgpu source is the authority for this implementation; newer
Bevy documentation uses some different diagnostic names.

For static inventory without starting the game:

```text
cargo run --release --locked -p skate-data --example map-performance-inventory -- MAP.skate
```

This reports texture payload identity, material bounds and a spatial-cell estimate.
For exact renderer canonicalization, the ignored, data-only
`skate_world::performance_inventory::runtime_batches` test accepts
`SKATE_MAP_INVENTORY` and `SKATE_MAP_INVENTORY_OUT`. Neither initializes Bevy or a GPU.
Keep private inventory paths and personal launchers out of published docs/artifacts.

For a capture summary using only Python's standard library:

```text
python tools/analyse_performance_trace.py CAPTURE.json > summary.json
```

The analyser streams the recorder's one-event-per-line format, reports frame-interval
percentiles and inclusive CPU slice statistics, and flags truncation, missing system
names and CPU-only captures. It does not infer GPU execution time from CPU spans.
