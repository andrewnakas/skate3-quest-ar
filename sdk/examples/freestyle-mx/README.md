# Freestyle MX

A freestyle motocross bike on the Rapier vehicle SDK. Lean-carved cornering,
suspension preload, wheelies and stoppies, whips and flips that straighten
themselves for the landing, a landing that judges you, and a rider who stands,
absorbs and hangs off with the bike.

## The model is not included

No third-party asset is committed. `bike.glb` is covered by the blanket `*.glb`
rule in `.gitignore`, so it stays out of the repository unless someone adds an
explicit exception for it — which is what `.gitignore:54` does for the kart's
model, the one GLB that is deliberately tracked. Supply your own and prepare it:

```powershell
# See what the importer found before committing to it
python tools/prepare_bike.py <your-bike.glb> --list
# Write bike.glb and print the wheels[] block
python tools/prepare_bike.py <your-bike.glb> sdk/examples/freestyle-mx
```

The importer scales by the wheelbase, faces the bike +Z, puts the origin on the
centreline above the axle line, and emits named `wheel_front` / `wheel_rear`
pivots. Paste the printed `wheels` array into `vehicle.json` if the radii differ
from the defaults. The GLB must embed its textures: the host rejects external
buffer and image URIs, and caps the model at 32 MiB.

A verified free option is **[KTM 450 EXC by mx-3d](https://sketchfab.com/3d-models/ktm-450-exc-d4a99a39c0ff48378ba47af8f8b74dea)**,
CC BY 4.0, 124.9k triangles — credit the author if you redistribute a package
built on it. Nothing truly CC0 was found at usable quality with separable wheels.

**A different model needs its rider re-fitted.** `seat`, and the `SEAT`, `GRIP`
and `PEG` constants in `tools/author_bike_poses.py`, are measured off this
bike's geometry. See *Rider animation* below.

## Controls

The two-stick split the freestyle motocross games use: **left stick is the bike,
right stick is the rider**.

| | |
|---|---|
| Left stick X | Steer. In the air, the bars whip the back end round |
| Left stick Y | Bike attitude. **Back builds preload**; back under power wheelies; in the air it pitches for flips and scrubs |
| Right stick X | Rider weight left/right — hang off the inside on the ground, roll for a tabletop in the air |
| Right stick Y | Rider weight fore/aft |
| RT / LT | Throttle / front brake |
| B | Rear brake (locks it for a slide) |
| **A** | **Clutch.** Hold it and rev, let go to loft the front |
| **LB or RB + right stick** | **Throw a trick.** Release to tuck back in |
| Y or E | Get on / off (below 3 m/s to get off) |
| R or R3 | Reset |
| F9 | Respawn the bike nearby |

Keyboard: `WASD` ride, arrows for bike attitude, `IJKL` for the rider, `Z`/`X`
trick modifiers, `C` clutch, `Space` brake, `Left Shift` handbrake.

### Cornering

The bars lean the bike and the lean carves the turn — a leaned bike follows the
radius its lean angle dictates, so the same stick gives a 3 m turn at walking
pace and a 23 m sweeper at 60 km/h without you changing anything. The tyres still
have to hold it: too much lean on the throttle steps the back end out, and the
rear brake will break it loose deliberately.

### Preload is the point

Pull the left stick back on the face of a jump and snap it forward at the lip.
The suspension stores the load and releases it, so jump height is a skill rather
than a lookup of your approach speed. Everything else follows from getting that
one input right.

### Whips, flips and landings

The bars in the air send the back end round and lay the bike over with it; a
held stick pitches for a flip, and the right stick rolls for a tabletop. **Let
go and the bike straightens itself** — it levels and swings the nose back to the
line you are travelling on, which is what makes a whip landable. Hold it out all
the way down and you have not whipped it, you have cased it.

Landings are judged against the face you land on, not against flat, so a steep
downslope is fine if the bike matches it. Land far enough sideways, rolled or
nose-down and you come off. Crash and, by default, the bike stands itself up and
puts you back on in about two seconds.

### Tricks

Hold a bumper and push the right stick. The extension follows how long you hold
it, so a half-thrown trick reads as half-thrown.

| | Up | Down | Left | Right |
|---|---|---|---|---|
| **LB** | Superman | Lazy Boy | Nac-Nac | Can-Can |
| **RB** | Heel Clicker | Kiss of Death | Cordova | No-Hander |

**Tuck in before you land.** Still hanging off the bike at touchdown is a case:
no score, and the combo resets. That rule is what makes a trick a decision.

## Scoring

Per air: trick base value scaled by how long it was held, plus 100 a quarter turn
of spin, 500 a flip and a bonus for a whip sent past ~30° and brought back
square, multiplied by air time and then by the combo count. Landing clean chains
the combo; casing or crashing resets it. Turn it off in the mod settings if you
would rather just ride.

## Recovery

Two failures a rider cannot ride out of are handled for you: a bike wedged
against geometry (throttle held against a bike that will not move) is stood back
up where it last had speed, and one that falls out of the world is put back at
the same place. Automatic remounting after a crash can be switched off in the
settings if you would rather walk back to it.

## Tuning

Handling is exposed live in the mod settings: lean angle and response, how much
bar input becomes lean, cornering bite, preload pop, whip and flip rates. **None
of these are recovered constants** — they are authored arcade values, meant to be
swept by ear while riding. The headless tests in
`crates/skate-vehicles/tests/bike.rs` check that the bike stays up, carves,
wheelies without looping out, pops off preload, straightens for a landing and
bails when it should; they say nothing about whether it feels good.

## Rider animation

`rider.json` holds seventeen single held poses: the seated stance, six riding
postures the host blends by what the ride is doing, two steering poses and the
eight tricks. Regenerate them with

```powershell
& $blender -b --python tools/author_bike_poses.py -- `
    --skater <your assets/private/skater.glb> --bike sdk/examples/freestyle-mx/bike.glb `
    --out sdk/examples/freestyle-mx/rider.json --preview .local/poses
```

The tool fits the rider onto the bike's own measured grips and pegs, checks its
own export basis against the reference rest pose, prints how far every hand and
foot missed, and renders a side and front view of each pose. **Look at the
renders**; the residuals will not tell you that a pose is ugly. `SEAT` there and
`seat` in `vehicle.json` are the same point and must stay in step.
