# Archive 5 re-merge

Baseline: the user's build at `ba92948`. Source archive: `crates (5).zip`,
SHA256 `ca57392b5625f228a46d32cb10b3455f9946f6bfefefb08cfd649ac3706569d5`.
The first attempt is retained on `lol/archive-crates-5-integration` for reference.
The replacement is on `lol/archive-crates-5-selective`, rebuilt from the baseline.
Previously reviewed adapter code was reused where it fits the agreed ownership;
the archive was not applied as a replacement source tree.

## Agreed ownership

| Area | Decision |
| --- | --- |
| Off-board movement, board possession, mounting/landing | Archive systems |
| Grinds, spline queries, material/family handling | Archive systems |
| Powerslides | Archive runtime and settings |
| Reverts | Dedicated controller enabled after reported non-working reverts |
| Manuals and FPS display | Add archive support |
| Session markers and automatic bail recovery | Keep user's features |
| Plants, boneless and handplants | Keep user's behavior; adapt query interfaces |
| Scoring/HUD, crash reporting, outfits/custom models | Keep user's features |
| Map management, multiplayer, menus, replay | Keep user's features |
| Pose sampling, authored clip overrides, blend-space evaluator | Keep current implementation |

The archive redirects `RevertGround` requests to ordinary ground physics and
does not supply a dedicated revert solver. After the user reported non-working
reverts, this redirect was removed so the existing revert Enter/Update/Fill
lifecycle can execute. Entry and exit now emit `REVERT_ENTER` / `REVERT_EXIT`
diagnostics. Scoring consumes the published State66 revert-active flag; the
previous State70 lookup was always false and suppressed revert HUD announcements.
The local native reference has a dedicated revert Update at
`0x82D43518` (vtable `0x82327330`); enabling the owner does not establish runtime
parity. Powerslide runtime, update and settings files still match the archive.

## Integration details

The new off-board/grind owners use the current marker and respawn services.
Manual marker returns retain the saved on/off-board state and clear pending
physics work. Manual markers and automatic bail checkpoints remain separate.
Plants query the new spline/trajectory interfaces. Multiplayer collision
publication reads the new per-part board-volume state. Optional climbing resumes
through the new off-board contact owners.

The preservation audit found 46 feature files unchanged from `ba92948`, and
eight adapter files changed: boneless launch, climbing approach/test queries,
footplant ground/edge queries, handplant queries, network collision publication,
and automatic-respawn observations. Shared frame/state coordination also adapts
these retained features to the archive owners; unchanged feature files alone do
not prove runtime compatibility.

The user's interpolated animation/replay presentation function is unchanged.
Only the archive's independent skater-culling fix was added around it. Existing
native bump Begin dispatch/arithmetic, decoded-key sampling, authored clip
replacements and weighted blend arithmetic remain. Unsupported stock predicates
are not replaced with guesses, and optional unsupported branches retain explicit
lazy diagnostics. The silent archive deck-angle no-op remains an explicit error
if reached.

Collection lookup accepts both readable and numeric AttribSys names. This fixes
the first build's `Missing stock collection anim_motion/manual` initialization
failure without changing or regenerating assets. Two data-only regression tests
passed in the investigation; a static audit resolved 157 literal lookups against
the prepared 6,849-collection export, including payload sizes.

## Validation and private build

`cargo check --workspace --tests --release --no-default-features --offline`
passed for the reconciled owners. Gameplay tests were compiled, not executed.
The final Windows executable is linked directly to an ignored private filename
with explicit `/OUT` and `/PDB`, using the shared target cache and static CRT.
The launcher is updated only after successful linking and PE/PDB provenance
inspection. The ignored build manifest records the exact source revision/hash.

The existing prepared customiser assets, ten maps, scoring HUD and marker overlay
are reused. No ISO extraction, Blender, clean build, game launch, Steam session,
push, merge into the main branch, or release is performed by this task.

## Manual checks

1. Start University and resume; check that initialization completes, the avatar
   remains visible and the HUD/FPS display appears.
2. Push, steer, brake, ollie, flip and manual. Try powerslides in both directions
   and at low/high speeds; watch for sudden spins, loss of control or stuck poses.
   Try reverts in both directions while rolling, including switch stance. Watch
   for missing turns, continuous spinning or failure to return to normal riding.
   If they fail, include the launch log and whether `REVERT_ENTER` appears.
3. Dismount, run, jump, fall, throw/retrieve and remount. Watch for floating feet,
   immediate landing poses, board separation and unexpected bails.
4. Enter/exit grinds from both sides; exercise plants, handplants and boneless.
   Check for missed contact, abrupt launches and states that fail to end.
5. Set a marker with LB + D-pad Down and return with LB + hold D-pad Up, on and
   off the board. Bail separately; confirm the manual marker is not overwritten.
6. Change maps, difficulty and outfits, then repeat recovery checks. Check replay
   interpolation and multiplayer appearances/contacts in a prepared session.

Report failures with the diagnostic report, map/difficulty and last controller
action. Compile and source-preservation checks do not establish gameplay parity.

## Takeoff heading regression

Restored the `c70726d` takeoff-heading separation removed by the re-merge.
GroundAnimation retains the rider heading while the authored deck begins to
spin; ordinary ground riding still follows the physical deck. The imported
processed ground-input observations remain in use. The existing
`nollie_inward_heel_keeps_body_heading_at_pop` regression compiles but was not
executed (private gameplay assets required). Check nollie inward heels and
other flip tricks, stationary and rolling, for an abrupt sideways body turn
at takeoff while ensuring the board still spins and intentional body spins work.
