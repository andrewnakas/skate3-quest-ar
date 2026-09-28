# CPU material binding experiment

## v8: repaired material-array shader

The v6 native access violation was reproduced without launching gameplay: an
isolated Vulkan pipeline compilation using the v6 material fragment shader exited
with `0xC0000005` at `Device::create_render_pipeline`. The test used the same RTX
5090, NVIDIA 616.56 and Vulkan backend reported by the user. The original shader
compiled in the same harness. Inlining the v6 shading body also compiled, narrowing
the trigger to that shader's function/resource-argument structure rather than map
simulation or residency. This does not establish a specific driver instruction
or rule out additional issues in the full game.

v8 retains material-array batching but replaces the large `shade` function taking
textures and samplers as arguments. Sampling helpers now receive only the material
slot and coordinates; they access global texture/sampler arrays at the sampling
site. One shading body serves both bindless and fallback paths. Existing shading
operations, texture selection, alpha rejection and user settings are retained.

Validation on that GPU passed for both bindless and fallback modes:

- Native compilation of the main and shadow pipelines.
- Offscreen main-pass drawing with 8x MSAA, command submission and pixel readback.
  Material slots 0 and 1 produced the expected `[64,64,64,255]` and
  `[191,191,191,255]` pixels; fallback produced the expected first value.
- Six Naga shader variants and the Rust material index/buffer contract.
- Eleven world adapter regressions.

The GPU probes use real retail shaders and Bevy vertex interfaces, with small
fixtures for unrelated lighting resources. They validate the reproduced pipeline
crash and basic material-array execution, not the entire Bevy render graph or game.
The two GPU tests are ignored by ordinary test runs and require explicit opt-in.
`SKATE_SHADER_PROBE_FALLBACK=1` selects the fallback probe; the default tests the
array path. The test-only `SKATE_SHADER_PROBE_SOURCE` can point to a directory with
`v6-retail_world.wgsl` and `v6-retail_material_bindings.wgsl` extracted from `bea625f`
to reproduce the old compiler crash using the main pipeline probe.

CPU batching is enabled again in v8. No FPS gain is claimed until a matched user
capture measures main opaque encoder finish, total command generation and frame
times. No gameplay was launched during this work.

Private v8 release build `de37c6e16722e2e68259e458cc12d5f78aeb71cb` compiled
with static CRT. The embedded revision, PDB presence and system-only DLL imports
were verified without executing the game. EXE SHA-256:
`b5c6d46b5861c3e34d75d47e061318f3ea3b69b7c8cb2aca87074fd7b2621532`.
`TRACE-UNIVERSITY-CPU-FIX.bat` launches this build against the existing assets and
records 20 seconds after F9. The former bindless launcher also selects v8;
the recovery launcher still selects v7 for comparison. Existing compiler warnings
remain. Headless GPU checks above passed; gameplay/FPS acceptance is still pending.

## v6/v7 history

**Withdrawn after the user's runtime test.** Build `bea625f` exited with native
access violation `0xC0000005` about 11.4 seconds after launch, shortly after map
initialization, on an RTX 5090 using Vulkan. The report contains no native stack
or module map, so it cannot establish the faulting subsystem or blame the driver.
The binding-array change is treated as a suspected regression, not a confirmed
root cause. Passing Naga tests did not establish runtime safety.

The recovery revision restores the exact pre-v6 main/depth shader source and
original uniform, texture and storage bindings. It removes the array shader
library and adds a regression assertion that RetailWorldMaterial has no bindless
descriptor or slot count. The v5 shadow proxies remain disabled. Recovery does
not claim an FPS improvement or a confirmed crash fix until the user runs it.
The remaining sections describe the withdrawn experiment for reference.

Recovery build `0b89cb8f47838bb1b7532a88f4bd3822c6d26ec6` compiled in release
with static CRT. Both shader regression tests passed (three original shader
variants plus the non-bindless material contract). Its embedded revision and
system-only DLL imports were checked without gameplay execution. SHA-256:
`ef9b2c8f0b51d80e43f2928eb3b7aba948fff4e4225d91c78435b38699a30b2e`.
The private v7 recovery launcher is ready; the former bindless launcher now also
points to v7. The v6 binary remains preserved but is no longer selected by it.

