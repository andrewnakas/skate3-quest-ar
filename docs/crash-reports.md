# Crash reports

Both game executables supervise a child process before loading assets or Bevy. On a nonzero exit (startup error, Rust panic or native process failure), the surviving supervisor saves a UTF-8 report in `%LOCALAPPDATA%/Skate3RustEngine/CrashReports`, falling back to the temporary directory. The small Windows popup provides **Copy report**, **Open folder** and a reviewable text preview. Nothing is uploaded. Reports persist until the player deletes them.

The report records embedded build provenance, process status (including Windows exception exit codes), Rust panic backtraces where available, bounded recent logs, GPU/driver details after renderer startup, map content fingerprint, difficulty, render settings and sampled gameplay/multiplayer state (provider, remote count and RTT, without peer identifiers). Missing information is labelled. The map fingerprint identifies content without sharing a personal filename. Mod contents, lobby IDs, player names, addresses and authentication details are not deliberately collected. Sensitive-context and absolute-path log lines are omitted; review reports before sharing. Existing launcher logs retain their original behaviour and are **not** copied wholesale into reports.

The supervisor retains 256 log lines, 64 recent transitions, 128 panic/native exception lines and six metadata records, each bounded to 4 KiB. Gameplay samples once a second and drains a fixed 64-entry queue of physics-state requests; recording those requests only copies scalars. It does not walk stacks, flush files or enumerate hardware each frame. Pending requests and short menu/map transitions may be missed at failure. Reports are assembled and written only after failure. Pipe readers have bounded line buffers, including for unterminated output; the supervisor allows two seconds for final output after exit.

Coverage limits: a Windows unhandled-exception filter attempts a fixed, allocation-free pipe write of exception code, fault address and executable image base; another handler, fail-fast or severe corruption may bypass it. No native fault-thread stack/register capture or memory dumps; native exit codes alone do not prove a particular exception occurred. A panic hook is best effort under OOM, stack overflow or runtime corruption. A successful exit, forced termination of the supervisor/process tree, missing executable dependencies, power loss, an OS failure or an unusable desktop/GPU may prevent a popup. A hung game is not forcibly killed. PowerShell/WinForms provides the full UI; if startup fails, a native message box gives the saved report location. A full/unwritable disk can prevent saving.

`Build-Release.ps1` links the release directly into fresh private staging and retains a matching EXE/PDB pair plus hashes and revision in the sibling `symbols` folder, outside the player ZIP. Keep that folder with each release for offline symbol matching; it contains no original Skate assets. Release line tables are enabled and stripping is disabled. Dirty builds are explicitly marked and carry a unique build timestamp; release from a clean commit for reproducible source matching. Development launches (`cargo run`, `PLAY.bat`, multiplayer and packaged executables) use the same supervisor automatically.

For manual UI validation, run the executable with `--crash-report-preview`. This generates a clearly synthetic report and opens the popup without initializing the game, setup, Steam or assets. Check Copy report, clipboard text, Open folder, saved UTF-8 text and Close. Static tests do not validate desktop presentation.

## Mixed-worktree dependency in f96be1d (2026-09-09)

The DownTown report at tick1020 repeats the nonfinite walking result before
the swept-line rejection. The full local stderr retains the finite GroundJob
and tiny surface components omitted from the bounded report.

The matching shipped PDB establishes that `movement_velocity/math.rs` was
compiled from worktree `8186`, with SHA-256
`4652072df4a51a6bf8eb2dd837e61d4fb6e01710d984334af8073deb4c4449ac`.
That source lacks the earlier subnormal refinement fix. The intended source
in this worktree has SHA-256
`76de0780adb5ea015c57fa4118105669f1897ef01eafb611eed14cc829c5eda1`.
The game build ID identified the game crate, but did not establish the identity
of reused path dependencies. Matching EXE/PDB GUIDs alone did not catch this.

`tools/build_verified_windows.ps1` now builds into a private
`target/verified-windows` under the current worktree. Its verification step
reads PDB source checksums and requires both current-worktree paths and matching
SHA-256 for referenced project crates and vendored Bevy sources. It fails closed
when required game/core/data source records are missing. The old shipped PDB
fails this check (619 mismatched path/checksum entries).

