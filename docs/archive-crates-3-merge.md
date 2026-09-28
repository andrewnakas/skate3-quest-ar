# Merge of supplied crates (3).zip

Source: `crates (3).zip`, SHA256
`beb4aabf83fe706b6678d78f4af10c85a6467e0fef1c3c2a8d92e922405134ad`.
The archive contains a source snapshot without Git history. It was compared with
the initial import (`a47da5b`), current HEAD (`0909877`), and the working tree.
Files absent from the archive were not treated as deletions.

## Integrated

- Powerslide start and continuous input publication; grab, dark-catch, board
  adjustment and tweak inputs; underflip gesture naming and dark-catch cleanup.
- ActionGraph BoardAdjust forwarding with authored angle/mirror filters.
- Grab type ownership, grab scoring, filtered tweak projection, EndGesture
  teardown, and moving-object resource registration. Registration does not add
  moving world geometry or skitch physics.
- Immutable fixed-tick controller input, typed ActionGraph/MotionGraph handoffs,
  bounded graph diagnostics, processed-input snapshots and physical output/event
  records. These wrap the current simulation owners and preserve their order.
- Camera snapshot tick validation, publication diagnostics and graph argument
  validation. Current subject selection, compass/orbit math, grind conditioning
  and render interpolation remain in place; no camera smoothing was added.
- The archive's on-board TimeToLand mapping to Air184, gated by Air437.
- Associated supplied input/action/air/powerslide test sources were retained;
  they were not executed.

## Conflict decisions

The user's current offboard implementation takes precedence, including walking,
jumping, falling, runout, possession, mounting/dismounting and climbing. The 135
existing files under the offboard/climbing/input-offboard paths and the camera
compass/subject-pose modules are byte-identical to the pre-merge backup. Shared
graph and physics files keep those same owner calls and physical publications.

The archive's alternative Biped states, collision toolkit, possession drives,
offboard graph handlers and offboard-related ragdoll/IK rewrites were excluded.
Existing ground/wipeout/reset APIs were retained rather than applying its removal
of methods now used by the current implementation. Uncommitted work present
before the merge was preserved and is not included in this commit.

Our later native blend-space/bump implementation, raw selection-space child
construction, grind acquisition/families/release, JumpInto, manuals and dark-catch
selection take precedence over the archive's older or approximate alternatives.
Its broad placeholder stock-condition module and unimplemented behavior variants
were not registered over existing handlers. Existing unsupported branches still
report errors when reached; optional unsupported branches do not prevent startup.

## Grab conflict corrections checked against TU3

The archive's blanket CurrentGrabType predicate was replaced by the actual enum
comparison: condition 82BA7848 calls ISkaterAnim+100, implemented by 82B97200.
SetGrabType Begin82BB9CB8/End82BB9D20 call setter82B971F8 with the selected enum/0.
This writes ISkaterAnim360; it does not emit a synthetic `GrabType` physics
attribute. FS/BS/Nose/Tail are 1/2/3/4, as decoded by constructor82BA7760.

IsCrouchedEnoughForBlendToGrabCycle82BBDE40 compares PhysOutSkeleton72 strictly
below constant821EE79C (`0x3f19999a`, 0.6), replacing the archive's `<= 0.5`.
Evidence was read from the generated PPC and the original executable image
`default_82000000_011B0000.bin` (base82000000, SHA256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`).

HasTweak82BA6B90 uses the filtered TweakX/Y signals, requires at least one
component to exist, and applies the authored numeric comparison to their vector
magnitude. TrucksOrDeckInContact82BA42E0 reads Collision3472 OR3475 (truck/deck
contact), rather than the archive's wheel/grind approximation. These conditions
were connected after the first launched build reached them and reported an
unsupported-condition error during user input.

## Validation and recovery

`cargo build -p skate-game --locked` succeeds. No automated gameplay or test
suite was run. This is an integration build, not proof of full native gameplay
parity; user playtesting is still required.

Local recovery artifacts are `logs/pre-crates-3-merge.zip`, the untouched extracted
archive in `logs/incoming-crates-3`, comparison reports in `logs/incoming-*.json`,
and `logs/archive-merge-build.log`. These are ignored local artifacts.
