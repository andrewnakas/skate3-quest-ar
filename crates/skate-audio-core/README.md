# skate-audio-core

The Skate 3 audio graph in Rust, translated from native C++ that was **first proved equal to
the original recompiled code**. Every function here is a transcription of a body the shadow
harness compared against the running game, so a divergence points at the translation rather
than at a misunderstanding of the engine.

## What is here

| module | what it covers | verification |
|---|---|---|
| `system.rs` | the command ring producer, all four paths | **6,710 replayed** |
| `player.rs` | the three `PacketPlayer` consumers, the FIFO, the liveness scan | **1,679 replayed** |
| `bitstream.rs` | `sub_82B26B70`, `sub_82B4F5F8`, `sub_82B46EA0`, `sub_82B46FB0`, `sub_82B47010`, `sub_82B47658`, `sub_82B50270`: the packet stream's decoders — an MSB-first bit reader, an unsigned and a zigzag-signed variable-length code, the run-length layer over them, and a paged bit cursor | **21,404 replayed** (8,807 + 5,713 + 1,960 + 980 + 980 + 245 + 1,315, and 1,404 for `sub_82B31D90`, the packed-header unpacker that drives the bit reader); `sub_82B335A8` (a voice's packed bit header into its record and slot, or the defaults), `sub_82B474B8` (four interleaved run-length streams walked to the record containing a target) and `sub_82B50100` (a packetised stream sought to a frame through its seek table), unit-tested, recording pending; and the seek-header family — `sub_82B472C0` (fixed 16-byte records), `sub_82B471D8` (the header dispatch), `sub_82B470D0` (the version dispatch and its wipe) and `sub_82B33780` (one packet header scattered into its slot and entry), unit-tested, recording pending |
| `buffers.rs` | buffer-pair init | **214 replayed** |
| `fp.rs` | the guest's scalar FP idioms: `lfs`/`lfd`/`stfs`, `fcfid`/`frsp`, the single-rounded forms, `fsqrts`, `fmsubs`, `fabs`/`fneg`, `fsel`, `fctiwz`, `fctidz`, `rlwinm` | unit-tested only |
| `spatial.rs` | `sub_82B453D8`, `sub_82B269C0`, `sub_82B454B8`, `sub_82B45788`, `sub_82B45B60`: a source's position becoming a gain per speaker — the unit-disc clamp, the panner placement, distance panning, the seven-sector angular pass, and the power-normalised scale | **2,369 replayed**, all five (474 + 473 + 473 + 482 + 467); `sub_82B45C50`, which lays a source's panner entries out about a centre point (a mirrored pair, or spread pairs, by count), and `sub_82B460A0`, which fills the 8x8 mix matrix from them, unit-tested against direct calls to the five spatial bodies they compose, recording pending |
| `gains.rs` | `sub_82B29AF0`, `sub_82B23B50` and `sub_82B298E0`: the channel gain matrix, the per-channel gain ramp, and the matrix applied through a ramp, all driving `dsp/` kernels over a `+4`/`+14` channel descriptor | **1,384 replayed** (795 + 185 + 404); `sub_82B29BE0`, which republishes a source's spatial mix — re-lays the panners and the matrix when a cached placement parameter moved (or the flag asks), then mixes hard or ramps out of the matrix it saved in its frame, and swaps the pair — unit-tested against direct calls to what it composes, recording pending; and `sub_82B238A8`, a voice's gain ramp advanced a block — latch, place, run one of the three curve writers or hold, then every channel multiplied by the envelope — unit-tested against a direct linear ramp |
| `mathlib.rs` | `sub_82F4DE80` (`floor`, 2.87 M calls a boot — the hottest body ported anywhere in this project), `Image`'s sine (`sub_82F4DED0`) and cosine (`sub_82F4DFB0`), `log10` (`sub_82F55068`) with the natural log `sub_82F54ED8` under it, and `atan2` (`sub_82F52318`) — the last four ported beyond the 216 because callers here needed them | **10,015 replayed** (2,000 floor + 999 sine + 1,001 cosine + 3,000 log10 + 3,015 atan2), all compared by the bits of the returned `f1` |
| `counter.rs` | `sub_82B1F360`, the six-word cascading counter the evaluator draws from | **7,479 replayed** — recorded in two sessions on 2026-09-12 and replayable all along; it lacked only a replay arm |
| `eval/` | the expression evaluator's 40-slot opcode table at guest `0x82FD3600` — **33 slots**: every one that has a verified C++ body, plus the two one-instruction accessors in slots 1 and 2, which are outside the audio corpus and not shadow-verified | verified C++ reference (31); unit-tested only |
| `symbols.rs` | `sub_828E2818`, `sub_828E3148`, `sub_828E3250`, `sub_828E3358`: install a `.csi` project (names relocated, generations stamped, project linked) and resolve a symbol into a slot in two passes, project first and then by name | **unverified new work** (load and game threads, no C++ body). Unit-tested, and `examples/bind_banks.rs` resolves all 1,059 real bank exports with **0 disagreements** against an independent search of the parsed files |
| `patch.rs` | `sub_828DC660`'s bank branch, `sub_82B1DF50`, `sub_828E2B48`, `sub_82B1DAD0`, `sub_82B1D880`, `sub_828E2FF8` and four instance callbacks: load and install an `.abk`, post a message, spawn an evaluator instance, and end it (evaluator slot 4, `sub_82B1C150`, with `sub_82B1BF98` and the unregister and release helpers), run by the interpreter through `interp::Host` | **unverified new work** (loader and game threads, no C++ body). Unit-tested; `examples/grind_instance.rs` runs real banks over the dumped image. Driven by a traced footstep post and its 409 updates, it opens the game's eight layers on the same update, with the same controls; random draws differ (`docs/audio-banks.md`) |
| `voice.rs` | `sub_82B1D240` (evaluator slot 27, the voice op), `sub_82B1F440`, `sub_82B1F4C8`, `sub_82B1F5F8`, `sub_82B1BE30`: open, refresh, pause and release a voice, and clamp and push its parameters, against a host `VoiceDevice` | **unverified**: C++ bodies exist but are gate-labelled or partial, since every one calls the device. Unit-tested with a mock device; `examples/grind_instance.rs` runs the real grind program through it |
| `stream.rs` | `sub_82B3CA60`: deliver frames from a stream object into a caller's buffer, draining the scratch then refilling through the stream's fill function, which a host supplies as `StreamFill` (the XMA decode in the game, the engine's decoded PCM here) | **unverified** (gate 1, never compared). Unit-tested against a ramp fill |
| `sndplayer.rs` | `sub_82B34278` and `sub_82B34108`: `SndPlayer1`'s process, rendering one block from its record ring (start time, pre-roll, format change, looping, slot retirement) through `stream::deliver_frames`, and the fade that ramps each channel's last sample to zero | **unverified**: the first is gate 1, the second unported. Unit-tested on six paths |
| `graph.rs` | `sub_82B44858` and `sub_82B444C0`: a voice graph's per-block pass (publish the descriptor records, then for each node fill its mixer block and run its children's process functions) and the node mixer fill (negotiate a length with every source, render, fall back to `advance_and_clear`, assemble 256 frames), with every indirect call and the timebase through `GraphHost` | **unverified**: gate 1, and the pass gate 3 (`mftb` profiling). Unit-tested |
| `kernels.rs` | a `GraphHost` routing the ten prepare and process addresses the voice classes name to their Rust ports, with the register each reads | the ports are verified (all but `SndPlayer1`'s process); the **routing** is read from the `.inc` bodies and unverified. Unit-tested |
| `modules.rs` | `sub_82B48C48`, `sub_82B49210`/`sub_82B49280`, `sub_82B46350`, the eight voice classes' size and constructor functions (`Gain`, `GainFader`, `Send`, `Rechannel`, `Resample`, `HighPassIir2`/`LowPassIir2`, `Pan2D1` with `sub_82B44FE8`/`sub_82B44F98`, `SndPlayer1` with the node pool `sub_82B39728`/`sub_82B395E8`): lay out a voice's module graph, construct it, and install it in the system's player list | **unverified new work** (game thread, no C++ body). Unit-tested; allocation through `patch::Heap`, and a failed constructor is an `Err` because the teardown `sub_82B48F28` is not ported |
| `classes.rs` | `sub_824A2FD8`, `sub_82481B08`, `sub_82B46770`, `sub_82B463D8`, `sub_82B48840`, `sub_82B3CCC8`, `sub_82B46260`: register the voice device's module classes, cook each class's row defaults into tagged values, and copy one parameter's defaults into a block | **unverified new work** (game thread, no C++ body). Unit-tested; the registry lock calls are not made |
| `device.rs` | `sub_824A3140`, `sub_824A2908`, `sub_82491108`, and the module `vtable+4` entries it calls (`sub_82B31370` Send, `sub_82B23798` GainFader, `sub_82B328E8` SndPlayer1 ids 0 and 5): open a voice over a built graph, play its sample, route its sends and post the panner's angles | **unverified new work** (game thread, no C++ body). Unit-tested; bus creation (`sub_824916E8`) is a `DeviceHost` call, and a failed build or a module callback is an `Err` |
| `eval/interp.rs` | `sub_82B1E290`, the evaluator's tick and interpreter: recount the period when the frame delta changes, count down, and on the tick walk the node list dispatching each program's opcodes and wiring results into the operand block | **unverified new work**: the C++ body is gate 1 and gate 2 and was never compared. Unit-tested, and its record grammar matches all 385 bank programs (`skate-audio-formats` `verify_patch_programs`) |
| `scheduler.rs` | `sub_82B489D0` and `sub_82B39690`: an instance detaching itself from the scheduler, and the bucket list mechanic that removal runs on | **4 replayed** — two calls each, read the count before quoting it; `sub_82B1DC00`, which unlinks a global list entry by key and hands the list record to the detach when the list empties, unit-tested, recording pending |
| `cursors.rs` | `sub_82B32550`, `sub_82B349A8`, `sub_82B3C9D8`: the three verified cursor advances — which ring slot, which entry, which segment comes next | **3,210 replayed** (2,462 + 714 + 34) |
| `leaves.rs` | ten small verified leaves that belong to no larger mechanism ported yet — four command-ring handlers (`sub_82B463A8`, `sub_82B49268`, `sub_82B34B10`, `sub_82B23828`), a `u16` field setter, a 64-bit identity, a stream's unconsumed byte count, a record-size calculation, a two-field zeroing, and a copy that marks its block filled | **13,260 replayed** (3,232 + 230 + 230 + 2,969 + 2,619 + 1,404 + 11 + 924 + 397 + 1,241, and 3 for `sub_82B29278`); `sub_82B2FE00`, the thin five-point ramp, is unit-tested only — its only vectors predate read spans; `sub_82B2FEA8`, a six-point ramp mapped through a 1,652-entry table in one forward sweep, and `sub_82B2F798`, two `log10` level terms settled into this object and a linked one, both unit-tested, recording pending; `sub_82B2F2C8`, six bands resampled through a blended knot curve in the red zone, normalised, and the ring reservations re-armed, unit-tested, recording pending; `sub_82B2FF88`, the level tables rearmed by layout, and `sub_82B2F590`, the filter recommit over the ramp, table, bands and weights, unit-tested, recording pending |
| `vmx.rs` | the VMX128 layer: RexGlue's lowerings, the flush-mode control, and the guest-memory vector accesses | **45 ops replayed** against `probe/vmx128`'s recorded C++; the rest unit-tested only |
| `dsp/sine.rs` | `sub_824531C8`, four-lane sine by range reduction and an 11-term odd polynomial | **2,000 replayed**, the result compared by the bits of `v1` — and the only recorded data in this project that tells a fused multiply-add from two roundings (`docs/vmx128-exactness.md` rule 1) |
| `dsp/scale.rs` | `sub_82B3BED8` and `sub_82B44B20`: `dst[i] = src[i]*k` and `dst[i] += src[i]*k`, each on a vector and a scalar path | **734 replayed** (548 + 186) |
| `dsp/gain_ramp.rs` | `sub_82B3C098` and `sub_82B44D18`: a gain-ramped copy of a fixed 256-single block, and the same ramp accumulated onto the destination instead of written over it | **197 replayed** (166 + 31) |
| `dsp/scale_add.rs` | `sub_82B3CF58`, `z[i] = x[i]·gain + y[i]` alongside a parallel copy `w[i] = x[i]` | **642 replayed** |
| `dsp/biquad.rs` | `sub_82B43AF8`, a biquad over a run of singles, eight a pass | **1,000 replayed** |
| `dsp/allpass.rs` | `sub_82B389A0`, the four-lane one-multiply allpass stage over an **unaligned** tap, accumulated at a gain — and the strongest check on `vmx.rs` rule 1 in the project: 800 of 800 replay with two-rounding multiply-adds, 99 of 800 with fused ones | **800 replayed**; `sub_82B38B68`, the dispatcher that runs a stage from its descriptor or clears its accumulator, unit-tested, recording pending |
| `dsp/clip.rs` | `sub_82B22678`, the hard clipper: clamp 256 samples a channel into `[-level, level]`, then swap the buffer pair — scalar throughout, and it declines to run at all unless the level is below the pool's 100.0 | **1,485 replayed** |
| `dsp/resample.rs` | `sub_82B43FB8`, linear interpolation walked by a 16.16 phase | **530 replayed** |
| `dsp/ramps.rs` | `sub_82B427D8` and `sub_82B42C98`: the envelope ramp writers — one 256-entry block, flat below zero and past the ramp, linear or square-root between, four samples a vector; the square root is `vrsqrtefp` plus one Newton step with a +0/+inf mask, and the alignment padding is linear in both | unit-tested (the vector root against the true root within 1e-4), recording pending; and `sub_82B43340`, the sine-curve writer through the four-lane sine kernel, unit-tested against the true sine |
| `ring.rs` | `sub_82B3DB90`, `sub_82B3DC48`, `sub_82B3DF90`, `sub_82B3DEA8`: copy out of the wrapping decode ring, rank and fill the segments, pad the tail with a rodata constant, and write a block back in | **3,943 replayed** (1,200 + 1,052 + 1,046 + 645); `sub_82B3DD90`, the window builder that drives the other three from a frame of its own, unit-tested against them, recording pending |
| `mix.rs` | `sub_82B34E08`, `sub_82B3C668`, `sub_82B443F8`, `sub_82B2C8F0`: flush the mix accumulator, fold the pending deltas into the rows, advance a fill position and clear ahead of it, and re-fold the rows when the channel count changes — the last builds both owners' row pointers in frame scratch and hands the remap to `routing::gather_bank` | **3,151 replayed** (1,086 + 1,000 + 265 + 800) |
| `stage.rs` | `sub_82B399D0` and `sub_82B39FA0`: the one-pole filter stage over a block, and the dispatcher that runs it through a descriptor or clears the buffer instead | **706 replayed** (353 each), the stage's result compared by the bits of `f1` |
| `contributions.rs` | `sub_82B225A0`, one contribution republished into its owner's running total — by the *change* since the last publish — with a correction through `mathlib::log10` when its coefficient is not the image's zero | **3,000 replayed**, 2,021 through the correction |
| `interleave.rs` | `sub_82B46B30`, six planar float channels into 24-byte frames — a channel *remap* rather than a straight interleave, and its length is a literal 1024 bytes rather than a descriptor field | **600 replayed** |
| `voices.rs` | `sub_82B34BD0`, `sub_82B31480`, `sub_82B31368`, `sub_82B31680`, `sub_828E30B8`, `sub_828E2D78`, `sub_82B49438`: voice and handle lifecycle — unlink a voice's node and fold its gains into the owner, release a voice through the gather, re-point a mix link onto another list, two handle-checked unlinks, and removing an object from its owner's handle array with a memmove | **8,000 replayed** (1,279 + 2,679 + 1,309 + 1,369 + 61 + 14 + 1,289); `sub_82B49100`, retirement — off one owner list, gains and handles reset, out of the handle array, onto the other list — unit-tested, recording pending |
| `pitch.rs` | `sub_82B2DAC8`: a voice's pitch ratio recomputed only when it moved, turned into a 16.16 step rounded half away from zero and clamped at 2^18, then the block's source frames counted — a 64-bit result whose clamp looks at the low word only | unit-tested, recording pending; and `sub_82B2DBA8`, one block resampled through an arena scratch run with the tail kept per channel, unit-tested, recording pending |
| `output.rs` | `sub_82B21F58`, `sub_82B21D98`, `sub_82B20E18`: the output stage — route the planes, interleave them into a 6,144-byte block, ramp its first 128 frames once if a flag asks, and clamp every word to ±1, storing only what it changes | **800 replayed** (399 + 400 + 1) |
| `routing.rs` | `sub_82B426D0` and `sub_82B468C0`: the scatter-mixer — run a table of route bytes, the first route to a destination overwriting it and later ones accumulating, then zero every slot no route wrote — and the bank gather above it, which either looks a channel-count pair up in a 64-entry route table and hands the whole job to the mixer, or copies buffer by buffer at unity gain and zeroes what the sources ran out for. Composes `dsp::scale`'s two kernels and `mem::memset` | **1,100 replayed** (500 + 600); `sub_82B46810`, the downmix program — each route byte one gain-ramped mix-add, landing at 64/65 of the target — unit-tested, recording pending |
| `crossfade.rs` | `sub_82B3D0A8` and `sub_82B3D4F8`: the two-source crossfade with a gain-weighted blend onto an accumulator, and the dispatcher that runs it, or a plain scale-and-add, through a descriptor | **1,248 replayed** (414 + 834) |
| `filters.rs` | `sub_82B27E20`, `sub_82B26568`, `sub_82B26740`, `sub_82B43D78`, `sub_82B2C658`: the per-channel low-pass and high-pass stages, the per-source high shelf and its coefficient builder, and the peaking equaliser | **4,152 replayed** (576 + 576 + 2,996 + 4), given the ported sine and cosine; the peaking equaliser is **unit-tested only** — its recording session lost its display and never reached gameplay |
| `bus.rs` | `sub_82B31838` and `sub_82B305C0`: one source's channels into an object's bus blocks — a downmix ramp for modes 1 and 3 or a moved gain, otherwise the routing row mixed flat, then each channel's gain published — and a descriptor's channels into consecutive 1 KB runs, ramped or scaled by mode, keeping each source's last sample. Composes `routing::downmix` and both `dsp` accumulators | unit-tested against direct calls to what they compose, recording pending |
| `layout.rs` | `sub_82B370E8`: three parameter blocks expanded into the six slots of a speaker layout, `{sentinel, value}` each, group 0 square-rooted | unit-tested, recording pending |
| `meters.rs` | `sub_82B373C8` and `sub_82B376B8`: per-channel mean-square and peak meters over one quarter of each block — four accumulators fed the same vector, so the sum is four times the squares, as the original computes it — kept in ring histories; and the tick that re-sizes the rings when the rate moves, then runs the meters and the layout expansion | unit-tested, recording pending |
| `mem.rs` | the write-set contract of `sub_82EDF460` (memcpy) and `sub_82EE5E80` (memset), which six of the bodies above call | not a port; see its module note; plus `sub_82F52FB8`, a chunked forward memcpy whose overlap behaviour the tests pin |

`cargo test` runs 678 unit tests. **Read the next two sections before reading that as one number:
the modules are checked in different ways, and only the ones whose table row gives a replay figure
have one.**

### The two kinds of green in this crate

**Replayed against recorded vectors** — every module whose table row above gives a replay
figure. Tier 1 is met for all of them:

| module group | vectors | result |
|---|---|---|
| the queue path | 8,603 | all pass, 0 unreplayable |
| scheduler and cursors | 3,214 | all pass, 0 unreplayable |
| ring, mix and crossfade | 7,542 | all pass, 0 unreplayable |
| spatial, gains and the math leaves | 7,349 | all pass, 0 unreplayable |
| DSP kernels, the filter stage and the two filters | 4,930 | all pass, 0 unreplayable |
| **total** | **31,638** | **all pass** |

**These are re-recorded numbers, and the reason matters.** The figure here used to read 8,607,
recorded before a defect in the recorder was found: it snapshotted the read set *after* the
original body ran, so any cell a function both reads and writes was recorded holding its own
output. Of 4,454 such bytes in one file, 4,454 held the post-call value and none held the entry
value. That defect is fixed at the recorder, every vector file above was recorded after the fix,
and the count differs from 8,607 only because two sessions of the same script do not produce
identical call counts.

**Read the scheduler and cursor counts per function, never as one total.** They are
`sub_82B3C9D8` 2,462, `sub_82B32550` 34, `sub_82B489D0` 2, `sub_82B39690` 2, and `sub_82B349A8`
714. That last one once read **zero**: it ran 881 times in the session, but a hotter function in the
same recording spent the vector cap first, so it was recorded on its own afterwards. Two calls is a real comparison and a thin one; `sub_82B39690`'s two both
arrive with the same `which` byte and with the node naming neither list head, so
`the_which_byte_decides_which_head_can_name_the_node` is carried by its unit test alone. Measured,
not assumed: breaking the head selection leaves all 2,500 vectors passing.

Of eight deliberate breaks replayed against these vectors, **three were caught** — the segment
index wrap (83 failures), the ring index wrap (2 failures), and the bucket-manager address (which
turned a pass into an *unreplayable*, because the wrong manager is not in the recorded read set).
The five the vectors miss are named where they belong: three are the equivalent transformations
listed below, one is the equality-versus-threshold end test (every recorded call lands exactly on
a segment end, so `<` and `!=` agree on all 2,462), and one is a limit of the recording — see
below.

**`detach_instance`'s 64-bit `r3` is now fed in full, and still not exercised.** The recorder
stores entry `r3`..`r10` at full width and the replay passes the wide value, so the 64-bit
`(r3 + 112)` chain that carries the high half into the returned manager address is no longer cut
off by the harness. But both recorded calls arrived with a high half of zero, so a port that
truncated the chain to 32 bits would still pass them. Until a session records a call whose `r3`
has a non-zero high half, that chain rests on its unit test alone.

```
cargo run --example replay_vectors -- VECTORS.tsv 00000000 3F800000
```

**`dsp::resample` replays now that the recorder stores `r8`.** It takes its 16.16 phase increment
in `r8`, which the recording's fixed columns omit, and that increment determines every address the
call reads, so a zero in its place would have turned a failure into a meaningless pass. The
recorder's wide columns carry it, and all 530 recorded calls replay. One trap survives into any
future dispatcher: `r3` is this function's **output sample count**, an input, so comparing the
recorded return against `r3` would compare the count with itself.

The vectors are recorded by the harness itself (`skate3_audio_vectors_path`) — real inputs the
game generated, not synthetic ones. An address the port reaches that was not recorded makes the
vector **unreplayable**, never zero-filled, because feeding it fabricated input would turn a
failure into a meaningless pass.

**`vmx.rs`'s operation table is replayed too, against a different reference.** Not the game, but
RexGlue's own lowerings compiled by clang-20 — the compiler the recomp is built with — and run over
the adversarial inputs of `probe/vmx128`, with the answers on disk:

```
probe/vmx128/run.sh                              # regenerate the vectors and the C++ results
cargo run --release --example check_vmx_primitives
```

**Measured 2026-09-13, after correcting rule 1: 45 of 45 operations bit-identical against `clang20_pinned`, `gcc_pinned` and `clang20_plain`, 56,880 lane comparisons each, in both flush-to-zero states** — with every reference now built with the recomp's own code-generation flags. Against `gcc_plain` it is 86 of 90 `(op, ftz)` pairs, diverging on `vmaddfp`/`vnmsubfp` in two NaN lanes of 632, which is rule 4 and not a fault: the winning NaN operand slot is a register-allocation decision and cannot be derived from source in either language. The earlier 2026-09-12 run reported 45 of 45 against references built with `-march=native`. That agreement was real, and it certified the wrong arithmetic — see rule 1. It is an
`example` rather than a test because the recorded files are gitignored build products; a `cargo
test` that skipped when they were missing would be the vacuous green this project keeps warning
about. What this covers is the arithmetic the kernels are built from, and **not** whether they are
composed in the right order. It also leaves seven primitives uncovered — `vrfin128`/`vrfip128`/
`vrfim128`, every store, `dcbzl`, `lvrx128`, and `vspltw128` at three of its four immediates — each
of which has a unit test against a hand-written model instead, which is weaker. `vmx`'s module
documentation carries the table.

**Unit-tested against a verified reference** — `fp.rs`, `counter.rs`, `eval/` and `dsp/sine.rs`. The
C++ body each of these was translated from was compared call-for-call against the original under the
harness, on real inputs, at zero divergence. The Rust has no vectors of its own, for one of two
reasons. `fp.rs`, `counter.rs` and `eval/` have no direct call site in the lifted tree at all (the
evaluator reaches all 40 slots through one `bctrl` on a data word), so the harness never bracketed
them individually. And `dsp::sine` is a register-only kernel the harness verified over 6,994,118
calls without ever writing it a vector. Either way what this
buys is a much smaller search space — a fault here is a transcription error, not a misreading of
the engine — and what it does not buy is a number.

**A shadow call count is the C++'s evidence, not the Rust's.** `sub_824531C8` is verified over
6,994,118 calls and `sub_82B3BED8` over 1,057,635 with `skipped=0`; those numbers say the body
being transcribed is right, and say nothing about the transcription. The crate's own evidence is
the replay figure in the table: 186 for `sub_82B3BED8`, and none yet for `sub_824531C8`. Do not
quote the first kind as if it were the second.

The unit tests are held to the standard the vector work is: each was checked by breaking the
function it covers and confirming the test fails. **224 negative controls** have been run:

- 28 across `fp.rs`, `counter.rs` and `eval/`; 25 now fail correctly, 21 of them on the first
  attempt;
- 25 across `scheduler.rs` and `cursors.rs` — one per test — all 25 failing correctly on the
  first attempt, with each break restored and re-checked;
- 51 across `vmx.rs` and `dsp/`'s first three kernels — one per test — of which 50 fail correctly.
  The one that does not is arithmetic rather than a weak test and is described below;
- 71 across `mem.rs`, `ring.rs`, `mix.rs`, `dsp/biquad.rs`, `dsp/resample.rs` and `fp.rs`'s new
  `nmsub_single` — one per test — all 71 failing correctly, with each break restored and the whole
  suite re-run afterwards. One of them passed on the first attempt and is described below;
- 49 across `spatial.rs`, `gains.rs`, `mathlib.rs` and `fp.rs`'s six new idioms — one per test — all
  49 now failing correctly. **Four passed on the first attempt and three of them were weak tests**,
  described below.

**The spatial batch's four first-attempt passes, since three of them are the same mistake.** A test
that names a boundary has to land on it, and a test that watches a word has to make that word move:

- `a_length_inside_the_snap_window_reports_one_and_keeps_the_position` claimed to check the snap
  test at exactly the 0.999 cell, and reached it by squaring the single-rounded square root of
  0.999 — which misses by an ulp either way, so `>` and `>=` agree there. It now patches the cell to
  0.25 and squares 0.5, which lands on it exactly. (Patching is legitimate here and everywhere else
  in these tests: the bodies read every constant live, which is the whole point of doing so.)
- `the_count_is_reloaded_before_sector_ones_centre_store` skipped a store whose value was `old + 0`,
  so the skip was invisible. It now arranges a non-zero centre share alongside the zero that
  clobbers the count — both at once, which takes `f1 = 1.0` and `left < right`.
- `a_zero_destination_count_writes_nothing_at_all` left the gain matrix at zero except for one cell,
  so a stray accumulating pass added nothing. The matrix is now poisoned throughout.
- The fourth was the control, not the test: a "hoist the count" break that inserted an unused
  variable and left the live read in place, so it patched nothing.

Five passed at first in the earlier batches. Four were fixed by writing sharper tests, not by
lowering the claim; the fifth was two tests that could not fail at all and were replaced.
`op_round_product`'s multiply order needed inputs where an intermediate product actually rounds —
over small primes a reversed quad gives the identical answer. `op_curve`'s upper-neighbour clamp
needed a non-zero interpolation fraction. `op_curve`'s nearest-mode test needed a scale *above*
1.0 as well as below. `op_envelope`'s mode-word reload needed the aliasing case where the mode
word *is* one of the words the call stores.

**Two tests in `dsp/` could not have failed and were replaced outright.** `scale`'s and
`gain_ramp`'s "overlapping buffers see the pre-call source" both put the destination a whole group
or block *behind* the source, where every store lands on bytes the loop has already consumed — so
no reordering of loads and stores could have been detected. They now put the destination one word
(one vector) *ahead*, where the first store falls exactly on a later load's address, and the
corresponding controls fail correctly. Two more assertions inside those tests were wrong about what
the original does rather than vacuous, and the code turned out to be right: an unaligned `stvx128`
masks its own address down to the containing 16-byte block, and the next block's `dcbzl` reaches
back over the previous block's last vector. Both are now asserted rather than written around.

**Two assertions in `vmx.rs` were backwards and failed on the first run.** `vmaddfp_rounds_once`
and its `vnmsubfp` twin asserted that the *fused* form cancels to zero, when in fact it is the
*unfused* form that does: a separate multiply rounds the product's low bits away, and the addend
then cancels what is left, while the FMA keeps them and returns exactly those discarded bits. A test
written the other way round would have passed against an unfused translation. The same error was in
`dsp::scale`'s fusion test and was corrected with it.

**And the corrected assertions were still testing the wrong arithmetic — found 2026-09-13.** The recomp has no `-mfma`, so its `vmaddfp` rounds twice and the *unfused* answer, zero, is what it computes. Those two tests and `dsp::scale`'s now assert that. The evidence is in real data: 4 of 2,000 recorded `sub_824531C8` calls would not replay under the fused reading. `docs/vmx128-exactness.md` rule 1 records how the probe came to certify the wrong form.

Four still pass, and each is an equivalent transformation rather than a weak test:

- the unrolled trip count in `op_round_product` and the two summers — reducing it leaves the
  remainder to the trailing loop, which visits the same words in the same order and reads the same
  addresses. The only residual difference is which operand of a commutative multiply comes first,
  and `docs/vmx128-exactness.md` rule 4 records that float ops are not NaN-commutative while the
  winning operand is chosen by register allocation and cannot be derived from source at all;
- deriving a completed segment's next slope from the value word rather than from the target just
  reached, in `op_envelope` — the word was stored from that target and the round trip is
  single-exact;
- moving `op_envelope`'s sign test after its byte store — both operands are already in locals;
- **rewriting four of `dsp::gain_ramp`'s seventeen `vmaddfp` sites as a separate multiply and add.**
  The group multipliers are splats of 1 … 7 and the block step is 8, and a product by a power of two
  is exact — so at groups 1, 2, 4 and at the block step the fused and unfused forms are the *same
  function*, not two answers that agree. Measured: the break at group 1 leaves all 155 tests
  passing, and the identical break at group 3 or group 5 fails
  `it_matches_the_independent_model_bit_for_bit`. All seventeen go through `vmx::vmaddfp`, which
  rounds twice as the recomp does — that measurement was taken while the layer was fused.

Those four are reproduced as the original has them anyway, and said so in their doc comments, but
no test in this crate would catch their absence.

**One of the 71 new controls passed on the first attempt, and it was a weak test.**
`fp::nmsub_single`'s fusion test was broken to `((c - a*b) as f32) as f64` — the whole expression
in double — and stayed green. That form is not the realistic mistranscription and the test now says
so explicitly: for two `f32` operands the product needs at most 48 bits and is **exact** in an
`f64`, so the double-intermediate form and the true `fnmsubs` differ only through double rounding,
which the test's input does not reach. The control was re-run against the form that actually
threatens the port — a separate `fmuls` then `fsubs`, two `.s` roundings — and fails correctly.

**Three more writes join the "no test can catch this" list**, each measured the same way:

- `dsp::biquad` stores the four-single history back even when the span wraps and the filter never
  runs. Those are the values it just loaded, through the same `lfs`/`stfs` round trip, so the store
  is a no-op in value. The one input that would distinguish it is a signalling NaN — and **measured
  on this target, Rust's `as` casts leave sNaN payloads alone** rather than quieting them the way
  the guest's widening load does, so even that does not work. Removing all four stores leaves the
  suite green;
- `ring::fill_tail`'s second cap on buffer 1 (`block + 255`, applied after `block + 127`) can never
  fire. That is measured by `the_two_buffers_are_capped_at_different_lengths` reading 127 rather
  than argued from the arithmetic, and both compares are written out because the original emits
  both;
- `dsp::resample`'s within-trip load/store interleaving is pinned only where the output aliases the
  table *inside one group of eight*. `the_stores_that_precede_the_second_batch_of_loads_are_seen_by_
  them` constructs exactly that layout and derives the answer by hand; outside it, reordering the
  block is invisible.

`scheduler.rs` and `cursors.rs` add **six more of the same kind**, each measured the same way — by
making the change and watching the whole suite still pass — rather than assumed:

- `recycle_node` re-reads the node's two link words between the neighbour stores instead of
  hoisting both loads, which differs only when a neighbour's link field overlaps the node's own;
- `detach_instance` loads `instance + 0` after storing the parked bucket index, which differs only
  if that store lands on the node pointer — the input the C++ `Windows()` refuses as gate 2;
- `advance_ring_cursor` reloads the cursor byte it has just written, and stores zero into `+432` a
  second time on the latch path. The second of those is the same value to the same address, so it
  is invisible to a single-threaded compare in either direction, like the ring's publish ordering;
- `advance_segment_position` reloads `+49` after retiring a segment, and `+49` and `+36` again
  before addressing the next one — all three differ only for a segment table that overlaps the
  object's header.

One reload of this family *is* pinned, by `scheduler.rs`'s
`the_free_head_is_re_read_after_the_nodes_link_words_are_written`. Read that test's comment before
quoting it: the aliasing layout it uses is one the C++ `Windows()` refuses to bracket, so what it
establishes is that the reload survived the transcription, **not** that the guest agrees with the
answer. Nothing establishes the latter.

**The 2,500 recorded vectors do not catch these six either**, which is the stronger version of the
same statement: removing `advance_segment_position`'s index reload, or `recycle_node`'s free-head
reload, leaves every vector passing. Real gameplay simply does not lay these objects out so that
they alias. That is evidence they are equivalences on the inputs the game produces, and it is not
evidence they can be dropped — the C++ has them because the original does.

### A recording defect these vectors exposed, and what the replay does about it

The recorder snapshots the **read set after the original body has run**, so any cell that is both
read and written is recorded holding its post-call value. This is measurable inside a single file,
without a second recording: across `sched_cursors.tsv` there are 4,454 bytes covered by both an
`I:` span and a `W:` span whose entry and expected bytes differ, and in **4,454 of 4,454** the
`I:` byte equals the *expected* byte — in none of them the *entry* byte.

`replay_vectors` therefore treats the `W:` entry column as authoritative wherever it overlaps an
`I:` span, and merges the spans byte-wise instead of storing one segment each. The overlay is
sound rather than convenient: the harness requires every byte a body writes to lie inside a
declared window (`docs/shadow-harness.md` — a write outside them is never rewound), so every stale
read-set byte is by construction covered by a window entry byte, and the overlay restores all of
them. It invents nothing; a byte no span recorded stays uncovered and the vector is still reported
unreplayable.

Until the recorder is fixed, **the `I:` columns of a vector file cannot be read on their own** as
the state a function saw. Reading them that way is what first made these ports look wrong.

### Reads no `Windows()` declares, and what they cost

A read the C++ `Windows()` does not declare is not in the recorded read set, so a vector that
reaches it is **unreplayable**: the replay cannot invent the bytes, and feeding fabricated ones would
turn a failure into a meaningless pass. The spatial batch found five such cells, across three
functions. Every one is a *rodata constant* reached through a callee, so the fix is one `spec.read`
line in the `.inc` and costs nothing against the window budget:

| port | undeclared read | why it is reached |
|---|---|---|
| `sub_82B269C0` | `0x8231A844` (1.0), `0x820ED800` (0.999) | its callee `sub_82B453D8` reads both |
| `sub_82B45788` | `0x82010108`, `0x820514D8` (8 bytes each) | its callee `sub_82F4DE80` reads both |
| `sub_82B45788` | `0x82165A10` (0.0) | sector 1's initial centre share — see below |
| `sub_82B23B50` | the thirteen cells `dsp::gain_ramp` names | its callee `sub_82B3C098` reads all of them |

The `0x82165A10` row is a difference from the C++ rather than from the original: `sub_82B45788`'s
port writes `double centre = 0.0;` with the lifted `lfs f12,23056(r11)` in the comment beside it.
The original loads the cell, so `spatial::add_angular` loads it too, per this crate's rule that
guest constants are read live. On the measured image the cell is `0.0` and the two agree exactly.

### What the spatial constants turned out to be

Nine rodata cells were read out of the validated image dump while writing `spatial.rs` and
`mathlib.rs`. Four of them settle what the code *means*, not merely that the addresses are right:

| cell | measured | consequence |
|---|---|---|
| `0x822F8904` | 0.15915494 = **1/2π** | `sub_82B45788`'s reduction is an angle **wrapped into one turn** |
| `0x820B411C` | 6.2831855 = **2π** | the same wrap, and the base the two reflected sector bounds subtract from |
| `0x822F8A6C` | −0.017453292 = **−π/180** | `sub_82B269C0`'s `f1` is an angle in **degrees** |
| `0x82060C44` | 3.1415927 = **π** | its non-positive-depth path is a reflection through the origin |
| `0x822F87E0` | 0.015625 = **1/64** | `sub_82B23B50`'s ramp spans 64 samples, matching `dsp::gain_ramp`'s own 64.0 cell |

And one correction. `probe/ports/notes/sub_82F4DE80.md` records its second pool double as
"by shape … 2^52", not measured. It is **1e18** (`0x43ABC16D674EC800`). The port is unaffected
because it reads the cell live, and 1e18 is the better constant for what the code does with it: it
is the largest round decimal magnitude comfortably below `2^63`, which is the bound `fctidz` needs
in order not to return its integer indefinite.

### Limits carried over from the harness

From `docs/shadow-harness.md`:

- A large share of `EVENT_SUBMIT` passes verify one write **vacuously**: on an empty FIFO that
  word is already 0 on entry and 0 in the expectation.
- All recorded liveness queries arrived with an empty FIFO, so the list-walk branch is
  **unreachable by these sessions**, recorded as a permanent limit rather than a pending task.
- `EVENT_PLAY` is verified at one input point: 48 kHz, six channels.

Four divergences from the original are deliberate and pinned by tests. The guest's `fctidz` and
Rust's saturating `as i64` disagree at exactly 2^63, so that conversion is written branch for
branch; `fctiwz` and `as i32` disagree on NaN, likewise; the ring's publish ordering is
inverted, which leaves memory byte-identical and is therefore invisible to a single-threaded
compare in either direction; and `scheduler::detach_instance` returns an out-of-segment `Error`
where a null node would have sent the original to guest address 8 — the same choice
`eval::state::op_shuffle_bag` makes, and for the same reason: the C++ `Windows()` refuses that
input, so nothing is known about what the guest does there and inventing a write would turn a gap
in coverage into a wrong answer.

One behaviour is reproduced rather than fixed and can hang the caller: `eval::wave::op_oscillator`'s
phase-wrap loop does not terminate on a NaN or infinite phase, because the guest's unordered compare
does not terminate either. Its doc comment says so.

## The evaluator

`eval/` is the crate's second real subsystem. The interpreter `sub_82B1E290` walks a list of nodes,
and for each record in a node's program it calls `TABLE[opcode]` with an operand block in `r3` and
stores the returned low word back into the block. The table lives at guest `0x82FD3600` and was read
out of the validated image dump: exactly 40 entries, of which

- **31 are ported**, every slot with a verified C++ body;
- **3 fail the port screen's gate 1** — each makes an indirect call, so neither language has a
  verified reference for them;
- **1 is a pending path split** (`sub_82B1C210`, given a C++ body on 2026-09-12, comparable only on
  the inputs that skip its listener broadcast). Nothing unverified is translated here, so it waits;
- **5 are outside the 216 audio-thread functions** and have never been screened at all.

`eval::TABLE` carries all 40 rows with the reason for each gap, and `eval::dispatch` routes an
opcode the way the `bctrl` would. An unported opcode is an `Error` naming the function, never a
zero: the interpreter would store *something*, and inventing a value would turn a gap in coverage
into a wrong answer.

The interpreter itself is **not** here. It fails gate 1 and gate 2, so it has no verified C++ body
to translate; a Rust version would be new analysis rather than transcription.

| submodule | slots | what they are |
|---|---|---|
| `eval/arith.rs` | 13 | add, subtract, multiply, divide, remainder, min, max, saturating subtract, capped multiply, two summers, two rounding multipliers |
| `eval/accessors.rs` | 7 | three take-and-clear accessors, a stack top and push, an any-nonzero reducer, a flag-table select |
| `eval/state.rs` | 8 | hysteresis window, stepping cursor, timer, ramp, delay ring, shuffle bag, and two draws from `counter.rs` |
| `eval/wave.rs` | 3 | oscillator (quarter-sine table, pulse, ramp, triangle), multi-segment envelope, sampled curve |

## Design

Guest structures are **byte-addressed big-endian accessors over `&mut [u8]`**, not idiomatic
Rust structs. These are recovered layouts with asserted offsets, and the per-function criterion
compares bytes against the verified C++ — a byte-addressed view makes that direct instead of
routing it through a serialisation step that could hide a discrepancy of its own. Offsets are
asserted at compile time, mirroring `docs/rw_audio_structs_check.c`.

Guest constants are read **live** through the `Guest` map rather than folded in, because that is
what the originals do, because two of them are not constants (the scheduler's tick scale is written
at run time), and because a patched image should reach the ports. `eval`'s module documentation
records each cell's dump value alongside a compile-time assertion of the `lis`/`addi` arithmetic
that forms its address — one misread constant in `sub_82B2FE00` caused this project's first shadow
divergence.

Every evaluator slot has the same signature, `fn(&mut Guest, u32) -> Result<u64>`, including the
ones that write nothing. That is the guest's own signature for all 40 slots and it is what makes
the table dispatchable; each op's doc comment states its write set, which the signature does not.
The `u64` return is the guest's full `r3`: the interpreter keeps only the low word, but several ops
legitimately leave bits above 31 set, and the harness compares all 64.

Two callees in `player.rs` are **closure parameters**, not ports: the decoder teardown and the
restart branch, both of which make indirect calls. Neither is portable, and the first is exactly
why `EVENT_STOP` has no comparable path under the harness.

## Not written yet

`xma.rs` (the paired ring protocol against a decoder trait) and `graph.rs` (instantiation from
descriptor metadata). Inside `eval`, the interpreter's node walk and the nine unported slots, for the
reasons above.

`dsp/` is **started, not finished**: six kernels. What is there is the families that carry the most
calls — the sine helper, the buffer multiplies, the gain ramp, the biquad and the resampler — and
what is not is the heavier vector kernels. The gain plumbing above them is now in `gains.rs`.

Two of the ten functions screened for the spatial batch were **left out on grouping, not on status**:
`sub_82B34268` (store a `u16` at `+460`, return 0; verified, 202,510 calls a boot) and `sub_82B463A8`
(a command-ring handler that stamps `{0x7FF7FFF1, float}` into a slot and returns 16; verified,
761,054 calls). Both are trivial and both belong with the command-queue layer — `sub_82B463A8` is
never called by a `bl` at all, only through a record's `+0` handler pointer — so they want a
`commands.rs` holding the handler table that `system.rs` produces records for, not a corner of the
spatial module.

**The image's sine and cosine have no port in either language.** `sub_82F4DED0` and `sub_82F4DFB0`
are outside the 216 audio-thread functions the sweep covered, so there is no `.inc` for either — the
same position `sub_82B43978` is in. `spatial::place_panner` and `spatial::add_angular` therefore take
a `mathlib::Trig` parameter, whose default `Unported` returns an `Err` naming the guest address.
Until those two are ported, **neither function can be replayed against a recorded vector at all**:
their results feed every value both bodies store, and substituting `f64::sin` would produce a number
right to fifteen digits and wrong in the bits the callers' `frsp` keeps. Note also that
`add_angular`'s own `stwu r1,-160(r1)` is not reproduced *because* the trigonometry is a parameter:
the frame exists only so those two helpers' red-zone spills land where the original put them, and
porting them as guest bodies would make it load-bearing again.

Two neighbours of the new modules were **considered and left out on their dependencies**, not on
their status:

- `sub_82B31838` (mix one source's channels into the bus blocks; verified, 626,736 calls a boot)
  builds two pointer arrays **on its own guest stack frame** and hands them to its mixers. A
  faithful port needs the guest `r1` as an argument and writes into a region no window declares and
  no recorded vector contains, so a replay of it would be unreplayable by construction. It also
  calls `sub_82B46810`, which is verified and not translated yet;
- `sub_82B43978`, the generic-count biquad `dsp::biquad` delegates to, has **no `.inc` at all** — it
  is outside the 216 audio-thread functions the sweep covered, so neither language has a verified
  reference for it. `dsp::biquad` returns an `Error` naming it rather than guessing.

**The scheduler tick cannot be written, and that is not a backlog item.** `docs/PLAN.md` section 6
asks `scheduler.rs` for the "two-bucket tick, per-plug-in profiling toggle, mid-tick self-removal";
only the last of the three is in `scheduler.rs`. The tick is `sub_82B48A50`, recorded `gate-1` in
`docs/ports.md`: it calls each node's process function through a `bctrl`, and it reads the timebase
through `sub_82B1F7E8` twice per node, which is gate 3 as well. Neither language has a verified
reference for it, and the profiling toggle is a field that tick writes. A Rust version would be new
analysis dressed as a transcription. Its two neighbours in the PLAN's Phase 2 list are out for the
same reason — `sub_82B48440` is gate-1 through the allocator and `sub_82B482F8` is gate-2.

Two verified functions in the same area were **considered and left out** on the status column
rather than the gate: `sub_82B376B8` (the metering tick) and its callees `sub_82B370E8` and
`sub_82B373C8` are recorded `thin` in `docs/ports.md` — verified, but on too few calls for their
size to carry promotion. Translating a `thin` body would blur a distinction this README exists to
keep. (`sub_82B373C8` is a VMX128 kernel besides; that is no longer the obstacle it was, since
`vmx.rs` exists, but the `thin` status is unchanged and is the reason it stays out.)

All 216 audio-thread functions have a verified or gate-labelled C++ reference in
`recomp/src/audio_ports/`, so the remaining modules are transcription work rather than analysis.
See `docs/PLAN.md` Phase 4 and section 6.
