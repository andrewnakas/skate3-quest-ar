# No-complys, footplants and handplants

The stock graphs now reach physical states 602 (no-comply/boneless/fastplant),
601 (air footplant) and 600 (handplant). The implementation connects the TU3
contact selectors, authored animation events, VLT curves, board drives, skeleton
roots, limb IK, release trajectories and state publication. `IsFootPlanting` is
registered in both graphs so its formerly unsupported leaf no longer pauses the
one-foot path.

## Controller controls

These are the existing Xbox mappings and stock graph transitions. Stance and
contact determine which animation and physical state can enter.

| Action | Input |
| --- | --- |
| No-comply | Begin a front-foot push (X in regular, A in goofy), then flick the right stick for an ollie during push entry. The graph accepts the Square gesture group. |
| Boneless / fastplant | Hold one grab trigger fully, then press A or X. The chosen trigger, push foot and stance select the stock variant. |
| Air footplant | Ollie, hold LT or RT for a grab, then press and hold A or X for a one-foot air while approaching a reachable surface. Contact prediction and the extended foot determine entry. |
| Handplant | Hold RB while riding up a steep transition toward reachable coping. Move the right stick to tweak; A/X select one-foot variants and B selects the no-foot variant. |

No-comply launches come from the clip's `FootJump` event. Footplants release at
the manager's calculated curve duration. Handplant entry requires upward travel,
the authored minimum speed/slope, a selected edge and a valid surface
investigation. Holding RB on flat ground does not force a handplant.

## Local build

Use the standard build and launch scripts with your prepared assets. The test
world supports no-complys and reachable footplant surfaces; handplants require
transition/coping geometry satisfying the native selector. Select a converted
map from the pause menu or supply it through `--map`.

Every launcher run now saves both console output streams, build identity and the
exit code to a separate `logs/plants-<timestamp>-<random>.log`. Rust panic
backtraces are enabled. On failure the launcher displays the saved output and
keeps the console open. Earlier launcher runs did not save output, so their
simulation error messages cannot be recovered from this directory.

The RMB/RB crash recorded at tick259 was an explicit Ground error on
`GrabWorld` (Processed2476 bit22). That older placeholder misidentified
`82D38430` as an unsupported Skitch query. The TU3 function actually submits
the handplant candidate via `82D61268`, or resets it via `82D62F20` when the
input is released. Those paths already run through `handplant::ground_query`
and `ground_update`. The obsolete error and its unused duplicate manager have
been removed; input still reaches the stock graphs and handplant selector.

The subsequent normal-map run failed at tick4225 in KnownAir because enabled
foot part15 carried volume group4. This is the existing native
DisableCollisionWithWorld classification, not an unsupported shape. Enabled
group4 shapes now stay in the assembly collision pass but are excluded from
the world query. Re-enabling world collision by restoring group0 takes effect
on the next query. This fixes that explicit exit.

