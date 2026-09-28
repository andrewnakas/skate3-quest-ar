# Player and board audio implementation plan

Implementation checkpoint: **2026-09-15, Windows**. The core now has the phase-four command
drain, the resident play-command branch, immutable PCM cursors, the host-backed stream pump, and
the Dac snapshot/sink/clock close-out. A private TU3 runtime image is now available, and the
70-frame footstep fixture plus a looping grind run both reach graph construction with real bank
programs. In graph mode the fixture now applies recovered setters, pan, suspend/resume, and query
behavior to those voices; placeholder buses and the fixture's absent invocation of the command
drain, PCM path, and output runtime still mean it does not render audio.

The next deliverable is **one traced footstep audible through the ported audio graph in `skate3rust.exe`**. Getting there requires a working voice device, buses, command processing, PCM delivery, and a block driver. Decoding a sample or printing a graph is insufficient. After that, expand to resident loops, shared bank programs, live gameplay producers, the separate wheel/grain paths, and complete lifecycle and mix verification.

The scope is **all sounds and authored variations for the local person and their board**: wheels, tricks, grinds, slides, landings, body and board impacts, footsteps, foot drag, clothes, bails, body movement, and speed effects. A loose board still belongs to the player after a bail. Preserve the full ordinary impact repertoire, including severity/material variations, layering, and randomized selection wherever the original programs use them.

**Scope clarification from the user:** dedicated Hall of Meat sounds, slow-motion presentation, and mode-specific logic are not required. Ordinary body/board impacts remain required even when they share a caller or asset with that mode. Classify shared sounds by their actual gameplay use before excluding them. Pedestrians, other skaters, environmental ambience, music, and unrelated physics props remain outside this implementation.

**Never commit or distribute proprietary game assets.** Archives, ISO/XEX files, decrypted guest images, decoded PCM, and recordings containing game audio stay in the owner's private local storage. CI and release packages contain code, synthetic fixtures, and reviewed observational metadata only.

## Evidence and corrections to the handoff

This plan starts from engine commit `cc41fb980c4bc3505e17ae97f6385347006318e8` plus the five pre-existing modified files: `skate_audio.rs`, `audio_check_main.rs`, `audio/ffmpeg.rs`, `scripts/Build.ps1`, and the handoff. Those changes were preserved.

The sk8Audio research branch was fetched into the ignored `.local/references/skate3-audio` directory at commit `d3d81ffd634a70299f504b39cccf0595fadd43af`. All 63 core-crate files and 30 formats-crate files match their vendored engine counterparts byte for byte, excluding `target/` and `Cargo.lock`. The local lifted C++ in `C:\dev\skate3recomp\generated` was also inspected; that checkout is at `87027a16bc0a2b7740a7842f1dbd830ce8c208e8`, on a different development branch. It supplied static call-site evidence, not a new audio capture.

Evidence labels used here:

- **Measured here:** executed against this Windows working tree or inspected directly in the owner's archives/traces.
- **Recorded previously:** the research project's reported comparisons or the Windows output checkpoint; not independently reproduced here unless stated.
- **Implemented, unverified:** code exists, but the required real runtime comparison has not passed.
- **Missing / unresolved:** implementation or evidence is still required.

| Check | Result and limit |
|---|---|
| Windows release unit tests, Rust 1.98.1 | **706 core tests passed**, with no failures. The documented formats count of 33 is stale; it is not an observed Windows regression. |
| `verify_banks` on the owned `audiofiles.big` | 376 ABKs, 9 CSIs, 23 EMSs, 20 BNKs; 1,509 checks agreed. Its printed 89,431 sample slots are capacity, not playable samples. |
| `bind_banks` | 1,059 exports checked: 187 first-pass bindings, 868 second-pass bindings, **4 unresolved**, zero disagreements with independent lookup. The unresolved names are non-player exports; do not describe this as all 1,059 resolving successfully. |
| `verify_bank_fixups` | 376 banks, 4,656 rebased words inside their banks, zero failures. |
| `verify_patch_programs` | 385 programs, 31,016 operations, zero structural failures. This checks program grammar and operand bounds, not successful execution of every opcode. |
| `verify_loop_headers` | All 5,168 used bank samples consistent: 399 looping, 4,769 plain. All 14 grain streams consistent; none sets the EAAC loop flag. |
| `grain_probe` | All 14 grain members locate a valid EAAC header with the duration matching the outer record. Grain scheduling semantics remain unresolved. |
| Owned footstep decode | `fstep_skateshoe1_sm.abk`, sample 0: mono XMA, 48 kHz, **15,966 declared and decoded frames**. This was decode-only, not playback or an independent bit comparison. |
| Session inventory | Recounted all eight compressed traces: every one of the original handoff's 18 object totals matches. The current scope retains 17 and excludes the dedicated Hall of Meat object. The footstep update file has 409 rows. |
| `sound_report.py`, `play4 --every 30` | Runs on Windows with the owned archive and built helpers. All 18 original objects appear, including 11 Hall of Meat posts retained as historical evidence. Bank-attribution ambiguity is visible in the report. |
| Windows output diagnostic | **Recorded previously:** the tone and real ambience stream reached the diagnostic output path; the checkpoint reports 204,416 decoded five-channel frames and stream completion. Its six diagnostic tests passed in the preceding task. They were not rerun here. |
| Private TU3 image / graph fixtures | The locally built TU3 runtime applied its patch before a private 18.5 MB guest-image dump was written. `TABLE[19]`/`[20]` read `82B1CEA0`/`82B1CEF8` from it. The prescribed 70-frame footstep fixture reproduced seven graph opens at frame 59 and one at frame 65 (6,116 evaluated operations); its complete 3,000-frame sequence stayed live through 278,000 operations with the same eight opens and no reported evaluator error. `Class_grind` ran 375 frames (7,192 operations) and opened two looping voices. A follow-up 10-frame grind run applied its recovered voice-control entries to real built graphs; these runs use placeholder buses and no command/PCM/output runtime, so neither rendered PCM nor reached hardware. |

The following corrections affect implementation order:

