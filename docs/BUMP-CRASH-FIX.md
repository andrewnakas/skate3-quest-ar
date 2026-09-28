# Bump reaction crash, 2026-09-06

The user's `logs/game-20260906-041143.stderr.log` stops at animation tick 4159,
MotionGraph behaviors 3623 and 3624. `B_BUMP` was rejected as unsupported tree
type 6, followed by unsupported `SetBumpCoefficients`. Map loading and character
initialization had already succeeded.

The implementation now reads and evaluates Andale BlendSpace records and binds
the missing Begin behavior. It retains the stock children, authored simplex
planes, coefficients, attribute operations and phase-coupled clip clocks.
It does not suppress either error by skipping the reaction or change movement,
collision, difficulty, or map settings.

## Evidence

Static analysis used the TU3 community IDA database copied into ignored
`logs/bump-analysis.i64`; the source database was not modified. Generated PPC
instructions were cross-checked in the local `SK8R15/Source/generated` recomp.

- `82BC7318`, vtable `823200C8`: constructor reads X/Y FastStrings; Begin at
  slot +48 is `82BB0660`, Update/End are empty `82B61BB8`.
- `82BB0660`: takes conditioned PhysOutAnimation +112, scales X, computes
  direction and magnitude, maps the stock min/max bump magnitude into the stock
  minimum blend value through 1, and reverses both coefficients for live
  ISkaterAnim virtual +28 (mirrored stance). Emits X and Z as X/Y parameters.
  All four settings come from `anim_motion/bumps`. The normalization threshold
  at `830BD350` is initialized by `82F826F8` to `1e-6`, not its on-disk BSS zero.
- `82D22DA0`, `82D23898`: type 6 layout, serialized simplex vertices, normals,
  inverse heights and child indices. Stock `B_BUMP` has 26 children, 48
  tetrahedra and parameters `DISTTOCOG`, `BUMPX`, `BUMPY`.
- `82D23080`, `82D23530`, `82D23A50`: original scalar solver, including ordered
  plane slicing and clamped barycentric reprojection outside the authored hull.
  The host uses this scalar path for three dimensions as well. It does not
  recreate the original accelerated job submission.
- `82D24240`, `82D24580`, `82D16370`, `82D16428`: weighted attribute intersection,
  scale and accumulation. The native single-attribute query's different child
  indexing is retained.
- `82D246A8`, `82D24730`, `82D24AB0`, `82D24B28`: weighted length and synchronized
  child advance/time/speed operations.
- `82D23EF8`, immediate ACS `828CBF68..828CC0B4`: accumulate weighted SQT poses
  in source order, then normalize rotation once. The original multi-pose path
  does not use the two-pose shortest-arc hemisphere decision.

Rust scalar/VMX-style arithmetic follows the recovered instruction order where
represented; native reciprocal-square-root estimate helpers are not a claim of
bit-exact Xenon execution. No console trace or visual parity claim is made.

## Verification

Focused core tests cover coefficient thresholds, axis scaling, mirrored stance,
plane projection, clip synchronization and multi-pose quaternion accumulation.
The private-stock regression decodes all 26 clips, evaluates all authored simplex
vertices and interiors (243 parameter probes including outside-hull points),
checks finite weighted poses and reconstructs authored parameter positions.
It executes the exact logged graph behaviors 3623/3624 in both mirrored states
through parameter application, pose generation and attribute publication.

Run the stock regression with `SKATE3_ASSET_ROOT` pointing to this repository's
`assets` directory:

```powershell
cargo test -p skate-game stock_bump_crash_regression --locked --offline -- --ignored --nocapture
```

Gameplay and visual testing remain with the user. The earlier separately logged
`AddRunoutAttribs` behavior 86 remains unsupported; this patch repairs the later
bump crash and does not claim every imported controller path is complete.
