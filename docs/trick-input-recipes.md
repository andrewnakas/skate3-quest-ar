# Trick input recipes

`docs/trick-reachability.md` says *whether* a scorable has a crediting path. It does not say how
to perform it. This does, for the families where the answer is not obvious, decoded from the
authored `ActionGraphIncludes/` and confirmed by the playback tests named against each one.

## The rule that governs every board-adjust grab: stick before trigger

This one cost a full debugging session, so it is first.

The air ActionGraph selects **one** active leaf. `GetNextState` (82C140D8, ported in
`skate-core/src/graph/selection.rs`) descends from the root taking the *first* child whose
expression activates, and `ActionGraphIncludes/air.xml` orders the air state's children
`HandPlant`, `Grabbing`, `BoardAdjusting`, ... `Grabbing`'s `T_Grab.xml` children are gated on
nothing but `LeftAirGrab` / `RightAirGrab` -- there is no angle constraint on them at all.

So a trigger held from the first airborne frame always lands in `Grabbing.LeftGrab`, a plain
`fsgrab`, and `BoardAdjusting` is never entered. The intents are all perfectly correct while this
happens -- `BoardAdjustAngle` sits at exactly pi and `BoardAdjustMag` at 1.0 -- which makes it
look like a defect in the board-adjust branch. It is not.

`BoardAdjustUp.xml`'s `Idle` requires only `BoardAdjustMag`, and carries
`<transition target="TailGrab"/>`. So the authored order is:

1. **Stick first.** With no trigger held, `Grabbing` has no activatable child, so the selected
   leaf becomes `BoardAdjusting.Up.Idle` (or `.Down.Idle`).
2. **Then the trigger.** `search_transitions` walks up from that leaf, finds `Idle`'s transition,
   and enters `TailGrab` from *inside* the board-adjust branch.

Six ticks of stick lead is enough (`STICK_LEAD` in `tests/grab_playback.rs`).

## Grabs

`BoardAdjustAngle` is `atan2(x, -y)` over the raw right stick (`trick_intentions::produce`), so
raw `+y` is the up branch and `-y` the down branch. The up branch claims
`abs(angle) >= 2.355`, the down branch `abs(angle) <= 0.785`.

| Trick | id | Right stick | Trigger | Authored branch |
|---|---:|---|---|---|
| `tailgrab` | 82 | up (`[0, +32767]`) | left | `BoardAdjusting.Up.TailGrab` |
| `seatbeltgrab` | 190 | up | right | `BoardAdjusting.Up.SeatbeltGrab` |
| `nosegrab` | 74 | down (`[0, -32767]`) | right | `BoardAdjusting.Down.NoseGrab` |
| `crailgrab` | 187 | down | left | `BoardAdjusting.Down.CrailGrab` |
| `rocketair` | 77 | down | **both** | `BoardAdjusting.Down.RocketAir` |
| `fsgrab` | 62 | centred | left | `Grabbing.LeftGrab` |
| `bsgrab` | 63 | centred | right | `Grabbing.RightGrab` |
| `dblgrab` | 61 | centred | both | `Grabbing.DBLGrab` |

Mirrored (goofy) stance swaps every left/right trigger above; each authored expression pairs
`HasAGIntent <hand>AirGrab` with `IsMirrored`.

The `_left` / `_right` / `_up` / `_down` directional variants are not separate inputs. Holding
the grab and then tweaking the stick moves `BoardAdjustAngle`, and `ScoringGrabs`'
`select_grab_score` picks the variant whose authored angular window contains it, falling back to
the base name inside a 0.5 dead zone.

Proven by `tests/fingerflip_playback.rs::held_grabs_reach_their_authored_names`.

## Grab shove-its (fingerflips)

Hold the grab as above, then flick an authored scoop **without releasing the trigger**. The
scoops live in their own recognizer, `skater_fingerflip.pat`, which authors exactly three
patterns: `Fingerflip`, `FS_Varial` and `BS_Varial`.

| Trick | id | Grab | Scoop | Authored branch |
|---|---:|---|---|---|
| `tailgrab_fingerflip` | 176 | tailgrab | `Fingerflip` | `Up.Grab.Grabbing.FingerFlip` |
| `nosegrab_fingerflip` | 175 | nosegrab | `Fingerflip` | `Down.Grab.Grabbing.FingerFlip` |
| `seatbelttonose_fingerflip` | 255 | seatbeltgrab | `BS_Varial` | `Up...SeatbeltGrab.SeatbeltFingerFlipShuv` |
| `crailtotail_fingerflip` | 256 | crailgrab | `BS_Varial` | `Down...CrailGrab.CrailFingerFlipShuv` |
| `fsgrab_fingerflip` | 173 | fsgrab | `Fingerflip` | `GrabsTweaks.FSGrab.FingerFlip` |
| `bsgrab_fingerflip` | 174 | bsgrab | `Fingerflip` | `GrabsTweaks.BSGrab.FingerFlip` |

The last two hang off `Grabbing`, not `BoardAdjusting`, so they do not need the stick lead.

