# Custom map lighting

Custom `.skate` maps use the existing StandardMaterial PBR path: albedo, normal,
ORM, emissive, alpha mask/blend, and UV1 lightmaps remain intact. A scene-owned
shadowed directional light and ambient light now illuminate those surfaces and
PBR characters. Shadow cascades cover 100 metres; local authored point/spot
lights retain their existing settings. Area lights remain unsupported.

The environment header supplies start hour and orbit azimuth; extended headers
also supply sun/moon colour, intensity and day/night ambient. The adapter maps
unit sun intensity to 10,000 lux and unit ambient to 1,000 brightness. These are
portable rendering choices, not recovered retail constants. The pause menu now provides a Day & night submenu with a saved start hour
(default noon) and cycle speed (default 60x, a 24-minute day). Existing
baked diffuse on lightmapped custom surfaces is retained. The package horizon
colour supplies the daytime background; native sky sidecars are only loaded for retail
scenes. Zero authored light intensity is respected.

## Provenance and ambiguity

Startup and in-process loading share RetailScene::for_map. It checks embedded
retail material definitions, WMET retail-manifest metadata, RWCM collision,
SKYB retail sky, native collision edges and native rails. It never classifies
from a filename, map title or ordinary lightmap presence. The Blender exporter
writes WMET only for its retained SKATE3_RETAIL_MANIFEST; the retail pipeline
also writes WMET/RWCM. This conservatively protects imported Skate 2 scenes
whose material definitions may be absent, and edited retail-derived maps.
WCFG, BMAT and MOBJ are general authoring records, not retail evidence.

A legacy retail export stripped of *all* native evidence is indistinguishable
from custom PBR geometry. A custom rebuild retaining native records is treated
as retail. No unreliable name heuristic or new authoring UI is introduced.
Retain provenance when re-exporting retail maps. This classification protects
the existing rendering path; it does not add support for missing retail shader
families or missing sky/irradiance sidecars.

## Scene lifetime

Directional/local lights, sky and backdrop are MapEntity-owned and retire with
their scene. Ambient/background are replaced on publication. The gameplay
camera switches retail HDR/tone on and off; the GPU exposure meter resets on a
new scene generation. Retail character conversion retains original material
handles and layers, restores them on each transition, and binds fresh authored
lighting only in a retail scene. Dedicated zero-illuminance retail shadow maps
are removed/recreated, preserving the separate player-only baked-world map.
Character conversion is restricted to PlayerRoot descendants; only the gameplay
camera acquires shadow layer 28. Scoring layer 31 and session overlays keep
their existing cameras/materials. Custom character PBR materials respond to the
custom scene lights without a separate character light rig.

## Manual visual checks

- Open an ordinary custom map: compare upward and vertical surfaces, moving
  player shadows, normal maps, transparent textures and emissive surfaces.
- Check a map with local lights and/or baked indirect lighting; confirm those
  remain visible. Check an authored night start and zero-intensity sun.
- Open vanilla Skate 3 and imported Skate 2 maps: verify baked scenery remains
  unchanged and the dedicated player shadow remains visible where supported.
- Switch vanilla -> custom -> vanilla several times without restarting. Check
  sky/background, exposure, character shading and shadows for stale state or
  increasing brightness. Repeat with a custom character and clothing changes.
- Open scoring HUD, session marker and customization overlays on both map
  types; verify no world/player silhouettes or lighting appear in overlays.
- Rename a custom package to a retail-like name: it must still get custom
  lighting. A retail-derived package retaining provenance must stay retail.

No game, recomp, Steam or gameplay automation was launched for this change.

## Automated validation

`cargo check -p skate-game --tests --no-default-features --release --offline`
passed with existing warnings. A static-CRT release test harness was linked
directly into the worktree's ignored private output using the shared Cargo
cache. All six focused CPU tests passed: `map_render::tests` (3),
`retail_character::tests` (2), and `camera::environment_tests` (1). These checks
do not validate GPU appearance; use the visual checklist above.

SKATE15 loading is also covered by ten data-reader tests (including legacy
versions, both compression codecs, all storage transforms, texture references
and malformed input). A user-supplied v15 custom map passed the ignored CPU
`supplied_custom_map_prepares_and_retires` check: decoding, runtime validation,
material/mesh preparation, publication, custom-light classification and scene
retirement. Supply `SKATE_TEST_CUSTOM_MAP` and `SKATE_TEST_CUSTOM_ASSETS` to run
that check on another local custom map; no user map bytes are committed.

A subsequent offboard-query exit was traced to validating the unused fourth
SIMD lane as a position coordinate. Static swept-line queries now validate
only XYZ and radius, matching their dot3/Vector3 collision math. The regression
checks identical hits with infinite/NaN fourth lanes and rejection of invalid
XYZ/radius. Six contact-toolkit tests pass. The existing
`embedded_static_rwcm_hits_distinct_actor_query_ids` test fails its metadata
expectation (`matching_group == -1 && geometry == 0`) before executing the
changed query path; that unrelated fixture expectation remains unchanged.

## Custom collision performance

