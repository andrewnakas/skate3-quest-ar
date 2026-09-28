# Native bail recovery

Automatic bail recovery now uses the TU3 actor checkpoint manager instead of
always replying with the map's initial spawn. The integration targets the
engine's existing single-player, static-map world. Gameplay validation is
reserved for the user; no game, recomp, controller harness, screenshot capture,
asset-check mode or test executable was launched during this work.

## Behavior

The actor first validates its current physical COM position using the retained
riding heading. If that fails, it ranks recorded checkpoints and consumes the
selected entry. Entries rejected by validation are also removed, so repeated
bails can walk back through history. Exhaustion uses the initial map transform
and current actor stance. Ordinary recovery preserves history; constructing a
new skater for a map constructs a new history.

Recording uses completed physical publications, not render transforms or a
last-grounded shortcut. It requires 120 empirical measurements, more than the
stock 15 frames in an eligible state, expired 20-update cooldown, and at least
1.5 units from every recorded position. The ring retains 32 entries. Surface
categories 5, 6, 9, 12 and 13 are excluded; category 8 requests offboard recovery.
Scores are 100 for categories 1/2, 50 for 3/4, 20 for 11 and zero otherwise. The
newest entry receives a 100-point penalty and entries older than five receive a
200-point penalty; newest wins ties.

The selected position, heading, stance and offboard byte go through the existing
State702 request/reply publication boundary. Selection does not reset physical
bodies. Existing wipeout timers, recovery countdown, physical reset and ragdoll
restoration remain in place. The explicit fixed-checkpoint helper used by the
existing manual test remains separate from automatic selection.

## Native evidence

Reference memory image `default_82000000_011B0000.bin`, mapped at 0x82000000:
SHA256 `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
Instruction comments in the matching generated PPC were inspected; numeric
constants were checked in the raw image. A read-only IDA lookup found no defined
function for 82BFB1E0 in the available dump database, so decompiler output was
not used as independent confirmation. This is a scalar translation, not a
claim of demonstrated bit-exact PPC floating-point equivalence.

| Address | Recovered behavior |
| --- | --- |
| 82BFB068 / 82BFCB38 | History construction and insertion |
| 82BFB1E0 / 82BFB3F8 | Recording gates, current orientation and COM publication |
| 82BFC9B0 | Current -> scored history -> fallback selector |
| 82BFC038 / 82BFC378 / 82BFC550 / 82BFC718 | Current candidate, history validation, scoring/removal, fallback |
| 82BFC828 | Separate non-destructive manual/session-marker query; not used here |
| 82592518 / 825926F8 | Save selected stance, queue deferred actor reset reply |
| 82D431F0 / 82D43280 | State702 reply capture and physical output |
| 82591E30 / 82592A00 / 82C01BF8 | Recording transform and solved deck source |
| 82BFBC18 / 82BFB6F8 | Ground/capsule and authored-edge validation |
| 82DE5858 / 82DE5588 | Empirical measurement count and reset lifecycle |
| 82B97350 / 82B97308 / 82B972A8 | Save15200 and deferred stance restoration |
| 82B98050 / 825953B0 | Selective animation and motion-output reset |

## Host bindings

Riding checkpoints use the solved board frame, with Processed2468 bit20
inverting X/Z, plus 0.2 Y. Above speed squared 0.25, a velocity-derived upright
basis is accepted only when projected forward length squared exceeds 0.9.
Offboard checkpoints use the animation-to-world frame with Processed2476 bit2
inverting X/Z; the native function returns before the velocity-heading branch.

Current position is Skeleton64 (physical COM16144); spacing uses Skeleton416,
the unmirrored animation-to-world translation. OffBoard333 comes from the
canonical contact-correction owner. Both native OffBoard52/56 fields receive
Processed2596; their category is packed bits7..11. State age comes from the
physical state publication. The actor-owned empirical count increments once per
completed physical output and resets with physical teleport, retaining history.

Ground323 is GrindManager460/result412 bit27. The producer was recovered in
82D875A8: after accepted-family attempts fail, the raw truck/deck/inverted/tip
contact path at 82D87DE8 sets 0x08000000. The host retains those same raw query
results and publishes the rejection flag separately from active grind state.
No new rail distance or last-grounded heuristic is used.

Ground validation casts from candidate +0.1 Y down ten units, checks normal Y,
maximum drop and category, then tests a vertical capsule above the ray start.
It reuses the canonical triangle query and matching groups. Edge validation
uses authored static edges in provider order, capacity40, with bounds
[0.3,0.6,0.3] and squared segment distance threshold0.09. It does not use triangle
diagonals. Current candidates repeat all predicates; historical candidates
repeat location/occupant checks only, matching native selection.

Normal-world location validation returns true before invoking the alternate
provider. Occupancy predicates in 82BFB928 belong to LivingWorldManager, not
static terrain. This engine scene has no pedestrian/vehicle actors or alternate
challenge controller, so those providers are empty; static obstacles remain
covered by the capsule. Adding living-world actors or alternate-world maps will
require binding their providers here, as with the existing offboard adapters.

Saved stance survives ResetSkaterAnimation and is consumed by
ResetToGivenStance at the actual MotionGraph dispatch. Updated flags/relative
stance are visible to later nodes in the same traversal, including TELEPORT
playback. The canonical actor publication then receives the modified values.
The animation reset now preserves the native local-player/board flags and saved
request rather than clearing all flags. The stock `teleport.xml` confirms the
ordered ResetSkaterAnimation -> ResetToGivenStance -> PlayAnimation sequence.

## Stock data

Class `Hash_12B64C0E804B0853`, key `default`, already exists in installed data.
No new tuning file or fallback tuning values were introduced.

| Field hash | Value | Meaning |
| --- | --- | --- |
| C0526C883AF0ECCA | 0.4 | Capsule height |
| CEB092E418A5B001 | 0.5 | Capsule radius |
| 8ABE098D3806D273 | 1.0 | Maximum ray drop |
| ADD032CACF6A1C15 | 0.75 | Minimum normal Y |
| 10B7C3A9CC8D3721 | 15 (Int32) | Minimum state age |

Native occupant-query radii E64C980EE2114070 and 59E1F2B3CDC4E658 are 1 and 2;
the current empty living-world provider does not consume them.
Source skatercollections.vlt SHA256:
`3b7dbd062bb1c906a085514355afff35cfa22f486ae70820c5ad1a42a7aab25b`.

## Validation and user testing

Release compilation uses the requested x86_64-pc-windows-msvc static CRT flags,
locked dependencies, and no default game features. The six core selector cases
compile with --no-run; they have not been executed. Added host cases cover real
scene recording/recovery, heading thresholds and the actual stock reset dispatch.
The whole game test target cannot currently compile because pre-existing tests
in graphics_menu.rs omit Menu.maps/selected_map, and skate_world.rs omits the
RetailWorldMaterial argument. Those unrelated map/render fixtures were not edited.

The final executable is copied into this task's ignored bin/native-bail-recovery
directory; Test-Native-Bail-Recovery.bat uses that exact copy and installed assets.
It has not been executed. Its sibling gameplay.log records passive BAIL_CHECKPOINT
selection messages. Test riding away from spawn for several seconds, bailing on
open ground and near an edge, repeated bails, both stances, and offboard recovery.
Gameplay parity and visual behavior remain unverified until the user runs it.