1. **Looping and stream type are separate fields.** `eaac::Header` reads stream type from bits 31–30 and looping from bit 29 of header word two. A direct inventory of every used bank sample found **stream type 0 for all 5,168**, including all 399 loops. `bitstream::parse_voice_header` puts stream type at record `+73`. Therefore the handoff's equation of looping bank samples with play-command kinds 1/2 is incorrect. Implement resident looping after the short play path; require the long path when the actual source type needs it.
2. **The graph example is not a runtime device.** `LoggingDevice::set` only records values, `query` always reports alive, and release/suspend/resume only log. Its bus addresses include `0xB0B0xxxx` placeholders. Copying this setup and adding a drain would still lack real property updates, valid send destinations, and completion.
3. **Player programs require both pure and host-mediated opcodes.** A focused structural scan of the original 18 banks (17 current targets plus `hom_slo_mo`) found slot **5** in `Seams_Bank`, `Rolling_Rattles`, `PatchBank_Rolling_Surfaces`, `Foley_Cloth`, and `PatchBank_RocksBounce`; slot **39** appears in `Foley_Cloth`. `PatchHost` now supplies slots 4, 5, 27, and 39 at the runtime boundary. Slot **20** appears in `PatchBank_Rolling_Surfaces` and `PatchBank_RocksBounce`; its pure counted-maximum port is derived from the generated recompile and has synthetic coverage, but no replay comparison. Slot 19 has the matching counted-minimum port and slot 38 remains unimplemented; neither occurred in the focused player-bank scan. Bank-level presence does not establish which exported input executes an opcode; trace-driven execution must resolve that.
4. **The full sample set is not entirely 48 kHz mono.** The 17 current target banks contain **1,073 samples: 912 plain and 161 looping**. `sense_of_speed` includes five mono 22,050 Hz samples; footsteps include ten mono 36 kHz samples; `Treatments` includes one **four-channel 48 kHz** sample. Full playback needs rate conversion and multichannel routing. The original handoff's 18-bank inventory totals 1,087 samples; its additional 14 belong to `hom_slo_mo` and are not a required dedicated sound family. These are bank inventories, not proof that every sample is reachable through an in-scope event.
5. **Footstep parity is narrower than the handoff summary.** The detailed research table records game batches of six then two voices, versus Rust's seven then one. It records a four-block delay after update ten in Rust, different pitch/variants/start gains, and a late voice receiving the next pan value. Eight layers, matching sample groups, and several payload-derived controls agree. Randomness is a hypothesis for the remaining differences, not a proved explanation.
6. **The 70-block footstep command consumes only 12 of the 409 updates.** It is a first-onset smoke check. The last update is scheduled at block 2,449 by the current example; a complete replay needs at least 2,450 blocks, plus a tail. `UPDATE_EVERY=6` is the reproduction's approximation, not a universal gameplay cadence.
7. **Dedicated Hall of Meat integration is no longer an acceptance requirement.** `physics/animation_phase.rs::publish_feedback` hard-codes `hall_of_meat_enabled: false`, but changing that mode input is unnecessary for this scope. Recover ordinary body/board impact behavior independently; its shared caller `824DD408` still requires analysis for `Class_Treatment`.

Several older README paragraphs still say the interpreter, graph, or bank formats are absent. Prefer the current source, detailed dated evidence, and measured results above. Update those status notes alongside implementation milestones; do not treat old prose as a reason to redo completed ports.

## Prerequisites and module boundaries

**Private development inputs.** Use the owner's extraction at:

```text
C:\Users\andre\AppData\Local\Skate3RustEngine\owned-disc\partial-skate3\data\audio
```

The three relevant files are present: `audiofiles.big` (104,748,800 bytes), `wheels.big` (256,304 bytes), and `grains.big` (3,446,976 bytes). The ISO remains `D:\skate3.iso`. The tested FFmpeg is in the original checkout's ignored `.local/ffmpeg/ffmpeg-9.0.1-essentials_build/bin/ffmpeg.exe`; an isolated worktree does not inherit that ignored installation. Set `SKATE_FFMPEG` explicitly there.

Obtain the owner's matching `g_8200.bin` … `g_8320.bin` image regions for development. Validate guest addresses, image identity, expected class tables, and required spans before initialization. Reset mutable globals and initialize the evaluator period as the game does; a boot dump's contents are not a running audio system. This image must match the executable/title-update version used by the research, which is TU3; a disc `default.xex` alone must not be presumed equivalent.

Use `%LOCALAPPDATA%\Skate3RustEngine\target` for normal Windows development builds, following the checkpoint's successful linker setup. This planning task used a separate `%TEMP%\skate-audio-plan-51bd` target for small read-only validation builds, also outside OneDrive. Establish a bootable asset-configured `skate3rust.exe` before the first in-game playback acceptance test.

**Proposed ownership:** keep the recovered guest layout and DSP separate from host decoding and gameplay. The names below are proposed files/API responsibilities, not features that exist today.

| Location | Responsibility |
|---|---|
| `skate-audio-formats` | Bounds-checked EB/ABK/CSI/EAAC parsing; explicit sample ranges, header sizes, stream types, loop metadata, wheel seek metadata, and eventually the understood grain-table fields. No output device or gameplay logic. |
| `skate-audio-core/src/commands.rs` | Address dispatcher and phase-four command drain; handler record lengths and explicit unsupported-handler errors. |
| `skate-audio-core/src/play.rs`, `pump.rs`, `pcm.rs` | Recovered resident play/slot transitions, host-backed stream pump, and immutable PCM `StreamFill` cursors. Host traits replace allocation, stream scheduling, and XMA services where needed. Preserve the distinction between 48-byte control slots, 80-byte records, and stream segment entries. |
| `skate-audio-core/src/device.rs` and a device adapter | Concrete `VoiceDevice`: open, setters, alternate pan setter, query, refresh/poke, suspend/resume, and release. A stopped recovered SndPlayer1 graph now returns its pool node, clears its segment state, frees its external buffer through `Heap`, and unlinks its player. Other module-specific teardown and a reclaiming long-session runtime heap remain required. Keep recovered device behavior here; host resources remain behind traits. |
| `skate-audio-core/src/runtime.rs`, `worker.rs`, existing scheduler/graph/bus modules | The recovered system control block now runs scheduler buckets, fade retirement, both deferred-list drains, command dispatch, and two node-pool retire passes in retail order. The Dac worker's two 6,144-byte buffer handoff is also ported. Still required: single-owner guest-state initialization, ordered player/bus passes, valid authored bus graphs, and a headless operation that produces real 256-frame native float blocks. |
| `skate-audio-core/src/player_events/` | Pure recovered message constructors and sound-object state machines, supplied typed observations. Encode exact words, clamps, flags, start/update/release behavior. No Bevy or file I/O. |
| `skate-data/src/audio/catalog.rs`, `pcm_cache.rs`, `pcm_stream.rs` | Resolve guest sample references back to owned archive/bank/slot ranges; decode/cache immutable PCM off the render thread; supply `StreamFill`; load private image regions and validate identity. `skate-data` already depends on both audio crates. |
| `skate-game/src/physics/audio_observation.rs` | Publishes one `PlayerAudioObservation` message per completed physics tick from the authoritative output snapshot. It carries ordered land/wipe/contact events, board and rider speed, state, contact count, and the raw wheel-surface vote. That vote is deliberately not presented as an audio-material mapping. Animation timing, grind-specific fields, and patch-message construction still remain. |
| `skate-game/src/player_audio.rs` and `skate_audio.rs` | Bevy integration, local-player lifecycle, bounded input/output queues, listener snapshot, one continuous mixer source, output-device handling, opt-in trace replay. Add a direct core dependency to `skate-game` only if these APIs require it. |
| sk8Audio `probe/trace/` and examples | Trace normalization, controlled input replay, command/slot/mix probes, comparisons, and evidence reports. Keep proprietary capture payloads external. |
| Owned-game setup tools | Private archive discovery and versioned image extraction; shipping must not depend on a developer's image dump or absolute paths. |

