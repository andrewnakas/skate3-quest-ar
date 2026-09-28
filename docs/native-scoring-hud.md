# Native scoring and HUD port

Work branch: `lol/native-scoring-hud`. This document distinguishes implemented
components from remaining integration; it is not a claim of gameplay parity.

## Evidence

The source executable is the locally owned TU3 image loaded at `0x82000000`,
size `0x011B0000`, SHA-256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
Static analysis uses a disposable IDA database and the existing generated PPC
translation. Original binaries, analysis databases and extracted assets are
not part of this repository. IDA incorrectly decodes some VMX instructions;
PPC instructions and the generated translation must corroborate those paths.

Implemented source components:

* `skate-core::scoring`: ScoreHolder accounting at `82DA6198`, `82DA6260`,
  `82DA6408`, `82DA6468`, `82DA6538`; point timers at `82DA4C28`; carrier
  announcement/completion at `82DA45C8`, `82DA46D0`, `82DA5DE0`, `82DA5F98`.
  Session publication at `82DA37B0` captures the multiplier before crediting
  this reward to the combo timer. Settlement at `82DA3B38` preserves repetition
  on an empty line timer while a collector is active.
* `skate-data::scoring`: resolves authored VLT points, labels, delay, repetition,
  announcement curves, combo thresholds and timer settings. Missing data is
  an error. The executable's 332-entry identifier metadata includes unused
  entries; the owned installed database resolves 300 records.
* The scoring display/timer defaults belong to class `349215E2E817703C`.
  They are global tuning, not user difficulty settings. Physical collector
  tuning is a separate class `546C36B656038E04`.
* `tools/prepare_hud.py`: decodes the original `hud2/trickdisplay2` APT,
  geometry, font metadata, textures and compact action stream into a private
  cache. Source asset hashes are retained in the compiled manifest.
* `apt_vm`: partial bounded interpreter for the original compact instructions.
  The data-only audit initializes the original class, builds the authored
  initial child hierarchy and checks the constructor's four trick-name
  visibility writes. Native method calls still use a dummy host, so this does
  not validate a functioning display.
* `apt_display`: retained depth-list placement flags and transforms. The
  data-only audit validates all controls across 38 original movie timelines.
  Unsupported clip actions and filters fail explicitly.

## Extractor provenance

`tools/vendor/skate3_ui` reuses the Python extractor from the local Custom
Engine Layer preview.18 source distribution. Its accompanying project MIT
license is preserved. `actions.py` is the new compact instruction decoder,
based on TU3 `_parseStream` at `82E67868` and dispatch table `82FC9BF0`.
Extracted game assets are not covered by that source license and must stay in
the ignored private output directory.

Example preparation (data extraction only):

```powershell
python tools/prepare_hud.py --game <owned-game-directory> --output assets/private/hud --collections <owned-assets>/private/stock/skater-collections.json
```

The original movie is authored at 1280 by 720. Its score, line, stance,
multiplier and trick-name timelines must control presentation; replacing them
with a new overlay would not meet this port's requirements.

## Production integration and validation

The production fixed tick now supplies animation descriptors, conditioned category,
grind IDs, landing output and physical motion to `scoring_runtime`. The runtime
loads native VLT points, repetition, announcement and collector curves, maintains
carriers and continuous distance rewards, and publishes sequence/line accounting.
It is a usable integration for testing, **not a finished native-parity port**.

The HUD executes the owned trickdisplay ActionScript and timelines, including
score, multiplier, line meter, stance and trick-name updates. It renders original
shape atlases and font glyphs through a separate camera with inherited
multiplicative/additive colors. Movie objects are collected and mesh/material
slots reused. Fixed updates pause with gameplay; map-generation changes recreate
the movie even while paused. Missing assets or unsupported actions are logged.

Font resolution follows FontManager initialization `82808AE8`, APT-name lookup
`82809208` and loading `82809308`. VLT class `FECFBCAF356518C4` maps AptName to
FileName; `FuturaOuterGlow` resolves to `futurashadow`. Glyph advance and placement
use native scale/ascent metrics. English text resolves through the owned language
asset. No replacement interface or bundled game assets are introduced.

Validation on 2026-09-08:

- All nine scoring unit tests pass, covering clock wrap, early completion,
  conversion, cancellation, repetition, timer thresholds and one-time banking.
