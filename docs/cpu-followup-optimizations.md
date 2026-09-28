# CPU follow-up after v8

The user confirmed substantially higher FPS in v8 and authorized all three next
targets. The new 20-second v8 capture completed without dropped events and confirmed
bindless materials enabled. Inclusive CPU scopes measured main opaque encoder
finish at 1.47 ms/frame, directional shadow visibility at 0.52 ms/frame, and
controller polling at 0.39 ms/frame. These costs overlap and are not promised savings.

## Resource reuse in material groups

The vendored Bevy allocator now chooses the fitting slab requiring the fewest new
resources, preferring an already fuller slab on ties. Previously it chose the
first fitting slab, even if a later slab already contained all the textures.
This targets duplicate texture references and the associated render-command
resource tracking. Capacity remains 64 for retail materials; shader code is
unchanged. Allocation, reference counting, retirement and GPU uploads retain the
existing implementation. Selection performs more work when materials are loaded
or replaced, not on steady-state rendered frames. An FPS benefit still requires
a matched capture; this is not a guarantee of fewer total slabs on every workload.

The explicit Vulkan test exercises the real allocator with GPU texture identities:
fill two slabs, free the first, reuse the second's resident textures instead of
duplicating them into the first, then retire and reallocate the resources.

## Static shadow spatial index

Retail map meshes receive a static-shadow marker. A cached binary bounding-volume
hierarchy rejects groups outside a directional shadow frustum. Each leaf still
uses Bevy's original per-mesh OBB test with near-plane rejection disabled and far
rejection enabled. Node bounds conservatively enclose transformed mesh bounds;
node render-layer unions also reject unrelated lights early.

The index rebuilds when relevant transforms, bounds, inherited visibility, render
layers or membership change. Camera movement changes queries, not the static tree.
Unmarked/dynamic meshes, missing bounds/transforms, explicit visibility ranges and
NoFrustumCulling use the ordinary per-mesh semantics. Shadow visibility is still
marked through deferred commands, retaining ordering relative to point lights and
newly hidden entities. No geometry, shadow distance, cascade count or visuals change.

Tests compare results with upstream Bevy across mesh movement, visibility and layer
changes, exclusion removal, disabled lights and entity retirement; another compares
the tree against flat OBB checks over 4,096 rotated/scaled boxes. Schedule replacement
is tested separately. A data-only University benchmark uses real material bounds
and representative shadow frusta; its result is not whole-frame performance.

## Controller metadata cache

XInputGetState still runs every host frame for every eligible slot. Raw packets,
packet-number handling, fixed-tick publication and simulation cadence are unchanged.
Only the controller subtype obtained from XInputGetCapabilities is cached. State
errors/disconnects and filtered-out slots invalidate it immediately. Metadata is
also refreshed once per second so a hot swap cannot retain a subtype indefinitely
if Windows never reports a disconnect. Failed refreshes remain errors and are
retried, never stored as successful data. Menu-only polling retains its original API.

This removes redundant capability queries; the 0.39 ms total polling cost also
includes state queries, so it is not all recoverable by this change.

## Verification scope

Headless Vulkan main/shadow tests retain the v8 pipeline-crash regression and 8x
MSAA pixel readback. The game and native gameplay are never launched by the agent.
Existing v8 and recovery binaries remain available. Final frame-time acceptance
requires the same scene/settings and route in the private v9 build and v8.

Final targeted checks passed: 47 ordinary tests (29 input, 3 shadow index/schedule,
11 world adapter, 2 scene lifecycle, 2 shader/descriptor), plus both GPU probes in
bindless and fallback modes. The real allocator reuse/retirement assertions run
inside those GPU probes. The explicit University inventory also passed geometry
preservation and indexed-versus-flat visibility equality.

For 4,840 static retail batches and 1,200 representative shadow queries, the final
single-thread kernel benchmark measured 22.1027 ms flat versus 4.1048 ms indexed
(81.4% lower query time), with a 1.8785 ms index build. It excludes ECS refresh,
dynamic casters, deferred visibility writes and the upstream parallel scheduler;
it is not an 81.4% reduction in whole shadow-system time or frame time.

The private v9 static-CRT release compiled from
`1c62e56fd62f5ffe7f4eee21d3b272db00d88c16`. Embedded revision, PDB presence and
system-only DLL imports were verified without running gameplay. EXE SHA-256:
`c83d68d2ccb344a353ffe9a27896966a2c0a9f541cb594e18adaf65693b4e4e2`.
`TRACE-UNIVERSITY-CPU-FOLLOWUP.bat` selects v9; the earlier CPU-FIX launcher keeps
v8 for comparison. The new capture is armed until F9 and records 20 seconds.
Existing compiler warnings remain. Runtime appearance and FPS acceptance are
pending the user's test; the agent's GPU runs were isolated headless tests only.


## v10: restore the measured shadow baseline

The completed user captures `university-cpu-bindings-v8-8702-22955.json` and
`university-cpu-followup-v9-5644-32244.json` each cover 20 seconds without dropped
trace events. Median frame time improved from 4.578 to 3.976 ms, but mean frame
time increased from 4.601 to 4.806 ms and p99 from 7.775 to 21.713 ms. These are
not matched camera routes; neither the aggregate improvement nor the hitches
can be attributed conclusively to a particular change.

Directional shadow system plus deferred-command time increased from approximately
0.525 + 0.036 = 0.561 ms/frame in v8 to 0.573 + 0.243 = 0.816 ms/frame in v9.
The indexed implementation repeats visibility publication across cascade lists.
A candidate using a reused entity set and a bulk mutable query passed visibility
parity tests but did not improve a complete headless schedule benchmark: 0.0610
versus 0.0590 ms/update for v9. That candidate was discarded.

The same fixture measured upstream at 0.1410 ms/update, which disagrees with the
relative result in gameplay. It uses actual University bounds but fixed frusta,
two overlapping views, no moving casters and an otherwise idle ECS schedule.
This makes it useful for detecting local overhead, not choosing the fastest
production path or predicting FPS. The inventory now includes full schedule
measurements, including deferred writes, alongside the older query-only kernel.

v10 restores upstream Bevy directional shadow visibility by removing the custom
system installation/replacement and the retail entity marker. The index remains
compiled only for headless tests and experiments. Material resource reuse,
controller metadata caching and the v8 bindless shader fix remain active.
Shadow geometry, quality and distance are unchanged. This is a conservative
return to the earlier measured path, not a claim that it resolves every hitch.

Visibility parity tests now also reset and verify per-entity ViewVisibility after
each lifecycle change, in addition to comparing cascade membership. The private
v10 launcher is `TRACE-UNIVERSITY-SHADOW-FIX.bat`; v8 and v9 remain available.
A matched user capture is required to determine the actual frame-time effect.


v10 verification: all three targeted shadow tests and the University data-only
inventory passed, including exact preservation of 1,645,617 triangles. The static
CRT release built from `54b8f989ca74b62b0c3ff6094f60f414f43a0983`; embedded revision,
PDB presence and system-only DLL imports were verified. EXE SHA-256:
`e21c5de6d17f117ee512a2197bcc6443bd2c1c9bb8de775e131b5368805d2154`.
No game was launched. GPU shaders were unchanged; v9 GPU probes were not repeated.
Existing compiler warnings remain. Runtime FPS acceptance awaits the user capture.