Develop the audio crates in sk8Audio and vendor them using its established sync workflow. Diff both trees before every sync; `sync_rust_engine.sh push` uses deletion semantics and must not overwrite unrelated engine changes. Engine host/gameplay glue belongs in this repository. No sync or source edits were performed for this plan.

**Threading and time.** One audio worker owns `Guest`, the evaluator, command ring, voice objects, scheduler, and DSP state. Gameplay sends immutable, timestamped observations; decode workers return immutable PCM. Never share the guest byte buffer concurrently with a render callback. This also avoids importing the original publish-offset-before-record race into host threading. The queue connecting threads must publish completed messages with proper synchronization; the core can retain original single-thread store semantics for comparison.

Set `SKATE_AUDIO_OBSERVE=1` during a playtest to log each meaningful local-player observation: landing/contact/wipe events and wheel-surface transitions with tick, state, contact count, and board/rider speeds. This is a capture aid only; the reported `wheel_surface` remains the raw physics vote until the retail audio-material selection is proven.

Render at 48 kHz in 256-frame blocks: 187.5 blocks/second, approximately 5.333 ms per block. Call the evaluator tick with that delta and let its own countdown govern evaluation. The period denominator is initialized to 30.0; do not replace this with a guessed Bevy 30 Hz system. Gameplay currently has a 60 Hz normal simulation cadence and can change its wall-clock cadence. Preserve separate simulation time, audio sample time, and presentation/replay time.

**Decoding.** For the first milestone, predecode the footstep bank before accepting the trace. Later, cache by archive identity, bank/member, sample slot/range, format, and decoder version; pin buffers referenced by live voices. Use the existing complete-chain FFmpeg path with packet padding and continuous codec history. Never start a new FFmpeg process per event or per 256-frame block. Measure memory and startup time before deciding how many of the 1,073 samples in the initial target banks to preload; account for channels as well as frames and add any proven shared dependencies. Keep decode failures sticky and attributable until the asset/configuration changes.

The Bevy callback must only read ready frames, adapt the final channel format, and provide defined silence on underflow. It must not spawn FFmpeg, read archives, block on work, or allocate a fresh graph. Keep its source alive during a transient underrun; signal permanent closure explicitly. Record underruns, late events, decode failures, live voices, command usage, and render duration without logging every sample.

## Milestones and acceptance gates

### M0. Reproducible inputs and baseline

The tests and archive checks above are complete for this plan. The locally built runtime launch applied TU3 before its image dump; its `default.xexp` and WebKit payload hashes match the recompilation's pinned values, and its evaluator table agrees at the required player-opcode slots. The split private image is sufficient for the current fixtures, but it is a pre-boot static image: mutable globals still need initialization by the runtime. Store a private manifest of archive hashes, image/version identity, tool versions, build profile, and test invocations.

The existing footstep example with `GRAPH=1` reproduced the 70-block baseline: seven opens at frame 59 and one at 65. Its complete 3,000-frame update sequence also completed 278,000 operations with the instance live and no reported evaluator error. It still exposes the expected logging limitation: the example truncates after the first 80 device lines. Capture structured output without that truncation before treating the sequence as evidence beyond graph construction, and treat printed `stopped` errors as failures even when the process exits successfully.

**Exit:** the expected baseline is either reproduced or every discrepancy is explained and recorded before implementation proceeds. The baseline is now reproduced; full-sequence logging and concrete playback remain separate M1 work.

### M1. One footstep audible in `skate3rust.exe`

Build this as one vertical integration, with small reviewable steps:

1. **Initialize a real runtime.** Load image spans, install the nine shipped CSI projects in the observed order, install the footstep bank, bind the object, allocate system/player/descriptor buffers, register classes, establish real bus storage, and initialize mutable globals. Keep archive bytes and relocated bank memory alive for all references. Replace the example's fake buses with the minimum valid buses actually used by this trace. Any bypass of an effects bus is an explicit first-milestone limitation and cannot satisfy mix parity.
2. **Apply voice properties.** **Partially implemented and synthetic-tested:** `device` now reconstructs setters `sub_824A29A8` and `sub_824A2C58`, query `sub_824A2DD8`, and suspend/resume `sub_82B1E8D8`/`sub_824A2E98`. It queues the resampling, gain, send, filter, panner, and conditional effect-send updates with the game’s scales and preserves the ignored property IDs. The existing `voice::set_property` only clamps and forwards; a concrete `VoiceDevice` must invoke these routines for real graph handles, then add graph-resource lifetime callbacks. Property 9 must not be guessed to be pitch; the documented setter ignores it.
3. **Drain commands.** **Implemented and synthetic-tested:** phase four of `sub_82B48530` dispatches install `0x82B49210`, player stop `0x82B49238`, player float `0x82B49268`, slot stamp `0x82B463A8`, send link `0x82B31680`, fader, and play `0x82B32DC8`. `PcmCommandHost` stops resident voices by removing them from the ordered scheduler or each recovered fallback list, clearing the constructed SndPlayer1 graph, returning its pool node, freeing its external buffer and player through `Heap`, then releasing the private decoded cursor. Teardown for other module/source kinds and a reclaiming long-session guest heap remain to be implemented. The drain validates snapshot bounds and nonzero returned sizes, preserves reentrant records for the next drain, refreshes high water, and increments the drain count. Named and secondary sends remain explicit unsupported handlers until their runtime contract is implemented.
4. **Play the resident sample.** **Partially implemented and synthetic-tested:** the short path of `sub_82B32DC8` covers pending-count decrement, ring claim at `+467`, admission/refusal behavior, record seeding, and index wrapping. It returns the request's actual record size at `+44`. Source kinds 1/2 are explicit errors. Header parsing, cache lookup, and source attachment still belong to the concrete resident host.
5. **Supply PCM through the pump and `StreamFill`.** **Partially implemented and synthetic-tested:** decoded immutable PCM can fill planar guest descriptors with EOF and loop-range safety, and the recovered `sub_82B31EE0` pump handles retirement, timing publication, slot states, and source-host dispatch. `PcmPlayHost` now binds a cache entry keyed by the game’s sample address to a private SndPlayer1 cursor, initializes the resident slot rate/queue fields, and serves its PCM to the renderer. The game runtime must populate that cache from owned archives and drive `sndplayer::render_block` through the existing stream code. Test an owned footstep after the ramp/impulse fixture. Do not mark a slot ready merely to bypass the pump.
6. **Run the whole block in the recovered order.** **Partially implemented and synthetic-tested:** `sub_82B48530` now runs its scheduler buckets, fade/retirement pass, both deferred-list passes, command-ring drain, timing stores, and both `sub_82B39820` pool-retirement passes through `runtime::run_frame`; `sub_82B39820` is ported through the existing heap boundary. `sub_82B219E8` snapshots the Dac’s clock/voice array/count, calls an explicit host sink submission boundary, mirrors submitted frames, and advances the double-precision clock. `worker::pump_once` preserves the ready/handed state of the two 6,144-byte PCM buffers and clears a silent block before submitting it. Still connect `sub_82B21520`, actual Dac graph passes, and the worker sink to a real runtime. Establish which bucket owns the evaluator and pump before choosing order. Preserve ordering by player `+73` and process buses after their contributors as the recovered graph requires.
7. **Bridge the output to Bevy.** A registered `LivePcm` source now supplies the required continuous float endpoint: an empty block produces silence without ending its decoder, and a bounded queue discards stale complete frames. It is tested in the game target, but no graph renderer pushes into it yet. First expose that headless block renderer to the diagnostic, then reuse the same runtime/source from the actual game's existing `SkateAudioPlugin` installation. `SKATE_AUDIO_BANK_SAMPLE=<archive>|<bank.abk>|<sample>` now auditions one owned inline bank sample through the existing Bevy mixer and honors its authored loop start; it is a bounded decode/audition aid, not a graph or gameplay path. Add an opt-in trace-startup hook separately. Retain a native six-channel capture point, and implement an explicit final stereo adapter if needed by Bevy's device path. The existing `StreamPcm` remains a finite `i16` buffer source unless its EAAC header supplies an inline loop.