Portable map collision previously occupied one query mesh, so nearby queries
scanned the entire map. It now uses contiguous 64-triangle bounds in the
existing static hierarchy, built once during loading. Triangle order, geometry
IDs, filters, packed surfaces and adjacency remain unchanged. Native RWCM
clusters retain their existing path.

A static query benchmark on the supplied custom map (89,613 triangles) sampled
257 local segments with identical candidate IDs/order before and after.
Triangle checks fell from 23,030,541 to 4,198,157; measured candidate-query time
fell from 16.51 ms to 3.82 ms total (about 4.3x). A 16-triangle partition was
also tried, but had higher query time (4.69 ms) despite fewer triangle checks.
These are CPU microbenchmark results, not gameplay FPS or GPU measurements.
Ten map tests passed. The supplied-map benchmark is opt-in via
`SKATE_TEST_CUSTOM_MAP`. Frame timing can be captured with the existing
`SKATE_PERF_REPORT` environment setting (10-second warmup, report/exit after
25 seconds); no automated gameplay was run.

## Frame capture: mesh binding preparation

The user-run capture after collision clustering measured 83.5 FPS / 11.98 ms
mean frame time. Main schedule time was 2.80 ms, physics 1.55 ms/frame, and the
render mesh-bind-group preparation section 9.75 ms. CPU query improvements
therefore did not address the dominant measured render preparation cost.

The vendored renderer now caches complete per-phase model/skin/morph/lightmap
binding sets, retaining at most four rotating allocation combinations per
phase. Buffer contents can update without recreating descriptors. Keys include
model allocation/offset/size, current/previous skin and morph buffer IDs, morph
mesh/texture/skin-layout identities, lightmap revision and layout identity.
Pipeline changes clear the cache; removed phases and replaced map resources
retire their entries. The existing opt-in GPU cache test covers rotating
allocations, reuse, size changes, skin/morph replacement and bounded retention.
A new gameplay capture is required to quantify the resulting FPS change.

Validation: release build check passed, and the explicit Vulkan GPU cache
regression test passed. No gameplay was launched for this change.

## Remaining hitch investigation

The next user capture measured 525.1 FPS, 1.90 ms mean frame time and 0.126 ms
mesh binding preparation. Two clusters of slow frames remained; the maximum
was 26.71 ms, with only 4.01 ms in the main schedule and 2.71 ms in physics.
Another 25.53 ms frame contained no physics tick. The aggregate render metrics
cannot identify those individual stalls. No specific hitch cause is confirmed.

The opt-in profiler now retains timestamped main/render samples, pending
pipeline counts, and bounded slow-section events (at least 2 ms, at most 256).
Main/render sample buffers are preallocated. Reports still write only at the
end of the capture; normal launches do not enable this instrumentation. The
release compile check passed. Another user-run capture is required to locate
the remaining stalls; no game or gameplay automation was launched.

## Day/night controls

Day & night in the pause menu offers 15-minute time steps and speeds Frozen,
1x, 10x, 30x, 60x, 120x, 360x and 720x (two minutes per day). Click or Right
advances; Left reverses. Both settings persist in settings/graphics.json.
The running clock wraps at midnight and pauses with gameplay. Menu adjustments
preview immediately. The cycle affects only custom map scene-owned lights;
retail and the test world keep their existing lighting. Local lights and baked
lightmaps retain their authored values, so baked surfaces may remain bright at
night. The existing background colour blends toward dark blue at night; this
is not a new sky dome or star renderer. No assets are rebuilt each frame.

CPU tests cover wraparound, pause/freeze, sun orbit, zero authored intensity,
settings compatibility and custom/retail scene retirement. Visual appearance
and runtime FPS require a manual run; the assistant does not launch gameplay.

Validation: seven focused CPU tests passed (graphics_menu::tests and
map_render::tests); the user-map preparation test was not rerun.

Ambient light in the same submenu defaults to Auto (the day/night ambient).
Left/Right or click cycles Auto and 0–100% in 5% steps. Manual levels override
the cycle's ambient brightness, mapping 0% to zero and 100% to 1,000 brightness.
They preview while paused and persist with graphics settings. Direct sun/moon,
local lights, emissive materials and baked lightmaps are independent, so zero
ambient does not make the whole scene black. Retail lighting remains authored.

## Visible sun and moon

Custom maps now have warm sun and cool full-moon discs following the same orbit
as their directional lighting. They follow the gameplay camera without parallax,
are hidden below the horizon, and use normal depth testing for scenery occlusion.
They are placed just inside the camera far plane and have fixed angular sizes.
These are lightweight unlit meshes, not extra lights or shadow casters. They
remain visible with zero ambient. Their scene-owned meshes/materials retire on
map switches; retail skies are unchanged. Moon phases are not added.

The sun and moon now have roughly 3.3x their original apparent diameter.
Two scene-owned 256px procedural textures add a feathered golden solar corona
and cool lunar halo with crater rims and mottled surface detail. The solar
corona rotates slowly with a subtle pulse while gameplay runs. Texture creation
stays in map preparation; runtime still uses two mesh draws and transform updates.