- The data-only scoring-flow example credits the authored 100-point stationary
  kickflip once and verifies idle stability and teleport cancellation.
- The original HUD data-only audit passes 1,800 frames, with 172 VM slots,
  116 display instances, at most 33 draw batches and finite geometry.
- The release target builds successfully with static MSVC CRT, without default
  features. Compiler warnings remain in the existing game and audit-only code.
- No game, recomp, controller harness, gameplay automation, `--check-assets`
  or screenshot capture was run. GPU rendering and interactive behavior remain
  unverified until the user launches the build.

The copied build and launcher are in ignored `logs/scoring/build`. The launcher
uses this worktree's HUD cache and the existing owned asset installation and
starts paused. It checks required paths before running and retains failures on
screen. The executable SHA256 is
`59c7c876d289c58e9661f669274fa4faa4e50a7bf51085461bcf33f2fc1bd809`.
The launcher now selects `skate3rust-hud-animation.exe`.

The missing-HUD startup defect is fixed: HUD setup now depends on presentation
setup, so Bevy applies the deferred camera spawn before the HUD queries it.
Previously the two PostStartup systems were unordered and a missing camera
caused HUD initialization to return permanently. The rebuilt launcher also
saves runtime output to `logs/scoring/build/scoring-test.log`. This fix is
release-compiled; interactive rendering has not been launched for verification.

## Landing lifetime and native font passes

The landing hide bug came from sending `CloseTrickDisplay` at every score
publication. Native `82775328` calls `82774E88` only on scoring output byte
14630; `82DA4238` copies that from module byte128, selected by `82DA4010`
for its reset/bail category. `82774E88` sets backend byte184, which
`825E4F40` turns into event3, resolved by `825D3B38` to
`_global.TrickDisplay.CloseTrickDisplay`. Normal banking no longer emits it.
Line expiry clears the score and lets the authored Clear/outro actions run.

The timer unit is now traced end-to-end: `82DA4238` writes line points divided
by drain rate to output+40; `82775328` copies it to backend+32;
`825C2910` and `825C2D90` truncate that value to seconds for ActionScript.
The previous raw-point binding was wrong. Changes in that seconds field now
also refresh `UpdateTrickScoring`.

Observed font code: `825D6B68` compares the APT name with `Futura Shadow`
(string at `8220BD44`) and attaches `futuraheavy` (`8220BD54`) as a secondary
font, setting X adjustment 1 and the other adjustment 0. `82CA1FD8` first
sets the primary RGB multiplier to black while retaining alpha, draws it,
then restores the color and draws the foreground with the X adjustment.
The scene renderer now follows those two passes. FuturaOuterGlow remains
its separately authored tinted atlas pass. No outline radius, new glow
color or replacement glyphs were invented.

The offscreen adapter also needed premultiplied-alpha compositing. The
mesh target already stores covered RGB; Bevy's ordinary ImageNode uses
straight-alpha blending, which multiplied coverage again and weakened
soft atlas edges. The dedicated HUD composite now uses premultiplied
blending to preserve the original layers' coverage.

Validation: supplied scoring data exercises landing, retained HUD visibility,
seconds conversion, natural line expiry and teleport cancellation. The
1,800-frame original movie audit checks a black shadow/foreground pair and
finite geometry (33 maximum batches). Release compilation passes. User
screenshots guided the investigation but did not supply rendering constants;
no game, reference executable or GPU capture was launched for validation.

## Output resolution and authored blue glow

The HUD target now uses the window's physical pixel dimensions and resizes with
the window. Its orthographic projection retains the authored 1280x720 coordinate
space. Previously the adapter rasterized at 720p and enlarged the finished image,
adding blur on larger windows. This change improves rasterization without
inventing higher-resolution source artwork.

A data-only inventory of `fedata.big`, `fedynamic.big`, `fetexture.big` and
`miscboot.big` found byte-identical duplicate trickdisplay APT/CONST/GEO/RX2
assets and 512x512 Futura font atlases. No higher-resolution variant was found
in those banks. The inventory and hashes are retained in ignored
`logs/scoring/hud-resolution-inventory.json`.

`GetGeneralInfo` had clean and sketchy reversed. The original
`UpdateTrickScoring` action stream reads slot2 as sketchy (0199D2) and slot3
as clean (019A30), matching backend bytes152/153 returned by `825C2D90`.
Correcting those slots selects the authored clean color animation. The native
FontPS/Font8PS programs contain texture/color modulation; the glow is supplied
by the authored layers, not a replacement procedural effect.

