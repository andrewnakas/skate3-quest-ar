# Engine defects found while porting player audio (as of 2026-09-20)

These are **engine/physics/gameplay** defects, not audio-port gaps. Porting the audio exactly made
them visible, because retail's audio reads native physics fields directly and therefore only sounds
right when those fields carry retail's values.

Each entry says how it was found, how confident the diagnosis is, and what would prove it fixed.
Confidence is deliberately separated from severity: some of these are measured, others were seen
once in a playtest and have no repro yet.

---

## 1. Jump height and the landing's weight — **resolved by measurement; no workaround left**

**History, because two wrong turns are recorded in the git log.** An earlier session measured
`jump_height_200` at 0.10–0.13 m against a retail capture's 1.1–2.2 m and concluded the field was
~10x short, so a 10x was applied to the copy handed to audio. On 2026-09-20 that was moved onto the
real launch (`ground_jump`'s `remaining` term) on the theory that the jump itself was short; the
owner playtested it and went into orbit, so the launch was ruled out and the change reverted.
`ground_jump` is untouched and retail-exact.

**What the trace actually shows.** Measuring Class_Treatment word 9 directly from a playtest
(`SKATE_AUDIO_TRACE`, 27541 updates, 1559 with a live landing):

| | word 9 (`trunc(clamp(h × 166.667, 0, 1000))`) |
|---|---|
| Retail | 183–367 |
| This engine **with** the 10x | min 62, **p50 1000, p90 1000** — pegged at the clamp |

So the 10x drove word 9 3–5x past retail and into saturation: every landing played at maximum
weight regardless of the hop, which is what the owner heard as "landings too loud for the height".
Back-solving the clamp puts the *reported* height at **≥0.6 m**, not the 0.10–0.13 m the original
measurement claimed — so the field is far less wrong than believed, and word 9 without any scale
falls around 100–300, inside retail's range.

**Resolution.** The scale is removed entirely. No workaround remains in the tree.

**Residual, low priority.** Reported height (~0.6 m) still sits below retail's 1.1–2.2 m plateau,
so word 9 may run slightly low on big drops. If that ever matters, the thing to check is that
`max_y` tracks the deck rigid body (`air_phase.rs:202-206`, `part_transforms()[6]`) while `start_y`
is a wheel-centre ground reference — a constant offset between two different references is
subtracted out of every hop. PhysicsAir and KnownAir also disagree on that reference: `+500`
(current, `air_phase/input.rs:49`) versus `+480` (previous, `known_air.rs:94`). **Do not "fix" this
at the launch** — that was tried and it sends the skater into orbit.

## 2. The engine rarely enters KnownAir — **diagnosed from code + capture**

**Symptom.** Originally: no landing impact at all. Every per-wheel landing bucket stayed 0.

**Evidence.** Retail is in KnownAir (state 201) for essentially every hop. This engine selects it
only while Processed2468 bit 10 (trajectory valid) is published, and nothing publishes that bit
except transiently, so ordinary ollies run in PhysicsAir (200/202). PhysicsAir publishes nothing
for Air+176 and a constant 4.0 for Air+184, so audio state `+236` stayed 0 and with it `+244` →
`+448`. Documented at `crates/skate-game/src/physics/audio_observation.rs:182-207`.

**Current workaround.** Audio reads the owning state's own timer/prediction instead
(`audio_observation::air_timing`). This restored landing impacts, but it is audio-side
compensation for a physics-side defect — anything else that reads air timing still gets the wrong
values.

**Fixed when.** An ordinary ollie runs in state 201, and `air_timing` can be deleted in favour of
reading Air+176/+184 directly.

### Retail's actual ollie path, recovered 2026-09-20 — and why the premise now needs re-measuring

**Retail does not go 100 → 201.** An ordinary ollie runs **100 → 103 (GroundAnimation) → 201**, and
the trajectory is launched *and completed in the same frame* inside GroundAnimation's
`sub_82D33E30`, gated on `Toolkit_CalcGroundJump` (`sub_82D93618`) returning `JumpInfo+20 != 0`:

```
86.cpp:6996   bl 0x82d93618      Toolkit_CalcGroundJump
86.cpp:7022   beq cr6,...        not active -> ordinary ground forces
86.cpp:7080   bl 0x82d67848      LAUNCH
86.cpp:7085   bl 0x82d68800      COMMIT, same frame -> selector+9658 valid = 1
```

Then PostInput (`sub_82DB5588`) calls `sub_82D68800` again — nothing is pending, so it early-outs
and returns the **retained** `valid` — and stamps bit 0x400:
`91.cpp:62477 rlwimi r4,r5,10,21,21`. The 103 arm of the selector reads `+2572 == 1` and yields
`200 | bit` = 201 (`90.cpp:13258` → `90.cpp:14056`). `valid` is sticky: only `sub_82D67848`
(launch) and `sub_82D67228` (reset) clear it. PhysicsGround launches **only** on the animated-board
branch (`86.cpp:19001 lbz r8,2708(r31)`), which is a contact condition, not the ollie.

**This engine already matches all of that.** `ground_animation/board.rs:71-104` launches on
`jump.active` and calls `trajectory.update` on the very next lines, and `Trajectory::launch` fills
`pending_results` synchronously (`air_trajectory/mod.rs:50-54`), so the completion really does
happen in-frame. `complete_batch` sets `self.valid = true` at `selector.rs:278` **before** it
starts any second pass, and `launch_pass` does not clear it — so a pending second pass does not
take `valid` away. Both selector arms (`selector/ground.rs:33-35` for 100, `:107-109` for 103)
match retail's `field_2572 == 1 → air_variant_from_2468()`.

**So the "nothing publishes bit 10 except transiently" premise is not supported by the code as it
now stands, and it has not been re-measured since the workaround was written.** Note also that
rolling off an edge *correctly* yields PhysicsAir in both retail and here — `selector/ground.rs:42`
and `:110` hard-code it, exactly as retail does — so a roll-off test proves nothing about ollies,
and `tests/air_playback.rs` only ever asserts `category() == 200`, which 200/201/202 all satisfy.

**Do this before changing any physics.** Play, ollie, and read the new `st=` field:
`grep -o "AS st=[0-9]*" <trace> | sort | uniq -c`. If 201 appears on hops, this defect is stale
like #3 and #7 were and the only work left is deleting the `air_timing` workaround. If it never
does, the thing to instrument next is `jump.active` and the selector's `valid` at the moment
PostInput publishes, since every other link above is confirmed present.

## 3. Powerslide squeaks — **the "never fires" premise was wrong (corrected 2026-09-20)**

**Symptom as originally recorded.** No powerslide squeak sound, ever.

**That premise does not survive the traces.** Retail's gate is: both feet inside the deck box
(`+615`/`+616`), more than one wheel down, and `|deck tilt +264| × 114.5916 ≥ 15` (the threshold is
vault-loaded, `Hash_A129B33B4A2C7961`). Counting over the existing `logs/audio-trace-*.log`, the
gate passes on thousands of frames per session and `Class_Squeaks` **is posted**: 85 posts in
`audio-trace-20260920-122739.log`, 70 in `...-111039.log`, with thousands of `UP` redeliveries.
`|tilt264|` reaches 0.38–0.64 rad, comfortably past retail's 0.14–0.28 band, so the tilt input is
not short either. The gate and its four inputs are fine.

**What is actually unknown.** `tilt264` is the *steering* tilt (an exponential blend in
`TruckSteeringState::update`), not a slide-specific angle, so a passing gate does not prove the
squeak fired *during* PhysicsSlideGround. The `AS` trace line carried no state id, so the logs
could not answer it. That is now fixed: `AS` carries `st=` (State+16) and `cat=` (State+12) as of
this commit.

**How to finish it.** Play, powerslide, and filter the new trace for `st=101`:
`grep "AS st=101" <trace>` — then check whether those frames pass the gate and whether a
`Class_Squeaks` `PO` sits on the same frame number. If they do, the squeak is firing and the
remaining question is its rendered level (the posted speed word runs as low as 19/1000 at slide
onset while the turn word saturates at 1000); if they do not, compare the four fields on those
frames against the retail capture's `state.tsv`.

**Note for whoever picks this up:** the `DirectMixer` block in `player_audio.rs` also references
`Brd_Squeaks.abk` on a different gate, but it is dead code — the struct is never constructed. It
is not a confound.

## 4. Bail / ragdoll native outputs — **was a gap; the outputs now exist, unverified by ear**

This entry used to say the bail families were silent for want of engine outputs. That is no longer
true, and the text stood long after the ports landed. What is actually there, re-checked
2026-09-21:

- **Ragdoll contacts reach the audio state.** `sub_82BD60C8`'s per-region work is ported at
  `crates/skate-game/src/physics/audio_observation.rs:135` (`body_contacts`), `sub_82773298` at
  `player_audio/audio_state.rs:286` (`Conditioner::body_contacts`), and words **496–611** are
  stored every frame at `audio_state.rs:798-806`.
- **The speeds and flags exist**: `+328` (`audio_state.rs:822`), `+672` (`:797`), `+676`/`+677`
  (`:830-831`), `+780` loose board (`:853`).
- **The consumers are ported and registered.** `c_body_slide` and `c_cloth_falls` are in
  `player_audio/components/clothing.rs`, wrapped on the `Clothing` component and registered at
  `player_audio/sound.rs:454`. There is no env gate.
- **The surface tag reaches them.** A contact's material rides word 23 of the narrow-phase row and
  word 55 of the compiled one (`solver/contact_build/publication.rs:72-83`), which is what
  `contact_feedback/spy.rs:43` reads; the 23→55 move is asserted whole-row by
  `crates/skate-core/tests/physics_rebuild_contact_build.rs:63,83`. Both real map sources supply a
  non-zero surface (`skate_world.rs:172` for RWCM, `:336` for portable `.skate`).

**What is genuinely unresolved.** Nobody has heard a bail and checked it, and there was no way to:
none of these fields appeared in any trace. The `BC` line added 2026-09-21 carries them, so the
next playtest can answer the open questions instead of re-deriving them:

1. Does `mat560` ever resolve? It is zero on the built-in test terrain (`physics/ground.rs:199`
   authors material 0) and legitimately zero for body-vs-body contacts
   (`solve/assembly_contacts.rs:108`), so an all-zero reading on a **stock map** would be the real
   defect, and every body slide would take `body_contact`'s default type 2.
2. Is `+328` right? The layout table (`player-audio-retail-drivers.md:74`) fixes the source as
   `|Skeleton+288|` and the port reads it, but whether that offset is the part's *angular* or
   *linear* velocity is not settled from the lifted C++. It drives `c_body_slide` w10 and
   `c_cloth_falls` w3 directly, so it is worth settling before tuning either by ear.

**Fixed when.** A playtest `BC` trace on a stock map shows `mat560` resolving, and the owner has
A/B'd a bail against retail.

## 5. Surfaces — **switched on 2026-09-23 for retail maps; portable maps still wrong**

*Originally: everything ran on default surface 2 (concrete_rough), which made rolling grains, seam
patterns, wheel-pop categories, grind timbre and skid sounds correct-but-monotonous — the right
sound for concrete everywhere, including wood, metal and grass. The exit condition was "the engine
publishes real per-wheel audio materials and the map's seam pattern". It does.*

**Verified before flipping it.** `WorldTriangle.tag` carries the retail RWCM per-triangle
`surface: u16` verbatim — decoded at `crates/skate-data/src/retail_collision.rs:186-190`, forwarded
at `crates/skate-game/src/skate_world.rs:172`, read back through `board_world.rs:236` →
`riding_outputs.rs:301` → `board_ground.rs:90` as `tag & 0x7f`. The `& 0x7f` split is confirmed
independently by the vendor extraction tool
(`tools/vendor/university/tools/vanilla_map_extraction/blender/import_hawaiian_dream.py:480`), and
`SKATE_AUDIO_OBSERVE=1` traces on University show a dozen distinct ids — 1, 3, 4, 5, 8, 16, 17,
41, 42, 53, 66, 79 — with 41 and 3 dominant. The seam pattern (`tag >> 12 & 0xF`) is published too
(`board_ground.rs:92`); the note in `docs/player-audio-retail-drivers.md:92` calling it "discarded"
is stale.

`BoardConfig::from_env` now defaults to `SurfacePolicy::Retail`.
**`SKATE_AUDIO_SURFACES=concrete` restores the pinned behaviour** so the two can be A/B'd in one
session.

Landed with it: `Board::rolling_kmh`'s out-of-range fallback was `0.0`, which makes `speed_word`'s
`v / kmh` infinite and pegs the rolling speed word at its 10000 clamp for every speed. It was
unreachable while every surface was forced to 2 and becomes reachable the moment the non-grain
selectors (9–12) can be chosen. It is now `DEFAULT_ROLLING_KMH` (45.0), which is what
`sub_824C6B30(-1)` returns. The owner's vault array has 16 entries —
`[70, 65, 65, 70, 100, 45, 45, 45, 45, 35, 45, 45, 45, 45, 45, 45]` km/h — so every selector is
covered and nothing was actually hitting the bad path.

**Still wrong: portable `.skate` maps.** `skate_world.rs:324` computes the correctly packed
`m.audio | (m.physics << 7) | (m.pattern << 12)` but stores it **only** in `packed_surfaces`; the
triangle's `tag` at `:336` takes `source.surface`, which the Blender exporter writes as a running
per-object counter (`tools/vendor/university/tools/blender_owned_map/.../exporter.py:3127`,
`:3443`). So on a non-RWCM map `tag & 0x7f` is the low seven bits of an object index and the real
audio id never reaches the wheel lines. `skate_world.rs:1331-1346` asserts `packed_surfaces` and
nothing asserts `tag`, which is how the two drifted. Fix `tag` to take the packed value and assert
it.

Also expected and not a decode failure: RWCM units with bit `0x80` clear decode to `surface = 0` →
material "none" (143), and if all four wheels sit on such triangles the vote falls back to surface
1 (`ground_runtime/surface.rs:37`).

## 6. Physics NaN torque crash on landing — **seen once, no repro**

A playtest crashed on a landing with a NaN torque in the physics solver. It is unrelated to audio
and has not been reproduced since; no current log carries it. Listed so it is not forgotten, but it
needs a repro before it can be chased. Treat the diagnosis as unconfirmed.

## 7. Broadphase disagrees with a full scan — **resolved 2026-09-20: the test was wrong**

The engine is correct; the oracle was not. `tiled()` lays all 1024 fixture faces in the plane
`y=0` and the probe volumes sit at `y=0.25`. The recovered candidate producer `82ACEA30` offers
only the triangle's own face normal for point/segment/triangle volumes — no edge or vertex axes —
so a coplanar tile 10 km away in x still projects to a ~0.05 separation and survives `82ACE968`'s
`separation > fat + limit` gate. The linear reference therefore handed the narrow phase 1024 tiles
it would never see natively and collected 1025 contacts for a sphere that touches two triangles;
the hierarchy returned the correct 2. The mismatch only surfaced once `capacity = 100` truncated
the bogus set, which is why it looked like a retention bug.

That is native behaviour, not a defect: the native narrow phase is only ever reached through a
broadphase that has already rejected laterally, and **every in-game `BoardWorld` is built with
`with_query_metadata`**, so the per-triangle bounds cull is always active. `BoardWorld::new`
without metadata is test-only. Staggering the distant tiles in Y — separating them along the one
axis the SAT does test — makes the full scan a valid oracle again; linear and accelerated then
agree at 2/4/8/6 contacts for the sphere, capsule, rounded box and triangle. Fixed in the commit
that added this paragraph; 606 `skate-core` tests pass.

*Original entry, kept because the wrong diagnosis is instructive:*

`physics::board_world::broadphase_tests::predictive_contacts_and_retention_match_full_scan_for_every_primitive`
(`crates/skate-core/src/physics/board_world/tests.rs:232`) fails deterministically:
`world.query_primitives(..)` returns a different contact set from the linear reference scan
`linear.query_primitives(..)` for some primitive/capacity/`deferred_reduction` combination.

Repro: `cargo test -p skate-core --lib --locked predictive_contacts_and_retention`

Confirmed pre-existing as of commit `330a3e9` — it is not a consequence of the pop-height change.
It matters because the broadphase is what feeds collision to the whole board simulation, so a
disagreement with the reference scan is a correctness bug in contact generation, not just a
failing test. The assertion dump is large; the first divergence is in the retention records, so
start by narrowing which `capacity` / `deferred_reduction` pair diverges.

## 8. Renderer crash on surface teardown — **seen once, did not reproduce**

`wgpu_hal::vulkan` panics with `Trying to destroy a SurfaceAcquireSemaphores that is still in use
by a SurfaceTexture` (`instance.rs:194`), taking the process down with exit code 0xC0000409.
Observed once at +24.8 s on 2026-09-20 (crash report
`report-1789928798791616000-15424.txt`); an immediate relaunch of the same binary ran fine, and the
four launches before it never hit it. Audio was up and healthy at the time (`player_sound ready`,
241 Treatment packets traced), so it is a renderer-side race on surface destruction, not audio.
Intermittent, so it needs repeated launches or a GPU-validation run to pin down.

## 9. Rail / grind landing sounds are not driven — **mostly closed; only `PopRoll` is left**

*(Header corrected 2026-09-23. The grind onset **is** posted and voiced now --
`ContactsOwner::grind_onset` then `ContactVoicePlayer::post_grind_onset`,
`contact_voices.rs:731-814` -- so a rail landing is no longer silent. What remains dropped with
a label is `ContactSound::PopRoll` (`contact_voices.rs:581-586`), which still needs
`[manager+668]` / `sub_82486EF0`. The rest of this entry is the original investigation and is
still the reference for the chain.)*

Landing *onto* a rail makes no grind-onset sound. `ContactSound::GrindOnset` and
`ContactSound::PopRoll` are deliberately dropped in `player_audio/contact_voices.rs` rather than
played from an invented sample: retail routes them through a different subsystem — the
contact-sound manager at `[manager+668]`, fed by `sub_82496C58`'s per-material level interpolation
and `sub_82486EF0`'s 48-byte message. That path is also what carries retail's *continuous*
per-material impact level (see `docs/player-audio-retail-drivers.md` §9), so porting it fixes rail
landings and adds material-dependent landing weight at the same time. `sub_82486EF0`'s sink was
traced as far as `[g[0x830CFDC4]+668]->vfunc12`; the handler beyond that is not yet followed.

### Decoded 2026-09-20 — everything except the sink's own dispatch

**Why a rail landing is silent specifically.** `Contacts::step` suppresses the ordinary landing
voice during a grind (`contacts.rs:708`, `else if !airborne && !grinding`), so the grind onset is
the *only* sound retail plays for that event — and it is the one that is dropped. The two
suppressions compound into silence. Retail has no such gap because its onset always posts:
`sub_824BB0E0` has no zero-level guard, unlike the landing `sub_824BA630`, which skips its message
when either level is 0 (`11.cpp:26040-26050`).

**The game-side chain is already complete and correct**: contact geometry → `impact_speed_128` →
`AudioState.grind_impact_228` / `grind_material_692` / `grind_family_192` →
`ContactsOwner::grind_onset` → `VoiceRequest { material, family_base, tier }` →
`ContactVoices::play(GrindOnset, ..)`. Only the sink is missing.

**The message is fully decoded.** `sub_82486EF0` allocates 48 bytes and writes: `+0x00` material A
(the family base, 95 or 96), `+0x04` material B (the grind material, 143→10), `+0x08`/`+0x0C` the
impact tier, `+0x10` a `vec4` world position copied wholesale (from audio state `+48` for the grind
onset, `+144` for a landing), `+0x20`/`+0x24` the two 0…32767 levels, and three flag bytes at
`+0x28`/`+0x29`/`+0x2A`. Delivery is two-stage: `[manager+668]->vtable[+12](msg)` returns another
object and the message is handed to *its* `vtable[+12]` too.

**The levels are decoded too.** `sub_82496C58` returns
`low + (high - low) * (min(value, hi) - lo) / (hi - lo)`, truncated, with `(low, high)` chosen by a
(mode × material-category) table and defaulting to `(0, 32767)`; it returns 0 for a negative
material or mode 3. The tier selects which impact-speed window the interpolation runs over —
`[0.0, 0.25]` for tier 0 and `[0.25, 0.5]` for tier 1, both vault floats
(`086B66C3D4FFEE8F`, `B2ACAFDBCD963C93`) on the grind-material class. The Rust `VoiceRequest`
carries the tier but not those two bounds.

**What is still unknown is only the dispatch behind `vtable[+12]`** — i.e. how the material pair
picks the actual sound. Nothing in `sub_82486EF0` or `sub_824BB0E0` names a bank or a sample, which
is why `contact_voices.rs` is right to refuse to invent one. **Porting the onset means porting the
contact-sound manager, not adding an arm to `ContactVoicePlayer::selection`.**

### The sink is pinned to three words of `.data`/`.rdata` (2026-09-20)

The manager global `0x830CFDC4` is written in exactly one place, `sub_826D5500` (`28.cpp:52333`):
a 2192-byte object allocated and constructed by `sub_824845B8`. There is **no `stw rX,668`
anywhere**, because `+668` is not a named field: the constructor zero-fills a **14-entry pointer
array at `+656..+708`** (`9.cpp:44075-44088`), and `sub_82484FE8` (`9.cpp:45097`) fills it with
`modules[i] = sub_828DED90(i)`. So **the sink is `manager->modules[3]`** — array slot 3.

`sub_828DED90` (`46.cpp:57277`) is a type-id registry lookup over a vector at `0x830BBE00` of
12-byte descriptors (`+0` id, `+4` pool, `+8` create fn). The 14 descriptors are registered by
`sub_8248D2A0` (`9.cpp:64145`), whose addresses, create functions, object sizes and vtable
addresses are all recovered. **But the descriptors' id words live in `.data`, and the vtables'
contents live in `.rdata`, and the recompiler lifts only `.text`** — so neither "which class is
slot 3" nor "what function is at its `vtable[+12]`" is derivable from `generated/`.

Also corrected: **`sub_828AAF88` is not a fallback sink.** It returns the default
`EA::Allocator::ICoreAllocator` (`45.cpp:7080`), and the `loc_82486FBC` tail is
`GetDefaultAllocator()->Free(msg, 0)` — the null-sink path *destroys* the message. Model it as
"drop the event", not as a second delivery route. (Its twin `sub_828AAE90` is what allocated the
48 bytes, which is how the `vt[8]=Alloc` / `vt[12]=Free` slots were confirmed.)

**Three reads close this**, against a decrypted retail image at base `0x82000000`:
1. the id word at `0x8302CD1C` and at `0x8302CD80 + 12n` for n=0..12 — find the one equal to 3;
2. that descriptor's `+8` create fn, cross-checked against the recovered table, which names the
   class (each create fn passes a distinct allocation-tag string, so the class name is readable);
3. the word at `<that class's vtable> + 12` — the concrete `vtable[+12]`, which can then be pulled
   out of `generated/` with the usual awk and read directly.

**Getting a decrypted image is the blocker, and it is not hard in principle** — the retail
`default.xex` at `out/build/windows-release/game/` is `encryption=0001, compression=0002` (AES +
LZX), and the SDK ships `rex/system/xex_module.cpp` and `lzx.cpp` that do exactly this at runtime.
Note that `rexglue.exe dump-xex` **looks like a stub in the SDK build on this machine**: it exits 0
in 0.2 s and writes nothing, at any log level. Either fix that subcommand, add a tiny host that
calls the SDK's `XexModule` loader and writes the image out, or read the words from the running
recomp's memory.

### The sink is no longer blocked: the image dumps, and the chain resolves (2026-09-20, later)

`rexglue.exe dump-xex` is **not** a stub after all. Its own `_putenv_s` does not reach the loader,
but the loader reads `REXGLUE_DUMP_XEX_IMAGE_DIR` from the environment, so setting that variable
*externally* dumps the decrypted image:

```
cd C:\dev\skate3recomp\outuild\windows-release\game
REXGLUE_DUMP_XEX_IMAGE_DIR=<out> rexglue.exe dump-xex default.xex <out>
```

That writes `default_82000000_011B0000.bin` (18,546,688 bytes, base `0x82000000`), i.e. `.data`
and `.rdata` included. File offset = address − `0x82000000`.

With it, the three reads are done:

1. **Slot 3 is the FIRST descriptor**, `0x8302CD1C`: id 3, name pointer `0x82247544`, create fn
   `sub_824F16D0`. (An earlier inferred bijection guessed reg pos 4 — it was wrong. Read the ids,
   do not derive them.) Full table, descriptor → id: CD1C→3, CD80→10, CD8C→2, CD98→0, CDA4→1,
   CDB0→4, CDBC→5, CDC8→6, CDD4→7, CDE0→8, CDEC→9, CDF8→11, CE04→12, CE10→13.
2. **The class is `CSTATEMGR_Collision`** (the name string at `0x82247544`).
3. **Its vtable is `0x822FD4FC`**, read from the create function itself
   (`lis r9,-32208; addi r8,r9,-11012; stw r8,0(r3)` = `0x82300000 − 0x2B04`) rather than from an
   inherited table, which had it 0x10000 too high. Slots: +0 `824F1B70`, +4 `828DE848`,
   +8 `824F17B8`, **+12 `824F1818`**, +16 `828DEE58`, +20 `828DEED8`, +24 `828DEF38`,
   +28 `824F16B0`, +32 `824F16C0`. (**Correction 2026-09-20:** an earlier revision of this section
   listed a tenth slot `+36 824F88A8`. That was an over-read — the manager vtable ends at `+32`,
   and `0x822FD520` is already the *next* class's vtable. See below.)

**`sub_824F1818` (hop 1) is a voice-slot router, not the voice starter.** It walks the linked list
at `[this+16]` through `[node+4]`, keeps the node with the lowest signed `[node+64]` (a priority or
age), calls `[node->vtable+28](node)` on the winner and returns it. `sub_82486EF0` then delivers
the 48-byte message to *that node's* `vtable[+12]` — hop 2, which is where the material pair and
the two levels finally choose a sound.

### The chain is closed: the sink is a state machine, and the sound choice is a table (2026-09-20)

`[manager+16]` is populated, the node class is read, and the chain runs all the way to the material
→ sound-index tables. **There is no single "handler" — the sink is a generic data-driven audio
state machine, and what the material pair and the levels actually pick is a small set of integer
indices plus two controller parameters.** Every address below was read from the dumped image or
the lifted asm, not inferred.

**There are two parallel registries, not one.** The one already documented (12-byte descriptors,
vector at `0x830BBE00`, looked up by `sub_828DED90`) holds the 14 `CSTATEMGR_*` managers. Two more
sit beside it: 16-byte descriptors at `0x8302CE1C + 16n` (vector `0x830BBE20`, pushed by
`sub_828DE8C0`) holding the 14 matching `CSTATE_*` **state** classes, and descriptors at
`0x8302CF70…0x8302D3A0` (vector `0x830BBE10`, pushed by `sub_828DE928`, 46 of them) holding the
`SFXObj_*` / `SFXCTL_*` **component** classes. Descriptor word 0 packs `category<<16 | kind<<4 |
ctor-arg`; `+4` is the class-name string, `+12` the create function. Dumping all three tables gives
the whole audio object model by name — `SFXObj_Contacts`, `SFXObj_Wheels`, `SFXObj_Rail`,
`SFXObj_Treatments`, `SFXObj_Tricks`, `SFXObj_OffBoard` and so on are all in there, which is worth
revisiting for defects 4 and 5. **All three tables are written out in
`docs/audio-object-registry.md`.**

**`[manager+12]` is the manager's own registry id, read not derived.** `sub_828DED90` calls
`obj->vtable[+4](obj, id)` right after construction, and `CSTATEMGR_Collision::vtable[+4]` is
`sub_828DE848`, whose entire body is `stw r4,12(r3)`. So `[manager+12] = 3`, and 3 is what selects
every `*_Collision` class out of the two other registries.

**The nodes on `[manager+16]` are `CSTATE_Collision`.** `CSTATEMGR_Collision::vtable[+8]`
(`sub_824F17B8`) loops **exactly 10 times** calling `sub_828DEC20(manager)`, then sets `[mgr+24]=1`.
`sub_828DEC20` picks the descriptor whose category equals `[manager+12]` — descriptor `0x8302CE1C`,
name `CSTATE_Collision`, create fn `sub_824F8808` — makes the node, and links it: `[node+12]=manager`,
`[node+16]=[manager+20]` (its index), `[node+20]=[manager+12]`, `[node+24]=0`, appended through
`[node+4]` (next) / `[node+8]` (prev) with the head at `[manager+16]`, and `[manager+20]++`.
So the list is **10 LRU voice slots**, built once at manager init.

**`CSTATE_Collision` is 80 bytes, vtable `0x822FD520`** (from `sub_824F8808`'s own
`lis r9,-32208; addi r8,r9,-10976`). Slots: +0 `824F88A8`, +4 `828DF098`, +8 `82B61BB8` (a bare
`blr`), **+12 `824F8990`**, +16 `828DF4C8`, +20 `824F8B50`, +24 `828DF518`, +28 `824F8B58`,
+32 `828DF648`, +36 `824F87E8`, +40 `824F87F8`.

**Hop 2 is `sub_824F8990`, and it is five instructions.** It does *not* choose a sound:

```
[node+64] = [[0x830CFD94] + 16]     ; a global frame/time stamp
[node+68] = msg                     ; park the 48-byte message on the slot
tail -> sub_828DF6F0(node, msg)     ; activate the state
```

That closes the loop on the router: `[node+64]` is a **timestamp**, so `sub_824F1818` picking the
lowest signed `[node+64]` is picking the **least-recently-used** of the 10 slots. Before returning
the winner it calls `vtable[+28]` (`sub_824F8B58`), which frees that slot's previous message through
the default allocator and nulls `[node+68]` — i.e. it *evicts* before reuse.

**Activation reaches exactly one component.** `sub_828DF6F0` sets `[node+28]=msg`, `[node+52]=1`
(active), walks the `[node+32]` child list calling `vtable[+28]`, then calls
`[node->vtable+16]` = `sub_828DF4C8`, which walks the `[node+36]` child list calling `vtable[+24]`.
The children come from `sub_828DF098(state, mask)` — called with **mask = 1** by `sub_824F17B8` — so
bit 0 only: one child of kind 0, built by `sub_828DEA00(manager, index, 0)` and appended to
`[state+36]`. For category 3 / kind 0 that descriptor is `0x8302D160`, name **`SFXObj_Collision`**,
create fn `sub_824D1B48`.

**`SFXObj_Collision` is 104 bytes, vtable `0x822FC890`** (from ctor `sub_824D1BD8`:
`lis r9,-32208; addi r6,r9,-14192`). Its tail is **two 32-byte voice records** at `+40` and `+72`,
each initialised `{0, 0, 4096, 0, 0, -1, -1, -1}`. `sub_824F1468` (vt+20) binds the owner:
`[child+16] = [child+32] = state`.

Its `vtable[+24]` is `sub_82E1F0A8`, a thunk straight to `vtable[+28]` = **`sub_824D1DB8`**, which
is what actually receives the activation:

```
[[this+12]+12]->+60 = 1
msg = [ [this+32] + 68 ]            ; the message off the owning state
[this+36] = msg
tail -> [this+28]->vtable[+56]( [this+28], msg+16 )   ; msg+16 is the vec4 world position
```

**The material pair picks a sound in `sub_824D2318`** (`SFXObj_Collision::vtable[+40]`, the voice
starter, reached from the state update). It runs both material words through two resolvers and
drives the two voice records:

- **`sub_824D20E8(this, material)`** — if `material >= 143` the category is forced to 8; otherwise
  `cat = sub_82496FD0(material)`, *the same material-category function `sub_82496C58` uses for the
  levels*. Then a jump table at `0x824D212C` maps **cat → index: 0→13, 1→14, 2→15, 3→16, 4→17,
  5→18, 6→12, 7→19, 8→20, 9→21**, and any `cat > 9` also lands on 20. The index goes to
  `this->vtable[+60]` (`sub_824AF240`).
- **`sub_824D22B8(this, material)`** — `cat == 9` (or `material >= 143`) → **22**, everything else
  → **1**. That index goes to `this->vtable[+56]` (`sub_824C5910`).

So each of the two materials yields a pair of small integer ids, and those four ids (locals at
`r1+80…92`) are what the voice records are started from, together with float constants at
`0x824D0664`, `0x824D078C`, `0x824D1664` and `0x8232520C`.

**The continuous landing weight is `sub_824D1E00`** (`vtable[+36]`, the per-frame update called from
`sub_828DF568`). This is the piece §9 of `player-audio-retail-drivers.md` wanted, and it is tiny:

```
param0 = 0 ; param1 = 0                        ; via [this+12]->vtable[+8](index, value)
if ![state+52]                       -> done   ; state not active
if [this+40] == 0 && [this+72] == 0  -> done   ; both voice records idle
param0 = 32767                                 ; gate on
sel = msg[+0x08], msg[+0x0C] with 3 as a "none" sentinel:
        msg[+0x08] == 3 -> sel = msg[+0x0C]
        msg[+0x0C] == 3 -> sel = msg[+0x08]
        otherwise       -> sel = max(msg[+0x08], msg[+0x0C])
param1 = 20000 if sel == 1, 32767 if sel == 2, else 10000   ; clamped to [0, 32767]
```

`[this+12]` is the controller block and `vtable[+8]` is `set(index, 0…32767)`. Note both parameters
are **rewritten from zero every frame**, so this is a continuously driven pair, not a one-shot.

**What this means for the port.** `ContactSound::GrindOnset` / `PopRoll` do not need an invented
sample: they need the ten-slot LRU, the message park, and the two controller parameters above, with
the sound identity coming from the cat→index tables rather than from anything in `contact_voices.rs`.
The remaining unread piece is the body of `sub_824D2318` past the resolver calls — the float math
that turns the four indices plus the two 0…32767 levels into the two voice records — and
`sub_824AF240` / `sub_824C5910`, which turn an index into an actual bank entry.

### The control plane is ported (2026-09-20, later still)

`player_audio/collision_states.rs` + `collision_materials.rs` now hold the manager, and
`contact_voices.rs` posts the grind onset to it instead of dropping it. 10 unit tests, plus an
`--ignored` test that resolves all 143 material categories against the owner's real vault.
What the port covers, and the three things the port itself established:

**The indices are MixMap controller outputs, not bank entries.** `SFXObj_Collision::vtable[+60]`
(`sub_824AF240`) and `vtable[+56]` (`sub_824C5910`) are the *same* two accessors the other
components already use — `[[this+12]+12]` indexed as packed 16-bit words, the first masked to 15
bits, the second sign-extended and scaled by `0x822F889C` = **4096.0**. So `sub_824D20E8` /
`sub_824D22B8` choose *which output id* a material reads, nothing more.

That predicts the ten Collision controllers are `0x40030000 + 0x800·slot`, because
`sub_828DEA00` packs a component key as `0x40000000 | category<<16 | index<<11 | kind<<4`.
**The retail MixMap confirms it exactly** (`cargo run -p skate-audio-core --example mixmap_dump`):
ten `SFXObj_Collision` controllers, `40030000` … `40034800`, whose outputs run `0..=22` with
**precisely 1 and 22 typed `t1`** — the signed/scaled type `sub_824C5910` reads — and the rest
`t0`. An independent source agreeing with the vtable walk end to end.

**The material category is a vault lookup, not a table.** `sub_82496FD0(material)` reads the
material's 64-bit AttribSys key from a 16-byte-stride table at `0x8302D6E8` (`+0` the material's
collision sound id, `-1` = silent; `+8` the key), looks the record up under class
`D40CB4C0FFE45676`, and returns the field `D5EF686287A57AFE` — retail type
`Sk8::Audio::eMaterialNicotineType`, exactly ten values, which is what the `0..=9` jump table
indexes. Over the owner's vault the histogram is `[12, 49, 2, 7, 11, 11, 3, 9, 34, 5]` = 143, so
every arm of the table is reachable from real data. Only material **94** has no record — its key
is all zeroes and its sound id is the table's only `-1`, the same slot `SurfaceMap` clamps to.
(Worth noting: the `skater-collections.json` export is missing five further material records that
the vault itself has. Read the vault, not the export.)

**Correction to the `grind_gate_124` note below: it is not a cooldown.** Across every lifted
function, `+124` on `SFXObj_Contacts` is written in exactly two places — zeroed by the constructor
`sub_824B7CA8`, and set to `0.5` (`0x8209975C`) at `sub_824BB0E0`'s tail. **Nothing ever clears
it**, including `sub_824BB330` and the `sub_82489058` reset (which only touches `[p+0]`). Read
literally, retail plays the grind onset at most *once per component lifetime*. That is odd enough
that it is more likely a clear exists through a pointer the lifter obscures, so the port
deliberately **does not write the gate**: implementing it as written would silence the very sound
this work is adding. The read at `contacts.rs:667` stays harmless while the field stays 0.

### The sample chooser is ported, and a rail grind resolves (2026-09-20, last)

`sub_824965D0` -> `sub_824967F8` is a **table, not code**, and the last piece needed to read it was
the AttribSys *class layout*, which `skaterschema.vlt` carries as a per-field offset
(`tools/asset_pipeline/vlt.py`'s `fkey, typ, offset, ...`). With it, every offset the lifted code
uses resolves to a named field:

* The image table at `0x8302D6E8` gives each material a **kind word** (`+0`) and its record key
  (`+8`). The kind selects which *family* of fields the sample comes from -- and an AttribSys
  field's retail **type name is its bank**: `Skate_Collisions` -> `Skate_Collisions.bnk`,
  `Skate_Metal` -> `Skate_Metal.bnk`, `HOM_Set_1` -> `HOM_Set_1.bnk`. All three are in a stock
  `audiofiles.big`. Kind 0 covers 88 materials, kind 1 (metal) 49, kind 2 five (102..=106), and
  `-1` the single silent material 94.
* Within a family, `(tier, the paired material's class)` picks one of seven fields -- `tier == 2`
  ignores the pair; `tier == 0` and everything else take one of three by class.
* The paired class is `sub_82497910`: materials 95..=113 answer from a jump table at `0x82497944`
  (96 -> 2, `98/103/107/108/109` -> 0, the rest -> 1, and 110..=112 fall through), and everything
  else reads the `AudioSurfaceMap` word at `+28`, whose out-of-range clamp to element 94 is the one
  `SurfaceMap::lookup` already implements.

**Kind 2 is the tell that this reading is right.** It is the only family retail looks up *by hash*
(`sub_82B72420`) instead of by offset -- and the class layout says exactly why: those are the only
fields with no static offset at all. Its one hole, `tier == 0` against class 0, goes through
`sub_824825D0`, an accessor that is not decoded, so the port leaves that combination silent rather
than guessing.

**What a rail grind now resolves to**, measured against the owner's vault:

```
family 95: Skate_Collisions.bnk #879 level 28000  /  Skate_Metal.bnk #399 level 20000
family 96: Skate_Collisions.bnk #883 level 27000  /  Skate_Metal.bnk #401 level 20000
```

Two voices, one per material, each chosen against the *other* material's class -- the board's
family base out of the collisions bank and the rail out of the metal bank. That is why a rail
grind was never going to be one sound.

`Skate_Metal.bnk` and `HOM_Set_1.bnk` are now in `OPTIONAL_SPLICE_BANKS`. They need ffmpeg to
decode, which is not present everywhere, so a decode failure on an *optional* bank now logs and
skips instead of taking the whole player-sound path down -- without that, listing them turned a
missing ffmpeg into "no player audio at all".

**Still approximated, and worth revisiting:** retail opens these voices through `sub_82975700` and
keeps their properties current under `sub_824D2318` / `sub_82975A60`; the port uses the same Splice
one-shot path the pops and the landing already use, with the material record's `+52` level as the
voice gain. The spatialisation and the per-frame property updates are not ported.

**One more gap that must land with the rest:** the two `+0x20`/`+0x24` levels in the message are
still zero. They come from `sub_82496C58`'s interpolation (decoded above but not ported); nothing
the port reads uses them.

As of this commit the two dropped sounds also **report themselves once per run** through the
existing `SKATE_PLAYER_AUDIO contact_voice_unavailable` channel instead of disappearing silently.

---

## Suggested order (updated 2026-09-20)

**Three of the nine entries turned out to rest on premises the evidence does not support** — #1
(2026-09-20, earlier), then #7 and #3 here. A fourth, #2, is now in the same position. The pattern
is consistent enough to be a rule: **re-measure the premise before writing code against it.**

**One playtest answers two of these at once.** Play, ollie a few times, powerslide a few times,
with `SKATE_AUDIO_TRACE=logs\...`, then:

- `grep -o "AS st=[0-9]*" <trace> | sort | uniq -c` → does an ollie reach **201**? (#2)
- `grep "AS st=101" <trace> | head` → does the squeak gate pass during a real powerslide, and is
  there a `Class_Squeaks` `PO` on the same frame? (#3)

Both questions were unanswerable before this session because `AS` carried no state id; it does now.

1. **The playtest above** — it is the gate on both #2 and #3, and costs one run.
2. **#5 (surfaces)** — large, but the audio side is already ported and waiting, and it is the
   single biggest audible gap now that the impacts are measured-correct: every surface currently
   sounds like concrete.
3. **#9 (rail/grind landings)** — everything is decoded except the contact-sound manager's
   `vtable[+12]` dispatch (see above). That one subsystem unlocks rail landings *and* the
   continuous material-dependent landing weight in `player-audio-retail-drivers.md` §9. Land the
   `grind_gate_124` re-arm with it.
4. **#4 (ragdoll/loose board)**, **#6 (NaN crash)** and **#8 (renderer race)** — all need a repro
   before they can be chased.

Resolved and needing nothing: **#1**, **#7**, and the offboard static matching-group bug below.

## 10. Static geometry was hidden from offboard queries — **fixed 2026-09-20**

Not previously listed; found because two tests contradicted each other. `skate_world.rs` filed the
RWCM cluster's packed unit group on `QueryMesh::matching_group`. Since the filter is
`a == -1 || b == -1 || a == b`, that made every offboard ground and line query **reject static
geometry unless the querying actor's matching id happened to equal the cluster's group**.

Retail writes **-1** there: the mesh-record constructor `sub_8276CB18` takes matchingID in `r8`
(`stw r8,180`), and every call site passes a constant `li r8,-1` except one pass-through that
reads an actor/unit object's `+40` — never cluster data. The static path pairs that -1 with the
`li r7,0` / `li r6,0` this code already reproduced as `geometry: 0` / `rejection_flags: 0`. The
cluster group is per-*triangle* in retail: `sub_82AC8A68` stores it at triangle+84, beside the
surface code at +88 that `packed_surfaces` carries. The group is still used to partition clusters
into meshes; only the misfiling is gone.

Worth a playtest look: anything offboard (walking, bailing, ragdoll) that seemed to pass through
static world geometry may simply have been unable to see it.

## 11. Landing dynamics — **three port bugs fixed; the premise behind the old plan was wrong**

Filed 2026-09-20 after "landings all sound the same and loud". Everything below was re-measured,
because the recorded plan rested on two claims that did not survive the evidence.

### The two dead premises

**"Our air time is several times retail's, which is why the level saturates."** False. Retail's own
`+236` is recoverable from the recomp traces: `Class_Treatment` update word 7 is
`fctiwz(air × 1000)` clamped to 10000, i.e. milliseconds. Over `probe/traces/sessions/*`:

| trace | airs | p25 | p50 | p75 | max |
|---|---|---|---|---|---|
| `play4` | 48 | 383 | 583 | 700 | 1583 |
| `play1` | 23 | 516 | 633 | 716 | 1049 |
| `play2` | 5 | 616 | 816 | 1083 | 1366 |

The owner's own landings sit in the same band (0.08–3.5 s, median ≈ 0.5 s). Our air time is
retail's. **Defect #2 is not the cause of anything here.**

**"Retail's continuous landing weight comes from `sub_82496C58` / `sub_824D1E00`."** Also false.
`sub_824D1E00`'s weight input is derived purely from the message's *tier* words — 10000 / 20000 /
32767 — so it is a three-step quantizer, not a continuous weight. And the level curve saturates
sooner than this port's did (see below), so it cannot be the source of loud-vs-quiet either.

### What retail actually varies

**Retail does not make a landing louder for being harder.** Measured from
`.local/captures/retail-levels-20260920-004114.log` (music off, `OUT` peaks against
`Class_Treatment` word 7), 15 landings spanning 350–783 ms of air: peaks run −7.8 to +3.7 dBFS with
**no correlation to air time** — 733 ms produced both −7.8 and +0.8. The MixMap agrees: probing the
real graph under the retail pre-roll (`contacts_input_probe`), the landing class moves Contacts
output 15 only from 2584 to 3650 and output 3 from 6590 to 9309 — **3.0 dB each**.

What changes is the *sample*. `sub_824BA630` starts three voices, and two of them pick by air time:

| voice | slot | sample chosen by | level |
|---|---|---|---|
| impact | `+56` | fixed (`0x447`) | fixed |
| **ladder** | `+496` | `sub_82494D78(deck material)` × `air ≥ 0.75 s` → `0x35C`…`0x35F` | fixed |
| class | `+52` | landing class (0.62 s / 1.02 s) via `sub_824BA3F0` | fixed |

### The three bugs fixed

1. **The ladder voice was never played.** `LandingTuning::ladder` / `ladder_sample` existed in
   `splice.rs` with no callers anywhere — dead code since it was written. Every landing played one
   fixed impact sample where retail picks between four. This is the one most likely to be audible.
2. **The level curve ran on seconds, not retail's ratio.** `sub_824BA630` @ `0x824BA7D0` computes
   `clamp(air / D, 0, 1)` with `D` = `Hash_6D68BC2D1A23C29A` = 0.4, *before* the tier test and both
   windows. The port passed raw seconds, stretching the curve 2.5×: retail's tier boundary is
   **0.04 s** of air and its ceiling **0.12 s**, not 0.1 and 0.3. Note the direction — fixing this
   makes the contact levels *more* uniform, because retail's really are.
3. **Two vault post-gains were missing, and the level/material pairing was crossed.** Each level
   word is scaled before it reaches the message (`fmuls`/`fctiwz` @ `0x824BAAC0`, `0x824BAB20`):
   `Hash_31DEEF8FA219950F` = 0.65 on the first, `Hash_0EC6EEF5366FEA85` = 1.0 on the second. And
   `sub_82486EF0(this, r28 = board, r27 = surface, …)` pairs `msg[+0x20]` with the **surface**
   while `msg[+0x00]` is the board; `sub_824D2318` reads `msg[0x20 + 4 × record]`, so record 0 (the
   board) is leveled by the surface's table entry. The port had them uncrossed.

### The level mechanism — found on a second pass, after a wider playtest

**The "retail keeps the gain flat" reading above was drawn from too narrow a sample** (15 landings,
all 350–783 ms, i.e. all class 0/1). A 35-landing capture spanning 16 ms to 1.2 s says otherwise:

```
corr(peak, air time) = +0.64        peak range 15.8 dB
  air    0-150 ms  n= 4  mean  -4.0 dBFS
  air  150-400 ms  n=12  mean  -4.4
  air  400-700 ms  n= 8  mean  -4.6
  air  700-1050ms  n= 5  mean  +1.0     <- +5.5 dB step
  air 1050+   ms   n= 6  mean  +2.3     <- +6.8 dB
```

Flat to ~0.7 s, then a step — landing exactly on the **landing-class** thresholds (0.62 s and
1.02 s; air factor 0.31 / 0.50 against factor = air × 0.5).

**`sub_824BA630`'s tail writes the class to Contacts controller input 2** — `[this+12]->vtable[8]
(2, clamp(word, 0, 32767))` with word = 0 / 16000 / 32767 for class 0 / 1 / 2 (@ 0x824BAF08 and
0x824BAF44). Input 2 drives output 15, the landing voice's owner send. **This port never wrote any
Contacts controller input**, so output 15 sat at its class-0 value forever: the class picked a
different sample but had *zero* effect on level. That is the direct cause of "every landing the
same loudness", and the port said so itself — `sub_824B90D8 without the controller-input resets`,
and "raises controller input 1 (the MixMap port owns the input)", which it never did.

Also recovered: `sub_824B90D8`'s head zeroes inputs **0, 1 and 6** every frame and pointedly *not*
2 — the class latches until the next landing, holding the send up for the voice's whole life.
`sub_824BA630`'s head raises input 1 to 32767 (the landing pulse).

**Known approximation.** Retail routes the class voice through the send bus, so its level follows
output 15 continuously as the MixMap settles. This engine gives the one-shot a fixed gain at open,
and reading output 15 back would be both a frame stale and pre-settling, so the class indexes the
*settled* measured values (2584 / 3103 / 3650) instead. Retail's send ramps across the sample;
ours is flat for its length.

### Still open

The measured retail step is ~6 dB but the MixMap supplies only 3.0 dB of it, so the remaining ~3 dB
must come from the class samples themselves being louder. Worth checking the three class samples'
own levels before adding any gain. Also unexplained: the 4–10 dB spread *within* a single air-time
band, which the owner hears as slope-vs-flat — a slope landing has lower vertical impact speed, and
`sub_82772B88`'s `landing_bucket_44` (|min of last four COM vertical velocities| against
1.5 / 2.6 / 3.45 m/s) is the obvious candidate, but in this port it reaches only the footsteps
packet. Where retail takes it beyond that is not yet traced.

**Do not** re-open this by widening the `[0.1, 0.3]` window or removing the `/0.4`: that window is
retail's, it is in ratio units, and it is meant to be spent by 0.12 s of air.

### The landing's two collision voices are off by default — a deliberate deviation

`sub_824BA630` posts to the contact-sound manager, so retail plays them. But `sub_824D2318` gives
a contact voice `controllerOutput × messageLevel × materialLevel`, the controller output coming
from `sub_824D20E8`'s jump table (material category → Collision controller output: 13, 14, 15, 16,
17, 18, 12, 19, 21, 20 for categories 0..=9). **This port applies the two level terms and not the
controller one.** It is not recoverable yet: the one-shot path has no per-frame gain update, and
every one of those outputs reads 0 in the only MixMap fixture available
(`crates/skate-audio-core/examples/collision_output_probe.rs` against `mixmap_replay_4400.bin`).

At full material level they swamp the class voice. Solving the mix from a playtest,
`(C + 1.95X)/(C + X) = 1.05`, puts the class voice at **~5 % of the landing peak**, and the class
step measured +0.5 dB against retail's +4.8. A/B'd by the owner at the same trim: with them on,
"low ollies still sound too loud and different than retail"; with them off, "pretty close to
correct". So they default **off**, because a voice at a knowingly wrong level is further from
retail than no voice.

**Corrected 2026-09-23: this is stale.** Commit `0bfcb22` flipped the default back **on** when
`collision_controller_scale` supplied the missing controller term from retail's captured reads,
so today the switch reads `SKATE_AUDIO_LANDING_COLLISION=0` to turn them *off*
(`contact_voices.rs:377`). That median table is itself a stand-in -- see defect 13 below, which
recovers the authored values it approximates.


---

## 10. Tricking out of a darkslide killed the session -- **fixed 2026-09-23; the state is inferred, not recovered**

Playtested 2026-09-23. Entering a darkslide is fine; throwing a trick *out of* one ends the
process with

```
Physical state transition GrindDarkslide -> PhysicsAirSecondary requires its native
Enter/Exit production adapter; state=GrindDarkslide; tick=3380
```

**The chain, all five links confirmed.**

1. `MotionGraphIncludes/onboard.xml`'s `DarkSlideTrick` state -- gated on `HasIntent Trick` **and**
   `IsDark` **and** `PhysFilteredState state="grind"` -- runs
   `<behaviour name="CreateAttribute" attName="GrindTrick"/>` and forces the board to
   `FORCE_ANIM_SKATEBOARD`.
2. `animation/skeleton_input/extended_attributes.rs:147`: `"GrindTrick" => flags2484 |= 0x100000`.
3. `player/selector/air.rs:138`, `select_grind`: `p.has_2484(0x10_0000)` returns
   `PhysicalStateId::PhysicsAirSecondary` -- checked second, right after wipeout, so it wins over
   every other grind exit.
4. `physics/player_state/registry.rs` does not list `PhysicsAirSecondary` among its supported
   states, so `can_transition` is false.
5. `physics/player_state/transition.rs:92` turns that into an `Err`, which `GamePhysics` treats as
   fatal.

**It is darkslide-specific**, because `IsDark` gates step 1 -- the first occurrence in 67 committed
playtest logs, which is why it survived this long. Ordinary grind trick-outs never set the bit.

**What it blocks.** The whole darkslide-out family, scorables 321-330
(`darkslideout_bsstraight/bsleft/bsright/fsstraight/fsleft/fsright`, `darktolightbsshuv`,
`darktolightfsshuv`, `darktolightheelflip`, `darktolightkickflip`). All ten are named by the
compiled graph and carry authored points -- see `docs/trick-reachability.md` -- so the only thing
missing is the physical state their exit lands in.

**What the fix needs.** `PhysicsAirSecondary` is state 202 with its own native lifecycle object at
`PhysicalPlayer` offset **1716**, distinct from `PhysicsAir` 200 at 1712 (`player/state.rs`'s
`native_owner_offset`). Its selector contract is already ported and is narrow: it is entered only
while `flags_2484` bit 20 is set, and `selector/mod.rs:150` leaves it for `PhysicsAir` as soon as
that bit clears, or for `WipeoutGround` on a wipeout. So it is a short, animation-driven air state
-- the board is `FORCE_ANIM_SKATEBOARD` throughout -- rather than a second run of the air solver.

Porting it means the registry entry, the `frame.rs` dispatch arm, the `transition.rs` Enter/Exit
assertions, the `pre_state`/`publication`/`wipeout` arms, and a runtime module. **The one thing
this repo does not contain is the native Enter/Update/Exit behaviour for the object at 1716** --
no vtable or method addresses for it appear anywhere, unlike `PhysicsAir200`
(`82D34388`/`82D346C0`/`82D346A0`, cited in `physics/air_phase.rs`) or `RevertGround102`
(`vtable 82327330`, cited in `physics/revert_state.rs`). That has to come from the binary before
the state can be written to this port's standard; guessing it would produce air physics that looks
right and is not.

### Fixed by sharing PhysicsAir200's lifecycle -- and what is still unproven

`PhysicsAirSecondary` is now wired at every seam it needs: the registry capability and
transition pairs, the Enter/Exit dispatch, `frame.rs`'s per-tick advance, `pre_state`'s
PredictFutureOfDeck list, `publication`'s Fill, and the wipeout probe. At all of them it runs
**the recovered PhysicsAir200 lifecycle**, and that identification is an inference rather than a
recovery -- there is no S2 debug build naming an S3-only state, which is the same reason the
sixth (darkslide) grind family was the last thing ported.

Five checks support it, all re-checkable from the tree:

1. Owner offsets are adjacent, 1712 and 1716, which is how a second instance of one class
   appears rather than a different class.
2. `air_phase::enter` already branches on `previous_physics_category_2516 == 400`, the grind
   family, so entry to air out of a grind was already a recovered path.
3. Both 200 and `GroundAnimation103` enter through `enable_angular_only`, so this lifecycle does
   not fight the `FORCE_ANIM_SKATEBOARD` the authored state sets. The crash line recorded
   `force_mode=2`, which is exactly that mode.
4. `selector/mod.rs` treats 202 as transient -- it leaves for 200 as soon as `flags_2484` bit 20
   clears, and for `WipeoutGround` on a wipeout.
5. 202 is only ever entered while the board is animation-driven, so any solver difference
   between the two native objects has nothing to act on.

**What would disprove it:** 202's native Update or Fill writing different PhysOut fields than
200's. The symptom would be wrong air metrics, scoring or audio on darkslide exits -- never a
crash. If a Skate 3 build with symbols ever turns up, this is the first thing to check, and the
reasoning above is recorded in `player_state/transition.rs` at the shared arm so it is found
from the code rather than only from here.

---

## 11. Skitching is unported, and deliberately left that way — **decided 2026-09-23**

State 104 has no adapter, so `registry.rs` pins it alongside `FollowPath` as the two remaining
unported states. That is a decision, not an oversight.

**Nothing is recovered.** Only two fragments carry native addresses -- `82D8BBB8`
(`condition_is_off_ground_skitching`) and `82BBBC88` (the `state == 104` graph query). There is no
Enter/Update/Exit for the object at offset 1772. The graph half is stubbed to match:
`EnterSkitchingBehaviour`, `SkitchingBehaviour` and `SkitchShimmyingBehaviour` parse but have no
execute arm, and `IsSkitchShimmying`, `IsSkitchingWithAbsorb` and `SkitchingPosition` are three of
the seven unimplemented graph conditions.

**Nothing can enter it.** Entry needs `flags_2476` bit 21 (`is_grabbing_object_72_304`, itself from
the riding state's `flag_2729`) together with `flags_2480` bit 22, and `skitch_value_40` is the
grabbed object's id. The living-world vehicle AI that would supply one is entirely unported --
`ContinueFollowingLane`, `WanderOnRoad`, `ContinuePullingOver` and the rest have no
implementation -- and `MovingObjectRegistry::register("ram:/world/objects/car")` appears only
inside a unit test. The `skate-vehicles` SDK is a separate simulation the player rides; it has no
grab or skitch interaction.

**Why not infer it the way `PhysicsAirSecondary` was.** That worked because the inference was both
constrained and *checkable*: adjacent owner offsets, an already-recovered air lifecycle that fit,
a transient animation-driven state with little physics to get wrong, and a crash that stopped
happening. Skitching has no sibling at 1772, is long-lived and physics-active, and -- decisively
-- cannot be entered, so an inferred implementation could never be shown right or wrong.

**Do it with the Skate 2 symbols instead.** Skitching is an S2 feature, so unlike the S3-only
`PhysicsAirSecondary` the debug build that named the rest of this port names it too. Port it when
there is traffic to test against, and take the names from there rather than guessing.

---

## 12. Heavy landings were silent on three surface categories out of four — **fixed 2026-09-23**

Filed after the owner reported that landings "all sound alike" and that *this port's* low ollies
sound unlike retail's while its bigger drops sound about right.

### Measured first, both sides

Retail's own landing curve, from `.local/captures/retail-lowollie-20260920-205434.log` — `OUT`
block peaks over the twelve frames after `Class_Treatment` word 7 returns to zero, binned by the
air time that word carried (35 landings):

| air ms | n | retail mean dBFS |
|---|---|---|
| 0–120 | 4 | −8.0 |
| 120–500 | 16 | −5.2 |
| 500–1000 | 6 | −2.5 |
| 1000+ | 9 | **+2.0** |

**Use means, not p50s, and do not over-read them.** Retail's peaks scatter about **8 dB inside a
single air-time band** — the 1000 ms+ band alone runs −2.8, −2.0, −1.2, −1.2, +2.3, +4.3, +4.6,
+6.8, +7.0 — so a p50 over the three or four landings a narrow band holds lands near its maximum.
An earlier revision of this entry binned narrowly and quoted p50s, which put the top band at +6.8
and overstated the shortfall below by about 5 dB.

This engine, through its own worker over the same rungs
(`player_audio/headless.rs::headless_landing_ladder_matches_retails_curve`, native six-channel
peak — the meter the recomp's output pass logs, so the two compare directly):

| air ms | this engine | retail | error |
|---|---|---|---|
| 50 | −7.1 | −8.0 | +0.9 |
| 100 | −4.7 | −8.0 | +3.3 |
| 150 | −4.7 | −5.2 | +0.5 |
| 250 | −4.7 | −5.2 | +0.5 |
| 450 | −4.8 | −5.2 | +0.4 |
| 800 | −2.8 | −2.5 | −0.3 |
| **1200** | **−3.7** | **+2.0** | **−5.7** |

Every rung is within about 1 dB except the two ends. The range is the problem: **4.3 dB across the
whole ladder against retail's 10 dB**, and a 1.2 s drop comes out *quieter* than a 0.8 s one when
it should be louder. That is what "all landings sound alike" is.

The rungs feed retail's own median jump height per band (`Class_Treatment` word 9 =
`height × 166.667`: 0.28 / 0.37 / 1.00 / 1.81 m), because holding it constant mismeasures the
heavy end. Doing so changed no rung by a measurable amount, which is itself a result: the
Treatment layer is not what carries a landing's weight here.

### The bug: `sub_824BA3F0`'s mode mask was inverted

`LandingTuning::class_sample` chose the mode array as `if class >= 2 { category } else { 0 }`.
Retail does the opposite. From the lifted code at `0x824BA420` (`skate3_recomp.11.cpp:25062`):

```text
cmpwi  cr6,r31,2        ; r31 = kind
bgt    cr6,loc_824BA444 ; kind > 2 skips the mask entirely
li     r11,2
subfc  r9,r11,r25       ; r25 = class; carry = (class >= 2)
subfe  r7,r10,r8        ; r7 = ~0 + carry -> 0 when carry, 0xFFFFFFFF when not
and    r3,r7,r3         ; category &= r7
```

`and` with 0 **clears** the category, so a class-2 landing reads mode 0 and the lighter classes
read their category.

It mattered because `Skate_Collisions.bnk` authors the class-2 slot in mode 0 only. Modes 1, 2 and
3 point at containers 204, 193 and 215, whose records (`0x02a0..=0x02a2`, `0x0277..=0x0279`) hold
**zero groups and a zero duration** — 27 such placeholder records in nine runs of exactly three,
out of 871. With the polarity inverted, every heavy landing on categories 1, 2 and 3 resolved to
one of those and played **nothing**. Measured through
`crates/skate-game/examples/landing_sample_levels.rs`, before and after:

| | class 2, cat 0 | cat 1 | cat 2 | cat 3 |
|---|---|---|---|---|
| before | 6 members, +12.2 dB | **0 members** | **0 members** | **0 members** |
| after | 6 members, +12.2 dB | 6, +12.2 | 6, +12.2 | 6, +12.2 |

`class_sample` had **no test**, which is how it survived; there is one now
(`a_class_two_landing_reads_mode_zero_and_the_lighter_classes_read_their_category`).

The headless ladder above does **not** move, because its fixture sits on category 0 — the one
combination that was already correct. The fix bites in play, where the per-wheel material varies
(University traces show ids 1, 3, 4, 5, 8, 16, 17, 41, 42, 53, 66, 79). Extending the ladder to
sweep categories is the obvious follow-up, and would have caught this.

### Still open: the remaining ~8 dB of the class step

With the polarity fixed the ladder is unchanged, so the 5.7 dB shortfall at class 2 is a separate
problem. What it is **not**, each checked:

* Not a missing class. The class resolves 0/0/0/0/1/2 across the rungs (the landing diagnostic now
  prints it).
* Not missing voices. `SKATE_AUDIO_VOICE_TRACE=1` counts 71 opens at the class-2 rung against 72
  at class 1.
* Not the member delays. The class-2 container's members draw 0–154 ms against samples 475–890 ms
  long, so they do overlap.
* Not extra retail voices. `sub_824BA630` calls `sub_824B8D48` exactly once.
* Not the Treatment layer — see the jump-height note above.

What it looks like instead is **a ceiling on the mechanism**. A landing is four or five voices of
similar level, and only one of them — the class voice — varies with the class. Its own variation
is the member-gain step (0.838 → 0.842, i.e. +0.04 dB on the loudest member; +1.1 dB on the sum)
plus `landing_send_scale`'s +1.4 dB, so about **+2.5 dB on one voice out of four or five**, which
comes out under +1 dB at the output. Retail moves the whole landing by ~10 dB. So retail's step is
carried by something this port applies flat, and neither the send (a 3.0 dB spread, probed) nor
the samples (a 5.8 dB spread, measured) is big enough on its own.

**Use RMS, not peak.** Retail's landing *peak* scatters 25.8 dB inside a single air band, so it
cannot resolve a 5 dB effect. Its block **RMS** — the `rms` field of the same `OUT` line, which
`trace::output` also writes — is clean and monotonic:

| air ms | n | retail RMS | this engine (mean of 6 landings) | error |
|---|---|---|---|---|
| 0–120 | 4 | −22.9 | −20.5 / −19.5 | +2.4 / +3.4 |
| 120–500 | 16 | −20.8 | −19.5 | +1.3 |
| 500–1000 | 6 | −17.4 | −17.9 | −0.5 |
| 1000+ | 9 | **−12.8** | **−18.8** | **−6.0** |

Retail rises 10.1 dB from the lightest band to the heaviest. This engine rises 2.6 dB and then
*falls* at class 2. The 500–1000 ms band matches within half a dB, so the path is not broadly
wrong — the heavy end is.

**Retail's own send is measured and this port already matches it.** The capture records 70 `VF`
reads by `0x824B8E88` — `sub_824B8D48`'s read of the landing voice's owner send, two per landing.
Matched to each landing's class by air time:

| class | n | min | p50 | max |
|---|---|---|---|---|
| 0 | 33 | 2572 | **2584** | 3650 |
| 2 | 13 | 2578 | **3646** | 3650 |

(The stray extremes are the second read of the pair, taken across the frame that writes input 2.)
That is exactly `LANDING_SEND_BY_CLASS` and exactly what `contacts_input_probe` measured, so the
send path here is correct and is **not** where the missing step is.

**The class step the authored data can actually supply is about +1.4 dB, and that is the whole
problem.** An earlier revision of this entry claimed the content steps +6.5 / +11.2 / +12.2 dB
across the classes, measured over 400 draws. That was an artifact: a fresh `SpliceState` always
picks the same kid of a container, so 400 draws measured one kid 400 times. What matters is the
mean over a *live* state, and by kid count the class-2 container is **not** the louder one:

| class | container | kids | children per kid | mean |
|---|---|---|---|---|
| 1 | 181 | 11 | 5,6,5,5,6,7,7,6,6,5,5 | **5.73** |
| 2 | 182 | 4 | 6,5,5,5 | **5.25** |

Confirmed in the game path: `SKATE_AUDIO_VOICE_TRACE=1` counts **41.7** voice opens per landing at
the 800 ms rung against **39.8** at 1200 ms — class 2 opens *fewer*, despite its script being
longer. So the only real class step is `landing_send_scale`'s +1.4 dB, partly cancelled by the
smaller container, which is why the measured class 1 → class 2 step is −0.9 dB.

**Retail gets +5.4 dB there (−17.4 → −12.8) from something this port does not model.** Neither the
send (3.0 dB across all three classes, and measured as correct above) nor the samples (no step)
accounts for it. Ruled out along the way: the MixMap (sweeping Contacts input 2 from 0 to 32767
raises outputs 3 and 15 and pulls *nothing* down); the measurement window (a 1.0 s RMS window puts
class 1 and class 2 equal at −25.8, so it is not energy spread over longer samples); the record
layout (the unparsed record words +0, +28 and +32 are all zero, so `value_range` really is 0 and
the container value is a constant 1.0); and delayed members being dropped (`drain` collects every
handle into `self.live` and ticks them).

The remaining candidate is `sub_824BA630`'s own call to `sub_82486EF0` — the contact message whose
`+0x20`/`+0x24` level words this port leaves at zero (`contact_voices.rs:726-730`). That is the
one term in the landing that is both unported and per-landing. Recover it before touching any gain.

**The arithmetic of the dilution.** A landing's voice gains here, with `CONTACT_TRIM` 0.625 and
`LANDING_TRIM` 0.274 both applied:

| voice | class 0 | class 2 |
|---|---|---|
| fixed impact `0x447` | 0.124 | 0.124 (constant) |
| ladder `0x35C`…`0x35F` | ~0.12 | ~0.12 (constant) |
| class voice | 0.759 × 0.708 × trims = **0.092** | 0.842 × 1.000 × trims = **0.144** |
| two collision voices | 0.030 / 0.050 | 0.030 / 0.050 |
| **sum** | **0.336** | **0.388** |

The class voice's own step is a healthy +3.9 dB, but it is under a third of the stack, so the total
moves **+1.2 dB** — which is what the ladder measures, to a tenth of a dB. The two constant voices
are what hold the step down, and the same two are what make the 50–100 ms rungs 0.9–3.3 dB *too
loud*. One cause, both ends of the error.

So the question is why retail's constant voices do not dominate its landing the same way. Note
that `sub_824BA630` makes **no** `vfunc60` read of its own in the capture — the only landing-time
reads are `0x824B8E88` (the class voice) and `0x824B9E48` (the pop) — so retail's impact and
ladder voices take no owner send either, and should be at their authored member gain just as they
are here. The next measurement is therefore the **member resolution**: container 182 has four
record kids and this port resolves six members from it, so check `pick`'s mode-2 selection and
`plays()` against `sub_82975A60` / `sub_829757D0`, and confirm how many members retail actually
opens for the impact and ladder containers versus the class one. `LANDING_TRIM` is a blanket
constant over all three and is the obvious thing to be wrong if retail's are not equal.

## 13. The contact bus has no emitter, so every contact voice takes a constant level

`sub_824D2318` gives a contact voice `controllerOutput × messageLevel × materialLevel`, and this
port supplies the controller term from `collision_controller_scale`'s table of medians scraped
from retail captures, because "every one of these outputs reads 0" in the live runtime.

**Why they read 0, found 2026-09-23.** `mixmap_dump`'s dependency walk says `SFXObj_Collision`'s
outputs depend on `40010010.7`, `60010000.9`, `60010800.9` and `60030000.99` — and *not* on the
Collision controller's own inputs 0/1, which is why `collision_output_probe`, which sweeps exactly
those, has always seen zeros. A new probe
(`crates/skate-audio-core/examples/player_input_probe.rs`) drove each candidate against the real
MixMap under the retail preroll:

* Contacts inputs 7 and 8, Collision inputs 0 and 1, and the remote player's state id 9 move
  **nothing**.
* VU id 0 — which `FREE_SKATE_MUSIC_VU` pins at 32767, and which was suspected of holding the mix
  down — moves `SkateBoard level(7)` from 858 to 861 across its whole range: **0.03 dB**. Ruled
  out; leave the pin alone.
* **Bit 0 of input 15 of the Collision object's own 3D-position controller `60030000`** switches
  the entire bus on. `ObjPos::update` (`mixmap/inputs.rs:517-529`) or-s that bit in when the
  emitter has a position. **This port never creates a 3D emitter for the collision object**, so
  the bit is never set.

With it set the ten category outputs come alive, and they are **distance-attenuated** —
attenuation contact sounds in this port do not have at all:

| distance | cat 0 (out 13) | dB vs near field |
|---|---|---|
| 0–1 m | 10325 | 0.0 |
| 3 m | 9118 | −1.1 |
| 10 m | 5277 | −5.8 |
| 30 m | 402 | −28.2 |
| 100 m | 0 | silent |

Near-field values against the medians they would replace. The two agree on shape, and the medians
read about 1.3 dB low because retail's captured reads average over distance — which is itself
evidence the attenuation is real:

| cat | out | probe @0 m | median stand-in | delta |
|---|---|---|---|---|
| 0 | 13 | 10325 | 8869 | +1.3 dB |
| 1 | 14 | 24657 | 22153 | +0.9 dB |
| 2 | 15 | 9202 | 6079 | +3.6 dB |
| 3 | 16 | 9202 | 7923 | +1.3 dB |
| 4 | 17 | 16365 | 14669 | +1.0 dB |
| 5 | 18 | 9202 | 7923 | +1.3 dB |
| 6 | 12 | 9202 | 8126 | +1.1 dB |
| 7 | 19 | 9202 | 7923 | +1.3 dB |
| **8** | **21** | **4110** | **7923** | **−5.7 dB** |
| 9 | 20 | 9202 | 7913 | +1.3 dB |

**Fixed when** the contact-sound manager is given a real emitter. `contact_voices.rs:726-730`
already records that the contact message's world-position field is left at zero, and that same
position is what `ObjPos::update` needs. Drive `60030000` from it each frame the way `sound.rs`
drives `SKATER_POSITION_CONTROLLER` / `BOARD_POSITION_CONTROLLER`, then read the category output
in `collision_controller_scale`'s place and delete the table.


## 14. The landing class step, re-measured properly — **real, and the only rolling/landing defect left**

Defect 12 measured this from one capture with wide air-time bins and means. Five other findings in
this project died to exactly that method (see `player-audio-retail-drivers.md` §11's note). This
one survives it.

**Method.** 60 landings pooled from three captures — `retail-lowollie-20260920-205434.log`,
`retail-nomusic-20260920-082353.log` and `retail-levels-20260920-004114.log` — taking the **median**
block RMS in the twelve frames after `Class_Treatment` word 7 returns to zero. The engine's column
is `headless_rolling_speed_and_surface_sweep`'s sibling, the landing ladder, at the same window.

| air ms | n | retail median | this engine | error |
|---|---|---|---|---|
| 0–150 | 5 | −22.6 | −20.5 | +2.1 |
| 150–350 | 6 | −18.3 | −19.5 | −1.2 |
| 350–600 | 15 | −19.3 | −19.4 | **+0.1** |
| 600–900 | 25 | −14.1 | −17.9 | **−3.8** |
| 900–1300 | 9 | −12.0 | −18.8 | **−6.8** |

Retail spans **10.6 dB** from its lightest band to its heaviest; this engine spans **2.6 dB**. The
mid-range matches to a tenth of a dB, so nothing is broadly wrong — the deficit is entirely at the
heavy end and grows with air time.

**What the authored data can supply, and why it is not enough.** The send is +3.0 dB across the
three classes and is *correct* — retail's own reads at `0x824B8E88` measure 2584 at class 0 and
3646 at class 2, exactly what `LANDING_SEND_BY_CLASS` encodes. The sample content supplies no step
at all: by container kid count the class-2 container averages **fewer** members than the class-1
one (5.25 against 5.73), and the game path confirms it — 39.8 voice opens per landing at class 2
against 41.7 at class 1. Net authored step is about +1.4 dB, partly cancelled. Retail moves the
landing by 8–10 dB.

**Ruled out** (each measured, not argued): the MixMap (sweeping Contacts input 2 from 0 to 32767
raises outputs 3 and 15 and pulls nothing down); the measurement window (a 1.0 s RMS window puts
classes 1 and 2 equal); the record layout (the unparsed record words are all zero, so the container
value really is a constant 1.0); dropped delayed members (`drain` collects every handle and ticks
them); and the Treatment layer (feeding retail's own per-band jump heights changed no rung).

**Where to look next.** `components/contacts.rs:398` lists as unported *"the landing voices past
the first"*, with vault fields `F262042EAA295711` = 860 and `1E86469556ACD80A` = 862 — that is
`0x35C` and `0x35E`, the **ladder** samples — behind thresholds 0.1 / 0.3 / 0.65 / 0.75 / 1.0 in
landing-weight units. Five thresholds on a term that currently plays one voice is the right shape
for a missing 7 dB. **But note the call structure does not obviously support it**: in the lifted
`sub_824BA630`, `sub_824B8D48` and `sub_82486EF0` each appear exactly once, so any extra voices are
not started by a loop there. Read the function properly before acting; do not infer voice counts
from call counts.


## 12. The flip ladder stops at four — **working as authored; extended on purpose by a mod**

**Symptom.** A held kickflip or heelflip never reaches a fifth flip, however much air the pop has.

**Evidence.** Not a defect. The ladder is authored, not computed: there is no flip-count variable
and no clamp to 4 anywhere in the Rust. `T_Kickflip.xml` authors `Cyc1`/`Cyc2`/`Cyc3` and no
`Cyc4`, `Cyc3`'s only exit is a bare `WillExpire InTime="0.05"` to `End.Out.Out4`, and `Out4`
carries no precondition at all — unlike `Out1`/`Out2`/`Out3` and their hold / `TimeToLand` /
`IsBodyFlipping` gates. Confirmed by suppressing `WillExpire` on `_CYC3` behind a throwaway env
var: `B_KICKFLIP_OUT4`, `B_AIR_CYC` and `BLEND_LAND` all vanished and the reward went to zero.

### Extended by the Endless Tricks mod — a deliberate deviation

Since 2026-09-24 an opt-in mod appends a self-transition to each of the six authored flip cycle
states, adding rungs 5..16. It is **off by default** and the retail path is unchanged: the four
pre-existing flip tests pass with identical rewards while the new transitions are compiled into the
graph. `crates/skate-core/src/scoring/{catalog,conversions}.rs` and `SCORABLE_COUNT` are untouched —
the extra rungs borrow the retail quad's ledger identity and override only name, label and points.

Full account, including the measured flip counts and the air budget: `docs/endless-tricks.md`.

**Do not** read this as licence to extend other authored ladders. The late flips were asked about
and settled separately (`docs/flip-ladder.md`), and retail-exactness still governs everything that
claims to *be* the port.

**What would disprove the extension being inert:** any of the four stock flip tests changing
reward, trick sequence or landed name while `endless_flips` is false.