The stream/PCM contract must be written down before step 5 is implemented:

| Structure | Required ownership and invariants |
|---|---|
| SndPlayer1 control ring | 48-byte slots; state at `+46`, format/channels at `+47`, rate at `+16`, total/cursor/loop fields as parsed. Write, retire, and render indices are distinct. |
| Associated source record | 80 bytes through object `+96`; header codec at `+72`, stream type at `+73`, header/version state at `+76`, source range and bookkeeping. Do not conflate these bytes with control-slot state. |
| Pump/consumer handshake | Pump selector is object `+473`; renderer uses `+474`. Each indexes a 16-byte area with readiness at `+113`. Trace producer/consumer advancement and retirement; setting one fixed byte is insufficient. |
| Stream consumed by `deliver_frames` | `+28` active cursor; `+36` segment-table offset; `+40` scratch offset; `+44` pending frames; `+46` channels; `+49` active segment; `+51` scratch mode. Each 24-byte segment entry has its total at `+12`. |
| Buffer descriptor | `+4` sample-data address, `+12` produced frames, `+14` planar stride. Write guest-order float samples in the expected planar layout, starting from decoded interleaved PCM. |
| Cursor ownership | The host PCM cursor and guest segment cursor must move together. `deliver_frames` owns its guest advancement; do not advance it again in `fill`. `stream_remaining` must read totals/cursors that agree with available PCM. |
| Short fill | The scratch path accounts for actual production. The no-scratch path credits the requested size even if `fill` returns less. Only use that path when the announced frames are all ready, or change the host contract explicitly and test the difference. |

**Exit:** on a working install, the trace causes an audible footstep in `skate3rust.exe` through post → evaluator → concrete voice device → commands → pump → SndPlayer1 → filters/gains/panner/sends → output. Save a private native block capture and output-device recording with the log and exact build identity. Require nonzero finite PCM, correct sample source, expected layers, no unknown handler/opcode, no underrun during the prepared test, and controlled completion. Include a silent control and waveform alignment/correlation; an `AudioSink` being alive alone is insufficient. This gate proves playback, not full parity.

### M2. Resident loops and reusable voice lifetime

Implement looping for **stream type 0 with the loop flag set**, including loop start, end, repeated cursor wrapping, resampler/filter continuity, and release tails. Test nonzero loop starts and crossing a loop several times within requested source frames. Do not implement it by applying `PlaybackSettings::LOOP` to an entire premixed output buffer.

Use `GRINDS.abk` first, then looping truck rattle, rolling surface, body-slide, speed, and Treatments samples. A sound-object lifetime is independent of whether a selected sample has the loop flag: all 30 `board_scrapes` samples are non-looping, for example, although the object can produce continuing scraping through its patch logic.

Finish release `sub_82B1E458`, its deferred `0x82B49238` command, required SndPlayer1 configure IDs 1–4, and all normal/failure exits. The stop command already performs the recoverable `sub_82B48F28` teardown for graphs constructed here: it clears SndPlayer1 segments, returns its pool node, frees its external buffer and player, and removes scheduler links. Separate message lifetime, patch-instance lifetime, individual voice lifetime, and cached-sample lifetime. Implement completion queries from real state; use `FreeListHeap` rather than `BumpHeap` for long-running sessions. An allocation failure must unwind partial registration and references.

**Exit:** repeated start/update/stop cycles and a long grind have bounded voice/instance/heap counts; no looping seams caused by restarts, clipped tails, stuck sends, stale handles, or double frees. Natural completion, explicit stop, pause/resume, failure, bail, and teleport work with several overlapping voices. The synthetic repeat tests must exercise ring wrapping and complete retirement, not only a successful first play.

### M3. Complete the player-bank program and routing dependencies

**Partial progress:** slots 5 and 39 now run through `PatchHost`. Slot 5 covers range clamping, stale broadcast-handle handling, and the known `ON_BROADCAST` subscriber callback. Slot 39 latches its raw requested value, clamps only the notified setpoint, and uses the same generation-checked subscriber path. Slot 20, present in the rolling-surface and rocks-bounce programs, now has a pure signed counted-maximum port; its matching minimum slot 19 is also ported. Synthetic tests cover these paths. Arbitrary handler-list callback targets remain explicit until trace/image evidence identifies their contracts. Cover `patch::subscribe`, broadcasts, releases, and utility initialization across bank boundaries. Preserve the same project-first/name-fallback binding behavior that the real archive requires.

The representative `Class_rolling` replay now crosses slot 5, runs 308 authored operations and opens the two selected resident voices. On its later walk it reaches another slot-5 broadcaster whose symbol field is still the archive sentinel `1`, even with all 17 target banks installed. That is evidence that loading banks alone does not initialize the required broadcast utility. Port the owning utility constructor and prove the resulting handle before enabling rolling in gameplay; treating `1` as a pointer or skipping the broadcast would hide the missing lifetime owner.

Install the 17 current target banks and their actually referenced utilities/exports, including any shared dependency needed for ordinary body/board impacts. A `Class_rolling` post can fan out to several bank inputs; reducing it to one guessed sample bank changes the program. The cloth utility is an initialized held object, not one newly spawned per trick. Preserve repeated export records and listener order.