Additional collector fixes follow `82DA8A70`/`82DA93D8`: published Air452
suspends continuous air metrics and grab accrual while preserving the carrier.
Distance collectors also start their first active frame at zero elapsed
time/distance, following `82DB0588`.

Validation: the 1,800-frame supplied-data HUD audit asserts that clean input
selects mLastClean rather than mLastSketchy; the scoring-flow audit verifies
Air452 suspension in addition to landing lifetime, expiry and cancellation.
Both pass, and the static-CRT release build succeeds. The game was not launched;
pixel appearance and GPU resize behavior still require interactive verification.

The first window-resolution build introduced a render-target invalidation
regression: Bevy 0.18.1 `Assets::get_mut` queues a Modified event even for a
read-only inspection. The size check therefore recreated the GPU image each
frame while the UI material retained its earlier texture binding. The check
now uses immutable access; a real resize refreshes the compositor material,
including a following-frame refresh to account for independent image/material
preparation order. An asset-only regression test checks that an unchanged
target emits no Modified event and a resize invalidates both image and material.
The test uses no window, renderer or gameplay systems.

## Multiplier timeline playback

The host previously called `UpdateLineDisplay` every fixed tick. The shipped
action at 019C39 computes `floor(GetLineTimeRemaining() * 60) - 30`, clamps
it, and seeks `multiTimer_mc` with `gotoAndPlay` at 019CF4. Since the native
binding returns whole seconds, repeated calls pin the 501-frame timer to the
same frame for a second, then jump it forward. The ActionScript dispatches
this method for event11 (0196CC) and ScreenShow, rather than onEnterFrame.
The adapter now dispatches when the exposed timer sample or line score changes,
allowing the authored timeline to advance between samples. This is an adapter
event policy; the entire native HUD event producer is not yet ported.

Original x3 effect clips are retained: character59 changes mBlueGlow alpha
with `80 + Math.random() * 20` every four frames; character70 changes mFlicker
with `90 + Math.random() * 10`. Original multiplier labels select the glow
and background timelines (characters66 and77); no replacement particles,
colors or universal x3 effect were added to lower multiplier levels.

The supplied-data movie audit now visits x1.5, x2 and x3 over 1,800 frames,
asserts consecutive timer frames between events, checks both flicker ranges
and changing alpha values, and reaches 40 authored draw batches. The scoring
flow audit still passes landing persistence, line expiry and cancellation.
No game or GPU capture was launched.

## The combo's lifetime: the two timer holds (fixed 2026-09-23)

Two owner-reported defects, both in the lifetime of a line: the score reset when a
combo was linked into a manual, and the score and trick name stayed on screen long
after they should have cleared. Both came from one line of this port handing the same
"a collector is active" flag to both point timers.

`sub_82DA33E0`, the ScoreModule's per-frame update, calls `82DA4C28` twice and the two
calls do not share a hold flag:

```text
r29 = ([module+4] == [module+16])    ; cntlzw/rlwinm 27,31,31 -- equality, not difference
r5  = bit30 of [holder+1832]         ; set by 82DA48B8, cleared by every 82DA37B0
if (!r5)       r30 = 0               ; no hold at all
else if (!r29) r30 = 1               ; every other collector holds without a bound
else           r30 = [module+112] < (int)([module+64]+1532 * 60.0)
bl 0x82da4c28  ; r3 = module+40, the line timer, r5 = r30
[module+112] = (returned hold && r29) ? [module+112] + 1 : 0
li r5,0
bl 0x82da4c28  ; r3 = module+44, the combo timer -- never held
```

* The collector at `module+16` is the **ground** one: `82DA3C68` writes its `byte+945`
  every frame, and `82DAA8E0`, a method of the class that reads that byte, is what
  tests the nose/tail manual bits `0x08000000`/`0x04000000`.
* `module+64` is the collector tuning class `546C36B656038E04`, so `+1532` is authored
  key `0x5fc` = **5.0**, and the constant at `0x8303745C` is **60.0**: a 300-frame bound.
* The multiplier timer is never held. This port held it, so a lingering ground carrier
  kept the multiplier's fuel from draining at all.
