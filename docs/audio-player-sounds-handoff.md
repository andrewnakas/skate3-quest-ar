# Player sounds in the Rust build: handoff (2026-09-15)

This is for a Claude session on Windows. The goal is to finish player-character sound in
`skate-3-rust-engine`, matching the game. It covers:
- where things stand;
- what the game was recorded doing;
- what to copy over, and how to build and check the work;
- what is left, in order.

**Scope, set by the user:** everything the player's skating makes heard. That means tricks,
landings, bails and Hall of Meat, plus the board's contact with the ground: rolling, seams, truck
rattle, wheel skids, powerslides, grinds and board slides. It also covers the skater's body and
clothes: footsteps, body slides, cloth foley and foot drag. Other skaters, pedestrians
("livingworld"), ambience and music are out of scope.

## Short answer: is it done?

**No. Player sounds are not audible in the Rust build yet.** What exists:
- **The game side is fully recorded:** every player sound object below has a real trace, and the
  traces are in git.
- **The Rust side reaches a built voice graph:** the chain from a posted event to that graph matches
  the game where it was checked.

What is missing:
- the last three audio-thread stages, so no frames come out;
- the engine's gameplay posting the events.

| stage | Rust | status |
|---|---|---|
| retail archives, banks, `.csi` symbols, EAAC headers | `skate-audio-formats` | validated on real data |
| XMA decode to PCM | `skate-data/src/audio` (the `ffmpeg` binary) | byte-exact |
| a stream played through Bevy | `skate-game/src/skate_audio.rs` (`SKATE_AUDIO_PLAY`) | confirmed at the sound card |
| bank install, symbol binding, message post, instance spawn | `skate-audio-core` `patch.rs`, `symbols.rs` | unverified new work; trace-confirmed binding |
| the patch evaluator and its opcodes | `eval/` | ported, most opcodes verified |
| the voice op (slot 27) and property routing | `voice.rs` | unverified |
| voice device open: class registration, graph build, sends, pan | `device.rs`, `classes.rs`, `modules.rs` | unverified; the module list matches the game |
| graph pass and every voice kernel | `graph.rs`, `kernels.rs`, `filters.rs`, `gains.rs`, `pitch.rs`, `mix.rs`, `bus.rs` | kernels verified; routing unverified |
| sound player render | `sndplayer.rs`, `stream.rs` | unverified |
| **command-ring drain** (`sub_82B48530`, phase 4) | — | **missing** |
| **play command** (`sub_82B32DC8`) | — | **missing** (C++ reference exists) |
| **stream pump** (`sub_82B31EE0`), where decoded PCM must enter | — | **missing** |
| per-block driver: drain, evaluator tick, graph passes, mix to output | — | **missing** |
| gameplay posting player events, with per-frame updates | — | **missing** |

## The target: every player sound object, and what the game did

The objects come from the game's object table at `0x8302D4A4` (72 entries; object *i*'s handle slot is
`0x8302EE28 + 8i`). For each one the table gives:
- the bank that plays it (`list_exports`);
- the message constructor that posts it (found by scanning for its handle slot);
- the gameplay function that calls the constructor;
- its posts across all eight recorded sessions.

The "sound" column reads the names, with two exceptions: the powerslides and Hall of Meat were
confirmed by the user playing them.