The user reports poor FPS while looking into University and good FPS looking out
of the map, still CPU bound. The map being resident does not establish that every
triangle is drawn. The captured CPU render work is the actionable bottleneck.

The v4 audit capture `university-audit-7802-28261.json` recorded 4,918 frames in
20 seconds without dropped trace events. Mean command generation was 1.782 ms per
frame; the main opaque encoder finish accounted for 0.995 ms, versus 0.349 ms in
opaque draw traversal. The v5 shadow batch capture
`university-shadow-batches-12344-4318.json` recorded 3,309 frames without dropped
events. Main encoder finish remained 1.012 ms per frame. Shadow visibility fell
from 0.466 to 0.349 ms, but that did not address the dominant main-pass cost.
These are different workloads, not a controlled performance comparison.

A discarded CPU wall-occlusion prototype split at most 256 coarse batches and
used conservative whole-AABB occlusion against real opaque wall triangles. Its
static inward view culled only 55 of 3,512 candidates while costing about 1.15 ms.
It was removed before shipping; adding that CPU cost was unjustified.

## Implementation

RetailWorldMaterial now uses Bevy's bindless material allocator, with a resource
limit of 64 per slab. Materials share texture/sampler arrays and a packed parameter
buffer. The existing shared frame/shadow state remains a buffer reference. This
reduces distinct material binding groups and allows compatible meshes to share
multidraw batches. Pipeline, mesh allocation and resource capacity still constrain
batching; this is not a promise of a 64-fold draw reduction.

The main and depth/shadow shaders use the material slot stored in Bevy's mesh
instance data. Both retain the original bindings when Bevy reports that bindless
resources are unsupported. The common shader module is permanently loaded using
Bevy's shader-library macro so custom imports resolve at runtime. The shading
body, alpha rejection, geometry and texture content are preserved. No user
graphics setting, native physics, input or simulation cadence changes.

The unsuccessful v5 extra shadow meshes and their light-layer adjustment are
removed from the active renderer. Normal world meshes cast shadows again. The
data-only shadow experiment remains test-only for historical reproducibility.

Trace configuration includes `retail_bindless`: true means the device supports
the selected material-array path, false means fallback, null means device data
was unavailable. This indicates capability selection, not measured draw counts.

## Validation and acceptance

GPU-free shader tests compose the actual material shaders and real Bevy vertex
interfaces, using fixtures for unrelated lighting resources/functions. They
validate six combinations: main, shadow depth and depth with normal/motion output,
each with bindless enabled and disabled. A separate test checks the Rust-generated
resource index table and buffer bindings against the shader contract.

The release shader tests passed (both tests, six shader variants). Eleven world
adapter tests and two scene lifecycle tests passed; the offline map inventory
test remains ignored in this ordinary regression run.
Four tracing regression tests also passed, for 19 targeted tests in total.

Runtime pipeline creation, appearance and FPS remain unverified until the user
runs the private executable. The agent does not launch gameplay. Compare v4 and
v6 from the same stationary inward-facing view with identical settings, then the
same route. Inspect `retail_bindless`, main opaque encoder finish, total command
generation and frame percentiles. Keep the earlier executables for rollback.

The private v6 static-CRT release executable compiled successfully from
`bea625f9878148347e0fda0529bd0d6ba7b1a327`. Its embedded revision, matching PDB
presence and Windows-system-only PE imports were checked without executing it.
EXE SHA-256: `cd52c92a709372f95a41d27f6fea794867c1e651f9b8f9d693662ff898dc7325`.
The local `TRACE-UNIVERSITY-BINDLESS.bat` launcher uses the existing extracted
University/assets and captures 20 seconds after F9. Earlier builds remain intact.
Existing compiler warnings remain; this build does not establish a measured FPS gain.
