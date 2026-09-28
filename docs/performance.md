# Large map performance

The September 6, 2026 performance pass preserves the full render geometry and
collision data. It does not add LODs or simplify physics.

Changes:

- A bounds hierarchy selects collision clusters instead of scanning all clusters.
  Results retain their original order, including equal-distance hits.
- Conservative triangle bounds reject impossible line and primitive contacts
  before the expensive narrowphase. Predictive separation and fatness are included.
- Byte-identical textures and equivalent rendered materials share resources and
  batches. Texture color space and all currently rendered material fields remain
  part of their identity. Every source render triangle is retained.
- Render meshes and textures release their CPU copies after GPU upload. The
  decoded map package is released after physics and rendering have consumed it.
- The normal development build now optimizes project code at level 3, matching
  the existing dependency optimization level.

## Repeatable measurement

From the project directory in PowerShell:

```powershell
./Build.ps1
$env:SKATE_PERF_REPORT = 'logs/performance.json'
./bin/skate-game.exe --assets assets --map maps/Skate_2_New_San_Vanelona.skate
Remove-Item Env:SKATE_PERF_REPORT
```

The benchmark runs continuously regardless of window focus, waits 10 seconds
for warmup, records roughly 15 seconds, writes JSON, and exits. Leave the player
stationary for spawn comparisons. The environment variable enables instrumentation;
normal launches do not collect reports or change their window-focus behavior.
Frame intervals include renderer synchronization. Main schedule and render timings
overlap and must not be added together. Detailed physics section timings include
warmup and nested scopes; these must not be summed either.

Local San Vanelona measurements at 1280 x 800, stationary spawn, RTX 5090:

| Metric | Before | After |
| --- | ---: | ---: |
| Average FPS | 43.6 | 50.7 |
| Main schedule per frame | 10.56 ms | 2.39 ms |
| Physics per tick | 6.44 ms | 0.74 ms |
| Frame interval p95 | 128.1 ms | 94.5 ms |

Sources are local ignored reports `logs/perf-baseline-continuous.json` and
`logs/perf-render-detail.json`. Repeated optimized runs averaged 49–52 FPS.
Process working set fell from about 10 GB to roughly 1–2.3 GB after loading.
These are spawn measurements, not a reproduction of the reported 10 FPS camera
position. Render preparation remains the largest measured cost, and frame spikes
remain. Low total CPU/GPU utilization does not exclude a CPU render bottleneck.

Spatial render splitting was also tested, but increased entity/batch overhead and
reduced FPS on this map, so it is not included.

Validation includes the full skate-core suite and game suite (43 passed, 21 existing
ignored tests), differential collision queries against the unfiltered implementation
across 420 varied poses, material/texture identity checks, and exact render triangle
retention tests. Startup capture checks rendering/integration, not retail parity.

## Follow-up: Bevy lightmap binding cache

GPU diagnostics confirmed the RTX 5090 Vulkan adapter with binding arrays,
non-uniform indexing, and indirect drawing enabled. Local source inspection and
profiling found repeated lightmap bind-group creation in Bevy's
`prepare_mesh_bind_groups`. The [tracked Bevy patch](../vendor/README.md) caches
those bindings with resource-identity checks and bounded retention.

Same stationary-spawn benchmark, before and after this patch:

| Metric | Before cache | After cache |
| --- | ---: | ---: |
| Average FPS | 52.3 | 237.1 |
| Frame interval p95 | 94.5 ms | 5.15 ms |
| Render preparation per frame | 15.52 ms | 1.19 ms |
| Mesh binding interval per call, including warmup | 10.39 ms | 0.66 ms |

Reports: `logs/perf-bindings.json` and `logs/perf-cache-repeat.json`. The first
cache trial averaged 158 FPS, but overlapped test compilation and is excluded
from the clean comparison. The normal window presentation settings are unchanged.
GPU telemetry during the clean repeat sampled roughly 23–33% usage, compared
with the earlier single-digit readings. Full utilization is not the goal: this
GPU can render this view at high FPS without approaching full utilization.

An additional render-only camera sweep averaged 91.1 FPS, with a 24.9 ms p95
frame interval (`logs/perf-cache-sweep.json`). This rotates through different
views to exercise changing visibility and newly extracted lightmaps; it does not
move the skater or alter collision behavior. Enable it alongside the usual
benchmark variable with `$env:SKATE_PERF_CAMERA_SWEEP = '1'`, and remove it
afterwards with `Remove-Item Env:SKATE_PERF_CAMERA_SWEEP`. It only runs when
`SKATE_PERF_REPORT` also enables the benchmark plugin.