Use this build script for subsequent crash-test builds and retain its
`source-checks.json` with the staged EXE/PDB and existing import/hash manifest.
Do not share Cargo artifact directories between independently edited worktrees.
This repair changes build provenance, not gameplay rules or collision validation.

## Walking velocity convergence (2026-09-09)

Observed user failure: clean revision `dff672a`, build timestamp
`1788969088174193000`, report timestamp `1788969335`. At tick4059 in
BipedGround on University, `OFFBOARD_GROUND_RESULT_NONFINITE` precedes
`Invalid offboard swept-line request`. The job has finite spatial inputs;
the controller publishes NaN velocity/position, a NaN right delta and a
forward delta of approximately `-3.11e-43`. The collision query then rejects
NaN XYZ coordinates. Its validation is correct and remains intact.

The walking velocity limiter's portable reciprocal-square-root refinement
formed `r*r` from a positive subnormal squared length. A finite correction
of `1e-21` produces squared bits `0x000002CA` and a finite inverse root near
`9.997e20`, whose square overflows to infinity. The two refinements then
produce NaN. A focused pre-fix regression reproduced an all-NaN limited
velocity from otherwise finite vectors. This establishes the numerical
failure independently of the collision system; it is not a replay of all
4059 input ticks.

The fix normalizes only positive subnormal squared lengths by the exact
power of two `2^24`, retains both refinement stages, and rescales the inverse
root by `2^12`. It preserves the ordinary-input path, movement limits and
invalid-input propagation. No global floating-point mode changes, collision
bypass, arbitrary epsilon, or replacement player state are involved. This
is a stability correction for the existing portable PC arithmetic, not a
claim of bit-exact Xenon instruction emulation.

Regressions cover the failing finite vectors, powers-of-two across the
subnormal/normal boundary, limiting tiny corrections, steady walking updates,
and the complete ground-controller publication (including position, lean and
cadence). Zero and NaN checks ensure invalid inputs are not silently repaired.
The report, matching stderr and arithmetic evidence are retained privately
with the test builds. Gameplay validation remains with the user.

Validation: the finite-delta regression failed before the arithmetic change
with `[NaN, NaN, NaN, NaN]`. Afterward all 11 movement-velocity tests passed,
followed by all 202 offboard library tests in the release MSVC configuration
(`cargo test --release --locked --target x86_64-pc-windows-msvc -p skate-core
--lib player::offboard::`). No game or renderer was launched.

## FingerFlipOut (2026-09-09)

Observed user failure: clean revision `1a6e95b`, build timestamp
`1788967727931108700`, report timestamp `1788968141`, University fingerprint
`c911af6099e02b65`. At tick3467 in KnownAir, MotionGraph behavior3080 returned
`MotionGraph stock gameplay producer FingerFlipOut is not implemented`.
This was an explicit unsupported-operation exit, with no recorded panic or
native exception. The matching launch stderr contains the same error.

Native recovery from the TU3 image mapped at `0x82000000`:

- Factory82BC7C18 binds authored `grabintent` and literal `Grabbing`.
  Vtable8232072C allocates a private float through82BBCB50, calls
  Begin82BB3198, Update82BB7C40 and empty End82B61BB8.
- Begin sets the float to zero. Update reads the MotionGraph intent map
  through Actor20/virtual16. Membership lookup82454FD8 counts a zero-valued
  intent as present. Presence subtracts the time-service delta; absence adds
  it. This does not read raw ActionGraph or filtered tweak input.
- Update clamps the retained time to layout190/198, evaluates eight curve
  keys at1A0 and values at1C0 through PointGraph82481E10, then writes
  `Grabbing` through ISkaterAnim80 (unnormalized, sequence -1).
- Global constructor8289FD10..30 selects collection `41DB0C4F82003A15` at
  offset164. Helper8289D4E8 identifies class `anim_motion`. The retail schema
  independently maps layout190 to field `E0C1407B688858AD`, type
  `Sk8::PointNegGraphData8`. Its stock timer bounds are 0 and 0.5 seconds,
  with a descending grab blend from 1 to 0. Numeric names remain supported
  directly; no shared asset edit or invented curve is required.

