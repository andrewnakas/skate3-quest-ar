# University render batching experiment

**Superseded in the active renderer:** the user's v5 test did not improve the
reported bottleneck. The v6 [CPU material binding experiment](cpu-material-binding-experiment.md)
removes the extra shadow meshes and layer adjustment. The inventory below records
the historical v5 candidate, not the current production spawn path.

The user authorized engine/content visual tradeoffs after the initial exact-output
audit, while keeping resolution, MSAA, user settings and native gameplay timing
unchanged. This first structural experiment targets redundant shadow submission;
it does not yet simplify triangle geometry or lower shadow-map quality.

## What the inventory established

Read-only analysis used the same University v14 package and actual material builder
as the preceding captures. Synthetic image handles in the inventory preserve
canonical texture identity, sampler role and invalid-cube fallback; no GPU starts.

| Quantity | Before | Experiment |
|---|---:|---:|
| Main world batches before visibility | 4,870 | 4,868 |
| Opaque static world shadow casters/batches | 3,414 | 680 |
| Opaque shadow triangles | 1,142,084 | 1,142,084 |
| All main world triangles | 1,645,617 | 1,645,617 |

Exact main-material consolidation saves only two batches (0.04%). It is not the
large main-pass optimization suggested by the original material count. Further
main-pass gains would require a different binding/submission strategy or an
intentional content change, not merely removing unused material metadata.

Opaque shadow batch count falls by about 80%. The remaining 1,414 masked, 12 blended
and 28 fallback world batches retain their existing shadow paths. Consequently,
the static-world caster candidate count is approximately 2,134 rather than 4,868
after main consolidation, before per-view/cascade culling. These are resource
counts, not measured draws, GPU time or an FPS improvement.

The additional proxy meshes contain 1,358,304 vertices and 1,142,084 triangles:
46,304,304 bytes (44.2 MiB) of position/normal/index payload. Allocator and renderer
metadata are additional. This trades memory and some mesh preparation for fewer
shadow candidates and material switches; only a runtime comparison can establish
the net benefit. The 64 m cell size is a conservative initial experiment, not a
measured optimum.

## Implementation

`RetailWorldMaterial::batch_key` compares every current uniform lane by float bits,
texture/sampler-role handles, shared storage, alpha cutoff and culling state.
Source-only metadata no longer forces separate identical retail batches. Blended
geometry retains its old grouping and sorting centers. Batches needing generated
tangents remain separate to avoid changing averaging across old boundaries.

For opaque static retail meshes with no alpha rejection, `retail_shadow_geometry`
assigns complete triangles to 64 m cells by centroid and separates one-sided and
two-sided casters. Bounds come from the full vertex positions, including triangles
that cross cell boundaries. The shadow meshes retain authored positions, normals,
indices and winding. They use two shared opaque depth-capable StandardMaterials,
so they do not bind all the world's surface textures for depth rendering.

Original opaque main meshes receive `NotShadowCaster`; their shadow geometry is
on reserved internal render layer 27. World lights whose masks contain layer 0
also receive layer 27 before visibility processing. Main cameras do not receive
that layer. The layer-28 player-only shadow light is unchanged. This also handles
world point/spot lights; University itself contains no authored map lights.

Alpha-tested foliage, transparent surfaces, unsupported/fallback materials, sky,
backdrops and character meshes retain their prior paths. Collision, rails, physics,
input sampling and simulation cadence are untouched. All proxy entities and meshes
use the normal map asset ownership/retirement path, including abandoned preparation.

## Verification and runtime acceptance

Thirteen release tests passed for the map/render adapter, including new checks for
effective material identity, distinct lightmap/blend groups, full boundary-crossing
shadow triangles, exact normal bits, culling modes, reserved light/camera layers
and proxy retirement. Two scene lifecycle tests and two character-lighting tests
also passed. The separate ignored University inventory test passed exact triangle
multiset equality for both main geometry and the opaque shadow geometry.

No gameplay or GPU was started by the agent. Runtime acceptance still needs the
private build to load and render correctly, especially main-world lighting on the
skater, player shadows on the ground, foliage silhouettes, camera movement and
map transitions. Compare shadow visibility, shadow queue/draw costs, render command
generation, frame percentiles and memory against the preceding build. The previous
EXE/PDB and captures remain available for comparison and rollback.

The capture includes `main_opaque_pass_close` and `main_opaque_encoder_finish` CPU
scopes added in the preceding build. They should identify whether the dominant
main-pass command time is draw traversal or pass/encoder completion before a larger
material binding redesign is attempted. No FPS gain is claimed from the inventory.

Private static-CRT release build `e109a0a` compiled successfully with the existing
shared Cargo target. Its embedded revision and Windows-system-only PE imports were
verified without executing it. The EXE SHA-256 is
`c919bf6d8166c8f6ca65cdb41d598f37e24f10174282dc0e67af33f676099e35`.
The local v5 launcher records 20 seconds with GPU diagnostics after F9. For an
isolated comparison, use the preceding v4 audit launcher first, then v5 with the
same scene, settings and route. Existing compiler warnings remain.
