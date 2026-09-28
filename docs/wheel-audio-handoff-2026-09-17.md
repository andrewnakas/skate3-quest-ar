# Skate 3 retail wheel audio handoff

## Objective and non-negotiable constraint

The user wants all Skate 3 wheel audio—rolling surfaces, truck rattle, seams/cracks, skids, and squeaks—to use the **retail authored paths** and match retail pitch, selection, envelopes, and routing. The user explicitly rejected decoded-sample fallbacks. Do not re-enable the legacy `DirectMixer`; it is presently dead code and must remain so.

The immediate blocker is more fundamental: the current Rust authored wheel graph opens retail wheel voices but either releases them in the same render block or faults before it can keep them alive. The user just tested the latest staged build and reported no rolling-wheel sound.

## Workspace and runnable build

- Main source worktree: `C:\Users\andre\.codex\worktrees\b0ad\Sk8EngineAudio`
- Other task CWD: `C:\Users\andre\.codex\worktrees\86c5\Sk8EngineAudio`
- Staged executable: `C:\Users\andre\.codex\worktrees\b0ad\Sk8EngineAudio\bin\skate3rust.exe`
- Cargo target used for playable builds: `C:\Users\andre\AppData\Local\Skate3RustEngine\target`
- Retail asset root: `C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets`
- Retail capture corpus: `D:\skate3-audio-captures\retail-isolated-20260917-120333`
- Recompile/port source: `D:\skate3recomp-audio-capture`

The currently running staged executable was launched with `SKATE_AUDIO_OBSERVE=1` and `SKATE_AUDIO_VOICE_TRACE=1`. Its worker has faulted, so it will not emit new audio. It may safely be stopped before staging any replacement.

### Build and stage commands

```powershell
$repo = 'C:\Users\andre\.codex\worktrees\b0ad\Sk8EngineAudio'
$target = Join-Path $env:LOCALAPPDATA 'Skate3RustEngine\target'
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_INCREMENTAL = '0'
cargo rustc -p skate-game --bin skate3rust --locked --target-dir $target -j1 -- -C codegen-units=256
& "$repo\scripts\Build.ps1" -StageOnly -TargetDirectory $target
```

The full `skate-game` compile is slow (roughly 10–15 minutes). Do not stage while any `rustc.exe` command line still contains `Skate3RustEngine\target`; Cargo can launch a second compiler after the first crate finishes.

## Current implementation state

### Edited files

1. `crates/skate-game/src/player_audio.rs`
2. `crates/skate-audio-core/src/authored.rs`
3. `crates/skate-audio-core/src/patch.rs`

The repository was already dirty. Preserve unrelated changes. `git diff` will be noisy due line-ending/formatting differences; inspect only these files plus this handoff before editing.

### Authored runtime additions

`AuthoredRuntime` now has `post_relocated` and `redeliver_relocated`. Retail packet traces contain pointers into the caller-owned packet. These methods allocate the normal guest message and replace selected payload words with the equivalent address inside the new guest allocation. This is essential; copying retail process addresses or zeroing them is wrong.

`AuthoredDevice` has opt-in `SKATE_AUDIO_VOICE_TRACE` open/release instrumentation. `RuntimeStats` exposes opened/live voice counts and the player worker logs them before and after each authored render block.

### Wheel producer status

`EventProducer` has live graph-only `Class_rolling` and `Rolling_Rattle_Class` posts. It deliberately no longer redelivers the earlier guessed per-frame vectors:

- Retail rolling updates are a **two-node linked control graph**. Update word 15 points to another rolling node, not to the same packet.
- The old Rust update used the wrong controls and overwrote that pointer with a self-reference; it had to be removed.
- The current rolling post is the exact observed 28-word constructor shape with its packet-local word-15 pointer relocated by `+0x7c`.
- Rattle likewise uses the traced constructor and does not send a guessed held update.

This is intentionally incomplete rather than falsely retail-accurate. Do not restore synthetic speed/surface updates until the actual retail caller and its companion-node ownership are recovered.

### Boot utility work attempted

The player worker now posts these captured boot objects, in retail trace order, before `Class_Treatment` and board messages:

1. `c_emitter_utility`
2. `Start_up_Play_ctl`
3. `c_foley_utility`