The two `*to*` entries genuinely cross branches: the seatbelt shuv takes
`<transition target="Down.NoseGrab"/>` and the crail shuv takes `<transition target="Up.TailGrab"/>`,
so the grab the skater is holding changes mid-air. The animations confirm it --
`F_FLIP_SEATBELT_TO_NOSE_GRAB` then `B_NOSEGRAB_CYC`.

Both `FingerFlipShuv` states also accept the late-shuvit gestures `L_FS_Shuvit` / `L_BS_Shuvit`
from `skater_air.pat`, so a late shove-it out of a grab reaches the same trick.

Because the recognizer is fed the held grab direction every tick, the hold has already walked the
scoop's first coordinate; the flick only has to complete it. The test picks whichever authored
variant starts nearest the held point, after the input manager's Y negation.

Proven by `grab_fingerflips_reach_their_authored_tricks` and
`grab_to_grab_fingerflip_shuvs_cross_the_board_adjust_branches`.

## The mute and stale grabs

Neither is a stick direction. `StaleMute` is authored `active="false"` and reached only by
`T_Grab.xml`'s `<transition target="Grabbing.StaleMute"/>` out of a plain grab's `FingerFlipShuv`.
So the recipe is two scoops, stick centred throughout:

| Step | Input | Result |
|---|---|---|
| 1 | left trigger (fs) or right (bs) | `fsgrab` / `bsgrab` |
| 2 | `FS_Varial` scoop | scores `fsgrabtostalegrab_fingerflip` (238) / `bsgrabtomutegrab_fingerflip` (239), and lands in `stalegrab` (78) / `mutegrab` (70) |
| 3 | `Fingerflip` scoop | scores `stalegrab_fingerflip` (178) / `mutegrab_fingerflip` (177) |

**The gap between the two scoops has to outlast the animation, not just the state change.** The
grab-to-grab clips (`F_FLIP_fs_GRAB_TO_Stale`) are `interruptable="false"` and a gesture intent
lives for a single tick, so a scoop thrown while that clip is still playing is simply dropped --
the AG enters its `FingerFlip` state and publishes the MG intent, and nothing happens. Thirty
ticks is enough; ten is not. A chain also needs a bigger takeoff boost to fit inside one air,
and running out mid-chain lands as a wipeout rather than as a missed trick.

Proven by `varial_out_of_a_plain_grab_reaches_the_stale_and_mute_family`.

## One-foot and no-foot, out of a held grab

Every board-adjust grab carries the same three children, gated on the push buttons rather than on
the stick:

| Addition | Intent | Button |
|---|---|---|
| `OneFootRight` | `RightPush` | A (`0x1000`) |
| `OneFootLeft` | `LeftPush` | X (`0x4000`) |
| `NoFootAirWalk` | `Dismount`, or both pushes at once | B (`0x2000`) |

| Trick | id | Grab | Button |
|---|---:|---|---|
| `tailgrab_onefoot_behind` | 169 | tailgrab | A |
| `tailgrab_onefoot_front` | 168 | tailgrab | X |
| `tailgrab_airwalk` | 172 | tailgrab | B |
| `nosegrab_onefoot_behind` | 167 | nosegrab | A |
| `nosegrab_onefoot_front` | 166 | nosegrab | X |
| `nosegrab_airwalk` | 254 | nosegrab | B |

The press has to arrive *after* the grab is holding. Pressed earlier it is an ordinary push and
never reaches the grab's own children.

`air_playback.rs::airborne_tailwalk_from_raw_controller_reaches_stock_cycle` targets
`tailgrab_airwalk` too, but it rebuilds `BoardWorld` for a deeper landing and dies on `Canonical
world has no authored query metadata` before it gets there -- the same hazard
`docs/flip-ladder.md` records. The recipe above reaches it instead by boosting takeoff velocity,
which leaves the canonical course's authored query metadata intact.

## The `Grabbing` branch's own extras

These hang off `GrabsTweaks` rather than off a board-adjust direction, so the stick stays centred
and no stick lead is needed.

| Trick | id | Trigger | Button |
|---|---:|---|---|
| `nofoot_air` | 138 | left | B |
| `christ_air` | 59 | right | B |
| `fs_onefoot_right` | 162 | left | A |
| `fs_backfoot` | 163 | left | X |
| `superman` | 252 | both | B |
| `coffin` | 60 | both | A + X |

`Superman` and `Coffin` are authored `active="false"` and reachable only through `DBLGrab`'s
transitions. Superman additionally wants the *rising edge* of `Dismount` (`NewDismount`) on top
of both triggers, so B has to be pressed while the double grab is already held rather than
alongside it.

Proven by `one_foot_and_no_foot_variants_reach_their_authored_grabs` and
`centred_grab_extras_reach_their_authored_names`.

## Late flips

Ollie, then run a `skater_air.pat` scoop in the air: `L_F_Kickflip`, `L_B_Kickflip`,
`L_F_Heelflip`, `L_B_Heelflip`, `L_FS_Shuvit`, `L_BS_Shuvit`, and no others. These never enter
the 270-row trick mapping; they reach the MotionGraph as bare intents matched by `HasIntent`.
Covered by `flip_playback.rs::air_scoops_reach_the_authored_late_flips`, and see that file's doc
for why there is no late double, triple or quad.

