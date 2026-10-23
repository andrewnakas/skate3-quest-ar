# Clean tier: what is not built yet (status 2026-10-23)

Everything below is in the port (the Rust rewrite, Skate 3's own data) or its trick list, and not yet
in sk8. "Probed" means tried with s3trace (scratch probe harness: `probe/probe.py`, `scen.json`,
`rail.py`, `batch2.py`, `dark.py`); "needs you" means the input was not found and is best shown by
playing the web port with the recorder (`recserver.py` on :8771), as for the plants.

## Moves

### Not found in the port with any input tried (needs you)
- **Dark catches** (flip, catch the board upside down; `DARKCATCH_*`, `RELEASE_DARKCATCH`, the
  `DSOUT*` darkslide exits). Every second-flick direction at six timings: nothing.
- **Kickturns, nose-pivot kickturns, manual snap-arounds** (`GROUND_TRICK_*KICKTURN*`,
  `*_MANUAL_SNAP_AROUND`). Left stick and bumpers in manuals, while stopped, and while braking: nothing.
- **Shifties** (`GRAB_*_SHIFTY_*`, `AIR_METRICS_*SHIFTY*`). A grab plus either stick: nothing. Board
  adjusts (the right stick held in a plain air) are built but score nothing, as in the port.
- **Grab hops, grab powerslides, grab manuals** (`GROUND_TRICK_*_GRAB_HOP_NAME`,
  `*_GRAB_POWERSLIDE`, `NOSE_GRAB_MANUAL`, `TAIL_GRAB_NOSE_MANUAL`). In the probes a powerslide or
  manual cancels the ground grab, and a pop from it is a plain ollie.
- **Airwalks** (`GRAB_NG_AIRWALK`, `GRAB_TG_AIRWALK`) and the named one-foot grabs (`*_ONEFOOT_*`):
  ours names any one-foot grab "<grab> One Foot".
- **Finger flips** (`FINGERFLIP_*`): the port's own notes say its stick-pattern file for them is
  missing, so they may not work in the port either.
- **Late backfoot flips** (`LATE_BACKFOOT_KICKFLIP/HEELFLIP`, `FLIP_BACKFOOT_*`).
- **Handplant "Up" and the tweaked handplant** (`*_HANDPLANT_UP`, `TWEAKED_HANDPLANT`): ours has
  Down / Left / Right and the feet-off ones.
- **Footplant grab variants** (`GRAB_BSGRAB_BS_FOOTPLANT` etc.).
- **Grind changes by the left stick on a rail**: in the port it switched to a lipslide or a back-foot
  5-0 (`BF_BS_5_O`); ours changes grinds with the right stick only.
- **Wipeout body tweaks** (cannonball, judo kick, swan dive, torpedo; `TYPE_INTENTIONAL_BAIL`) and
  the broken-bone slow motion.

### In the port, but could not be tested on its test course
- **Drop-ins** (stall on the coping, then drop in): the course has no deck to stand on. Needs the
  KenneyPark map in s3trace (load it as the map argument), or you in the web port.
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
- Spins named frontside / backside (`AIR_FS_SPIN`, `AIR_BS_SPIN`), reverts frontside / backside,
  180 / layback powerslides, tail drag, ride-up curb.
- Skitching, moving objects, other off-board actions.

## Scoring (built today: our own rules after the port's)
Built: repeated tricks in a line decay (rails exempt), Big Air / Long Air bonuses, personal bests
(line, air time, grind length, manual length, tricks per line) with a "New best" HUD line.
Not built:
- the port's exact multiplier behaviour (it refreshes on a big enough reward; ours is the trick count);
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