| sound | object | bank (samples) | constructor | gameplay caller (instructions) | posts, 8 sessions |
|---|---|---|---|---|---|
| flip tricks | `Class_Flips` | `Sk8_Air_Flip_Tricks.abk` (33) | `sub_824AFAD8` | `sub_824CBFB8` (373) | 87 |
| trick cloth foley: ollie, kickflip, shuv, spins | `cloth_trick` | `Foley_Cloth.abk` (28) | `sub_824B71C0` | `sub_824CC590` (59), `sub_824CC680` (86) | 133 |
| bail cloth | `c_cloth_falls` | `Foley_Cloth.abk` | `sub_824B72D8` | `sub_824DBF10` (118) | 20 |
| landings and impacts ("treatments") | `Class_Treatment` | `Treatments.abk` (18) | `sub_824B0080` | `sub_824DD408` (185) | 8 (one held message per session) |
| **powerslides** (board squeaks; user-confirmed) | `Class_Squeaks` | `Brd_Squeaks.abk` (18) | `sub_824AFF48` | `sub_824C7738` (185) | 106 |
| wheels over seams and cracks | `Class_Seams` | `Seams_Bank.abk` (234) | `sub_824AFDD0` | `sub_824C13D0` (62) | 200 |
| rolling surface | `Class_rolling` | `PatchBank_Rolling_Surfaces`, `_Objects`, `_SpiderCracks`, `_RocksBounce` | `sub_824C4C18` | `sub_824C5CA8` (315), `sub_824C9830` (70), `sub_824C9F68` (51) | 17 (held) |
| truck rattle | `Rolling_Rattle_Class` | `Rolling_Rattles.abk` (18) | `sub_824B0248` | `sub_824C6198` (568) | 173 |
| wheel skid (reverts, stops) | `Class_wheels_skid` | `WHEEL_SKID_BANK.abk` (96) | `sub_824AF678` | `sub_824C7438` (191) | 343 |
| grind | `Class_grind` | `GRINDS.abk` (123) | `sub_824AF8C8` | `sub_824C28B0` (276), `sub_824C39E0` (378) | 52 |
| board slide | `c_board_slide` | `board_scrapes.abk` (30) | `sub_824B0670` | `sub_824CB3C8` (62) | 23 |
| body slide | `c_body_slide` | `Bodyslide.abk` (20) | `sub_824B7070` | `sub_824DC0E8` (113) | 67 |
| footsteps and foot landings | `playercharacter_footstep` | `fstep_skateshoe1_sm.abk` (192) | `sub_824B73E0` | `sub_824E9FD8` (635) | 100 |
| foot drag (braking, stepping back on) | `Class_foot_drag` | `FOOT_DRAG.abk` (168) | `sub_824AF498` | `sub_824BB540` (229) | 33 |
| speed wind | `SenseOfSpeed_wind` | `sense_of_speed.abk` (17) | `sub_824B0388` | `sub_824E7980` (203) | 302 |
| speed rattle | `SenseOfSpeed_rattle` | `sense_of_speed.abk` | `sub_824B0520` | `sub_824E7980` | 139 |
| foley utility | `c_foley_utility` | `Foley_Cloth.abk` | `sub_82488120` | `sub_82487A60` (77) | 8 (once per boot) |
| **Hall of Meat** slow motion | `hall_of_meat_slo_mo` | `hom_slo_mo.abk` (14) | `sub_824AF368` | `sub_824DD408` | 11 (Hall of Meat mode only) |

**What the traces show:**
- **Pop.** Every ollie or flip that popped posted `Class_Flips`, `cloth_trick` and a wheel skid.
- **Landing.** Each popped trick opened a `Foley_Cloth` voice, a flip-bank voice and a `Treatments`
  voice, plus a squeak or impact voice on landing.
- **Held messages.** `Class_Treatment`, `Class_rolling` and `c_foley_utility` are posted once and
  held, and they sound through per-frame re-deliveries.
- **Hall of Meat is gated.** `sub_824DD408` creates that message only when
  `(flag && timer < 1.0) || mode != 7`. The mode word is `[[0x830CFDC4]+1060]`; free skate appears to
  be mode 7. It fired only in Hall of Meat mode (session `play4`, 8:30 to 10:00, with 66
  `hom_slo_mo` voices), with `fade_to_white` (object 68) beside it. The full gate, flag and timer
  included, is in sk8Audio `docs/audio-banks.md`.

**Unused in retail:** `Class_wheels_flip`, `Class_pre_lands_whsh`, `c_board_tumble`,
`SenseOfSpeed_tone` and `Ollie_Rattles`. They are named in the `.csi` tables, but no bank exports them
and no constructor was found.