Replace provisional bus setup with the required `sub_824916E8` behavior and authored defaults for all player routes. The recovered stamping tail now posts its two three-value groups to children 2 and 3 and its output value to child 1, while the exact modulo-11 single-precision interpolation is available as `interpolate_bus_property`. `Sub0`, `DCl0`, and `PI20` now have their recovered builder sizes/constructors and graph-kernel dispatch; `Sub0` also owns its recovered accumulator allocation, deferred install command, and intrusive registration. The manager constructor now builds its eight real `Sub0 → DCl0 → PI20 → PI20 → Send` players, records their module tables and lazy-profile flags, and configures every `Send` with the recovered default bus. A focused graph test covers the exact layout and command sequence. Resource-profile lookup remains the host boundary before first use. Cover ordinary sends, effect routes identified by records 4096/8192/16384, and all three recorded graph shapes, including `GainFader`. Unsupported additional classes/handlers remain visible gaps. Do not expand to all music or ambience DSP simply because those functions exist in the corpus.

The final output owner is a distinct constructor (`sub_82490270`): it creates two seven-node order-150 graphs from `Sub0`, `Del0`, `PI20`, `Sen0`, `Gai0`, and `Pn21`, then publishes their module arrays to the manager’s output fields. The two graph layouts, both send routes, `Del0`'s 192-byte constructor, history allocation/resize, parameter-refresh boundary, and stable 256-frame 15 ms ring-delay path are now ported and regression-tested. `Del0`'s source transition callbacks (`sub_82B3D4F8` / `sub_82B3D578`) remain required for exact live delay-property fades.

**Exit:** complete traced post/update/release sequences for every target input run without an unimplemented opcode, callback, class, bus, or property path. Report execution coverage by bank input/program, not just a global opcode count. Validate each observed graph's configuration and routes, not just its list of class names.

### M4. Connect authoritative gameplay, one sound family at a time

Create a timestamped `PlayerAudioFrame` containing the local player/board identity, simulation tick, ordered event edges, continuing controls, and listener state. Keep a registry of held object handles; create, redeliver, and release at the recovered times. Preserve transient events when multiple simulation ticks run before an audio block. Never coalesce a contact edge or a required per-frame update simply because its values match the preceding packet; permit coalescing only after the object's semantics prove it harmless.

Use `physics/frame.rs` and `SimulationExchange` as the integration boundary. `PhysicalOutputSnapshot` supplies useful state/velocity/landing/bail observations, but its `PhysicsEvent` variants lack material, impulse, foot identity, and slip detail. Publish the additional authoritative values from the solver/animation owners rather than trying to reconstruct them from rendering or input buttons.

Capture `AudibleFootStepStrength` before `finish_output_publication` resets it to zero. Preserve animation `push_contact`/`brake_contact` flags and bone/phase data. Preserve pre-impact velocity, contact identity and surface, wheel/truck/deck contacts, lateral motion, and grind state before their owners reset them. Review the early climbing return and other paths that bypass normal publication so audio still gets correct stop/reset behavior.

For each row below, the constructor/caller mapping and post count are recorded evidence. The proposed engine sources are candidates: their exact argument mapping must be derived from that caller and tested. Post counts are not numbers of audible effects.

| Order / sound | Object → bank; posts across eight traces | Original constructor ← caller | Engine observations to connect and verify |
|---|---|---|---|
| 1. Feet | `playercharacter_footstep` → `fstep_skateshoe1_sm` (192 samples); 100 | `824B73E0` ← `824E9FD8` | Animation footstep strength, contact bone/phase, foot landing and surface, speed, board possession. Caller retains two messages; do not assume one new message per audible step or assign left/right until verified. |
| 2. Braking foot | `Class_foot_drag` → `FOOT_DRAG` (168); 33 | `824AF498` ← `824BB540` | Brake/push contact phase, foot speed, material, mount/dismount and braking state. |
| 3. Pop / flips | `Class_Flips` → `Sk8_Air_Flip_Tricks` (33); 87 | `824AFAD8` ← `824CBFB8` | Actual launch/pop, flip/shuv/spin state and timing from air/animation state; not the press of the jump button. |
| 3. Trick clothes | `cloth_trick` → `Foley_Cloth` (28 shared); 133 | `824B71C0` ← `824CC590`, `824CC680` | Ollie, kickflip, shuv and spin phase; cloth counters and utility dependencies. |
| 3. Cloth utility | `c_foley_utility` → `Foley_Cloth`; 8 | `82488120` ← `82487A60` | Player initialization and lifetime, subscriptions/broadcasts. Must be initialized before dependent cloth playback. |
| 4. Landings / impacts | `Class_Treatment` → `Treatments` (18); 8 | `824B0080` ← `824DD408` | Held treatment message; actual impact/landing severity, surface/body/board state, continuing parameters. `Landing` alone is insufficient. Include the four-channel sample. |
| 5. Rolling | `Class_rolling` → `PatchBank_Rolling_Surfaces`, `PatchBank_Objects`, `PatchBank_SpiderCracks`, `PatchBank_RocksBounce` (16/18/18/26); 17 | `824C4C18` ← `824C5CA8`, `824C9830`, `824C9F68` | Ground-contact speed, wheel-surface vote, roughness and held-message updates. Preserve bank fan-out. |
| 5. Seams | `Class_Seams` → `Seams_Bank` (234); 200 | `824AFDD0` ← `824C13D0` | Individual wheel/contact transitions and surface/crack evidence; do not fire once each rendered frame. |
| 5. Truck rattle | `Rolling_Rattle_Class` → `Rolling_Rattles` (18); 173 | `824B0248` ← `824C6198` | Native contact/velocity controls and rattle's continuing program; identify actual thresholds in this large caller. |
| 6. Skids | `Class_wheels_skid` → `WHEEL_SKID_BANK` (96); 343 | `824AF678` ← `824C7438` | Wheel slip, revert/stop state, pop-associated skid, contact material and intensity. |
| 6. Powerslides | `Class_Squeaks` → `Brd_Squeaks` (18); 106 | `824AFF48` ← `824C7738` | Sustained lateral contact and turn/revert state. Powerslide attribution is user-confirmed in the recorded sessions. |
| 7. Grinds | `Class_grind` → `GRINDS` (123); 52 | `824AF8C8` ← `824C28B0`, `824C39E0` | Retained grind family/substate, entry/exit, speed, variant, impact, and authored audio surface. The updater manages more than one held grind message. |
| 7. Board scrape | `c_board_slide` → `board_scrapes` (30); 23 | `824B0670` ← `824CB3C8` | Deck scrape/slide contacts and speed; establish relation to grind families and loose-board contacts. |
| 8. Bail clothes | `c_cloth_falls` → `Foley_Cloth`; 20 | `824B72D8` ← `824DBF10` | Wipeout entry/recovery and body motion; preserve cloth dependencies and timed updates. |
| 8. Body slide | `c_body_slide` → `Bodyslide` (20); 67 | `824B7070` ← `824DC0E8` | Ragdoll/body-part contacts, sliding speed, surface, impact and recovery. |
| 9. Speed wind | `SenseOfSpeed_wind` → `sense_of_speed` (17 shared); 302 | `824B0388` ← `824E7980` | Actual native speed and state controls, including rate/pitch/filter behavior. |
| 9. Speed rattle | `SenseOfSpeed_rattle` → `sense_of_speed`; 139 | `824B0520` ← `824E7980` | Same caller, separately recovered controls; do not assume it duplicates truck rattle. |