* `module+112` counts only the frames the hold actually took effect, which is why
  `82DA4C28` returns that as a bool. This port discarded it, so a manual, a powerslide
  or a revert pinned the line just above one point **for ever** and the HUD never
  cleared. Releasing the hold can take more than one full 300-frame window: the frame
  that ends a hold subtracts a single drain, and the hold re-arms unless that leaves
  `previous` under `82DA4C28`'s 1.000001.

The hold's `bit30` gate is not reproduced. It means "82DA48B8 has republished the line
to the display", and this host has no display-live bit; the hold is a no-op at zero
points either way, because `82DA4C28` returns before touching an empty timer.

### The manual gate is State+70, not State+66

`82DAA8E0`, the ground collector's manual slot, gates the manual on
`[[[collector+4]+28]+70]` -- PhysOut, then State, byte **70** -- and counts suppressed
frames at `collector+932`, admitting the manual again only while that count is `<= 6`:

```text
r4 = bit4([this+104]) || bit5([this+104])       ; nose or tail manual
if (r4 && r10) { [this+932]++ ; r4 = 0 }
else if (!bit30([holder+1832])) [this+932] = 0
if (r4) r4 &= ([this+932] <= 6)                ; the subfic/subfe pair
```

This port read State+**66** instead, which is the byte `82D43B10` -- `RevertGround`'s
Fill -- sets for the revert's whole active lifetime. Every frame of every revert
therefore dropped the manual carrier; with no carrier the sequence went idle,
published itself, and the line then drained the multiplier back to x1. That is the
reported "the score resets if you connect in a manual". `82DB6EC0` fills State+70 from
`[base+1888]+56`, which this host does not publish, so `Frame::manual_block_70` stays
clear -- which is also what retail does whenever that byte is clear. Identifying that
source is open work. The counter's reset condition is the one deliberate deviation:
retail resets on `bit30([holder+1832]) == 0` and this port uses an empty line, the
nearest state it does publish.

### The publication gate: the collectors' own answer (2026-09-23)

The owner then reported three more things: popping out of a manual reset the combo,
grinds did not combo, and spins were missing from trick names. A captured session
(`SKATE_SCORING_TRACE`, 38,714 frames) showed the first two exactly:

```text
tick=844 col=Ground flags=04400000 active=true  idle=0   <- the manual
tick=845 col=Ground flags=00400000 active=false idle=1   <- the manual bit drops
tick=846 col=Ground flags=01000000 active=false idle=2   <- the flip is announced
tick=847 SCORE_PUBLISH reward=303 mult=1.50 line=303     <- the sequence is cut
tick=851 col=Air     flags=01000000 active=true  idle=0  <- the pop, too late
```

`idle_ticks >= 3` was never retail. `82DA37B0` asks the **active collector**, through
its `vtable+20`, whether the sequence continues, and publishes only on a no. The six
collectors are named in `.rdata` at `0x823280F4` ("Air", "Grind", "Ground",
"Handplant", "Offboard", "Other") and their vtables run from `0x8232812C` in 40-byte
strides with `GetName` at slot 6; the Air table at `0x823281A4` is confirmed twice over,
by slot 3 being its Enter `82DA8078` and slot 9 its publisher `82DA9A18`.

| collector | slot 5 | answer |
|---|---|---|
| Other | `8274CA90` | `li r3,0` -- never |
| Grind | `8281DD70` | `li r3,1` -- **always** |
| Handplant | `8281DD70` | `li r3,1` -- always |
| Air | `82DA98C8` | `r4 \|\| [this+2352] > 5 \|\| ([this+104] & 0x01000000)` |
| Offboard | `82DAC1F8` | a live scorable id in `(-1,332)` and byte `[this+316]` |
| Ground | `82DAB2A8` | a long OR, below |

`82DAB2A8`'s terms: `[this+116] > 0`, byte `[this+944]`, a revert bit
`0x20000000`/`0x10000000` while `r4`, the published scoring-trick bit `0x01000000`,
either carrier slot (`[this+652]`/`[this+540]` with `[this+716]`, and
`[this+252]`/`[this+140]`), the manual grace `[this+932] <= 6`, and -- again only while
`r4` -- a speed test on `[[PhysOut+32]+268]` and a VMX compare on `[[PhysOut+0]+80]`.

