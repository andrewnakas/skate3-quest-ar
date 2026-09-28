# Endless Tricks

Hold a kickflip, heelflip, 360 flip or laserflip and keep going for as long as the pop lasts.

**This is not a Skate 3 trick.** Retail's ladder is four rungs and stops there — `T_Kickflip.xml`
authors `Cyc1`, `Cyc2`, `Cyc3` and nothing after it. This mod adds the fifth rung and beyond, and
the rest of the engine stays retail-exact: with the mod off, the ladder, the names and the points
are exactly what the game shipped with.

## Using it

1. `PLAY-ENDLESS-TRICKS.bat`
2. Escape → Mods → enable **Endless Tricks** → resume.
3. Find some height. Hold the flip flick through the whole air.

**There is no height bar.** Hold the input and you keep flipping for as long as you are off the
ground — more air simply means more rotations, not a threshold to clear. A flat pop still gets you
past what retail does.

The trade is that the ladder will happily start a rotation it cannot land out of, so you will bail
more than stock. **Require air for each flip** turns that around: it only begins a flip it has the
room to finish, which lands more and flips less. Retail's own air-time gates on the first four
kickflip rungs are untouched either way.

## Settings

| Setting | Meaning |
|---|---|
| Endless flips | Off restores the stock four-rung ladder exactly |
| Maximum flips | Most flips one pop may reach, 5 to 16 |
| Require air for each flip | Off by default. On, only start a flip you can land out of |
| Show status | Small on-screen reminder of the current limit |

## Scoring

The extra rungs are named and scored. They continue retail's own step — the authored ladder is
100 / 150 / 200 / 250, a flat +50 a rung, so a quintuple is 300 and the sixteenth flip is 850.

They also **convert** the way retail's rungs do rather than accumulating: a septuple banks the
septuple, not the sum of every flip under it. That is how the stock quad works too, and the extra
rungs are not allowed to behave differently.

Two families, named the way each is actually counted.

**Kickflips and heelflips** count by flip: `QUINTUPLE KICKFLIP`, `SEXTUPLE`, `SEPTUPLE`,
`OCTUPLE` — and from the ninth on, simply `ENDLESS KICKFLIP`. Past eight nobody is counting in
the air, so the named counts stop there and the trick takes the mod's own name.

**360 flips and laserflips** count by rotation, because a doubled 360 flip is a 720:
`720 FLIP`, `1080 FLIP`, `1440 FLIP` … and from the ninth on, `ENDLESS 360 FLIP`. Laserflips read
`720 LASERFLIP` and so on.

Nollie kickflips, heelflips and laserflips are included. **The nollie 360 flip is not** — it is
the one trick here with no authored cycle clip of its own, and the alternative looked bad enough
to be worth leaving out.

Every repeated rotation plays a purpose-built cycle clip that retail shipped and never wired up,
so a hold looks like a hold rather than the skater performing the trick over and over. One
consequence: rotations run at the cycle clip's own pacing, which is slower than the pop, so a given
drop buys fewer rotations than it would at trick speed.

## Limitations

- **Cannot run alongside the Native Trainer.** Native tuning is single-owner, and both mods want
  it. Enable one at a time.
- An endless flip cannot be underflipped past the fourth rung. The underflip and dark-catch
  transitions keep their authored priority, which is deliberate.
- Replays recorded with the mod on carry rungs a stock build has no names for.