The implementation retains one timer per bound behavior and publishes only
on Update. Begin resets time; End leaves the last parameter alone. Other
unimplemented operations still report errors.

Evidence identities (SHA-256): TU3 mapped image
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`;
tested stock MotionGraph
`806b2665e435be4e313d70a06fab4538e0adf60a2dfd3e80e82ae8d0b4a2e7c1`.
Bounded decoded instructions, schema lookup and the original report are
retained privately with this test build. IDA changes were confined to a
disposable database copy. The instruction/data mapping is observed;
successful gameplay reproduction after the fix remains user validation.

Focused regressions cover release/regrab timing, clamping, entry reset,
the actual stock behavior3080, zero-valued intent membership, raw-input
separation, independent behavior instances and Begin/End publication rules.
They do not launch the renderer or replay the user's full input sequence.

Validation: `cargo test --release --locked --target x86_64-pc-windows-msvc
-p skate-game --bin skate3rust --no-default-features finger_flip --
--include-ignored` passed both tests against the prepared assets (0.18 seconds).

## SetDeckPitchAndYaw (2026-09-08)

The supplied report stopped in KnownAir at tick4813, MotionGraph behavior3373:
`SetDeckPitchAndYaw has no recovered native implementation`. This was an
unimplemented graph behavior, not an animation overflow.

TU3 factory82BC7B20 retains the authored `skateyaw`/`skatepitch` attribute names
(defaulting to those same strings). Vtable8231FF24 calls Begin82BAF100, which
copies completed Skeleton PhysOut536 then540 to scalar animation parameters.
Update and End both point to empty82B61BB8. Skate2 constructor82CB03E8 and
Begin82CB04D0 corroborate the same behavior and output offsets.

The connected producer is Skeleton::Fill82BE1AE8: read Skeleton6512
(animation-record pose0 column2), negate it when animation packet10368 is
nonzero, calculate yaw from X/Z with native atan82473B98 and quadrant/zero
selection, and calculate pitch as DOT3 with up82139A20. Pitch is the signed
forward-Y projection, not an Euler angle or asin. VMX estimate bit identity is
not claimed.

`SkeletonOutputFields::publish_deck_angles` implements those calculations.
The frame output phase publishes them after reset/physics completion from
`animated_skeleton.record.pose[0][2]` and the current packet's `board_flipped`.
The next animation phase supplies that retained output to MotionHost; the
behavior copies it on Begin only. No extra per-frame attribute writer or
missing-output fallback was added.

Saved native evidence: research root
`physics-comparison-2026-09-02/evidence/deck-pitch-yaw-20260908`;
the existing Fill decompilation is in `air-motion-graph-2026-09-05` and its
decoded instructions in `riding-outputs-2026-09-04/skate3.decoded-82be1ae8.txt`.
Read-only extraction verified the saved database hashes were unchanged.

Focused regressions cover native quadrants, zero denominators, flipped stance,
raw pitch projection, stock graph binding, missing-output errors and
Begin/Update/End/reentry. These do not replay the user's original input sequence
or establish rendered gameplay parity.

Validation (2026-09-08):

- `cargo test -p skate-core deck_angles --locked`: 1 passed.
- `cargo test -p skate-game --bin skate3rust stock_deck_angles --locked -- --ignored`:
  1 passed against `SKATE3_ASSET_ROOT` pointing to the clone's `assets` directory.
- The existing
  `raw_repeated_walk_turn_throw_recall_jump_remount_preserves_full_roots_and_joints`
  test passed: 5,400 uninterrupted ticks, three walking/turning/throw/recall/jump/
  remount cycles, 7.92 seconds, with its original assertions and collision fixture.
  This is not the University crash input replay.
- The first game-test compilation found the missing procedural
  `maps/format-demo.skate` fixture. Regenerated it with `tools/make_skate_demo.py`;
  no stock world or test assertions were replaced.
