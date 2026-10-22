# Clean tier trick parity (sk8 vs the faithful port)

Working list for the clean-room move set. Every trick is our own: engine logic in skatecore-cpp
`clean/`, clips authored in `scripts/cleanroom/blender/skater_anims.py`. Port behaviour (which input
does what, roughly how it plays) is the reference; no port code, data or numbers.

Verify each: `make sk8-test`, `SK8_DEMO=<slot> SK8_ANIM_TRACE=1 SK8_FIXED_DT=1`, a 3-view capture
(`SK8_VIEWS=3 SK8_CAPTURE=dir`, `SK8_DEMO_FROM=<s>` to start at a slot), audit (names/values).

## Done
- Ollie, nollie, 14 flip tricks (kickflip ... laser, nollie flips/shove-its), late flips;
  360 shove-its (the shove-it sweep carried on to straight up), impossible (down, centre, down)
- Held flicks like the port: the stick kept at a flick's end adds turns (double, triple, quad),
  only when the air can fit them (port: the second flip ~21 ticks after the pop when held);
  shove-its held at the sweep's sideways end: 360, 540
- 21 grabs (17 + 4 tweaks) picked the port's way (s3trace-probed): the right stick held, then the trigger(s).
  Centre: melon / indy / double; up: tail / seatbelt / tail; up-left: judo / Christ / superman;
  left: stiffy / tuck knee / double; down-left: stiffy / tuck knee / no foot; down: crail / nose /
  rocket; down-right: stalefish / method / method; right: stalefish / mute / mute (left / right /
  both triggers; swapped in switch). Not matched: the port's tweak-while-held variants (Japan...)