For each family, first port the constructor's payload layout and clamping as pure functions, then map the original caller's observations, then add the engine adapter, then compare traces. Unknown payload words retain neutral names and explicit provenance; do not label them by sample-name guesswork.

Available useful source boundaries include `riding_outputs`, `ground_runtime/surface`, `BoardRuntime::contact_reports`, `grind/observation`, `grind/output`, `air_phase`, `known_air`, `landing_quality`, `wipeout_states`, and offboard board-possession code. Grind already publishes a distinct `audio_surface_1468`/`audio_surface_216`. Wheel surface IDs and friction categories must not be assumed to share those audio IDs. Custom maps need a documented mapping/default policy after the stock path is established.

**Impact and variation coverage for both person and board.** Build a coverage inventory from the reachable patch programs and original callers, separating body contacts from deck/truck/wheel and loose-board contacts. Record each event's material and severity inputs, contact/body-part distinctions where used, sample groups, layers, random choices, pitch/gain/filter changes, and lifetime. The existing traces are a starting corpus; their few treatment posts cannot establish that all impact variations were exercised. Probe controlled light/hard landings, body hits during bails, repeated ragdoll contacts, board strikes/bounces, and slides on the supported materials. Identify which distinctions actually affect the original selection before naming or inventing parameters.

Exercise every reachable authored selection branch with controlled inputs/random state, then verify normal gameplay selection and distributions. A representative clip per family does not satisfy full variation coverage. Keep ordinary impact branches of `824DD408` and any shared assets they need; the dedicated `hall_of_meat_slo_mo` object and its mode/timer gate are outside acceptance. Retain its historical evidence in the handoff for future work without making mode implementation a prerequisite.

**Exit per family:** the same controlled move gives the expected object posts, complete payloads, redeliveries, releases, sample groups/layers, controls, and timing within the stated comparison method. Audition standing/walking/pushing, normal/fakie/nollie pops, flips/shuvs/spins, clean/hard landings, several ground materials, seam crossings, reverts/stops/powerslides, rail/ledge grinds, board/body slides, body impacts, loose-board strikes/bounces, bails/recovery, and high speed. Include inactive/negative cases and evidence for all reachable authored variations for the person and board. Do not call all player sounds complete while any target family is reachable only through the debug replay.

### M5. Resolve separate wheel, grain, and loose-board playback

These are required coverage investigations, not optional polish. Their probes can be prepared while the earlier runtime work proceeds, but they require the Linux recomp/harness environment described by sk8Audio. No new probe session was run for this plan.

| Path | Existing evidence | Required experiment and resulting work |
|---|---|---|
| Wheel spin | `SFXObj_Wheels` constructor `824CD6F8`; `wheels.big|name`; opens through `828DC158` / `8298ED88`; two approximately 14.8-second loops plus seek companions. | Log owner/stream identity, path, exact header/seek metadata, start/update/stop, speed/pitch/gain and sample clock. Isolate grounded rolling, manual, airborne wheel spin, landing, bail and remount. Implement the actual source-kind/seek path the probe observes. |
| Surface grains | `SFXObj_Moving` `824E48D0`, `824E7658` for `x_jet_rolling`; loader `828DCCF8`; 14 prefixed EAAC streams at 44.1/48 kHz. | Probe grain selection, offsets, overlap/envelopes, rate, random draws, surface switches and stop behavior at controlled speeds. Decode the prefix table and playback algorithm from its reader. A whole-file repeating loop cannot stand in for granular rolling. |
| Loose board | `c_dynamic_rolling_objects` / `c_dynamic_sliding_objects` appear in player sessions, but their emitter ownership was not captured. | Separate a thrown board from nearby props. Log the emitting object's identity and collision/contact events; follow it through bail, impacts, rolling, sliding, stopping and pickup. Add the proven board-owned route, or record a controlled exclusion with supporting evidence. |

When a source is actually type 1/2 or otherwise needs it, implement the long play-command path: `sub_82B47A68` voice acquisition/release callbacks, name lifetime, and scheduling services corresponding to `sub_82EC0E08`. Existing C++ bodies are gate-labelled, and the latter has no existing audio port. Use host scheduling primitives with explicit contracts instead of pretending the Xbox XMA/locking services are portable. Test failure and cancellation as well as a successful allocation.

The current generic `PlayStream` decoder probes a header at the member start or treats it as a bare block chain. `.grain` needs its outer header offset handled explicitly; the format probe proves that prefix exists but is not a production grain player.

**Exit:** all three investigations have an evidence-backed conclusion about player scope. Every in-scope path has playback, live controls and lifecycle tests. An inconclusive probe or inaccessible Linux session remains an open completeness item.

### M6. Mix, timing, lifecycle, and performance acceptance

Validate the native bus mix before any desktop adaptation. `interleave.rs` maps input planes to frame slots **[0, 2, 1, 4, 5, 3]**; six channels cannot be blindly interleaved as a conventional device order. Verify speaker meaning using per-channel impulses and the recovered layout, then test the explicit stereo/downmix policy. Do not apply Bevy spatialization or playback pitch again to an already panned/resampled graph.

Exercise the four-channel treatment through rechanneling, effects sends, master gains, panning, clipping and output ramps; test 22.05/36/44.1/48 kHz input where present. Validate the listener transform and distance/angle behavior from the original callers, including camera movement. Keep simulation and audio clocks explicit when pause/resume or an existing playback mode changes time.

Handle player destruction, board release/return, bail/recovery, restart, respawn, teleport, map transition, pause/resume, focus changes, device loss, and failed decoding. Use generation-tagged player/voice handles so an old decode completion cannot play after a map switch. Audio failures should report a named cause and keep gameplay running; do not silently replace the bank logic with a random sample.

Replay currently stores presentation poses, not sound events. During replay, prevent live simulation audio from leaking into the replay and avoid inventing new contact sounds from interpolated poses. If replay sound reproduction is included in the finished player experience, record/replay the audio event timeline and required random/state snapshots; test pause, scrub, seek and rate changes without repeated stale transients. Treat that as explicit additional playback-state work, not a property of the current pose buffer.

