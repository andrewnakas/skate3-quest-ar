# Double, triple and quad flips, and the late flips

Measured 2026-09-21 with `crates/skate-game/src/tests/flip_playback.rs`, which drives the
production controller -> gesture recognizer -> ActionGraph -> MotionGraph -> physics -> scorer
path. Run it with:

```
SKATE3_ASSET_ROOT=C:/s3/installations/<install>/assets RUST_MIN_STACK=134217728 \
  cargo test -p skate-game --bin skate3rust flip_tests -- --ignored --nocapture
```

`RUST_MIN_STACK` is not optional: without it rustc itself dies with `STATUS_STACK_BUFFER_OVERRUN`
(0xc0000409) while building any `skate-game` bin test harness at `opt-level=3`.

## They already worked; the gate is air time, not recognition

The whole ladder is authored, and this port already walks all of it. A held kickflip publishes
`Kickflip` -> `Kickflip2` -> `Kickflip3` -> `Kickflip4` over `B_KICKFLIP_IN_A`, `_CYC1`, `_CYC2`,
`_CYC3`, `_OUT4`, and the landing is named `#ID_TRICK_FLIP_QUADRUPLE_KICKFLIP`. The heelflip does
the same over its own clips. So does the pre-existing playtest evidence:
`logs/launch-20260921-124517.err` already contains a `heelflip4` (scorable 134) off `B_HEELFLIP_CYC3`.

**The earlier premise -- "the graph probably does not recognise a triple" -- was wrong.** What
limits the count is the authored air-time gate:

| Authored state | Precondition | Effect |
|---|---|---|
| `End.Out.Out1` | `not HasIntent <Trick>Hold` OR `TimeToLand < 0.525` OR `IsBodyFlipping` | ends at the single |
| `End.Out.Out2` | same, with `TimeToLand < 0.8` | ends at the double |
| `End.Out.Out3` | `not HasIntent <Trick>Hold` OR `IsBodyFlipping` | ends at the triple |

`Cyc1` lists `End.Out.Out2` *before* `Cyc2.StillDouble`, and both transitions share the same
`WillExpire` expression, so whenever `Out2`'s precondition passes the exit wins and the ladder
stops. Every step therefore needs the pop to still have the authored time left.

`TimeToLand` is `PhysOutAir.time_until_collision_184` = `collision_time_196 - time_in_state_180`
(`skate-core/src/air/known/output.rs`). `KnownAir` takes that prediction **once**, on entering the
air, and then only counts it down -- changing the world mid-air does not extend it. A stock flat-
ground pop leaves about 0.77 s, just under `Out2`'s 0.8 s, which is why a flat kickflip stops at
the double and why triples and quads only show up off a ledge or a ramp.

That also rules out one tempting test shortcut: dropping the ground out from under the skater
mid-air does nothing, because the prediction was already taken. The harness pops harder instead
(a vertical velocity boost on the takeoff frame) rather than rebuilding `BoardWorld`, which would
drop the canonical course's authored query metadata and fail with
`Canonical world has no authored query metadata`.

Measured thresholds on the flat course at 8 m/s, boost applied one tick before takeoff:

| Trick | boost reaching the quad | landed reward | landed name |
|---|---|---|---|
| Kickflip | 8 m/s | 497 | `#ID_TRICK_FLIP_QUADRUPLE_KICKFLIP` |
| Heelflip | 10 m/s | 564 | `#ID_TRICK_FLIP_QUADRUPLE_HEELFLIP` |
| Kickflip, flick released | - (single) | 336 | `#ID_TRICK_FLIP_KICKFLIP` |

The heelflip needs the taller pop because its authored clips are longer, not because anything is
broken.

## Late flips

`air.xml`'s `Lateflip` state and its `T_Lateflip.xml` leaves work too. An ollie followed by the
authored `skater_air.pat` scoop plays `B_LATE_KICKFLIP` / `B_LATE_HEELFLIP` and lands as
`#ID_TRICK_FLIP_LATE_KICKFLIP` / `#ID_TRICK_FLIP_LATE_HEELFLIP` (scorables 105 / 104). The
`L_*` gestures are deliberately absent from the 270-row trick mapping: they reach the MotionGraph
as bare intents and are matched by `HasIntent` in the authored state.

