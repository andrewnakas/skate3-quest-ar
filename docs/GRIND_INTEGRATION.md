# Grinding integration

## Current milestone: authored geometry, not playable grinding

The test course now owns seven named paths in `grind_world.rs`: two flat bars,
both half-pipe lips, and the three exposed platform edges. The downhill entrance
is not an edge, and triangle diagonals/transition tessellation are not splines.
Bar top centres and coping paths share the collision/render dimensions.
Climbing geometry from the other task is preserved.

`GrindGeometry` retains a relocatable Pegasus tSplineData blob. Its layout matches
the existing recomp's `owned/world/src/grind_spline.cpp`: 16-byte header,
32-byte rail records, 144-byte segments, signature `2C7017070007004A`, straight
segment delta at +0, origin at +48, inverse length at +64, bounds +80/+96,
length +112, cumulative length +116, and owner/previous/next +120/+124/+128.
IDs are stable host-authored integers, not guessed retail asset IDs.

This registry is not yet connected to contact acquisition or state selection.
Startup explicitly logs `native_acquisition=not_connected`. The game therefore
still treats the bars as ordinary collision geometry. Do not call this playable
grinding or replace the existing empty-edge providers with a proximity snap.

## Native integration audit

Evidence image: TU3 `default_82000000_011B0000.bin`, base `82000000`, SHA-256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
Community symbols plus generated PPC instruction comments were consulted.
The disposable IDA database's decompiler misdecodes some VMX128 operations;
its truncated pseudocode is not sufficient to port the nonempty branches.

- `82D8A828` GrindManager input update: constructs an AABB around processed
  position112 with half-extent1.2 (`821659F4`), unless byte2476 bit0 disables it.
- `82C1EAD8`: static asset/segment bounds collection, source order, maximum40;
  entries are48 bytes with endpoints and retained owner/flags. The final16
  bytes must not be invented as a surface normal.
- `82D875A8`: candidate acquisition/scoring. Dispatches truck/deck contact
  queries (`82C1FDC0`, `82C20130`, `82C1FB98`, `82C1F968`) before scoring. For
  example `82D89150` handles a two-truck candidate; it is not a nearest-rail
  angle classifier. Selection output has candidate byte384 in a416-byte record.
- `82C1E2C8` is now ported in core `physics/grind_contact.rs`: the exact
  segment/triangle leaf used by those truck/deck queries. It retains the
  zero-denominator rejection and closed0..1 bounds, unlike wheel probes with
  their endpoint tolerance. It does not itself accept a grind.
- `82D8AB08`: post-input manager publication precedes state selection.
- Native selector maps actual candidate grind type0..5 to states401,400,402,
  403,404,405. `player_state/selection.rs` currently publishes no candidate.
- Boardslide Enter `82D41100` calls shared grind entry `82D3F318`, then clears
  bytes240/241. Shared entry resets board controller settings, skeleton state
  and the grind manager; merely allowing state400 in transition.rs is unsafe.
- The active force/contact owner, output conditioner and graph attributes
  still need wiring. `player_state/publication.rs` currently uses default grind
  output; camera output has the same inactive assumption.
- Air acquisition also needs the nonempty trajectory path: `82D69F80`,
  `82D6A168`, `82D6A398`, `82D6A840` and GrindAirAdjust `82D712E0` onward.
  The clone's GRIND_CONTACT_SPEC explicitly leaves parts of this arbitration
  unresolved, so it cannot serve as a complete implementation.

Next implementation boundary: native endpoint/contact queries and candidate
record/scorer, then lifecycle/constraint/output integration as one coherent
vertical slice. Do not enable selector candidates before those owners exist.

Validation: build only; no gameplay or automated tests, per user instruction.

## 2026-09-07 collision and contact follow-up

The rail boxes were emitted with only ONE_SIDED. Native contact fixup82AD3130
rejects an edge-region contact when its convex bit is absent. Authored rail
faces now set the two perimeter-edge bits on each triangle and leave the
coplanar diagonal non-convex (0x60 /0xC0 for the existing triangle winding).
This change applies to rail/coping/support boxes only. Gameplay confirmation
is outstanding; a build does not establish that this explains every missed hit.

Core truck_contacts ports82C1FDC0's two local-board probe rectangles, using
TruckToWheel and DeckCenterToTruck, the -.02 start offset and -.20 depth,
first-triangle priority, strict squared-distance comparison and first-entry
tie retention. It remains a contact query, not an engagement decision.

Checked the task Research readable game recreation and its referenced
Skate3CustomEngineLayer/src/skate3_native_grind.cpp. That integration registers
splines in the original guest GrindData owner; it does not contain replacement
grind forces. Expand Skate 3 replay toolkit likewise supplies research and
capture infrastructure rather than a standalone grind controller.

The remaining native state/force and graph-attribute contracts are unresolved.
Do not advertise this build as grind-ready. Build log:
logs/grind-collision-build.log (successful,35 warnings); no tests executed.

## Additional static recovery

The reference process was closed at the user's request. Continue using static
analysis; do not relaunch it. IDA database
`research/reverse-engineering/XexDump/Skate3TU3_dump.i64` was copied into local
ignored `logs/grind-research-copy.i64`. The original was not modified. The
database has sparse function boundaries; bounded definitions are needed even
where the community symbol map supplies a name. VMX128 decompilation remains
unreliable; cross-check vector operations against generated instruction text.

New core routines (not yet called by the game's active state):

- `fifty_fifty_candidate`:82D89150 same-primitive two-truck branch. Rejects
  balance magnitude >=.9 and local-up contact depth >=.13. Adjacent-segment
  arbitration is deliberately outside this routine.
- `upright_normal`:82D370F8; the vertical-line fallback is the actual constant
  X axis at82139A10. `within_approach_angle`:82D88518's plane projection and
  strict cosine comparison.
- `grind_forces::lateral_pin`:82D3FA18. Native50-50 caller82D41D70 supplies
  strength800, forward offset0, up offset.07, damping scalar.133.
- `grind_forces::friction`:82D3FD88 after material selection. Receives actual
  support, material and engagement inputs; does not invent them. Native50-50
  caller supplies strength50 for engagement0,40 otherwise.
- `animation::grind_control::Fade`:82BB0D40; Begin82BB0A30 evaluates attribute
  bounds. First Update only clears the Begin latch. Later updates apply the
  native target, response, acceleration and step bounds.
- `grind_control::crouch`:82BB10F8. Begin82BB1078 seeds from physical crouching
  output. Runtime collection names for owner312 offsets1640..1652 and owner340
  offsets1760/1792/1816 still need resolving before host integration.

Vtable823283D0 identifies50-50: Enter82D3F318, Exit82D3F430,
PreUpdate82D3F4E8, Update82D41D70, Fill82D41FF8. Its angular target path
82D41E38 ->82D40890 uses existing matrix interpolation82BD3150 with weight.1,
then hook target82BD4318 and angular-only hook drive82C05658. It does not
teleport the deck to a spline. This should use the existing core interpolation
and hook implementation when connected.

The playable-state integration is still incomplete. In particular no selector
candidate is published, the active state adapter is absent, and MotionHost
still rejects the unbound stock grind behaviors. These ports alone are not a
grind-ready game build.