Profile cold load separately from steady-state rendering. A 256-frame block has a 5.333 ms deadline. Set the voice/cache/queue limits from measured worst-case player scenes and keep clear headroom below that deadline; record p50/p95/p99/max, underruns, command high-water mark and live allocations. Run at least a 30-minute varied skate/bail/grind session and repeated transitions. Do not substitute a guessed low voice cap that drops valid layers; any overload policy must be documented and tested.

**Exit:** representative live sessions have no steady-state underruns, leaking or stuck voices, repeated decode work, non-finite samples, incorrect speaker routing, or duplicate event playback. Functional differences caused by intentionally changed host scheduling or budgeting are listed rather than included in an exactness claim. The original `sub_82B3C2D0` uses timebase measurements for budget/retirement decisions, so replacing every profiling value with zero is not automatically behavior-neutral.

### M7. Owner-supplied setup and release readiness

Implement private setup extraction of the required guest regions from the owner's executable and matching update: XEX2 decrypt/decompress and version validation, as needed by the selected image. Ship the extractor and definitions, never the extracted regions. If setup cannot create a supported image, fail with a specific prerequisite message instead of shipping a developer dump or guessed constants.

Archive/sample/image caches live under private local application data, keyed by content/version; invalidate them deliberately. Cover installed-path FFmpeg discovery, missing tools/assets, partial extraction, Unicode/spaces/drive-letter paths, a fresh install, and cache rebuilding. Preserve the existing diagnostic hooks. `SKATE_AUDIO_PLAY` still needs all five colon-separated fields on Windows; `SKATE_AUDIO_BANK_SAMPLE` uses `|` delimiters so its archive argument preserves drive letters. Neither hook establishes a gameplay selection path.

Audit packaging inputs, build scripts and CI artifacts so neither the ISO/archives, image dump, PCM cache nor private audio captures can be swept into a release. Update per-module verification notes and measured test counts. Test the package with owner-provided data on a clean setup and with no game data at all.

**Exit:** all live target families and the resolved wheel/grain/loose-board coverage pass, the owner can recreate the private runtime inputs from their game, the shipped package contains no proprietary assets, and remaining parity limits are explicit.

## Verification strategy and reproducible checks

Use four distinct claims instead of one broad green status:

| Claim | Evidence required |
|---|---|
| Container/decoder works | Every targeted sample resolves within its archive; metadata and decoded frame counts agree; compare PCM bytes with the independent sk8Audio decoder reference for the exercised formats. Existing sample-exact history does not replace validation of new loop/seek adapters. |
| Event semantics match | Same initial state and ordered input observations produce the same object, payload words, updates/releases, layers and effective parameters. Compare block/sample timestamps and state transitions, not only totals. |
| Runtime/DSP matches | Replay recorded input windows and compare changed guest bytes/registers where applicable, then compare staged source/filter/gain/send/native-output buffers. Record failures, skipped calls and unexercised paths separately. |
| It is audible and usable | Run the actual game, record the host output privately, align it with the pre-device capture, audition transitions, and measure latency/underruns. Device downmix/resampling is a separate comparison boundary. |

Create a structured diagnostic record for each stage: session/build identity, simulation tick, audio block/sample clock, object + message generation, instance/program, archive/bank/sample slot, random-counter snapshot or draw sequence, descriptor/properties, command handler/size, slot transitions, and output frame counts. The current `sound_report.py` is useful for coverage but groups opens by the second EAAC header word and intentionally reports multiple candidate banks. For strict attribution use the descriptor sample index plus headers and owner/instance identity; temporal adjacency is not proof. The recorded footstep/Seams misattribution is a concrete reason for this requirement.

Randomness comes from the shared six-word counter at `0x830775F0`. Record the state before a controlled trigger and reproduce its consumption order. A player-only runtime omits other game patches that consume this global generator, so one startup seed does not ensure whole-session variant equality. Use an isolated deterministic scene or recorded draw/state inputs for exact comparisons. For uncontrolled live sessions, compare the selection logic and distribution; do not demand the same random variant on each move or use randomness to excuse an unexplained timing mismatch.

Two recomp runs are not presently bit-identical. Bit-exact whole-output acceptance therefore requires a controlled capture/replay with the same input state, random stream and timing. Until that is available, retain the narrower per-function, event, staged-buffer and audible evidence. The footstep fixture's approximate six-block update schedule must not be advertised as sample-exact retail timing.

Targeted tests to add with implementation:

- Command batches containing every required handler, variable-length play records, exact ring end, malformed size, unknown address, overflow and reentrant/deferred work; verify state and no loss of pending work.
- PCM ramps/impulses for partial scratch fills, exact EOF, source starvation, multiple segments, 48/80-byte ring wrap, looping/nonzero loop start, rate changes and multichannel planar strides. Detect duplicate cursor advancement and dropped/duplicated frames.
- Real short sounds, long grinds, layered footsteps, the four-channel impact, lower-rate speed/footstep sounds, and each wheel/grain format, all from external owner paths.
- Property clamp boundaries and effective device writes, master/send gain composition, conditional effects sends, pan conversion, suspend/resume/query/release, mid-pass self-removal, and allocation/decode failures.
- Constructor boundary vectors and traced payloads; held-message fan-out, cloth utility broadcasts, no double posting, all material families, every reachable person/board impact selection branch and authored variation, event order when ticks bunch up, and changes in simulation cadence.
- Layered pop → flight → landing, rolling → skid → stop, grind entry → sustained contact → release, and bail → loose board/body slide → remount; include silent controls and no-contact cases.
- Teleport/map changes during decoding and active loops, replay scrubbing, repeated player recreation, device recovery, and the long-session performance gate.

Tests should assert externally meaningful invariants or compare an independent reference. Do not count a test that merely repeats the implementation's own arithmetic as parity evidence. Recorded Rust replay vectors are not included in the fetched trace corpus; obtain or regenerate the private vector inputs when those comparisons are needed. Windows unit-test success alone does not prove Linux-to-Windows numerical equality for libm, NaNs or floating-point modes.

The following commands are available **now**. They are verification commands, not proposed new player-playback CLI flags:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'Skate3RustEngine\target'
$audioRoot = Join-Path $env:LOCALAPPDATA 'Skate3RustEngine\owned-disc\partial-skate3\data\audio'
$researchRoot = Join-Path (Get-Location) '.local\references\skate3-audio'
$env:SKATE_FFMPEG = 'C:\Users\andre\OneDrive\Documents\ChatGPT\Sk8EngineAudio\.local\ffmpeg\ffmpeg-9.0.1-essentials_build\bin\ffmpeg.exe'