Each uses a zeroed 28-word packet with self-relative references at word indices `3, 7, 11, 15, 19, 23, 27`, pointing to byte offsets `0x1c, 0x2c, 0x3c, 0x4c, 0x5c, 0x6c, 0x7c`. This is taken directly from the retail boot traces, not inferred. The post is persistent: the returned message handle remains retained by the runtime.

This boot ordering is still not sufficient: later rolling reaches a slot-5 broadcast binding whose symbol field is `0x00000009`, yielding an invalid guest address.

### Slot 39 port added

`PatchHost` now handles opcode 39 (`sub_82B1C450`) in `patch.rs`.

The implementation was transcribed from:

`D:\skate3recomp-audio-capture\src\audio_ports\sub_82B1C450.inc`

Its behavior:

- compares requested `block+20` with applied `block+16`;
- latches the raw requested word to `+16` when changed;
- clamps only the published value against signed `[+8, +12]`;
- validates `{holder, generation}` at `+0/+4` against holder generation `+12`;
- updates holder value `+4` and walks holder listeners;
- supports known `ON_SUBSCRIBE` (`0x82B1D7E8`) callbacks, copying the published word to `context+24`;
- rejects other callback targets explicitly.

This unblocks `c_foley_utility`; before it, the worker always failed immediately on opcode 39.

## Hard evidence from latest user-tested build

Latest log:

`C:\Users\andre\.codex\worktrees\b0ad\Sk8EngineAudio\logs\game-retail-finalboot-20260917-222622.stderr.log`

Relevant lines:

```text
SKATE_PLAYER_AUDIO foley_utility_start handle=0x60000c70
SKATE_PLAYER_AUDIO rolling_start tick=2 handle=0x60002880
SKATE_AUDIO_VOICE open voice=64016f00 sample=50136a76 live=1
SKATE_AUDIO_VOICE open voice=64017dd0 sample=50133e45 live=2
SKATE_AUDIO_VOICE release voice=64016f00 live=1
SKATE_AUDIO_VOICE release voice=64017dd0 live=0
SKATE_AUDIO_VOICE open voice=64016f00 sample=5014b2cd live=1
SKATE_AUDIO_VOICE release voice=64016f00 live=0
SKATE_PLAYER_AUDIO wheel_render block=666 opened=3 live=0 peak=0.000000
...
SKATE_PLAYER_AUDIO rolling_start tick=155 handle=0x600051d0
SKATE_PLAYER_AUDIO rattle_start tick=156 handle=0x60005750
SKATE_PLAYER_AUDIO failed=at 0x00000009: evaluator opcode 5 at record 0x50188784, block 0x60006c50: no segment covers this address
```

Meaning:

1. The Rust graph does select actual retail wheel samples and opens their real voice graphs.
2. At initial startup those voices are released within the same block and no authored PCM is produced.
3. Once the player rolls later, rattle starts and the runtime faults in slot 5 because a required broadcast handle has an invalid symbol value (`9`).
4. After worker failure, `LivePcm` drains to silence; this is why the user hears no sound.

An earlier utility-only run failed sooner at opcode 39. An intermediate slot-39 build produced a short nonzero output count, but then faulted at the same `0x00000009` slot-5 binding. Treat that nonzero count only as evidence that the output pipeline itself can render; it is not evidence of correct rolling audio.

## Retail trace evidence

### Object families and known banks

| Retail object | Purpose | Bank |
|---|---|---|
| `Class_rolling` | surface rolling, multi-bank fan-out | `PatchBank_Rolling_Surfaces`, `PatchBank_Objects`, `PatchBank_SpiderCracks`, `PatchBank_RocksBounce` |
| `Rolling_Rattle_Class` | truck/rattle layers | `Rolling_Rattles.abk` |
| `Class_Seams` | seam/crack contact transients | `Seams_Bank.abk` |
| `Class_wheels_skid` | skid/revert/stop | `WHEEL_SKID_BANK.abk` |
| `Class_Squeaks` | powerslide squeaks | `Brd_Squeaks.abk` |

Only the first two are currently driven in the graph. Do not add seams/skid/squeaks with approximated packets while the shared broadcast/utility layer is broken.

### Captured constructor packets

From `retail-audio.log`:

```text
Class_rolling payload base=40C88624
[00000000 00000000 00001000 00000000 00000002 00000000 00000008 00000000
 00000000 000061A8 00000000 00007FFF 000057E4 00000001 00000007 40C886A0
 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
 00000000 00000000 00000000 00000000]
```

Word 15 is packet base `+0x7c`.

```text
Rolling_Rattle_Class payload base=40C88564
[00000000 00000000 00001000 00000000 00000000 00000000 00000001 00000000
 000061A8 00000000 00007FFF 00000008 000057E4 00000000 00000007 40C88620
 00007FFF 0000187F 00000333 0000FFEA 00000FF6 0000618B 0000004D 00000000
 00000000 00000FA0 00001194 00000FA0]
```

Word 15 is packet base `+0xbc`.

### Critical rolling update shape

From `retail-audio.6.log`, repeated retail updates have this form:

```text
Class_rolling node=40C02F10
[00007FFF speed 00000FF6 speed2 surface 00000000 00000001 00000000
 00000000 0000618B 0000004D 0000329A 00000000 00000000 00000000 40C02F20 00007FFF]

Class_rolling node=40C02F20
[00007FFF speed 00000FF6 speed2 surface 00000000 00000001 00000000
 00000000 0000618B 0000004D 00001320 00000000 00000000 00000000 40C02FF0 00007FFF]
```

The two rolling nodes link to each other/rattle. The old one-node Rust update was wrong. Recover the gameplay caller/owner that creates the companion nodes and map its real controls; do not encode these addresses as packet-local relocations.

### Utility trace

From `retail-audio.13.log` boot:

```text
c_emitter_utility payload=40C002E4: word 3=40C00300, 7=40C00310, ... 27=40C00360
Start_up_Play_ctl payload=40C00304: word 3=40C00320, 7=40C00330, ... 27=40C00380
c_foley_utility payload=40C00324: word 3=40C00340, 7=40C00350, ... 27=40C003A0
```

All are self-relative `+0x1c`, then +16 per field. The calls occur as object messages 0, 1, 2 respectively.

## Recommended next work, in order

1. **Instrument the exact invalid slot-5 binding before altering gameplay packets.**
   Add a diagnostic in `clamp_and_broadcast` that prints the operand block, computed binding address, symbol, generation, and the authored program/record identity when `symbol < 0x40000000`. Do not dereference it first. Determine which bank program owns `record=0x50188784` and why its binding remains `9`.

2. **Recover the constructor that establishes that table-1 binding.**
   Loading all ABKs and posting the three generic boot utilities is not enough. The existing planning document already identified the archive sentinel problem: `docs/player-audio-implementation-plan.md` around lines 149–153. Locate the actual utility/profile constructor that writes this binding. The missing state could be a resource-profile lookup, handler registration, or a later startup message with nonzero data—not a wheel payload.

3. **Port `sub_828E2F38` fully and identify every callback target.**
   The source exists at `D:\skate3recomp-audio-capture\src\audio_ports\sub_828E2F38.inc`. The current slot-39 port only supports `ON_SUBSCRIBE`; retain explicit errors for unknown handlers. Capture the real listener target(s) and implement their documented contracts instead of treating a handle value as a pointer or silently ignoring callbacks.

4. **Recover the two-node rolling lifetime and update caller.**
   Build a helper API to patch a payload field with a *different message node* only after the actual ownership order is known. The current `post_relocated` API is correct only for packet-local pointers. Add handle-to-node relocation only if the retail caller evidence proves it.

5. **Validate native PCM before testing subjective sound.**
   Success criteria for rolling: `wheel_render` shows nonzero `live`, `rolling_pcm native_peak` logs, and stream `nonzero` grows while rolling. Only after these should device gain/downmix be examined.

6. **Then add remaining wheel families one at a time.**
   Port exact constructor/update/release sequences for `Class_Seams`, `Class_wheels_skid`, and `Class_Squeaks`, with trace comparisons for selection, layer count, pitch, gain, and routing. Avoid direct PCM playback entirely.

## Useful source locations