The grind answering *always* is why a rail links with no idle window at all, and the
`0x01000000` term is what carries a sequence across the pop above. That bit is a pulse,
not a latch: over the captured session it was set on 4.8% of frames and never for more
than 13 consecutive grounded frames.

Not ported: `r4` is the display-live bit (below), so the terms it gates are taken
unconditionally, and `[this+116]`, `[this+944]` and the two speed tests are unidentified
fields. Every one of those is a *keep going* term, so leaving them out can only end a
sequence earlier than retail, never later.

### The score fades because bit30 goes clear

The line timer's hold gate, `bit30 of [holder+1832]`, is not a constant. `82DA48B8` sets
it when it republishes the line to the display; `82DA37B0` clears it every frame and only
calls `82DA48B8` while a line is open. So once a sequence has been banked and nothing
further is happening, the bit stays clear and the line gets **no hold at all**: it drains
out and the authored Clear/outro runs. That is why a retail score fades when you stop.

This host publishes no display-live bit, so `sequence_active` stands in for it -- the hold
exists to stop a line expiring underneath a trick in progress, which is exactly when a
sequence is open. Taking the bit as always set, as the first pass here did, re-armed the
hold long after the last trick and the score sat on screen. With the substitution a
143-point line left alone fades in 172 frames.

### Spins never reached a trick name

`825E51A0` decides whether a name may carry a spin from the TrickType of the **named**
scorable's record (`desc+120`, its `r23`). This port set `base_trick_label` on the
announcement but `base_trick_type` only on the flip-ladder *conversion* path, so every
ordinary trick composed its name with type 0, which `decorates_spin` refuses. The spin was
measured and scored the whole time -- `SCORE_AIR spin_deg=-349 turns=-2` -- and simply
never appeared. Both are now set together, and both are cleared together when the display
closes.

### What was checked and deliberately not changed

`landing_countdown = 2` is set **only** by `82DA8550`, the air collector's landing
bank, so a grind that lands gets no such grace in retail either; this port already
matches. `82DA3310` is a faithful port of the landing-quality latch and that countdown.

`[module+128]`, `[module+129]` and output byte 14657, which `82DA37B0` uses around the
collector's answer to select its banking paths, are still not ported.

Validation: `crates/skate-data/examples/scoring_flow_data.rs` gained five scenarios --
a rail -> manual -> flip line that must keep its multiplier with and without a revert;
the captured rail -> manual -> pop -> air frame pattern, which must stay a *single*
sequence and must still end when the trick does; a banked line left alone, which must
fade inside one ground hold (143 points fades in 172 frames); a parked manual, which
must still let the line expire and clear the line score; and a full rotation, whose
name must carry a degree count. All pass against the owned data together with every
earlier assertion. The spin and gate scenarios were both confirmed to fail with their
fixes reverted. `scoring::timer` gained a test for what a denied hold does.
`SKATE_SCORING_TRACE` gained a per-tick `SCORE_TICK` line, because every other trace is
edge-triggered and a pinned timer looks identical to a healthy one unless it is sampled
every frame -- that line is what found all of this.

## Remaining native parity gaps

These are implementation gaps, not merely missing gameplay validation:

- Collector activation gates still use conditioned categories. Publication now uses
  the collectors' own `vtable+20` answers, but four of `82DAB2A8`'s keep-going terms
  are unidentified fields and `82DA37B0`'s banking paths are not ported.
- Air spin uses accumulated board heading rather than the complete native
  transform accumulator. Body-flip direction and some landing modifiers remain
  incomplete.
- Gap/context collectors and their native ground-query inputs, contextual
  bonuses and off-board height rewards are not wired.
- Revert recognition depends on an unpublished physical state flag in the
  current host. Full native revert scoring is not yet available. The ground
  collector's manual gate needs State+70, which 82DB6EC0 fills from
  `[base+1888]+56`; until that is published the gate stays clear.
- The line timer's hold gate substitutes `sequence_active` for `bit30 of
  [holder+1832]`, the display-live bit this host does not publish.
- The trick name is now composed as retail composes it (`sub_825E51A0`): one `#`,
  then space-separated localisation ids and a bare degree count, resolved token by
  token by `apt_text::localize`. The five metric slots are *not* a label plus four
  modifier flags -- `sub_825C27C0` returns `[name, switch, fakie, clear, new-trick]`
  -- and the modifiers are tokens inside the name string. Spin degrees and the
  front/back flip suffix are supplied; still missing are the Cab/Half-Cab tokens,
  the nollie record swap through the metadata link column, and the Miracle Whip
  case. The FS/BS parity needs a skater stance (`M+187`, the negation of
  `PlayerStance_IsRegular`), which this engine does not publish, so it assumes
  regular.