The sweep is a different workload, not an FPS comparison against the stationary
baseline. Exploration can still encounter heavier views and loading/shader spikes.
Validation: GPU cache regression test passed, game suite passed (43 passed,
21 existing ignored), and the staged executable completed a normal city startup
capture with benchmark settings disabled.

## City-facing dips at 1440p / 8x MSAA

The next pass uses the user's saved 2560 x 1440, 100% internal resolution,
8x MSAA, unlimited FPS settings and the same rotating-camera benchmark.
Do not compare these results directly with the earlier stationary 1280 x 800 run.

| Metric | Before | After |
| --- | ---: | ---: |
| Average FPS | 84.9 | 119.1 |
| Frame interval p95 | 28.9 ms | 23.5 ms |
| Render preparation per frame | 5.42 ms | 1.49 ms |

Reports: `logs/city-baseline.json` and `logs/city-double.json`. Temporary direct
instrumentation measured roughly 3–4 ms in mesh binding preparation before the
final fix and about 0.05 ms after warmup. GPU telemetry during the optimized run
sampled roughly 24–39% usage. Heavy views and frame spikes remain.
The final clean build, with temporary tracing and GPU telemetry polling removed,
averaged 145.0 FPS with an 18.9 ms p95 frame interval (`logs/city-final.json`).
The 119–145 FPS range illustrates run/instrumentation variation; it is not a
guaranteed minimum while exploring the whole map.

Bevy's `collect_buffers_for_phase` exchanges two buffer allocations every frame.
A single-table cache therefore misses every frame even with static resource
contents. The patch now retains both phase tables, checks resource identities
and lightmap revisions, and shares unchanged tables without visiting every slab.
The preceding one-table trial did not materially improve the sweep and is not
the implementation shipped here.

The game also reserves geometrically growing instance-buffer capacity before
Bevy writes it, bounded by device limits. This reduces reallocations when visible
counts increase; it does not change logical counts or dispatch extra instances.
Mesh morph-target lookup is refreshed on GPU asset changes, and identical PBR
materials can share handles while each mesh retains its own baked lightmap.

No triangles, collision detail, lighting, or draw distance were removed. LODs
remain a possible next step for distant geometry, particularly hierarchical LODs
that also reduce the number of draws. They do not address the buffer-cache miss
fixed here. GPU occlusion culling is also relevant to a dense city, but the current
engine's version is experimental: [Bevy's migration notes](https://bevy.org/learn/migration-guides/0-18-to-0-19/#occlusion-culling-is-no-longer-experimental)
describe correctness fixes in 0.19. It was not enabled as an unverified workaround.

Validation: the explicit GPU regression test and game suite (46 passed,
21 existing ignored) passed. The staged build completed the normal San Vanelona
startup capture at the user's saved settings, with benchmark overrides disabled.

## Runtime occlusion culling

The Escape graphics menu now supports GPU occlusion culling, default On. Existing
settings files without that field pick up the default. `SKATE_OCCLUSION=0` or `1`
overrides the saved value at launch for A/B runs. The two-phase GPU algorithm
uses a depth prepass and dynamically rejects hidden meshes; no map baking, LOD
generation, distance cutoff, or collision changes are involved.

This backports [Bevy #22603](https://github.com/bevyengine/bevy/pull/22603) to
0.18.1's core pipeline: conservative depth-pyramid sampling at non-power-of-two
resolutions. It also fixes a reproduced GPU validation failure in 0.18.1's late
preprocessing pass by making its indirect argument buffer binding read-only.
Details and provenance are in `vendor/README.md`.

Same saved 1440p / 100% / 8x MSAA settings, rotating view, no concurrent builds:

| Trial | Average FPS | Frame interval p95 |
| --- | ---: | ---: |
| Original material batches, culling Off | 124.1 | 22.1 ms |
| Original material batches, culling On | 128.0 | 21.3 ms |
| 128-unit spatial batches, culling On | 72.4 | 51.2 ms |

Reports: `logs/cull-off.json`, `logs/cull-on2.json`, `logs/cull-cell128.json`.
The small On/Off difference is within plausible run variation, not evidence of
a major FPS win. CPU/render submission overhead remains significant even when
the GPU skips hidden geometry. These sweeps are not a minimum-FPS guarantee.
Spatial splitting increased batches from 30,097 to 55,850 and was removed from
the final implementation. All 8,214,063 render triangles remain available.

Validation: city sweep and normal startup capture completed without GPU errors
at 8x MSAA. The game suite has 47 passing tests and 21 pre-existing ignored tests,
including changing culling, MSAA and internal resolution together. The explicit
GPU lightmap cache regression test also passed. The startup
image was visually inspected. Manual menu testing was interrupted by the user
stopping Computer Use, so live UI toggling has not been visually verified.
