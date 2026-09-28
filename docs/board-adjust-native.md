# BoardAdjust native lifecycle and release

The cleanup-only change `fd280fd` was reverted in `411260a` after a user
reported a snap during return to neutral. This replacement ports the complete
BoardAdjust lifecycle, rather than changing animation timing to hide the snap.

## Reference and observed behavior

Read-only sources under `Documents/Skate3Research/research/reverse-engineering`:

- `generated/skate3_animation_research_recomp.71.cpp`, SHA-256
  `a4b47427402cfc4fc6aa0a6e5b08d4fe81b974da669c107e610a960ec2652257`.
- `XexDump/default_82000000_011B0000.bin`, SHA-256
  `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
- The same guest instructions decoded with IDA 9.3 in a disposable database.
- Stock ActionGraph BoardAdjust includes, `MotionGraphIncludes/air.xml`, and
  `MotionGraphIncludes/BoardAdjusts/T_BoardAdjust.xml` from `MiscloadState`.

Addresses below are TU3 guest virtual addresses (image base `0x82000000`), not
addresses in the Windows reference executable. The reference installation was
located at `C:/CLEAN/Skate3CustomEngineLayer-0.1.0-preview.18-Windows`;
its `skate3.exe` SHA-256 is
`4d5234b2297fa7f1e02808ce2780ad37c960e4da6d26ae5004591cba91fe6904`.
It was not launched or modified; no runtime parity claim is made for that build.

Observed in recompilation and corroborated by IDA:

| Callback | Guest address | Behavior |
|---|---|---|
| Constructor | `82BA2A30` | Magnitude name at object+28, angle name at +52, angle filter at +76, mirror-negation default **true** at +80. |
| Allocate | `82BA2F48` | Separate 16-byte instance for each behavior. |
| Begin | `82BA2BA8` | Clear instance wrap mode at +8 and previous raw angle at +12. No intent publication. |
| Update | `82BA2BC8` | Require both source intents; copy magnitude, filter angle, optionally negate for mirrored stance, apply persistent angle seam logic. |
| Missing input | `82BA2E88` | If either source is missing, remove both outputs and clear both instance fields. Present zero remains a valid value. |
| End | `82BA2EB8` | Remove both authored outputs; no instance reset here. |

The seam logic at `82BA2D70` detects a sign change when the previous raw angle
has absolute value greater than pi/2 (`822538BC`, bits `3FC90FDB`). In mode 0,
crossing from negative enters mode 1, crossing from positive enters mode 2.
Mode 1 holds -pi (`822F8908`, bits `C0490FDB`), mode 2 holds +pi
(`82060C44`, bits `40490FDB`). Crossing back from the opposite side clears
the mode. Every successful update stores the **raw filtered angle**, even while
the published angle is held at pi. This state was missing from the Rust host.

## Release and animation handoff

The stock graph filters missing axes toward zero with blendOut 0.08,
clampAcc 0.03, clampVel 0.2. Recompilation `82BB17D0` agrees with the existing
filter arithmetic, including the missing-input branch. The board-adjust state
exits when the directional intent is absent and both absolute filtered values
are below 0.4. The default airborne tree is `B_AIR_CYC` with authored time 0.2
and transitionUnder enabled. Blend evaluation/weight functions `82B961F8` and
`82B963D8` agree with the existing blend progression. No duration, threshold,
filter coefficient, pose offset, or extra smoothing has been fitted or changed.

The new tests cover the native seam branches, mirror signs, separate instances,
entry/update/exit, partial input loss, present zero, stock constructor defaults,
and the stock filtered release followed by its 0.2-second animation-tree blend.
These are source-level tests, not a replay of the user's controller sequence.
The reported visual snap's precise cause remains unconfirmed until gameplay
validation; the additional native discrepancies are independently established.

Validation (Windows MSVC release/static CRT): seven action-host tests passed,
including stock graph parsing; the stock release-to-air animation test passed.
That test observes a zero-weight incoming tree at handoff and the authored blend
progression to full weight. It does not measure rendered bone-pose continuity.