- The HUD manager's stance transitions are incomplete: `sub_825C2700` returns
  `[switch, fakie, idle latch (M+168), is-nollie-variant && display active]`, and
  the port's fourth slot is a placeholder.
- The compact VM supports the exercised movie paths, not arbitrary APT programs.
  Superclass/native constructor behavior is limited, and Math.random uses a
  local presentation RNG rather than the original engine RNG stream.

Do not describe this build as fully finished or an exact native recreation.

## Why every score was far below retail (fixed 2026-09-21)

`scoring_runtime` computed all five air metrics every frame and then discarded
them at the landing that was supposed to bank them. The landing resolved each
metric with `ScoringData::by_id`, which only returns scorables the vault holds a
`Hash_6918469984A8C596` record for. Of the executable's 332 scorables exactly 300
are authored, and the 32 that are not include **129..=133** (`air_horizontal_distance`,
`air_height_to_peak`, `air_total_height_gain`, `air_player_spin`, `air_player_flip`),
**237** (`air_metric`) and **253** (`generic_metric`).

Retail never needs a record for these: `82DA6260` indexes the fixed metadata table
at `820862A8` by bare id, and `82DA8550` credits them the same way (`li r4,129` /
`bl 0x82da6260`). Their reward is a curve value, not authored points, which is
precisely why they carry no authored record. `catalog::metadata` now resolves them
from that table. A 10 m, 3 m-high 360 air banks **508** where it banked **100**;
a stationary kickflip still banks exactly its authored 100, because the curves are
zero at zero input.

### Authored magnitudes, for judging what is still missing

| curve | input | peak |
|---|---|---|
| `0x3C0` | horizontal distance | 500 at 34.45 m |
| `0x410` | height to peak | 500 at 12 m |
| `0x320` | total height gain | 500 at +12 m, **and 500 at -12 m** |
| `0x370` | spin | exactly 1 point per degree, to 1260 |
| `0x640` | body flip | 300, flat |
| `0x4B0` | gap run above 6 m | 1800 at 20 m |
| `0x500`/`0x550`/`0x5A0` | the other three gap runs | 900 each |
| `0x190` | off-board peak height | 700 at 10 m |

### Still missing, with the evidence already gathered

- **The gap/context collector** (`82DA89A0`, run accumulators `82DA7B50`, sum
  `82DA88E8`). Four runs of horizontal distance, gated on a surface byte, a second
  surface byte, height > `0x680` = 3.0 m and height > `0x67C` = 6.0 m; banked as
  scorable 237. Blocked on the ground query `82DA7A38`, whose reference point is
  the unidentified **player+1632**.
- **The landing-context multiplier and four flat bonuses** in `82DA8550`
  (`0x644`=1.5, `0x654`=1.15, `0x64C`=1.5, `0x65C`=1.15 multiplying all five
  metrics; `0x648`=500, `0x658`=200, `0x650`=300, `0x660`=100 paid as id 237),
  gated on four flags in a 10x68-byte context block filled by `82DA14A0`.
- **The class-3 one-shot** in `82DA93D8`: `carrier.reward += 0x600 (=4.0) * points *
  factor`, one-shot per air, gated on `82DAC780`'s geometric test -- which also
  needs player+1632.
- **Off-board height**: `82DAB7D0` tracks a peak height and banks `curve(0x190)` as
  id 253 at exit. The port's off-board collector instead accrues `curve(0x140)`,
  whose authored y array is **all zeros**, so it earns nothing. Same missing input.
- **The spin accumulator**: `82DA8BE0` sums **two** rotation accumulators (`this+816`
  over `[[owner+20]+368]`, `this+896` over the root transform `[owner+0]`), each a
  `82DAC880` measuring a signed angle about the object's *own* up axis and wrapping
  to (-pi, pi]. The port accumulates a single world-yaw heading delta, which
  degenerates when the board pitches toward vertical. The threshold arithmetic
  itself, `floor((|deg| + 0x63C=80) / 180)` signed by the rotation, is retail-exact.