**Board-related, not yet tied to the player. Investigate before calling the job complete:**
- **The loose board after a bail.** `c_dynamic_rolling_objects` and `c_dynamic_sliding_objects` (the
  world's physics props) posted a few times in `play1` and `play4`. Nothing shows yet whether the
  thrown board uses them.
- **`wheels.big`** (two 14.8 s wheel-spin loops). It plays outside the bank path, so the voice-open
  probe never saw it.
  - The `SFXObj_Wheels` constructor `sub_824CD6F8` formats `data\audio/wheels.big|%s`.
  - It opens streams through `sub_828DC158` and `sub_8298ED88`.
- **`grains.big`** (14 surface grain files). It also plays outside the bank path.
  - The `SFXObj_Moving` path is `sub_824E48D0`, plus `sub_824E7658` for `x_jet_rolling.grain`.
  - It opens through `sub_828DCCF8`.
- Recording either needs a new probe hook on those loaders. That can only be done on Linux.

**What "match perfectly" can mean.** Say this to the user before promising more.
- **Can match exactly, and can be checked against traces:**
  - which object is posted for an event, with its payload and per-frame updates;
  - which layers open, from which sample groups;
  - every voice property;
  - timing;
  - the graph and the mix.
- **Random by design:** which variant of a sample plays, small pitch offsets, some start gains.
  - They come from **one global counter at `0x830775F0`** that every patch in the game draws from.
  - The game itself does not repeat them between runs.
  - The practical target is the same logic and the same distribution.
- **Bit-for-bit output** needs a reproducible scene. Two recomp runs do not produce the same capture.

## The recorded sessions (in git)

sk8Audio `probe/traces/sessions/LABEL.audio-trace.log.gz` holds eight sessions, 24 MB, with a README.
They contain only the audio probe's lines:
- `skate3-audio-msg`: a post, with its payload words;
- `skate3-audio-update`: a re-delivery;
- `skate3-audio-open`: a voice open, with the EAAC header words, descriptor and parameter records;
- `skate3-audio-graph`: a graph build, with its module classes;
- the script markers.

| label | how | notes |
|---|---|---|
| `msgs1` | script `bail_replay_v2.txt` | first trace (2026-09-14) |
| `tour1`, `flips2`, `ollies2`, `bails2` | scripts `sound_tour_v1`, `late_flip_v5`, `ollie_check_v4`, `bail_attribution_v3` | markers name the intended move |
| `play1` | by hand, 166 s | board slides, flips, bails |
| `play2` | by hand, 220 s | big falls, powerslides |
| `play4` | by hand, signed in, 13 min | free skate, then **Hall of Meat mode** |

`probe/traces/fstep_updates_40C02FD0.txt` holds the 409 footstep updates the reproduction below
replays.

**Reading a session on Windows:**

```powershell
cargo build --release -p skate-audio-formats --example find_samples --example list_exports
$env:SKATE_AUDIOFILES = "<game>\data\audio\audiofiles.big"
$env:SKATE_AUDIO_EXAMPLES = "<skate-3-rust-engine>\target\release\examples"
python <sk8Audio>\probe\trace\sound_report.py <sk8Audio>\probe\traces\sessions\play4.audio-trace.log.gz --every 30
```

The report prints posts, re-deliveries and bank-attributed voice opens per marker, or per time bucket
with `--every`. It ends with which player objects the session posted.

**Recording more is Linux-only.** The recomp, its probes and the input harness live there. From
sk8Audio's root:

```sh
# By hand (the user closes the window); SIGNED_IN=true is needed to enter game modes like Hall of Meat.
OUT=$PWD/probe/harness/out SHADOW=false AUDIO_DUMP_FRAMES=0 PLAY_MOVIES=true SIGNED_IN=true \
  EXTRA_ARGS="--skate3_auto_install_dlc=false --skate3_audio_probe_messages=true --skate3_audio_probe_messages_count=500000 --skate3_audio_probe_opens_count=50000 --skate3_audio_probe_updates_count=3000000" \
  probe/harness/run_session.sh play5 &
probe/trace/keep_log_pieces.sh play5      # the logger keeps only 10 x 5 MB pieces without it
```

For a scripted run, add `DURATION=<s>` and `INPUT_SCRIPT=probe/trace/scripts/<script>.txt`. It needs
a free, unlocked desktop and no running `skate3`. Never kill `skate3` by name: the build directory is
shared with other launchers.

## Evidence the Rust side matches

This uses `crates/skate-audio-core/examples/grind_instance.rs` against real data.

- **Footstep, against the trace.** `playercharacter_footstep` in `fstep_skateshoe1_sm.abk` was driven
  by the traced post and its 409 updates.
  - It opens eight layers on the same update as the game, from the same sample groups.
  - Voice properties 3, 5, 6, 7, 8 and 9 are identical to the game's.
  - Variant, pitch and two start gains differ, consistent with the random counter's state (not
    proved).
- **Voice graphs.** With `GRAPH=1` the Rust device open builds the game's module list for every layer:
  `SndPlayer1, Rechannel, Resample, HighPassIir2, LowPassIir2, Send, Gain` at one channel, then
  `Pan2D1` and `Send` at six.
