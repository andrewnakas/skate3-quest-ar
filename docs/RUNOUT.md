# Runout crash investigation

The Hardcore 360-flip reports at ticks 583 and 378 terminate at MotionGraph
behavior 86, `AddRunoutAttribs`. This operation was unsupported before the
difficulty-menu changes. Entering the stock runout graph now succeeds in Easy,
Normal and Hardcore. A full-frame regression then exposed the next missing
owner: `PhysicsGround -> BipedGround` has no production adapter yet.

## Reference

TU3 dump `default_82000000_011B0000.bin`, guest base 0x82000000,
SHA256 f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4.
Registration 0x82F89700 selects factory 0x82BC9BA8; vtable 0x82320BDC has
Begin 0x82BBA4A8, Update 0x82BBA770 and empty End 0x82B61BB8.
The initializer pair 0x82F85870/0x82F85888 identifies BipedStartAngle/BipedSpeed.
Addresses are guest addresses; subtract 0x82000000 for dump offsets.

Observed: Begin reads Skeleton0 (effective animation root Z),
SystemReckoning16 (COM velocity), SystemReckoning96 (up), and mirrors through
ISkaterAnim slot28. OffBoard331 selects alternate velocity OffBoard128.
It uses 0x8286CD88 for the projected signed angle, wraps with the strict
fraction > 0.5 condition, converts radians to degrees using word 0x42652EE1,
and captures full three-dimensional speed. Update emits the retained values
with normalized=false and sequence=-1. End performs no writes.

The game currently publishes the ordinary velocity branch; existing riding/air
states leave OffBoard331 at its native reset value. The alternate branch is
implemented and tested but requires the future BipedAir output owner.

The host numerical routines follow recovered arithmetic; no hardware-matched
360-flip replay has been recorded. A test-injected runout request verifies the
stock animation branch in all three modes, not complete on-foot physics.

## Remaining integration

User explicitly selected implementing native on-foot/runout instead of an
error-recovery screen. BipedGround requires the contact toolkit (82D811C8 /
82D81610), ground job producer, controller placement/update, skeleton mode4,
board manager, state publications, and BipedAir/remount transitions. Existing
recovered modules are being compiled and connected; enabling modules alone is
not a working BipedGround implementation.

## Contact toolkit progress

The compiled offboard modules now include the existing native ground lifecycle,
sync, reckoning, board possession, reset animation, stock settings loader and
Skeleton mode4 adapter. The retained board transform has a distinct owner, as
required by the recovered BipedGround skeleton update.

`contact_queries` reconstructs82D856C8's three support probes and44 obstacle
queries, including reverse rays and the constructor's accumulated float rounding.
All41 probe definitions are checked word-for-word against an independently
evaluated constructor using the dump above. The game adapter queries actual map
triangles through the existing acceleration tree and retains query observations.
Nonidentity mesh transforms use the complete geometry path to avoid incorrect
world/local-space culling. Authored-floor and empty-space query tests pass.

`contact_records` ports82D81F80's projection, normal fallback, record routing,
flags, distance selection and64-record surface capacity. `contact_packet` ports
82D2DDD0's reset and82D81610..82D818A0's primary/side-support consumption.
Boundary tests cover the strict side-height limits, central-contact requirement,
world-Y normal replacement, deep-support flags, lateral-normal fallback and
record capacity. These tests validate recovered host arithmetic, not bit-exact
Xenon arithmetic or a hardware replay.

`contact_promotion` now implements82D82498's promotion stage before its final
sort, retaining the original vertical-group scan and cursor advancement.
`contact_segments` implements82D83438 and candidate insertion82D82380. The
normalization threshold at830BD350 is initialized by82F826F8 from82181A88
(word358637BD); its zero value in the raw dump is not the runtime constant.
`ground_job` implements PreUpdate82D30D30's job fields, contact completion and
fallback ordering, twenty-frame teleport suppression, geometry adjustment and
retained publication flags. None of these components enables a physical state.

The full contact chain is now connected to real world queries: native record
sorting, edge intersections, normal constraints, reduction, segment candidates,
slope history and final packet publication. The reduced forward limit reaches
packet172, which the subsequent ledge query consumes. Tests cover actual floor
queries at speeds0,1,3 and6, along with step and slope boundaries.

Player Update82DB4048 calls refresh82D81610 before state PreUpdate (call at
82DB4094). Reset82D30BD0 and Exit82D30CC0 finish pending queries before clearing
the completion latch and classifier history. This ordering is retained in the
contact owner. A mesh's stable host identity replaces the original query body's
guest pointer; current map providers contain static authored geometry.

Still required: production lifecycle owners and job integration; BipedAir,
board possession and remount integration. These component tests do not enable
BipedGround or establish end-to-end runout correctness. The production runout
transition still fails and the playable binary has not been replaced.

## Live owner and job input boundary

SkaterRuntime now loads the stock Biped settings and shares animation metadata
with MotionAnimation. Its persistent offboard owner holds the controller, ground
state, contact history and retained contact packet. Player Update completes its
pending contact observations before physical input/state update.

The ground job adapter reads existing ProcessAnimAttributes owners directly,
including the native reset cadence gate of-1 (zero activates animation velocity
override). It preserves the third processed line1056/valid1092 fallback and the
actual skeleton collision displacements16288/16304, now retained after collision
feedback. Ground's vslot24 at82327158 points directly to82D2D860, the same deck
prediction called through the riding wrapper.

Validation:456 core tests and81 game tests passed together; the subsequent
third-line/reset-attribute regression also passed (82 game tests total). The
stock controller's real-floor test runs60 updates each for idle, half input and
full input. This is controller/query coverage; production transitions still
reject BipedGround until its complete physical adapter is available. No playable
binary has been staged or launched from these changes.

## Board possession and shared solver integration

Native hand drives82D74FD8/82D751D8 now append to the existing board/skeleton
island. Factory82AE64F8 stores its fifth argument as body A and fourth as B:
hands3/7 therefore react against deck6. The skeleton's drive-spy results retain
their original row count. Tests verify opposite hand/deck reactions and zero
reaction after releasing the hand drive.

The controller now has production hold/release, update and output adapters over
the actual animated hand frames, physical hand contacts, board toolkit and
retained collision normals. Its standard/released drag, hook drive and collision
volume policies feed the existing physical owners. Native82C08818's surface vote
is the producer of the controller's observed board840; it is not a sleeping flag.
The lifecycle callback accepts the live controller fields so LetGo can preserve
its eight-update throw countdown across a state change. Board positioning uses
82C0B2C8 without clearing rates or pending forces, verified by regression test.

Observed code comes from the generated TU3 functions and raw image
default_82000000_011B0000.bin, SHA256
f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4.
Lookup8 proves893DA1B966231D76 is MaxDistance (hide trigger) and
6CFF6F2C43F0AD41 is DistanceToHideSkateboard (hidden position offset), correcting
the earlier reversed inference. The biped skeleton adapter also borrows the live roots,
IK, collision and shared reckoning owners, including native body-spin input.

The thrown-board update750F0 calls82C07328 three times. These outputs are angular
displacements:07328 removes the current per-step motion along the requested axis
and uses inverse-tensor/step/tensor conversion to add angular acceleration. It
does not apply the outputs as raw torque. A regression covers braking overshoot
and correcting opposite motion while preserving perpendicular angular velocity.

Validation:459 core tests and83 game tests passed, including the final
output/skeleton-owner wiring, private map and stock difficulty tests. These
are still component and existing-state tests: BipedGround/BipedAir state
integration and remount remain unfinished. The runout crash is not yet fixed
in a staged playable build.