Note `input/gesture_input.rs::permitted`: actor flag bit 4 refuses every `L_*` gesture outright,
so late flips are unavailable in whichever difficulty sets it.

## The ladder converts, it does not accumulate

`conversions::LINKS` is the flip-count ladder (`93 -> 92`, `94 -> 93`, `134 -> 94`, and the
kickflip/nollie equivalents). `Runtime::carrier` converts the Air carrier instead of finishing it,
and `Carrier::convert_to` marks the previous carrier completed **with its reward discarded** and
credits the replacement immediately. So a quad banks the quad's authored 250, never 100+150+200+250.
`a_single_flip_is_the_ladder_baseline` pins that the quad still out-scores the single.

## Open: the class-3 one-shot flip bonus never fires in the live path

`SCORE_TRICK air ... bonus_paid=false` on **every** run above -- single, double, triple and quad,
kickflip and heelflip. `pay_flip_bonus` (82DA93D8, `reward += 4.0 * points * factor`) needs
`f.hips_ground` to be a hit that lies *between* the deck and the hips (`dot(deck-contact,
hips-contact) < 0`). On flat ground the hips-line hit is below both, so the dot is positive and the
bonus is skipped. The "kickflip 100 -> 500" figure recorded earlier came from
`examples/scoring_flow_data`, where that geometry is supplied synthetically; it has never been
observed live.

This is not specific to the ladder -- it is the same for a single flip -- so it was left alone here
rather than guessed at. Settling it needs the retail meaning of `PhysOut.Ground +144/+160/+192/+208`
confirmed against the binary: the port currently feeds the deck *body* position where 82DA93D8
reads `block[144]`, which the handplant work (`82DAB7D0`) suggests is the **deck downward probe**,
a probe `board_ground.rs` does not issue at all.

The Air collector's bank had no trace of its own until now; `SCORE_TRICK air ...` was added in
`Runtime::finish` so the next investigation does not have to infer the components from the total.

## Retail has no late double, triple or quad

Asked and settled 2026-09-23, so it does not get re-opened: the ladder and the late mechanic are
separate systems in retail and they do not compose. Three independent confirmations.

**The scorable table has no numbered late entries.** `scoring/catalog.rs` carries exactly six late
ids -- `latekickflip` (105), `lateheelflip` (104), `latebsshuv` (102), `latefsshuv` (103),
`latebackfootkickflip` (101), `latebackfootheelflip` (100) -- plus `lateflip_darkcatch` (331).
There is no `latekickflip2/3/4`, where the ordinary flips carry `kickflip2/3/4` at 97, 98 and 135.

**The conversion table confirms it.** `conversions::LINKS[100..=105]` is `(-1, 128)` for all six:
they link into `ollie`, not into a ladder chain. A late flip has no rung to be converted from.

**The authored state has no cycle.** `Tricks/T_Lateflip.xml` is a single state with one
`PlayAnimation` and two `WillExpire` exits. `T_Kickflip.xml` is what a ladder looks like -- the
`Cyc1`/`Cyc2`/`Cyc3` chain and the `End.Out.Out1..3` air-time gates above. `skater_air.pat`
authors exactly six patterns, `L_F_Kickflip`, `L_B_Kickflip`, `L_F_Heelflip`, `L_B_Heelflip`,
`L_FS_Shuvit` and `L_BS_Shuvit`, and no held or repeated variant of any of them.

Adding a late ladder is therefore not a port fix; it would be inventing a trick Skate 3 does not
have, and it would need scorable ids beyond the retail 332, new `LINKS` rows and new clips.


## And the ordinary ladder has no fifth rung either -- but a mod now adds one

Everything above still describes the port. Retail stops at the quad, and that has not changed.

What is new, as of 2026-09-24, is an **opt-in mod** that extends the ordinary flip ladder past it:
`docs/endless-tricks.md`. Keep the two claims apart. "Retail has no fifth rung" is still true and
still what this engine does by default; the Endless Tricks mod adds rungs 5..16 on top, off unless
enabled, and the four flip tests above pass unchanged with its transitions compiled into the graph.

One correction to the measured table above, from re-running it: the kickflip quad now banks 451.67
at the 8 m/s boost rather than the 497 recorded here on 2026-09-21. The heelflip's 564 still holds.
That drift predates the mod work and has not been chased down.
