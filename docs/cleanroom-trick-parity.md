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
- 15 grabs: indy, crail, tail, stalefish, melon, nose, tuck knee, judo, double, superman,
  no foot, stiffy, Christ air, method, rocket air (triggers + left stick: centre / nose / tail /
  right / left)
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
- Flick mapping = the port's (measured with s3trace): kickflip finishes up-right, heelflip up-left,
  pop shove-it sweeps round the right, frontside round the left
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

## Feel vs the port (scripts/cleanroom/feel.py, flip_* scenarios)
- Flick to takeoff 6 ticks (port 6), single flip level 33 ticks after the flick (port 32-34),
  held flick 2 flips in a flat ollie (port 2), landing a 360 body spin 5.1 m/s (port 5.2),
  air spin 341 deg (port 338)

## Next (in order)
1. Polish list below (every move on the list is in)

## Open polish
- Deep bends (noseblunt) show skin through the shirt (mesh interpenetration)
- Landing a held flip before its clip ends: ankles ~54 deg/frame (was 100)
- Handplant: the legs twist as they swing up overhead (~88 deg/frame, mostly twist along a near
  straight leg); the arm is fixed (hand plants first, then the body tips over it)
- Grabs now <= 27 deg/frame (were 118-163)
- Plant at ~8 m/s is a fast leg sweep (real motion)
- audit `bytes`: glTF JSON keys + PNG IHDR headers match retail files (boilerplate)