- Controller = the port's layout (measured with s3trace): A push, X mongo push, B brake, Y off the
  board; RB (Touch: right grip) powerslide / revert; manuals on the right stick part-way;
  left stick click old school (our own: the port's stick clicks do nothing while rolling)
- Off the board: Y steps off to stand / walk / run (left stick), the board carried on its edge;
  Y again drops it and steps on; on foot A jumps; running, Y is a caveman (a hop onto the thrown-down board)
- Footplant / handplant: foot-off (+ a grab for the hand) held reaching an edge: a stall, then
  back off it
- Grinds by the left stick at the catch (along the rail): 50-50, 5-0, nosegrind, crooked,
  overcrook, smith, feeble; slides (across it): boardslide, noseslide, tailslide, lipslide,
  noseblunt, bluntslide; the board turns back to the line of travel coming off a slide;
  darkslide (a kickflip-type caught across a rail half way round: upside down on it);
  salad, willy, suski (the crouch held into the catch with a diagonal)
- Powerslide (left stick back to a diagonal, the port's way; or the slide button + steer); revert
  (X held into a landing, rolling away fakie)
- Flick gestures = the port's, found by trying ~1200 stick paths in it (89% give the same trick;
  the rest are odd multi-direction paths): a flick goes by where it gets to (up-right kickflip,
  up-left heelflip), a slow sweep round the rim is a shove-it (a quick one just passes through),
  doubling back past the crouch adds the 360 (tre D L D R, laser D R D UL, 360 shove-its, 360
  hardflip, 360 inward heelflip), a brief lean to one side before crossing to up varials it; nollie
  versions of all of them; a flick nothing matches pops nothing. scripts/cleanroom/trick_table.py
  runs every trick through both engines (names, flips, shove turns, timing)
- Fakie and switch like the port: rolling fakie ~0.6 s once settled, the feet shuffle round to
  switch (0.7 s; the crouch held keeps it fakie; a push from fakie switches at once, quicker);
  a 180 from switch back to rolling forwards switches back. Tricks are named Fakie / Switch;
  switch flips and shove-its turn the goofy way. The body is the regular one mirrored (mirror.rs),
  one clip for the shuffle (to a pose the same both ways, played back mirrored); SK8_DEMO=switch
- Wallride (an ollie glancing into a wall rolls along it) and wallie (flick on the wall to pop
  off it); a Wall piece in the park builder and the desktop park
- Old school on the foot-off button (left stick click / F) + a flick: no comply (ollie), boneless
  (nollie), no comply 180 both ways (shove-its); the front foot steps down while crouching;
  hippie jump (crouch with the button held, let go with no flick: the board rolls on underneath);
  body varial (the button held in the air: the left stick turns the body alone);
  primo / casper (the button with the right stick part-way down / up)

- Bails in stages like the port: thrown (arms flailing), tumbling and sliding to a stop
  (~5.4 m/s^2, port ~5.5), lying still, getting up, back on the board where the body came to rest
  (~4.9 s, port ~5); thrown forward (face down) or slipped out backward (on the back; a wall slam
  bounces off the wall); SK8_DEMO=bail

## Feel vs the port (scripts/cleanroom/feel.py; every scenario within ~10%)
- Flick to takeoff 6 ticks (port 6), single flip level 33 ticks after the flick (port 32-34),
  held flick 2 flips in a flat ollie (port 2), landing a 360 body spin 5.1 m/s (port 5.2),
  air spin 341 deg (port 338)
- Grinds (grind_low: the test world's low rail, the port steered round onto it): speed kept on
  the catch, rail drag 0.83 m/s^2 (port 0.84), the board drawn down onto the rail, rolling off the
  end tips off with no hop and scrubs ~11% (port the same), a 50-50 leans onto the tail at the end
- Powerslide: bites in, eases off as it slows (speed kept at 0.25/0.5/1/2 s within 3%)
- Manuals: no wobble-out (the port holds one indefinitely with the stick still), 0.38 m/s^2
- Landings: sideways never bails (the port lands even 90 deg sketchy), a crooked landing skids its
  speed off over 0.3 s by angle (46 deg keeps 0.69, port 0.69)
- Brake: foot down 0.2 s, then 4.1 m/s^2; foot back up 0.42 s before a push can start
- Push: a burst as the foot bites, ~61% of the way to top speed (9.15) per stroke, strokes 1 s,
  first contact 1.0 s after a brake (port 1.0); coasting ~0.01 m/s^2
- Fakie/switch: grinds named for how they're ridden (Switch stance, Fakie when backwards on the
  rail), manuals for the stance; after a landing the feet only shuffle to switch once it has settled
  (0.8 s clean / 1.26 s sketchy, then 0.6 s: port ~85 / 105-122 ticks); a fresh manual waits 0.75 s
  after a landing unless held into it (port 45 ticks)
- Held flicks: the next turn is tried when the air looks long enough, judged hopefully
  (held_reach 1.26): out of a barely-crouched ollie (crouch 1-6 ticks) the held double comes down
  under-flipped, from 8 ticks it lands - as the port; landing lenience is counted in turns left
- Grab tweaks like the port: a one-hand grab started with the stick centred, the stick then
  pushed out: left trigger up nosebone, right tuck knee, down tailbone, left stiffy; right trigger
  up melon, right method, down japan, left cross bone (both triggers: no tweak)
- Ramps (sk8trace --world test: the port's test-course shapes as our own mesh): rolling down the
  roll-in within 1% of the port (slope_gravity 0.78), steering eases in like the port (two 0.11 s
  stages: turn-in matches tick for tick), push from a standstill 3.6 m/s (push_cap); a half pipe
  carve climbs the wall like the port; vert airs come back down into the transition with their
  speed (the port's air height there is cut by the test course's coping box, so not compared)
- Grinds by the right stick at the catch like the port (up nosegrind, down 5-0, right crooked
  held on into a boardslide, left lipslide, up-left overcrook, down-left willy; left stick back for
  smith/feeble/salad/suski), moving it on the rail transfers, let go it stays; pop off a rail as
  high as the port (grind_pop 0.7)
- Walls: at riding speed (7.5-9 m/s measured) the port stops against a wall without bailing; sk8
  now bails only above 11.6 m/s
- Not measured: kickers (KenneyPark geometry isn't loadable in sk8trace), the hard-landing
  threshold (no tall drop in the test world), plants and wallrides (port input unknown / no port
  wallride found)

- Poses vs port captures (web capture, analog pseudo-keys RY=/LX= in a local copy of the page):
  manuals stand tall, arms hanging, eyes ahead (board tilt 0.13 rad); the powerslide sits back over
  the heels with the arms spread; grinds/slides from the reference medians (taller, hands low)

## Next (in order)
1. Quest APK + headset test (needs the headset), then the 0.03 release decision

## Open polish
- Pre-existing, not yet looked at: worst frames walk 33, wallride 46, primo 52 deg/frame (primo
  62 foot slips)
- Port grinds were not captured on screen (the spawn rail's catch window is too narrow for the
  browser path); grind poses follow the reference-footage medians instead
- Grabs now <= 27 deg/frame (were 118-163)
- Plant at ~8 m/s is a fast leg sweep (real motion)
- audit `bytes`: glTF JSON keys + PNG IHDR headers match retail files (boilerplate)