## Reverts

Reverts are neither a flip nor a grab, they live on the **left** stick, and no scoring leaf ever
spells them. `MotionGraphIncludes/revert.xml`'s `Fs` and `Bs` children are gated on `SlideFs180`
/ `SlideBs180` from `skaterls.pat`, and each carries
`<behaviour name="SetScoreAugmentation" augment="FSRevert" mirrorAugment="BSRevert"/>`.
`ScorePacket::set` turns that into a flag bit -- `FSRevert` is bit 29, `BSRevert` bit 28 -- and
`scoring_runtime` reads the two bits straight into `revert_id` 4 and 5.

`Revert` is authored `active="false"`; `ground.xml` transitions into it from `Riding.Turning` and
from `Riding.Sliding`, so it is thrown out of a turn or a powerslide.

**Enter from a deflected stick, not a centred one.** Sweeping the arc from centre produces no
slide gesture at all. `BackFlip`'s first authored coordinate sits within tolerance of centre, so
idling has already advanced its node, and the arc's second point completes `BackFlip` one tick
before the slide's third point completes the slide. Only one pattern wins a file, `permitted`
then drops `BackFlip` because no grab is held, and `skaterls.pat` yields nothing. Holding the
pattern's entry coordinate first leaves `BackFlip` parked at its first coordinate -- and is also
how the trick is really thrown.

**Sweep one coordinate per tick.** The recognizer culls a node after
`NumTicksPatternNotInRangeBeforeCulling` ticks whose sample is not yet within tolerance of the
next expected point, and that budget is read **per stick**; the left stick's is tighter than the
right's. Three ticks per coordinate resets the node mid-pattern.

A revert extends a line rather than opening one, so thrown on its own nothing is banked and the
snapshot stays at zero. That is retail behaviour, not a gap.

Proven by `tests/revert_playback.rs::left_stick_slides_reach_both_reverts`.

## The ground plant family

All four `PushTrick` children are entered from the ground. A ground grab is stricter than an air
grab: `trick_intentions::produce` advances the ground timers only when the trigger reads
*exactly* 1.0, so a partial analog value is not a held grab.

| Trick | id | Trigger (held fully) | Push |
|---|---:|---|---|
| `fsboneless` | 215 | left | A |
| `fsfastplant` | 217 | left | X |
| `bsboneless` | 216 | right | A |
| `bsfastplant` | 218 | right | X |
| `hippyjump` | 234 | none | A + X, then **release** |

**The hippy jump is a charge.** Its antic cycles for as long as both pushes are down
(`B_HIPPYJUMP_ANTIC` into `T_HIPPYJUMP_ANTIC_CYC`) and launches on the release. A button held
for the whole run never leaves the anticipation, which reads as a missing trick rather than as an
unfinished one.

Proven by `tests/footplant_playback.rs`.

### Open: the no-comply

`nocomply` (229) is named by `Motion.OnBoard.Trick.PushTrick.NoComply.FootPlant` and has authored
points, so it is reachable in principle, but no input here has reached it yet.
`T_PushTrick.xml`'s `PlantingFoot` wants two MotionGraph intents together --
`HasIntent NewFootPlant` **and** `HasIntent NoComply` -- and a front-foot push followed by an
ollie scoop produces `Ollie`, not `NoComply`, however the two are spaced: the push enters
(`B_PUSH_INTO`) and the scoop pops an ordinary ollie straight over the top of it
(`B_ANTIC_INTO`, `B_OLLIE_G`).

`docs/plants.md` records the intended recipe as "begin a front-foot push, then flick the right
stick for an ollie during push entry; the graph accepts the Square gesture group". The open
question is which gesture group `CreateTrickIntentFromGesture` is meant to resolve while a push
foot is planted, and what publishes `NoComply` rather than `Ollie` from it. The same question
covers the four `no_comply_*180/360` ids, though those have no VLT record at all and so score
nothing even once reached.

## Dark catches

An ordinary flip scoop with the dark-catch button held. `trick_intentions::produce` reads the
request from the packed action flags rather than from gesture timing --
`dark_flags = (1 << 20) | (1 << 28)`, so two bits arm it -- and publishes `DarkCatch` while held
plus `NewDarkCatch` on the rising edge. `gesture_mapping::select` resolves the flick through the
dark-catch column of the 270-row table, and `MonitorUnderflip` latches `trick_requests.dark_catch`
when no `U_*` underflip intent is present instead.

**Use RB, not B.** Only one of the two armed bits reaches the catch from a rolling approach: the
other is B, and B while riding is `Brake` (`riding_intentions` emits it from the same button), so
the pop never happens at all.

| Trick | id | Scoop | Button |
|---|---:|---|---|
| `kickflip_darkcatch` | 301 | `Kickflip` | RB, armed before the pop |
| `heelflip_darkcatch` | 297 | `Heelflip` | RB, armed before the pop |

The rest of the darkcatch range (298-300, 302-320, 331) is the same recipe over the other scoops
and the flip-count ladder, and is not separately covered here.

Proven by `tests/dark_playback.rs`.
