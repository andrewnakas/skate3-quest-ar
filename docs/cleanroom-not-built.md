# Clean tier: what is not built yet (status 2026-10-23, after the recorded port session)

## Built from the recorded session (2026-10-23, 29,640 ticks, rec.jsonl in scratch)
Darkcatch (RB during a flip) / dark to light (flick again) / landing dark runs it out on foot; darkslide
out (flick off a darkslide); late flips named "Late ..."; late backfoot kick / heel flips (flicked from
up in the air); finger flips (a stick sweep while grabbing, quick); shifties (grabbing, right stick down,
left stick to a side); airwalk (B in a nose / tail grab); superman (B in a double grab); footplant out of
a one-foot grab coming down; kickturns (slow, left stick hard over: eases in, ~180 deg/s while held, as
probed); left stick on a 5-0: salad / willy; handplant Up; powerslides named (left: Backside) and
scored; line multiplier a flat 1.5 (the port never went higher) and lines bank after 1.2 s; desktop
camera framed from the port's measured shots (riding 2.6 m back, ~2 m up; big airs up over the rider
looking down; plants low to the side; ~75 deg wide).

Everything below is in the port (the Rust rewrite, Skate 3's own data) or its trick list, and not yet
in sk8. "Probed" means tried with s3trace (scratch probe harness: `probe/probe.py`, `scen.json`,
`rail.py`, `batch2.py`, `dark.py`); "needs you" means the input was not found and is best shown by
playing the web port with the recorder (`recserver.py` on :8771), as for the plants.

## Replaying the recorded session through sk8 (2026-10-23)
Tools (scratch): `rec.jsonl` (the port session), `replay.py` (windows from the port's last ground tick),
`airs.py` (each air launched at the port's takeoff, `sk8trace --vel`), `grinds.py`, `flips2.py` (each
recorded stick path through s3trace and sk8, both regular stance). The park is KenneyPark dumped by
`build/mapdump` (collision + the port's derived grind lines) for `sk8trace --mesh ... --host-map`.
- Flicks: 39/43 recorded ground flicks named as the port names them for the same stick path. Fixed from
  the session: the up sector is wider toward the kick side (-18.5..+38.5 deg: 27/27 ollies), quick
  sweeps round the rim are shove-its, a flick read on arrival can be re-read during the takeoff.
  Open: the 3 flicks started from a crouch held LEFT (tre, varial kickflip, 360 inward heelflip from
  L-L-DL-D-DR-...) - the port reads them, we don't; one kickflip flicked up then on to up-right.
- Air moves: late backfoot flips are "stick held up, then snapped down" (2/3 replayed), grab names now
  the port's (Backside / Frontside / Crail / Seatbelt / Mute Grab), spins named on the trick ("Ollie 180")
  or Frontside / Backside 180 with no trick (left stick left = backside, probed).
- Grinds: the port pulls an air sideways onto a rail (~0.8 m, grind_assist 4.4 m/s^2), never catches a
  rail while still rising, pops off only on an up flick, lets the left stick spin the board near rails
  (how slides are set up) and changes a 50-50 with the left stick. 1/8 replayed catches named alike: the
  rest depend on the stance the recording doesn't hold (switch swaps sides), on ground-started flips
  (darkslides) and on rough launch velocities (the recorded position is the body root).
- Open: slide kinds by the right stick (the port gave a boardslide where we give a tailslide with the
  stick down) and the frontside / backside naming of slides.
- Drop-ins: not doable in the port either (Y never dropped in for the user); dropped from the list.

## Moves

### Not found in the port with any input tried (needs you)
- Darkslide exit directions (`DSOUT` BS/FS left/right/straight): ours is one "Darkslide Out".
- **Nose-pivot kickturns, manual snap-arounds** (kickturns are built; these weren't in the session).
- **Grab hops, grab powerslides, grab manuals** (`GROUND_TRICK_*_GRAB_HOP_NAME`,
  `*_GRAB_POWERSLIDE`, `NOSE_GRAB_MANUAL`, `TAIL_GRAB_NOSE_MANUAL`). In the probes a powerslide or
  manual cancels the ground grab, and a pop from it is a plain ollie.
- The port's finger-flip names say which grab it goes to (`STALE_TO_FSGRAB`...); ours "<grab> Finger Flip".
- The named one-foot grabs (`*_ONEFOOT_BEHIND/FRONT`): ours "<grab> One Foot".
- The tweaked handplant (`TWEAKED_HANDPLANT`).
- Left stick on a 50-50 (the port went to a lipslide / back-foot 5-0); ours does salad / willy off a 5-0 only.
- **Wipeout body tweaks** (cannonball, judo kick, swan dive, torpedo; `TYPE_INTENTIONAL_BAIL`) and
  the broken-bone slow motion.

### In the port, but could not be tested on its test course
- **Drop-ins**: the port's code has a dropping-in state but it never came up in the session (maybe
  not reachable on KenneyPark).
- **Pumping**: the port has a pumping system, but its half pipe keeps ~10 m/s with no input, so no
  gain showed. Needs a real map's transitions and a crouch-timing sweep.
- **Vert air height**: the course's coping box takes the port's speed at the lip, so its vert airs
  can't be compared with ours. sk8 currently airs ~2 m above the lip at 10 m/s.
- **Backflips / frontflips**: need more air than the course gives.

### Names / variants the port's trick list has that ours doesn't
- Underflips beyond the plain kick/heel (`*FLIP2/3/4_UNDERFLIP`, `POPSHUV_UNDERFLIP`,
  `FSPOPSHUV_UNDERFLIP`, nollie ones).
- Darkslide doubles / nose / tail versions (`*_DARK_SLIDE_DOUBLE/_NOSE/_TAIL`), and board-slide and
  lipslide nose / tail / double versions.
- "Authentic" names for spins with tricks: Cab, Half Cab, Big Spin, Bigger Spin, El Camino,
  McTwist, Rodeo, Alley-oop, Frontside flip (ours names spins "<n> Spin" with the trick).
- Spins named frontside / backside (`AIR_FS_SPIN`, `AIR_BS_SPIN`; both seen, but the recording has
  no body rotation to tell which way is which), reverts frontside / backside, 180 / layback
  powerslides, tail drag, ride-up curb.
- Skitching, moving objects, other off-board actions.

## Scoring (built today: our own rules after the port's)
Built: repeated tricks in a line decay (rails exempt), Big Air / Long Air bonuses, personal bests
(line, air time, grind length, manual length, tricks per line) with a "New best" HUD line.
Not built:
- the port's point scale (its kickflip is 20, ollie 2; ours 240 / 80) and its in-air height part of
  the sequence (it rises and falls with the air);
- per-air sequence rewards and the flip / spin / height metric curves (ours: fixed bonuses);
- the bail penalty as the port takes it (ours just loses the line);
- trick classes (air / flip / grab / grind / plant / manual / ...) driving anything;
- gap, step-up and drop-height metrics; the Quest (XR) HUD doesn't show the "New best" line yet.

## Camera (built today: our own shot set after the port's shot list)
Built (desktop): chase, crouch, manual, wall (riding up a ramp), low air, big air, re-entry, grind,
slow grind, plant, off board, bail; eased between, pulled in in front of walls, a landing shake.
Not built / not checked:
- never compared side by side with the port's camera (it's data-driven; no s3trace output for it);
- the port's ledge-left / ledge-right grind shots, frontside / backside re-entry and handplant shots,
  boneless / footplant shots, off-board air, the wipeout shots, the drop-in shot;
- the low / high camera choice (the port has both as an option);
- the port's authored shake curves (ours: one damped bob).
- Vert: riding up a quarter pipe the camera still looks a little down onto the rider, and after a
  drop back in it sits ahead of the rider for a moment while the heading swings round.
- On the Quest the camera is the headset, so none of this applies there.

## Animation
- Underflip: the board reverses but the body plays the plain flip clip.
- Board adjust: only the deck turns (the body doesn't react).
- Grind grab: the ground-grab pose blended ~75% over the grind (the hand isn't IK'd to the board).
- One-foot lift is procedural (the foot kicked out and down past its end of the board).
- Dismount plays the normal bail.
- Older polish still open: walk 33, wallride 46, primo 52 deg/frame worst frames.

## Not done at all yet
- None of the 2026-10-22/23 work has been tried on the headset (Quest APK + your test).
- Which pack the clean APK bundles (build-apk.sh still fetches the old non-clean core.pack), then
  the 0.03 release.