- `crates/skate-game/src/player_audio.rs`: audio worker, startup utilities, wheel producer, diagnostics.
- `crates/skate-audio-core/src/authored.rs`: guest message allocation/relocation, voice lifecycle, render stats.
- `crates/skate-audio-core/src/patch.rs`: patch listener system, host opcodes 4/5/27/39, broadcasts.
- `crates/skate-audio-core/src/eval/interp.rs`: evaluator node walk and record dispatch.
- `crates/skate-audio-core/src/symbols.rs`: table-0/1/2 projects and binding resolution.
- `docs/player-audio-implementation-plan.md`: prior research and identified M3 utility/broadcast gap.
- `docs/audio-player-sounds-handoff.md`: object/bank/caller census.
- `D:\skate3recomp-audio-capture\src\audio_ports\sub_82B1C450.inc`: opcode-39 source.
- `D:\skate3recomp-audio-capture\src\audio_ports\sub_828E2F38.inc`: changed-value handler-list notifier.

## Verification performed

- `cargo check -p skate-game --bin skate3rust --locked` passed after each final source change (with pre-existing warnings).
- `cargo test -p skate-audio-core --lib --locked latch_and_notify -- --exact` ran successfully but selected zero tests because no dedicated test was added; this is not meaningful slot-39 coverage.
- Full playable builds were staged and launched repeatedly.
- The user tested the final staged build and confirmed no rolling-wheel sound.

## Important honesty notes

- Do not call this retail-matched or complete. It is not.
- The latest worker fault is deterministic and evidence-backed: invalid slot-5 broadcast binding at address `0x9`.
- The direct sample mixer remains present as unused legacy code in `player_audio.rs`, but the active worker does not instantiate, observe, or mix it.
- The user asked for a handoff because the final test was silent. Do not continue claiming success without first making the runtime keep rolling voices live and proving nonzero native PCM.

## Addendum — 2026-09-18: diagnostic, headless repro, and a ruled-out hypothesis

Still not fixed. This session did four things, in order, following the "Recommended next work"
list above; do not skip re-reading it.

**1. The crash is now self-diagnosing.** `broadcast()` (`patch.rs`) and `latch_and_notify()`
(`patch.rs`) both validate their `symbol`/`holder` word against `g.segments()` before
dereferencing it, via a new `is_mapped` helper, and return a named `Error` instead of an opaque
guest fault. The old `"no segment covers this address"` text is gone for this failure class; the
message now reads `"broadcast binding {binding:#x} has unresolved symbol {symbol:#x} (generation
{gen:#x}); likely an un-relocated table-1 export"`. Two regression tests cover it
(`slot_five_reports_an_unrelocated_symbol_instead_of_faulting_blind`,
`slot_thirty_nine_reports_an_unrelocated_holder_instead_of_faulting_blind`). All 730
`skate-audio-core` tests pass. `install_bank`'s export loop also gained an opt-in
`SKATE_AUDIO_EXPORT_TRACE=1` tracer that logs every export whose `lookup_table0/1/2` fails.

**2. A headless, human-free reproduction now exists.** `cargo run --release -p skate-data
--example wheel_repro -- <assets-root> [frames]` builds the real `AuthoredRuntime` from the
owner's actual assets (no Bevy, no physics, no controller), posts the same three boot utilities
`player_audio.rs::run()` posts, then the exact captured `Class_rolling` and `Rolling_Rattle_Class`
constructor packets (with the same `post_relocated` relocations `update_rolling`/`update_rattle`
use), and pumps the evaluator until it faults or completes. It reproduces the **exact same fault
address**, `record=0x50188784`, as the real staged `skate3rust.exe` did in
`logs/game-retail-finalboot-20260917-222622.stderr.log`, with the new diagnostic showing symbol
`0x00000001` (not `0x9` — see point 4). Use this tool instead of a manual playtest for every future
iteration on this bug; it takes ~1s once the PCM decode cache is warm
(`%LOCALAPPDATA%\Skate3RustEngine\audio-pcm-cache`).

**3. The faulting operand is a template constant, not anything we post — confirmed
experimentally, not just inferred.** Disassembling `Rolling_Rattle_Class`'s compiled program
(`cargo run --release -p skate-audio-core --example grind_instance -- <archive> <image-dir>
Rolling_Rattles.abk Rolling_Rattle_Class` with `DISASM=1`) shows opcode 5 (`clamp_and_broadcast`)
appears exactly once, at instruction `[74]`, operand block `+1048`, and **no earlier opcode in the
242-instruction program writes to `+1048`** (checked by grepping the disassembly's `r3->+N` wiring
column for `+1048`; nothing matches). Posting the object with `PAYLOAD` word 6 forced to
`0xCAFEBEEF` (a deliberately-invalid pointer, in place of the real captured `1`) still ran fault-free
for the requested frames — proving this exact word is **not** copied from any payload we post. It is
baked into the shipped `Rolling_Rattles.abk` template bytes.

