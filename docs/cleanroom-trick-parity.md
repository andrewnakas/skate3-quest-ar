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
  only when the air can fit them (port: the second flip ~21 ticks after the pop when held)
- 15 grabs: indy, crail, tail, stalefish, melon, nose, tuck knee, judo, double, superman,
  no foot, stiffy, Christ air, method, rocket air (triggers + left stick: centre / nose / tail /
  right / left)
- Manual + nose manual on the right stick part-way (and bumpers); push, mongo push (B), brake (Y)
- Grinds by the left stick at the catch (along the rail): 50-50, 5-0, nosegrind, crooked,
  overcrook, smith, feeble; slides (across it): boardslide, noseslide, tailslide, lipslide,
  noseblunt, bluntslide; the board turns back to the line of travel coming off a slide
- Powerslide; revert (X held into a landing, rolling away fakie)
- Wallride (an ollie glancing into a wall rolls along it) and wallie (flick on the wall to pop
  off it); a Wall piece in the park builder and the desktop park
- Old school on the foot-off button (left stick click / F) + a flick: no comply (ollie), boneless
  (nollie), no comply 180 both ways (shove-its); the front foot steps down while crouching;
  hippie jump (crouch with the button held, let go with no flick: the board rolls on underneath)

## Feel vs the port (scripts/cleanroom/feel.py, flip_* scenarios)
- Flick to takeoff 6 ticks (port 6), single flip level 33 ticks after the flick (port 32-34),
  held flick 2 flips in a flat ollie (port 2), landing a 360 body spin 5.1 m/s (port 5.2),
  air spin 341 deg (port 338)

## Next (in order)
1. Salad, suski, willy grinds
4. Footplants (on a wall / ledge / coping in the air), handplants
5. Darkslide, primo, casper, body varials; held shove-its (their flick ends at the centre)
6. Off-board: walk, caveman (jump on), get off / on

## Open polish
- Deep bends (noseblunt) show skin through the shirt (mesh interpenetration)
- Landing a held flip before its clip ends snaps the ankles (~100 deg/frame once)
- A sideways left stick held through the air spins the board as well as picking the grind
- Landing while still holding a grab blends grab + air + land; the back arm's blended rotation can
  jump for a frame (stalefish, ~118 deg/frame once)
- Plant at ~8 m/s is a fast leg sweep (real motion)
- audit `bytes`: glTF JSON keys + PNG IHDR headers match retail files (boilerplate)