Off-board GrabWorld no longer exits when the host has no interactable object
instances. The current static-world adapter returns an empty candidate result
(`82D4D150`'s zero-count path), cannot manufacture a candidate key, and commits
no object action. Authored input, static line/edge queries and skeleton updates
continue. Actual interactable-object loading, queries and actions still require
an object-scene adapter; this change does not implement those features.
The separately logged off-board non-finite follower velocity remains under
investigation, with source target values included in diagnostic builds.

The handplant outgoing selector now distinguishes the surface contact used
for admission from the trajectory position at contact time used for the
landing anchor (`82D66B64..98`, `82D66BC0..BF8`). The host query adapter also
rejects lifted landing targets at or above the apex: those cannot produce a
descending `82D608A0` arc and previously reached its negative square root.
It continues the six authored queries and uses the existing three-second
fallback if none is feasible. This feasibility gate is a host query adaptation,
not a claim that the additional gate occurs in TU3.

The tick2032 grind crash reached `IsDroppingIn` in the ActionGraph, which
previously registered the condition only in MotionGraph. Both now read the
same completed Grinds324 flag. Handplant attempts additionally log admission
inputs every half second while GrabWorld is held on ground, every selected
candidate and its surface classification, and relevant state transitions.
These traces diagnose rejected entries and bails; they do not establish that
handplants are gameplay-validated.

The task executable is a separate copy, built with:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --target x86_64-pc-windows-msvc -p skate-game --no-default-features
```

The shared target is retained. Copy its `x86_64-pc-windows-msvc/release/skate3rust.exe`
to `bin/plants/skate3rust.exe` after rebuilding this branch. No installed or
running desktop executable is replaced. The launcher contains a machine-local
asset path; other installations must update that path.

If another worktree has populated this shared target with different workspace
crates, refresh the modification times of `crates/skate-core/src/lib.rs`,
`crates/skate-data/src/lib.rs` and `crates/skate-game/src/main.rs` before building.
That forces the local crates to rebuild without deleting the dependency cache.

## Native provenance

Evidence is the owned TU3 executable image `default_82000000_011B0000.bin`, base
`82000000`, SHA256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`, generated PPC
instruction listings, stock OnBoard graphs, OnBoard.abin, and skaterschema/VLT.
Registration strings and vtables identify the graph leaves. VMX128 instruction
operands take precedence over unreliable decompiler output and community names.

| Behavior | TU3 evidence |
| --- | --- |
| Input | `8259A554/8259AF60` emits GrabWorld from packed bit28; `8259BB18` emits five handplant controls, named by constructors `82F84F28..82F84F98`. Existing A/X pushes feed the authored no-comply and one-foot paths. |
| No-comply / boneless | State602 vtable `82327434`; entry `82D4C900`, update `82D4C9B8`, launch `82D4CB18/82D4CC18`. Anchors the completed physical toe, uses the stock linear/angular drive modes and four launch curves. |
| Footplant | State601 vtable `82327400`; manager Start `82D704B0`, adjustment `82D70CE0`, update `82D70E40`, publication `82D71040`, nearby-edge refinement `82D6FBB0`. Foot/toe volume setup `82D91330`; postphysics `82D4C6D8`. |
| Handplant contact | Manager `82D61040`; ground query `82D38430`, admission `82D61D88`, position selector `82D63D40`, frame/window/edge clipping `82D640F0..82D65D58`. Uses actual world grind primitives, capped at 40 broadphase candidates. |
| Handplant motion | Launch `82D613B0`; six outgoing queries `82D66318`, selection `82D66948`; arc constructors `82D60770/82D608A0`; COM `82D625C0`, Bezier `82D62408`, orientation `82D62C88`, time warp `82D631A8/82D63280`. Collision position accessor `82D2D988` and landing-normal accessor `82D2D9E8`. |
| Handplant skeleton | State600 vtable `823273CC`; entry/update `82D4C350/82D4C3D8`; reckoning `82D8E1C0`; hand IK `82D633B0`, target clamp `82BD9728/82BD97D0`, upper-body volume suppression `82D91298`. |
| Shared skeleton | Anchored-foot path `82BDEC48`, COM path `82BDF090`, root positioning `82BDEEB8/82BE0E80`. Uses the existing completed animation, physical skeleton, board and four-limb IK owners. |
| Graph timing | `IsFootPlanting` `82BBD888`; preparation `82BBD7F0`; FootPlantAbsorb Update `82BBE4E0`; handplant phase/new-position `82BBDA98/82BBDB98`; antic Begin `82BBE5D0`; handplant scoring Update `82BBF758`. |
| Lifecycle | Footplant release `82D8B970` reads big-endian byte2480 bit0, hence full-word bit24. Common handplant publication `82DB7130..82DB7190`; FootPlant wipeout check `82D8FDC0`; HandPlant post `82D4C530` calls Air `82D90358(false)`. Teleport/reset paths clear active plant state. |

Settings are loaded from the installation, using the schema's cached layout:

| Collection class | Used data |
| --- | --- |
| `CCB95A83C78B4FF9/default` | Four PointNegGraphData8 launch curves for state602. |
| `physics_footplantmanager/default` (`172D553937D8E07C`) | Radial speed curve; contact/deck dimensions; leg lengths; Bezier handles; minimum/maximum plant duration; release and wipeout limits. |
| `55B5940075F78842/default` | Seven PointGraph4 query-window curves; depth and vertical drop. |
| `physics_handplantmanager/default` (`0F38084037CB9175`) | Time warp; orientation/heading curves; hand-radius curve; entry/release/IK timings; admission thresholds; apex and outgoing-velocity parameters. |
| `anim_handplant/default` (`F190522E738EC5CB`) | Out/into/antic thresholds at offsets0/4/8. |

The antic blend reads actual clip lengths: `INVERT_HANDPLANT_FS_0_ANTIC` is
0.2 seconds and `INVERT_HANDPLANT_FS_0_ANTIC_LATE` is 1/30 second. FootPlantAbsorb
publishes `absorblength` from the physical manager; no host animation timer or
extra jump impulse selects these launches.

## Validation and boundaries

Release compilation succeeded. Static inspection checked all 32 operation names
in the seven plant templates against source registrations, the five stock
collection classes and point-graph layouts, both antic clip lengths, and the
`FootJump` attributes in the no-comply/boneless/fastplant clips. This is not a
runtime graph-coverage or gameplay-parity result.

The game, launcher, recomp and gameplay harnesses were not executed. In-game
entry/release, contacts, stance combinations, wipeouts, visuals and preservation
of skating/off-board/grind behavior remain for the user's validation.

Queries use the engine's existing synchronous static-world collision and grind
geometry adapters. The outgoing batch is consumed on handplant entry rather
than using TU3's asynchronous job-completion gate and redundant ground-probe
completion dependency. Moving-object coping is not supplied by this adapter.
Scalar vector/trigonometric helpers and quaternion-derived axis-angle rotation
reuse existing engine math; bit-exact Xenon/VMX execution is not claimed.
Handplant scoring updates the existing internal score packet, whose presentation
remains outside this change. Stock assets and private research/build files are
excluded from the commit.

## Handplant bail and delayed revert crash (8 September)

The supplied run entered HandPlant at tick745 and WipeoutGround at746, then
stopped at tick1318 on the missing PhysicsGround -> RevertGround adapter.
HandPlant incorrectly used FootPlant collision checks with unconditional trick
impact limits (stock XZ0.1/Y0.6). It now dispatches the native Air(false) checks,
including their contact, difficulty, cooldown and actual danger-state gates.
The original bail reason was not recorded, so the causal gameplay result still
requires a user retest. PLANT_BAIL now records reason indices and collision/pose
observations when either plant requests a bail.

RevertGround102 now owns Enter82D43488, Update82D43518, empty Exit, Ground
Post82D4C070 and Fill82D43B10. It loads the five fields from stock class
018E8A2E5028AB3F, captures approach velocity after the authored delay, applies
the directed rotation-speed curve, manual/anti-flip corrections and pumping,
and publishes active State66 for the existing selector's normal exit.
No gameplay execution was performed during this repair.

Validation for this repair: six core wipeout tests and two revert numerical
tests passed. Two pre-existing game test fixtures were updated for their current
map-menu and render-material API signatures so these tests could compile.

## Folded-pose report and wall-jump exit (8 September, later run)

Log plants-20260908-174254-418-287 entered HandPlant at tick581, returned to
PhysicsGround at603 and stopped at605 in Ground's wall-jump callback. That
callback was still an explicit error stub. It now converts the complete retained
launch packet to the existing trajectory selector and executes Launch/Update,
including the wall-jump velocity in both native velocity fields. A focused packet
transfer test passes. No gameplay was executed to validate the result.

The user's folded-body report remains unresolved. HANDPLANT_POSE now records
COM/orientation, root transform, target versus physical hips/head/hands/toes and
external hand targets at entry and every six ticks. This distinguishes pose/IK
errors from physical tracking errors on the next user-provided reproduction.

## Handplant apex correction (8 September, folded-body trace)

The 17:48 run shows the head/hips tracking their targets while feet fall roughly
0.5m behind at tick486. A later attempt at1353 produces a NaN incoming arc from
COM[-6.9291053,3.925457,4.9562364]. These are observations; visual recovery still
requires the user's gameplay validation.

TU3 Launch82D61458 calls cosine82473930, then82D61464 calls sine824531C8.
Raw byte-permute mask822FBAC0 is00010203000102030001020314151617. Applying it
to the sine/cosine registers and shifting eight bytes produces[sin,cos,0,0].
82D61520..C4 therefore multiplies UP by radius*cos and side by radius*sin.
The port had these components reversed. With stock radius0.8/angle-0.3 and
copingY3.8850045/Z5.5987973, the corrected apex isY4.649274/Z5.362381;
the old target wasY3.648588/Z6.363067. This placed the old target below and
beyond the lip, consistent with the blocked feet and invalid late-entry arc.
The port now uses the native component order and standalone sin/cos helpers.
Regression tests cover the observed late-entry COM, apex arrival/zero vertical
velocity, and zero-angle symmetry. Pose traces remain enabled for verification.

## Arm and variation investigation (8 September, 17:54 run)

User reports the normal body animation improved, with one arm misplaced;
A/B variations still bail. In plants-20260908-175441-712-2596.log, HandPlant
requests reason1 (pose displacement) at ticks2209 and5093, without board
contact or closing velocity. The previous HANDPLANT_POSE samples compare
current targets with the previous physical pose and cannot identify the
within-tick source of that displacement.

HANDPLANT_SOLVED now captures authored, IK-adjusted and solved arm positions
in world coordinates, arm drive strengths, four limb modes/blends, partial
ragdoll state and per-part pose errors before the handplant bail check. It
samples every six ticks, or every tick when maximum pose error reaches0.15.
This is diagnostic only: neither the arm problem nor variation bails are
claimed fixed. Native82D633B0,82BE3220,82BD9728 and82BEDC10 were inspected;
the selected arm, two0.65m clamps and feet-only external post pass match the
existing port. Need another normal/A/B reproduction to locate the divergence.
The same run later terminates in unsupported IsGrindBluntingBackslash at8059;
that separate grind condition remains unresolved by this diagnostic change.

## Handplant collision-mask correction (8 September, 18:06 run)

The requested normal/A/B reproduction is plants-20260908-180656-119-4993.log.
Normal plant316..395 returns to Ground; variations767..788 and1139..1166
request pose-displacement bail1. Solved traces show the free hand tracking
within0.006m at324, then falling0.57m behind at340. Its IK adjustment is
only0.002m at340. Collision weight drops to0 and the arm switches to soft
drives with local/root strengths0.5/0.0. Both variation attempts repeat this
sequence. No crash was recorded in this run.

Confirmed source error:82D91298's five volume-pointer writes use offsets
3120,3104,3124,3108,3096; matching counter writes are8156,8140,8160,8144,8132.
The generic EnableBone82BE7190 independently establishes the volume base as
4*773=3092 and counter base as4*2032=8128. Both address sets therefore resolve
to parts7,3,8,4,1 (hands, forearms, head). The port incorrectly disabled
8,4,9,5,2, leaving the hands and head collidable. TU3 dump identity remains
SHA256 f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4.
Evidence: logs/recomp-82d91298.asm and logs/recomp-82be7190.asm.

Handplant now calls a shared collision-owner method with the corrected
five-part mask and native two-frame countdown. A regression test checks
refresh during the plant, survival across normal driven-state setup, other
bones retaining contact, and automatic re-enabling after exit. Bail
thresholds and authored animation/IK targets are unchanged. The native mask
error is fixed; visual recovery and successful variations need gameplay
confirmation from the user. The game has not been launched by the agent.