- Tests: 678 in `skate-audio-core`, 33 in `skate-audio-formats`, passing on Linux.

Full write-ups are in sk8Audio `docs/audio-banks.md`, and the plan is sk8Audio `docs/PLAN.md`,
Phase 6.

## What to bring to Windows

Everything in git is pushed (2026-09-15):
- `github.com/andrewnakas/skate-3-rust-engine` `main` (this repo, with the audio crates vendored under
  `crates/`);
- `github.com/andrewnakas/skate3-audio` `phase0-measurements` (sk8Audio: the crates' source of truth,
  the C++ references, the traces and the report tool);
- `github.com/andrewnakas/skate3recomp` `audio/phase1-harness` (the recomp's native audio and probes).

`SK8-ENGINE/skate-3-rust-engine` was deliberately not pushed, because its `main` rebuilds the public
release.

Not in git, and copied by hand:
1. **Game audio archives**, from the user's own disc: `audiofiles.big` (every player bank),
   `wheels.big` and `grains.big`. On Windows they are in the extracted game's `data\audio`.
2. **The decrypted guest image**, `g_8200.bin` … `g_8320.bin`, 21 MB. The Rust runtime reads guest
   rodata from it: class descriptors, constants, the trig table, the object table. It is decrypted game
   code, so never commit it.
   - On Linux it is in sk8Audio `probe/harness/out/image/`.
   - **The Windows partition is mounted read-only from Linux**, most likely Fast Startup or
     hibernation. Copy it with a USB stick. Or turn off Fast Startup in Windows and shut down fully,
     after which Linux can write to the drive.
3. **ffmpeg**: `ffmpeg.exe` on `PATH`, or `SKATE_FFMPEG` set to its path.
4. **Python 3**, for `sound_report.py`.

## Building on Windows

Per this repo's README: Rust with the MSVC toolchain, LLVM in its default location, then
`BUILD.bat` and `PLAY.bat`. For the audio crates:

```powershell
cargo test --release -p skate-audio-formats
cargo test --release -p skate-audio-core
```

Guest addresses are constants and host layout is never touched, so these should behave the same on
x86-64 Windows. **Run the tests first and report any difference from Linux before building on
them.**

To develop the crates, edit them in sk8Audio `rust/`, then copy them into `crates/` here without
`target/` or `Cargo.lock`. On Linux that is `tools/sync_rust_engine.sh push`; in Git Bash on Windows,
set `SKATE_RUST_ENGINE` and run the same script.

**Windows path trap.** `SKATE_AUDIO_PLAY=<archive>:<entry>:<channels>:<rate>[:<blocks>]` splits on
`:` from the right, so a drive letter breaks the four-field form. Always pass all five fields, for
example `C:\game\data\audio\ambience.big:0:5:48000:0`.

### Reproducing the footstep result

```powershell
$env:PAYLOAD = "00000000 00000000 00001000 00000000 000061A8 00000000 00000000 00007FFF 00000000 00000000 00000000 00000000 00000001 00000001 00000000 00000000 00000001 00000001 00000000 00000000 00000000 00000000 00000000 00000000 0000000C"
$env:UPDATE_FILE = "<sk8Audio>\probe\traces\fstep_updates_40C02FD0.txt"
$env:UPDATE_EVERY = "6"; $env:FRAMES = "70"; $env:GRAPH = "1"
cargo run --release -p skate-audio-core --example grind_instance -- <game>\data\audio\audiofiles.big <image dir> fstep_skateshoe1_sm.abk playercharacter_footstep
```

What to expect:
- seven `open voice` lines at frame 59 and one at frame 65;
- each followed by `graph for voice …: 9 modules [SndPlayer1x1 Rechannelx1 Resamplex1 HighPassIir2x1 LowPassIir2x1 Sendx1 Gainx1 Pan2D1x6 Sendx6]`.

Other examples: `voice_classes <image dir>` (cooked class defaults); in `skate-audio-formats`,
`find_samples` (bank of a traced open; `*:WORD` matches any slot) and `list_exports`; in core,
`bind_banks`.

## How a player sound flows, with addresses

1. **Gameplay** calls the object's message constructor (the target table). Messages are held, and
   the game re-delivers updated payloads every frame (`sub_828E2D18`, `patch::redeliver`; the grind
   updater is `sub_824C39E0`). Payload layouts known so far: grind (`docs/audio-banks.md`) and footstep
   (25 words, from the trace). The traces hold every other object's real payloads.
2. **Post** `sub_828E2B48` (`patch::post`) → the bank listener `sub_82B1DAD0` → **instance**
   `sub_82B1D880`, linked onto the evaluator list `0x83036F4C`.
3. **Evaluator** `sub_82B1E290` (`interp::tick_with`), once per 256-frame audio block at 48 kHz. Game
   init sets the period denominator `0x82FD35F4` to 30.0.
4. **Voice op** (slot 27, `sub_82B1D240`, `voice::voice_op`) → the device's open.
5. **Device open** `sub_824A3140` (`device::open_voice_graph`):
   - it registers the classes (`classes.rs`);
   - it builds the graph (`modules::build_graph`);
   - it enqueues commands on the system ring (`system+48`, write offset `+204`): install
     `0x82B49210`, player gain `0x82B49268`, play `0x82B32DC8`, send bus `0x82B31680`, property stamp
     `0x82B463A8`.
6. **Drain** (missing). Phase 4 of `sub_82B48530` walks the ring, calls each record's handler, and
   advances by the size it returns. It then raises the high-water mark `+208`, zeroes `+204` and counts
   drains at `+256`. These handlers are already ported: `modules::install_command`,
   `leaves::publish_float`, `leaves::stamp_slot` and `voices::repoint_link`.
7. **Play** (missing). Transcribe `sub_82B32DC8` from sk8Audio
   `recomp/src/audio_ports/sub_82B32DC8.inc`:
   - claim the SndPlayer1 ring slot at write index `+467`;
   - run `bitstream::parse_voice_header` and `decode_packet_header` (both ported);
   - advance the index.

   Record kind `+73` is bits 31-30 of the sample header's second word. Non-looping bank samples
   (footsteps, flips, impacts) are kind 0 and take the short path. Looping or streamed samples (grinds,
   slides, rolling) take the long path, which needs `sub_82B47A68` (C++ `.inc` exists),
   `sub_82EC0E08` (no port) and a name allocation.
8. **Stream pump** (missing), the node callback `sub_82B31EE0` (gate 1: an XMA submit under critical
   sections). It moves a record to state 2 or 3 and readies the consumer slot
   (`object + 16*[object+474]`, byte `+113` = 1), which `sndplayer::render_block` waits for.
   **Engine-decoded PCM enters here.** Replace the XMA submit with `skate-data` decode, so that
   `stream_remaining` and `deliver_frames` (with a `StreamFill`) see the frames the pump would have
   produced.
9. **Graph pass** per block for each installed player: `graph::run_pass` with
   `kernels::VoiceKernels`, then mix the buses to output. The 6-channel panner means a 5.1 bus, so
   downmix for Bevy. The top-level per-block function on `RwAudioCore Dac` is not yet mapped for Rust;
   start from `sub_82B48530`'s caller.

## Work order

1. **Drain.** A Rust dispatcher by handler address over the ported handlers; an unknown handler is an
   `Err` naming it. Locking and the `mftb` timing stores are not reproduced; say so in the module note.
2. **Play command**, the short path from the `.inc`; the long path behind a host trait.
3. **Stream pump replacement.** Read `sub_82B31EE0.inc` and `sndplayer.rs` first. Design the
   engine-PCM `StreamFill`, and write down exactly which fields it sets and why.
4. **Per-block driver.** Evaluator tick, drain, graph passes, then output into `skate_audio.rs`'s Bevy
   source. **First milestone: one footstep audible in `skate3rust.exe`**, driven by the traced payloads.
5. **Looping sounds**: the play command's long path, for grinds, board and body slides, and rolling.
6. **Voice lifetime.** The voice release `0x82B1E458` (C++ `.inc`), SndPlayer1 configure ids 1-4, and
   bus creation `sub_824916E8` (a host stub today).
7. **Gameplay events, object by object from the target table.**
   - Transcribe each message constructor; they clamp arguments and post, so each holds its payload
     layout.
   - Read each gameplay caller to learn which skater state feeds which argument.
   - Map that onto the engine's skating state, and post and update at the same moments.
   - Check every object's posts, updates and voice opens against the traces for the same move.
   - Hall of Meat: reproduce `sub_824DD408`'s gate.
8. **The board-related open items** above: the loose board, `wheels.big`, `grains.big`. Each needs a
   Linux probe session first.
9. **Guest image at runtime.** Development loads the dumped `g_*.bin`. A release must build those
   regions from the user's own `default.xex` during setup (XEX2 decrypt and decompress). Not started.

## Rules carried over from sk8Audio (read its `CLAUDE.md`)

- **Exact means exact.**
  - Transcribe store order, reloads, 64-bit adds and `fctiwz`/`fctidz` edges.
  - Scalar `fmadds`/`fmsubs` are fused; vector multiply-adds round twice.
  - Compute `lis` constants as `((hi & 0xFFFF) << 16) + lo`; never read them by eye.
- **Label unverified work** in module notes and the crate README table, and keep the test count current.
- **Real data beats unit tests.** Check every stage against `probe/traces/sessions` and the retail
  archives.
- **Never commit or distribute game assets**: audio archives, the image dump, the soundtrack.
- Commit messages end with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

## A prompt to start the Windows session

> Read `docs/audio-player-sounds-handoff.md` in skate-3-rust-engine, then sk8Audio's `CLAUDE.md` and
> `docs/PLAN.md` Phase 6. First run the two audio crates' tests, the footstep reproduction and
> `sound_report.py` on one trace, and report any difference from Linux. Then work the "Work order"
> list from item 1: drain, play command, stream pump replacement, per-block driver. The first
> milestone is one footstep audible in `skate3rust.exe`; after that, every object in the target table,
> checked against the traces. Scope is the player's own skating sounds, board and body.

## Windows audio checkpoint — 2026-09-15

The Windows output path is now proved with the user's own Skate 3 ISO; this is a diagnostic and
archive-decoding milestone, **not** automatic gameplay-sound integration.

- Rust stable MSVC is installed. Build products must use
  `%LOCALAPPDATA%\\Skate3RustEngine\\target`: linking in this OneDrive checkout repeatedly failed
  with `LNK1104` while the local target succeeds. `scripts/Build.ps1` now selects that location by
  default.
- The project-local ignored `.local/ffmpeg/*/bin/ffmpeg.exe` is discovered before PATH, with
  `SKATE_FFMPEG` remaining the explicit override. FFmpeg 9.0.1 essentials was used to verify the
  real decoder path.
- `skate3-audio-check --tone [seconds]` exercises the same Bevy/CPAL `AudioSink` without game
  archives. `SKATE_AUDIO_TONE` is implemented in `crates/skate-game/src/skate_audio.rs` and has a
  bounded-one-second test.
- `SKATE_AMBIENCE=<resident.big>:<payload.big>:<place>` now supports two absolute Windows paths
  (including both drive letters); `|` is also accepted as an unambiguous separator. The original
  colon syntax is preserved.
- The owned ISO remains `D:\\skate3.iso`. The partial, private extraction containing the audio
  archives is `C:\\Users\\andre\\AppData\\Local\\Skate3RustEngine\\owned-disc\\partial-skate3\\data\\audio`.
  It includes `ambienceresident.big` (1,568 bytes), `ambience.big` (143,708,256 bytes), and
  `audiofiles.big` (104,748,800 bytes). Never add these to Git.

Evidence from the final real-archive check:

```text
ambience for "univ_mt_low": 07_univ_mt_high.sns ... (looping=true)
skate-audio: ... 07_univ_mt_high.sns decoded, 204416 frames of 5 channels (4.26 s)
skate-audio-check: the stream finished after 7.9s
```

Run it again with:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'Skate3RustEngine\target'
$env:RUST_LOG = 'info'
$audio = 'C:\Users\andre\AppData\Local\Skate3RustEngine\owned-disc\partial-skate3\data\audio'
$env:SKATE_AMBIENCE = "$audio\ambienceresident.big:$audio\ambience.big:univ_mt_low"
cargo run -p skate-game --bin skate3-audio-check -- 15
```

Current tests: `cargo test -p skate-game --bin skate3-audio-check --locked` passes 6/6. The
modified files are `crates/skate-game/src/skate_audio.rs`,
`crates/skate-game/src/audio_check_main.rs`, `crates/skate-data/src/audio/ffmpeg.rs`, and
`scripts/Build.ps1`. No commit has been made. The attempted `skate3rust` development build was
intentionally stopped before launch when the planning handoff was requested.