**4. This rules out "unresolved bank export" and confirms the existing "missing broadcast
utility" theory instead.** `SKATE_AUDIO_EXPORT_TRACE=1` during `wheel_repro` produced zero
unresolved-export lines across all 17 installed banks — every `lookup_table0/1/2` call at bank-install
time succeeded. So the `1` is not a failed static export resolution (ruling out the theory this
handoff's step 2 originally proposed testing first). Grepping the retail capture corpus
(`D:\skate3-audio-captures\retail-isolated-20260917-120333\retail-audio*.log`) for
`Rolling_Rattle_Class` shows the **real retail process also carries the literal word `1` at this
exact payload position across every one of dozens of captured deliveries**, and retail rattle audio
plays correctly — so `1` is retail's own genuine "not yet subscribed" placeholder, not a
mistranscribed capture. This is now hard, reproducible confirmation of
`docs/player-audio-implementation-plan.md:151`'s prior conclusion: some other object's constructor
must `broadcast()` (or otherwise publish) a real value into this instance's subscribed cell before
slot 5 runs, and that object has not been identified or ported yet. A candidate lead investigated
and **ruled out**: `c_emitter` (obj=36, posted immediately before `Rolling_Rattle_Class` in one
trace window) is a generic world-ambience emitter constructor used by ~30 unrelated environmental
banks (`truck_by_1.abk`, `water_lapping.abk`, `Church_Bell_1.abk`, etc.) — it is out of this
project's scope and almost certainly an unrelated, coincidentally-interleaved log line, not the
missing broadcaster.

**Next step, unchanged in substance from item 2/3 above, now narrower:** find which retail object
registers a broadcast subscription (`allocate_instance`'s `record+34` broadcast-subscription loop
in `patch.rs`, which `link_head`s onto some *other* symbol's subscriber list) inside
`Rolling_Rattle_Class`'s own compiled record, identify that other symbol's owning object by name,
and confirm whether/when the real retail game constructs and broadcasts through it before the
first `Rolling_Rattle_Class` post. This requires either disassembling `Rolling_Rattles.abk`'s raw
record bytes at the `record+34`/broadcast-subscription byte offsets (not yet attempted), or finding
that constructor's call site in `D:\skate3recomp-audio-capture`. Do not guess a synthetic broadcast
value into this cell — per the existing rule in this document, only port the real call once its
evidence is found.

## Addendum — 2026-09-18, continued: the crash is fixed; a much bigger, pre-existing gap remains

**The deterministic worker crash described above is now fixed**, not just diagnosed. Given the
retail evidence in points 3-4 above (retail's own live process carries this same placeholder
value and plays rattle fine), `broadcast()` and `latch_and_notify()` in `patch.rs` now treat an
unmapped `symbol`/`holder` as "no holder yet" (status `-6`/`0`, matching the existing
`symbol == 0` early-exit already in the same function) instead of returning a fatal `Error`. This
is not a guess: it mirrors an early-exit shape the function already has, is consistent with the
retail trace evidence, and matches the module's own doc comment that `clamp_and_broadcast`
"always returns zero; the guest deliberately ignores the broadcast helper's ordinary negative
status codes" — i.e. retail's own caller already ignores this status either way. Set
`SKATE_AUDIO_BROADCAST_TRACE=1` to log every time this path is taken (silent by default, matching
`symbol == 0`'s existing silence). Two regression tests updated:
`slot_five_skips_an_unrelocated_symbol_instead_of_faulting_blind`,
`slot_thirty_nine_skips_an_unrelocated_holder_instead_of_faulting_blind`. All 730
`skate-audio-core` tests pass. Verified with `wheel_repro`: the same session that used to fault at
block 5 now runs 400 blocks with no fault, `voices_opened=3` (matching the real staged build's
prior `opened=3`), `live_voices=0`, `peak=0.0` throughout.

**That last part is the important, sobering finding.** `live_voices=0, peak=0.0` for the entire
run means the crash fix alone does not make wheels audible. Voices open and are released within
the same block — and per `defer_stop_graph`'s own doc comment, that is retail-normal ("a patch is
allowed to open and release a voice in the same evaluator walk, after its play command was
queued"), so an immediate release is not itself the bug. The real problem is that no PCM is ever
produced regardless.

**To find out whether this is a wheels-specific bug or something bigger, `Class_grind` — this
project's own most-verified, most-mature object — was tested through the exact same pipeline**
(`cargo run --release -p skate-data --example render_player_audio -- <assets>` with the object's
own real payload). Result: `live_voices=2` (voices *do* stay open, unlike wheels), but **`peak`
stays exactly `0.0` for all 200 rendered blocks**, and the example's own built-in assertion
(`"authored graph produced no audible PCM"`) fires. **No object currently produces audible PCM
through this pipeline in this working tree, wheels included but not wheels-specific.**

This matches `docs/player-audio-implementation-plan.md`'s own M1 milestone status, read carefully:
"a headless block renderer" exists but "no graph renderer pushes into it yet" and the gate's own
exit criteria ("Require nonzero finite PCM") is explicitly **not yet met**. That document's M1
section (drain, play command, stream pump, per-block driver, output bridge) is the real remaining
work, and it is substantial — spanning `device.rs`, `pump.rs`, `sndplayer.rs`, `stream.rs`,
`commands.rs` and `worker.rs` — not a wheels-local fix. Do not attempt a quick patch here without
the same trace-driven, PPC-exact discipline the rest of this project uses; a rushed change to the
PCM path risks producing audio that is wrong in a way that is hard to detect (nonzero but
incorrect), which is worse than staying silent. **Treat "make any authored sound audible at all"
as the actual next milestone, ahead of wheel-specific work** (seams/skid/squeaks), since wheels
inherit whatever the shared PCM pipeline can do.

**What a build with tonight's fix will show:** the worker no longer crashes when rolling near
wheels or when rattle starts (previously fatal — the whole audio thread died and stayed silent for
the rest of the session). Wheel sound itself, and in fact all authored player sound, will very
likely still be silent, because of the pipeline gap above, not because of anything wheel-specific.
This is a real, verified improvement (a crash is gone), but it is not yet an audible fix.

## Addendum — 2026-09-18, final: wheels and footsteps driven by the recovered retail callers

**Two earlier conclusions were wrong and are withdrawn.**

1. *"Retail rolling is a two-node linked control graph."* False. Retail trace lines print a fixed 28
   words (updates: 17), but the constructors fill small objects: `sub_824C4C18` (Class_rolling) and
   `sub_824B0248` (Rolling_Rattle_Class) each allocate 52 bytes and post object+4, i.e. **12-word
   packets**. Word 15 of a dump is the *next heap object's* message-node pointer. The `+0x7c`/`+0xbc`
   relocations and every word past 11 were heap garbage and have been removed.
2. *"No object produces audible PCM; the pipeline is broken project-wide."* False. The live game log
   already showed nonzero device PCM (e.g. `grind_pcm native_peak=0.0032`). Rolling/rattle sat at
   ~-80 dB because the port never sent the per-frame retail updates, so the patches saw speed 0 and
   gain 0. The earlier `Class_grind` headless test used a synthetic payload.

**Recovered retail drivers (lifted C++ at `C:\dev\skate3recomp\generated`, `skate3_recomp.11/12.cpp`):**

| Object | Constructor | Poster | Per-frame updater |
|---|---|---|---|
| Class_rolling (layers 0, 3) | `sub_824C4C18` | `sub_824C9830` → holders +1304/+1308 | `sub_824C9948` |
| Rolling_Rattle_Class | `sub_824B0248` | `sub_824C6198` → holder +1300, re-posted per trigger | `sub_824C80C0` |
| playercharacter_footstep ×2 | `sub_824B73E0` | `sub_824E9FD8` → holders +36/+220 | `sub_824EAEA8` |

**Exact tuning from the owner's AttribSys vault** (`conversion/database/data/db/skatercollections.vlt`,
read with `tools/asset_pipeline/vlt.py`, which already emits full arrays): rolling max speed
`0x880C82E8EF647EC4` = [70, 65, 65, 70, 100, 45, …] km/h; rattle divisor `0x12275AA8AC4A63FB` = 30;
rattle EQ chain `0xC04832978CDED925` = 8; footstep words 18..23 `0x636464FBAD0D71A3` =
[32767, 10000, 15000, 25000, 32767, 28000]. Footstep word 23 multiplies the patch's start gain
(op 21 at block +7616), which is why steps were silent before.

**Measured with `wheel_repro` (headless, owner's assets):** rolling+rattle at 6 m/s → continuous
output, ~-28 dBFS native peak; footstep plants → the patch opens ~8 layers per step with start
gains 0..8484 (retail opens show 0, 3734, 7852, 8186, 8937), -32 dBFS (slow step) to -17 dBFS
(hard push).

**Still approximated (documented in code):** default surface 2 for all wheel/foot surfaces (user
request); azimuth word 0 (centre); footstep word 10 for pushes mapped linearly from the engine's
push speed to the recorded retail distribution; walking-step word 10 = retail median 48; rattle
retrigger every 64 ticks (retail p50 1.09 s) instead of the unrecovered state-335 trigger; gain
ramp (cap × speedWord/460) fitted to recorded rows rather than recovered from vfunc 7. Seams,
skid and squeak families are not yet driven.

## Addendum — 2026-09-18, playtest fixes (landing pitch, echo, graininess, walking steps)

User playtest of the wheel/footstep build reported: landing pitched up and fast with echoes,
everything grainy, no footsteps off the board. Causes and fixes, each traced to retail code:

- **Landing played up to 2x fast.** `Class_Treatment` word 10 is not a strength. The retail updater
  `sub_824DD6F0` writes `timeScale * 500` (image constant 0x820BD5C4 = 500) from the global
  slow-motion timer that also gates Hall of Meat; the constructor's 500 is normal speed. The port
  wrote a guessed landing strength of 350..1000 there during landings. Word 10 now stays 500.
  Words 7/8/9 (retail: audio-state +236/+240 × 1000, +260 × 166.67) still replay the captured
  landing curve; the engine does not yet publish those native PhysOut fields.
- **Echo on pushes, walking silent.** Footstep word 8 is a foot *down level* (`sub_824E9270` copies
  audio-state bytes +724/+725 every frame; the patch starts its layers on 0→1). The port raised a
  5-frame pulse on every frame the engine's `push_contact` was active (≈10 frames per push) and on
  every frame `AudibleFootStepStrength` was nonzero (it is a continuous level, 2–3, while walking),
  retriggering the patch each frame. Now: foot A level = push contact active (on board) or the
  native offboard foot manager's left support flag (BipedGround); foot B = its right support flag
  (published as `PhysicalOutputSnapshot::feet_supported`). Word 13 is `AudibleFootStepStrength`
  (retail `fctiwz` of audio-state +796, clamp 1..99).
- **Grainy output.** The render could not keep real time: `Guest::locate` scanned ~290 image
  segments linearly for every emulated load/store. Measured per 256-frame block (budget 5.33 ms):
  idle 1.06 ms, rolling+rattle 2.26 ms (12 blocks over budget in 10 s); the playtest log shows
  939,000 underflow samples. `Guest` now keeps a sorted index and last-hit cache, used only when
  segments are disjoint (then exactly one segment can contain an address, so results are identical
  to the scan). After: idle 0.12 ms, rolling+rattle 0.21 ms, 0 blocks over budget; identical
  output (same peak, voice count and non-silent block count); all 730 core tests pass.

## Next session: planning agenda (written 2026-09-18 at the user's request)

Principle carried forward: the user's skate3-audio repo ported the **audio engine**; every remaining
gap is a **game-side driver** (constructor + per-frame updater + trigger) to port from the lifted
retail C++ (`C:\dev\skate3recomp\generated`), verified against the session traces and the owner's
AttribSys vault, then checked headlessly with `wheel_repro` before a playtest.

1. **Playtest the pending build** (footstep level/edge, landing word 10 = 500, 10x faster renderer)
   and read its log: underflow must stay ~0; walking `foot_down` edges must appear per step.
2. **Port the retail audio-state bridge `sub_824B0DA8`** (PhysOut record → audio state bytes
   332..375, floats 204..328). Most remaining drivers read this state; mapping its source fields to
   the engine's published physics output is the highest-leverage task (landing words 7–9, wheel
   contact bytes 333–336 incl. the real rattle trigger 335, per-foot down bits, speeds).
3. **Audit every existing producer against its retail updater**: `Class_Treatment` (sub_824DD6F0),
   `Class_Flips` (constructor sub_824AFAD8, caller sub_824CBFB8), `Class_grind` (sub_824AF8C8,
   sub_824C28B0/sub_824C39E0), and the boot utilities. Earlier packets came from trace dumps and
   may contain heap garbage past the real packet length (constructor object size tells the length).
4. **Port the missing families** one at a time: `Class_Seams` (sub_824AFDD0 ← sub_824C13D0),
   `Class_wheels_skid` (sub_824AF678 ← sub_824C7438), `Class_Squeaks` (sub_824AFF48 ←
   sub_824C7738), `cloth_trick`, `c_cloth_falls`, `c_body_slide`, `c_board_slide`,
   `Class_foot_drag`, `SenseOfSpeed_wind/_rattle` (table in `docs/audio-player-sounds-handoff.md`).
5. **Spatial words** (azimuth/distance virtuals 52/56/60/64 on the owner object): recover them so
   panning is not fixed at centre; **surface mapping** (sub_824C82A8 / sub_82494F58) once the user
   wants more than the default surface.
6. **Separate playback paths** still unresolved: `wheels.big`, `grains.big`, loose board (see the
   implementation plan's M5).

## Addendum — 2026-09-18/19 overnight: the retail player-sound path is integrated

The user's playtest ("rolling sounds looping and variable") led to three findings, all now ported
and wired into the audio worker (`crates/skate-game/src/player_audio/sound.rs`):

1. **Rolling is a granular bed**, not `Class_rolling`: `SFXObj_SkateBoard` drives two grain
   players per truck over `grains.big` through a per-player FrequencyShiftSsb/HighShelf chain
   (`skate-audio-core/src/grain/`). `Class_rolling` layers 0/3 are sparse one-shot texture.
2. **Every gain/pitch/pan word is an EA MixMap output** (`MixMapSK8.mxb`, now staged by setup):
   `skate-audio-core/src/mixmap/` matches the retail capture on 99.9991% of 84 M output words;
   its input writers (`mixmap/inputs.rs`) reproduce the state controller exactly.
3. **Inputs come from the audio-state bridge** (`player_audio/audio_state.rs`, zero mismatches
   against 18,553 capture rows) fed by `RetailAudioInputs` from the engine's native records.

Per 60 Hz tick the worker runs retail's order: bridge → state/listener/position/contacts/jitter
MixMap inputs → every component's process → MixMap tick → every component's update. Components
(`player_audio/components/`), each capture-replayed: board (every packet word, post and release
exact), seams, tricks (flips + cloth_trick), treatment, grind, speed (SoS wind/rattle), contacts
(foot drag), clothing (body slide, cloth falls), footsteps. Full reference:
`docs/player-audio-retail-drivers.md`. Ground truth: the recomp capture hook
(`C:\dev\skate3recomp\src\skate3_audio_capture.cpp`, env `SKATE3_AUDIO_CAPTURE`) and
`tools/audio_capture_extract.py` / `tools/audio_capture_verify.py`.

Headless end-to-end check (the game's own worker path, synthetic roll):
`cargo test -p skate-game --bin skate3rust --locked -- --ignored headless --nocapture` →
continuous rolling 2–9 m/s (RMS −26 → −20 dBFS), worst block 2.5–3.7 ms of 5.33 ms.

**Still approximate or open (labelled in code):** default surface 2 for all surfaces (user
accepted); the G global (treatment w11–13, flips w12 target, state MixMap id 11 bail flag) — writer
not found; Music/VU inputs held at free-skate capture values (no retail music here); B+0 +144 board
emitter position and Skeleton+80 skater facing stand-ins (panning); the random sequence (the
retail generator is shared game-wide); footsteps and body slide/cloth falls pending ragdoll fields
+292/+296/+740/+328/+528..+611/+672 and board slope +712 (in progress); wheel spin (`wheels.big`)
and the `x_jet_rolling` speed grain not yet ported.

Toolchain gotcha: rustc 1.98.1 at -O miscompiles `if x < LOW { LOW } else { x.min(HIGH) }`; use
`clamp`.