cargo test --release -p skate-audio-core -p skate-audio-formats --locked --offline
cargo test -p skate-game --bin skate3-audio-check --locked --offline
cargo run --release -p skate-audio-formats --example verify_banks -- "$audioRoot\audiofiles.big"
cargo run --release -p skate-audio-formats --example verify_bank_fixups -- "$audioRoot\audiofiles.big"
cargo run --release -p skate-audio-formats --example verify_patch_programs -- "$audioRoot\audiofiles.big" Foley_Cloth Seams_Bank Rolling_Rattles PatchBank_Rolling_Surfaces PatchBank_RocksBounce
cargo run --release -p skate-audio-formats --example verify_loop_headers -- $audioRoot
cargo run --release -p skate-audio-core --example bind_banks -- "$audioRoot\audiofiles.big"

# OUTDIR enables decode in this existing example; keep it private.
cargo run --release -p skate-data --example bank_sound -- "$audioRoot\audiofiles.big" fstep_skateshoe1_sm "$env:LOCALAPPDATA\Skate3RustEngine\audio-validation"

cargo build --release -p skate-audio-formats --example find_samples --example list_exports
$env:SKATE_AUDIOFILES = "$audioRoot\audiofiles.big"
$env:SKATE_AUDIO_EXAMPLES = Join-Path $env:CARGO_TARGET_DIR 'release\examples'
python "$researchRoot\probe\trace\sound_report.py" "$researchRoot\probe\traces\sessions\play4.audio-trace.log.gz" --every 30
```

After supplying a validated private image directory, reproduce the existing graph fixture:

```powershell
$imageRoot = '<private matching guest-image directory>'
$env:PAYLOAD = '00000000 00000000 00001000 00000000 000061A8 00000000 00000000 00007FFF 00000000 00000000 00000000 00000000 00000001 00000001 00000000 00000000 00000001 00000001 00000000 00000000 00000000 00000000 00000000 00000000 0000000C'
$env:UPDATE_FILE = "$researchRoot\probe\traces\fstep_updates_40C02FD0.txt"
$env:UPDATE_EVERY = '6'
$env:FRAMES = '70'
$env:GRAPH = '1'
cargo run --release -p skate-audio-core --example grind_instance -- "$audioRoot\audiofiles.big" $imageRoot fstep_skateshoe1_sm.abk playercharacter_footstep

# Full fixture plus tail; still graph/logging only until M1 is implemented.
$env:FRAMES = '3000'
cargo run --release -p skate-audio-core --example grind_instance -- "$audioRoot\audiofiles.big" $imageRoot fstep_skateshoe1_sm.abk playercharacter_footstep
```

For these examples inspect the reported failure/stopped/unsupported counts as well as the process exit code. `verify_patch_programs` reports a global histogram even with focus arguments; use its final **focused banks** section to determine player dependencies. These details were checked while preparing the plan.

## Source map and completion decision

| Source | Why it matters |
|---|---|
| [Windows handoff and checkpoint](audio-player-sounds-handoff.md) | Starting scope, constructor/caller map, trace totals and established Windows output path. Corrections above narrow several claims. |
| [Core status table](../crates/skate-audio-core/README.md), [formats parser](../crates/skate-audio-formats/src/eaac.rs) | Per-module verification limits and the distinct stream-type/loop fields. |
| [Graph example](../crates/skate-audio-core/examples/grind_instance.rs) | Reproduction setup, fake buses, recovered property/lifecycle control entries, cadence and output limits. |
| [Voice](../crates/skate-audio-core/src/voice.rs), [device](../crates/skate-audio-core/src/device.rs), [patch host](../crates/skate-audio-core/src/patch.rs) | Existing trait seams, missing concrete device behavior and runtime opcode coverage. |
| [Stream delivery](../crates/skate-audio-core/src/stream.rs), [SndPlayer1](../crates/skate-audio-core/src/sndplayer.rs), [graph pass](../crates/skate-audio-core/src/graph.rs) | PCM handshake, partial-fill hazards, ring/descriptor layout and ordered execution. |
| [Current Bevy audio](../crates/skate-game/src/skate_audio.rs), [diagnostic](../crates/skate-game/src/audio_check_main.rs), [FFmpeg](../crates/skate-data/src/audio/ffmpeg.rs) | Proven stream/decoder path to reuse, with a new continuous graph source required. |
| [Physics transport](../crates/skate-core/src/physics/phase.rs), [tick coordinator](../crates/skate-game/src/physics/frame.rs), [animation input](../crates/skate-game/src/physics/animation_input.rs) | Authoritative event publication and transient-data reset boundary. |
| [Grind output](../crates/skate-game/src/physics/grind/output.rs), [surface vote](../crates/skate-game/src/physics/ground_runtime/surface.rs), [animation feedback](../crates/skate-game/src/physics/animation_phase.rs), [replay](../crates/skate-game/src/replay.rs) | Existing material/state observations, feedback publication, and presentation-only replay. |
| [sk8Audio audio-banks evidence](https://github.com/andrewnakas/skate3-audio/blob/d3d81ffd634a70299f504b39cccf0595fadd43af/docs/audio-banks.md) | Detailed footstep comparison, property mapping, shared impact caller and unprobed wheel/grain paths. |
| [sk8Audio Phase 6](https://github.com/andrewnakas/skate3-audio/blob/d3d81ffd634a70299f504b39cccf0595fadd43af/docs/PLAN.md), [working rules](https://github.com/andrewnakas/skate3-audio/blob/d3d81ffd634a70299f504b39cccf0595fadd43af/CLAUDE.md), [Linux environment](https://github.com/andrewnakas/skate3-audio/blob/d3d81ffd634a70299f504b39cccf0595fadd43af/docs/environment-linux.md) | Port provenance, exact arithmetic, host-specific restrictions and probe setup. Historical status paragraphs require the corrections above. |
| [C++ audio ports](https://github.com/andrewnakas/skate3-audio/tree/d3d81ffd634a70299f504b39cccf0595fadd43af/recomp/src/audio_ports), [session corpus](https://github.com/andrewnakas/skate3-audio/tree/d3d81ffd634a70299f504b39cccf0595fadd43af/probe/traces/sessions) | Gate-labelled drain/play/pump/worker references and all eight observational sessions. |

The practical sequence is **M0 → M1 audible traced footstep → M2 reusable resident playback → M3 complete player-bank dependencies → M4 live families**, with M5 research prepared early and required before the final M6/M7 acceptance. Do not place all resident loops behind the type-1/2 loader work. Do not leave device setters, real buses or basic retirement until after the first audible test.

The critical unresolved inputs are runtime initialization evidence beyond the static private image, the wheel/grain/loose-board and full impact-variation probes, and controlled timing/random-state evidence for exact mix comparisons. Player audio implementation is underway, but completion still requires evidence for every live target family and its reachable authored variations for the person and board, every in-scope external path, and a reproducible owner-supplied setup that ships no proprietary game content. Dedicated Hall of Meat sounds and mode logic are not a completion gate.
