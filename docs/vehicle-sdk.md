# Vehicle SDK v1
Arcade assist additions (kart package 1.5): optional `ground_stability` (0..1,
default 0) adds road-normal roll stabilization while at least two wheels touch;
optional `air_control` (0..10 rad/s², default 0) enables occupied-aircraft-style
pitch/roll torque control when no wheels touch. These are authored gameplay
assists inspired by the requested Burnout Paradise feel, not recovered constants.
`VehicleControls.pitch` is optional, -1..1 (positive nose down), mapped from left
stick vertical or Up/Down arrows. Existing steering maps to airborne roll.
Neutral air input damps pitch/roll without adding linear acceleration.
The kart uses ground_stability=0.8, air_control=6, broader inertia dimensions,
crash_delta_v=12 m/s, rider hit_impulse=1200 N·s and a 1-second grounded inversion
timer. Inversion without chassis/rider contact no longer causes an ejection.
Default safety settings on other vehicle definitions are unchanged.


`skate-vehicles` pins Rapier 3D 0.35.3 and owns an independent, fixed-step rigid-body
world. The existing skating solver remains authoritative outside vehicles. Rapier
uses the installed map's collision triangles and resolves chassis collisions with
the map and other vehicles. Wheels use Rapier raycast suspension. Each host tick
is split into substeps of approximately 1/120 second or smaller.

### Force-based handling (September 2026)

The shared `skate-vehicles/src/handling.rs` now owns tire forces. Rapier's built-in
arcade tire impulses are disabled; its suspension queries and chassis collision
solver remain active. Steering has a 2.5 rad/s travel rate, speed-sensitive lock
and Ackermann front-wheel angles. Wheel torque integrates angular momentum;
longitudinal slip and lateral slip angle produce contact impulses constrained by
one load-dependent friction circle. Contact-point application produces chassis
pitch/roll, and dynamic supports receive opposing tire and suspension impulses.

`tire_grip` is now the tire friction coefficient, multiplied by the contacted
collider's friction. The kart uses 1.3 for a dry setup. Its new `tire_friction`
setting intentionally replaces the old arcade `grip` setting so a saved value of
3 is not reused as a physical coefficient. Other saved settings remain compatible.
Older third-party definitions should review their grip values with this build.

Service brake overrides throttle; an opposing throttle request brakes before
engaging the other direction near rest. Handbrake locks non-steering wheels.
Brakes scale with simulation time (the legacy `brake_impulse` setting retains its
120 Hz reference scale). Torque tapers with wheel speed rather than switching off
at a chassis-speed threshold. Reverse is limited by torque taper at up to 8 m/s.
Aerodynamic drag and axle rolling resistance slow coasting. Published speed is
the post-solve longitudinal velocity; falling or sliding sideways does not count
as forward speed. Reset clears steering, wheel angular momentum and displayed spin.

This is a simplified physical kart model, not a measured tire/engine dataset:
wheel inertia is estimated from chassis mass, lateral stiffness is a fixed model
coefficient, and suspension still uses raycasts. There is no gearbox, tire heat,
deformation, ABS, or validated moving-platform suspension damping. Engine sound
still follows speed/throttle. Automated physics tests do not establish rendered
handling quality; steering feel and unusual map surfaces require playtesting.

Chassis contacts use the minimum friction combine rule: the kart's 0.05 body
friction against a 1.0 map stays 0.05 instead of averaging to 0.525. Tire friction
still reads the map material independently. The imported collision mesh welds
identical vertices and enables internal-edge normal correction while retaining
all authored triangles. Crash delta-v is sampled immediately before collision
integration, after tire/suspension impulses, so ordinary driving forces are not
included in that impact measurement. Rider contact and inversion tests remain active.

## First mod: Mario Kart

Run PLAY-MARIO-KART.bat, enable **Mario Kart** in Mods, resume and press **F10**.
Walk/skate within four metres and press **E** to enter. **W/S** accelerate/reverse,
**A/D** steer, **Space** brakes, **left Shift** handbrakes, **R / right-stick click** rights/resets.
**E** exits once speed is below 3 m/s. F10 replaces a parked kart or resets an
occupied kart. Live engine, speed, brake, steering and grip controls appear in its
own draggable, resizable, scrollable mod window.

Controller: **Y** enter/exit, **RT/LT** accelerate/reverse, **left stick** steer,
**A** brake, **B** handbrake, **right-stick click** reset while driving. The reset bind can be changed to left-stick click in the mod settings. F10 remains the keyboard spawn shortcut.
Release controls before switching between driving and skating.

`sdk/examples/mario-kart/kart.glb` is generated from the user-supplied ZIP. It is excluded
from Git. The importer preserves meshes/materials/embedded textures, normalizes
the model to metres and creates named wheel pivots. To regenerate:

```powershell
python tools/prepare_mario_kart.py <kart-source.zip> sdk/examples/mario-kart
```

NumPy is required by the model importer. Editable sources live in `sdk/examples/`;
`Build.ps1` packages them into top-level `mods/*.zip` and stages a release.
The project launcher loads top-level mods/; a standalone executable defaults to its
adjacent mods directory. See [mod-packages.md](mod-packages.md) for the ZIP layout,
validation, update rules and development-folder support. No game assets are committed.

## Lua API

All keys belong to the calling mod. Commands cannot control another mod's vehicle.
The host allows eight vehicles per mod and 32 total. Definitions are validated
before allocation. Commands from failed Lua callbacks are discarded. Disabling,
removing or reloading a mod removes its vehicles and releases its driver.

```lua
sdk.vehicle.spawn('kart', 'vehicle.json', {x, y, z}, heading_radians)
sdk.vehicle.enter('kart')
sdk.vehicle.control('kart', {
    throttle = 1,       -- -1 reverse through +1 forward
    steering = 0.25,    -- -1 right through +1 left
    brake = 0,         -- 0..1
    handbrake = false,
})
sdk.vehicle.tune('kart', {engine_force=1800, max_speed=25, tire_grip=3})
local car = sdk.vehicle.read('kart')
sdk.vehicle.exit('kart')
sdk.vehicle.reset('kart', {x, y, z}, heading_radians)
sdk.vehicle.remove('kart')
```

`read` returns nil for an absent vehicle, otherwise position, quaternion rotation
(x/y/z/w), heading, signed speed in m/s, occupied, ready and phase. Phases are
`parked`, `entering`, `driving`, `exiting`. Snapshots update at host callback boundaries;
a spawn command is visible on a later callback. `input()` returns normalized
keyboard/controller throttle, steering, brake, handbrake, interact and pad_buttons.
Do not pass the full input table to `control`; copy only its four control fields.
`control` must be refreshed; after 0.25 seconds without a command, throttle clears
and a parking brake is applied. Controls are ignored during enter/exit animations.

`tune` supports optional engine_force (0..100000 N), max_speed (1..100 m/s),
engine_volume (0..1), brake_impulse (0..10000), steering_angle (0.01..1.2 radians), tire_grip (0.1..20).
Maximum speed limits engine application; it is not an absolute downhill speed cap.

Events passed to `on_event`: `vehicle_spawned`, `vehicle_entering`, `vehicle_entered`,
`vehicle_exited`, `vehicle_exit_blocked`, `vehicle_reset`, `vehicle_removed`,
`vehicle_bailed`.
Each includes owner and key. Map changes use the existing `world_changed` event;
spawn new vehicles in the new world as needed. Invalid assets/commands appear as
mod errors; they retire the mod's vehicles instead of crashing the game.

## Vehicle definition

See `sdk/examples/mario-kart/vehicle.json` for a complete working definition. Coordinates
are metres, +Y up, +Z forward, +X driver-left, with heading in radians around +Y.
`half_extents` describes the outer chassis collision bounds. `mass` is kilograms. `model_scale`,
`model_offset` and `model_yaw` affect the model only, not its collider.

Each wheel defines a chassis-local suspension mounting `position`, radius,
steering/driven flags and optional unique model node name. Support is 2..8 wheels
with at least one driven wheel. Wheel nodes should have local +Y steering and +X
axle axes. Pivots must be correctly positioned; the host adds suspension travel,
steering and spin to their rest transforms. The supplied kart importer does this.

Seat and exit are chassis-local offsets. Exit checks the nearby fixed ground and
standing clearance; if unavailable, it returns the rider to the saved entry spot.
The camera follows using camera_distance and camera_height. Model GLBs must embed
all textures and buffers; filesystem/network references inside GLBs are rejected.
A package may total 64 MiB; a model is limited to 32 MiB and an animation file to
16 MiB. Paths stay within the owning mod folder.

## Rider animation support

The local kart package supplies fitted clips. Other definitions default their animation slots to null. Without a matching
clip, the skater is hidden and entering/exiting completes immediately. This avoids
showing an unrelated standing/skating pose on the kart.

Set `animations.file` to a package-relative JSON file and set slot names:

```json
"animations": {
  "file": "rider.json",
  "enter": "get_in",
  "exit": "get_out",
  "drive": "drive_loop",
  "idle": "seated_idle",
  "reverse": "look_back",
  "brake": "braking",
  "steer_left": "turn_left",
  "steer_right": "turn_right"
}
```

The file is:

```json
{
  "version": 1,
  "bone_names": ["EXACT_NAMES_FROM_sdk.animation.info().bone_names"],
  "clips": {
    "drive_loop": {
      "fps": 30,
      "frames": [["ONE_16_NUMBER_MATRIX_PER_BONE"]]
    }
  }
}
```

The strings in the example frames/bone list are explanatory placeholders.
Actual frames must contain one 16-number, column-major, native model-space matrix
per bone, in the exact native skeleton order. Matrices use metres and +Y up,
matching the existing authored animation convention. They are full poses, not
additive deltas. Obtain names from `sdk.animation.info()`; do not invent bone names.
The renderer performs the existing native-to-GLB skin-basis conversion. Author the
pose relative to the seat offset; enter/exit root motion is visual and does not
move the Rapier chassis. The native skater is restored through its teleport path
when exiting. Files with a mismatched skeleton, invalid matrices or missing named
clips are rejected. Maximum 32 clips, 3600 frames per clip, 1..120 fps.

Enter/exit clips play once and their frame count/fps determines transition duration.
Driving slots loop; brake and reverse select their respective base clips, with drive
as fallback. Steering continuously blends that base towards the left/right pose.
Vehicle phase changes blend with native bone-local interpolation; brake/reverse
base-clip changes currently cut, so author compatible seated poses.
Edit vehicle.json or rider.json in the authoring folder, rebuild its ZIP, then reload
the mod (or wait for automatic rescan) to install new clips. Never edit .cache.

## Current boundaries

The native skating simulation is suspended while driving and resumes on exit.
Offline vehicle time pauses with Escape; online physics continues while menus are
open and unattended controls fall back to braking. Replay entry and session-marker
controls are blocked while driving. Vehicle motion is not recorded in skating replays.

Vehicles collide with the map, other cars and nearby native player/board proxies.
Native skating collision queries include car chassis. Matching enabled packages
replicate vehicles, tuning, wheel/rider poses, occupancy and engine audio. Crash bails
return to normal native ragdoll networking. See [Multiplayer mod SDK](multiplayer-mods.md)
for owner simulation, latency limits, hitbox approximations and late joins. Driving
another player's vehicle, passengers, weapons, damage and race rules are not built in.

Verification uses headless Rapier tests and window-free Lua tests. The game is
not launched automatically; rendering, entry/exit and handling need manual playtesting.

### Rider transitions and steering
The host blends vehicle phase changes over 0.4 seconds and the return to vanilla over
0.5 seconds, using the game's bone-local translation/scale lerp and quaternion slerp.
Steering input is smoothed and blends the current driving pose towards steer_left or
steer_right continuously, so these slots should contain compatible seated poses.
Vehicle camera hand-offs ease over 0.5 seconds. While driving, chassis, wheels, rider
and camera share the same interpolated fixed-step motion sample, avoiding relative
jitter from separate camera damping. Resets discard the old motion sample.
The board root is scaled away during vehicle playback and restored by the vanilla blend.
The local Mario Kart package supplies fitted entry/exit, seated and steering clips;
its fitted rider.json is included in source control with the prepared kart model.

## Ramp clearance and mass distribution
These are shared definition fields, available to any mod, not Mario-specific code:

| Field | Meaning | Default |
| --- | --- | --- |
| `collider_offset` | Chassis-local collision centre, metres | `[0,0,0]` |
| `collider_rounding` | Rounded edge radius; below 95% of smallest half extent | `0` |
| `chassis_friction` | Body contact friction, 0..2 | `0.3` |
| `center_of_mass` | Chassis-local mass centre, metres | `[0,0,0]` |
| `inertia_half_extents` | Box dimensions used for mass distribution, independent of contact shape | null: use half_extents |

Rounding stays inside half_extents. Shorten low front/rear overhangs so wheel contact
can lift the chassis before the body catches the ramp. Keep the collider large enough
to protect the cockpit; the visual model does not define collision. Inertia dimensions
let a shorter collision shape retain stable pitching/rolling behaviour. A lower centre
of mass helps resist nose-diving under braking. These fields require respawning.
The example passes headless 20/30/40-degree incline tests plus braking and steering tests;
this does not guarantee every map seam or vertical ledge is traversable. Wheels remain
raycasts, and vertical walls remain obstacles. No teleporting or artificial ramp boost is used.

## Engine sound
Opt in per definition:

```json
"engine_audio": { "enabled": true, "volume": 0.45, "idle_pitch": 0.7, "max_pitch": 2.8 }
```

The host synthesizes an original layered exhaust pulse with filtered noise. No downloaded
sound asset or extra file is required. It plays for occupied local and remote vehicles, with distance attenuation for remote engines; parked engines
are silent. Pitch follows speed and absolute throttle, including reverse and free revving
while stopped. Full throttle increases volume; releasing it smoothly drops the engine back
towards idle. Pitch/volume changes are smoothed, Escape/replay fades it silent, and exiting
or disabling the mod fades/removes the voice. This is a driver-focused mono sound, not a
spatial multi-car mixer or a simulated gearbox. There is currently no custom sample slot.

Volume is 0..1, idle_pitch 0.25..2, max_pitch idle_pitch..5 (multipliers of the 80 Hz
synth fundamental). All are validated. `sdk.vehicle.tune(key,{engine_volume=0.5})`
changes volume live; use zero to mute. Mario Kart exposes this in its mod settings.

## Authoring and reuse
See [Mixamo vehicle animation workflow](mixamo-vehicle-workflow.md) for the full sequence,
calibration, Blender fitting, native export and packaging commands. The host's animation
slots, steering blend, stance hand-offs, board hiding and shared motion interpolation
apply automatically to every vehicle definition. The example fitting scripts contain
kart geometry targets; adjust those targets for another vehicle without changing the host.

## Rider hitbox and crash ejection

Every vehicle can use the shared `rider_safety` definition. It is enabled by default;
explicit example values are shown below. Changes require respawning the vehicle.

```json
"rider_safety": {
  "enabled": true,
  "offset": [0, 0.5, 0],
  "radius": 0.25,
  "half_height": 0.25,
  "crash_delta_v": 6,
  "hit_impulse": 180,
  "inverted_up_y": -0.2,
  "inverted_seconds": 0.2,
  "eject_up_speed": 2
}
```

A solid capsule is attached to the occupied chassis, centred at `seat + offset`.
It covers the seated torso/head, including roof/ground/overhang contact when rolled.
It is not a sensor: contacts affect the vehicle. It adds no mass, so existing mass and
inertia tuning remains authoritative. Empty vehicles have no active rider collider.
The capsule is an approximation, not separate animated hand/foot hitboxes; adjust it
for your cockpit and expected character proportions. It is active through entry/exit
as well as driving, and is removed from collision when ownership ends. Disabling
rider_safety disables both this collider and automatic ejection.

Ejection triggers on any of:

- A chassis collision changing linear velocity by at least crash_delta_v in a physics
  substep. This is delta speed in m/s, not total driving speed; braking/ordinary ramps
  should not meet the default threshold.
- A rider-capsule contact impulse at least hit_impulse, in N·s. This is an impulse
  threshold, not force in newtons, and is evaluated on actual Rapier contacts.
- Chassis-local up having world Y below inverted_up_y continuously for inverted_seconds.
  The timer clears when upright again. Reset clears pending impacts/inversion history.

The host stops driver controls, releases occupancy and fades engine audio. It queues
an actor reset just above the seat, enters the native Wipeout ragdoll after normal reset
initialization, and seeds the native body and board velocities. Seat point velocity
includes the vehicle's angular motion. Pre-impact velocity is retained when the car
stops abruptly; if a hit accelerates the car from rest, the stronger post-impact point
velocity is used instead. A small world-up launch speed helps clear the seat. Output
linear/angular speeds are bounded to 60 m/s and 15 rad/s. This is a momentum hand-off,
not a fully coupled passenger rigid-body simulation or an exact seated ragdoll pose.
The crash visual hand-off lasts 0.12 seconds; normal entry/exit blends remain unchanged.
Native map collision and ordinary bail recovery take over. After release, native rider
collision against the Rapier car itself is still not bridged, as described above.

Lua receives **vehicle_bailed** instead of the normal vehicle_exited event:

```lua
on_event = function(event)
    if event.name == 'vehicle_bailed' then
        -- event.key is this mod's vehicle key; reason is crash/rider_impact/inverted.
        sdk.log('Ejected: ' .. event.reason)
        -- event.position: world seat position at detection, metres
        -- event.velocity: carried world velocity including launch lift, m/s
        -- event.angular_velocity: world angular velocity, rad/s
    end
end
```

The vehicle remains spawned and can be reset or entered again after bail recovery.
Use vehicle.read().occupied/phase for ownership; do not keep sending driving input
while unoccupied. Physics events are delivered through the existing mod callback queue.
The shared Rust simulation also exposes set_occupied and take_ejection for host tests;
Lua cannot directly write native ragdoll bodies or bypass the hand-off lifecycle.

Bounds: offset components ±5 m, radius 0.1..1 m, half_height 0.05..1 m (capsule cylinder
half-length; total height is 2*(half_height+radius)), crash_delta_v 1..50 m/s,
hit_impulse 10..10000 N·s, inverted_up_y -1..0.5, inverted_seconds 0.05..3 s,
eject_up_speed 0..10 m/s. Nonfinite values are rejected. Headless tests cover wall
momentum retention, overhang rider hits and sustained inversion versus empty vehicles.
Native rendering/recovery and unusual map geometry still need manual playtesting.

## Single-track (two wheel) vehicles

Optional `bike` section, absent and disabled on every existing definition, so
nothing here changes how a four-wheel vehicle drives. It requires exactly two
wheels, one of them steering.

```json
"bike": {
  "enabled": true,
  "lean_max": 0.95, "lean_rate": 9, "counter_steer": 1, "upright_gain": 60,
  "steer_falloff": 0.06, "cornering_scale": 2, "lean_yaw": 8,
  "preload_force": 1100, "preload_release": 500,
  "air_yaw": 9, "air_pitch_down": 11, "air_pitch_up": 11, "air_roll": 9,
  "flip_rate": 5.5, "whip_rate": 3.5, "air_level": 3,
  "wheelie_limit": 0.85, "stoppie_limit": 0.5, "clutch_boost": 1.6,
  "landing_roll": 0.75, "landing_pitch": 0.95, "landing_yaw": 0.9
}
```

`ground_stability` and `air_control` are not the right tools for a bike:
`ground_stability` needs two contacts (a bike rides on one wheel often enough
that this alone disqualifies it) and stabilizes roll **toward the road normal**.
With `bike.enabled`, the `assists` module is replaced by `bike`.

### Lean is handling state, not body roll

**The chassis body never rolls.** While a wheel is down it is held upright over
the contact line, and `lean` is a separate scalar the handling owns. This is the
single most important thing to know about the module, and it is not a stylistic
choice — a physically leaning chassis cannot work here:

Rapier casts each wheel's suspension ray along `direction_cs` **rotated by the
chassis**. A body rolled 50 degrees therefore casts its rays sideways-down, reads
its suspension as extended, sinks, and grinds its collider along the floor. That
is what a leaning bike actually did: it rode badly, wedged itself on flat ground
and could not be ridden out of. The arcade motocross games all separate the two
for the same reason.

So `lean` does three jobs:

* **It steers the bike.** A leaned bike carves the radius its lean dictates:
  steady cornering balances gravity against centripetal acceleration, so
  `v²/R = g·tan(lean)` and the turn rate is `g·tan(lean)/v`. `lean_yaw` is the
  authority toward that rate. Faster is wider for the same lean, exactly as on
  a real bike, and it falls out of the physics rather than a tuned curve.
* **It rolls the model and the rider**, about the contact line rather than the
  chassis origin, so the bike visibly hangs its mass over the inside. The host
  reads it through `Simulation::bike_state` and applies it in `present`.
* **It drives the rider's posture**, through the `lean_left`/`lean_right` layer.

Tyre forces still go through the friction circle in `handling::tires`, so too
much lean on the throttle still steps the back end out. Lean is what the bike is
asking for; grip decides whether it gets it.

Roll damping is derived from `upright_gain` rather than authored, so raising the
gain cannot tune the bike into an oscillation. The controller also **cancels the
moment the ground reaction makes about the mass centre**, measured live, so the
angle it is asked to hold is the angle it settles at. Two things about that are
worth stating, because both were wrong at some point and only one of them was
visible on flat ground:

* Without any cancellation the controller is fighting a load that grows with the
  roll, and below about 84 rad/s² on this geometry it loses and the bike slowly
  lies down. That is not a number anyone should have to find by eye.
* The reaction acts along the **contact normal**, not along world up. A bike
  already square to a cambered slope has its contact patch directly beneath its
  mass centre *along that normal*, so the real moment there is zero — while a
  feed-forward written against world up computes `m·g·h·sin(camber)` and injects
  a torque that drags the bike back toward world-vertical. The symptom was a
  bike that took up only about half of a side-slope, and almost none of one that
  also climbed: ride onto an off-camber face and the tilt fought you. On the
  flat the normal *is* world up, which is exactly why it read as correct.
  `tests/bike.rs` pins this with a cambered ground plane.

### Berms, and how much of one is already free

A berm is a banked corner, and the useful thing about it is that **most of the
effect is already there without any code**. The suspension pushes along the
contact normal; on a berm that normal leans toward the turn centre, so the
ground supplies centripetal force directly and the tyres never have to. This
is easy to miss because it cannot be seen on a tilted plane -- there the same
component points downhill, and the bike simply runs away from the corner
instead of round it. It only appears on a *curved* bank, which is why
`tests/bike.rs` builds a bowl rather than a slope.

`berm_assist` adds the measured bank to the rider's lean inside `carve_rate`,
on the reasoning that a bike leaned `l` on a bank `b` corners as though leaned
`b + l`. That reasoning is correct and the term is still only worth **0.2**,
because the ground has already done most of the work. Measured on a 26 degree
bowl entered at 14 m/s, holding full lock for three seconds:

| `berm_assist` | swept | speed out |
| --- | --- | --- |
| 0.00 | 3.50 rad | 20.6 m/s |
| **0.15** | **4.05 rad** | **18.5 m/s** |
| 0.30 | 4.78 rad | 11.2 m/s |
| 0.45 | 5.09 rad | 6.9 m/s |
| 1.00 | 4.06 rad | 8.1 m/s |

Past about 0.45 the bike sweeps *less* for more command, which is the
signature of a slide: the carve is asking for a yaw rate the tyres cannot
convert into a change of direction, so it scrubs instead. The physically exact
value of 1 is the worst of both -- it leaves a corner at 8 m/s that it entered
at 14, where no assist at all leaves at 20. A berm should sling you out.

Two smaller things fall out of the same term for nothing. Off-camber is a bank
of the opposite sign, so it subtracts and the bike pushes wide. And riding
straight across a camber pulls you downhill, which is what a real bike does.

**The clamp before the tangent is required, not tidy.** `lean_max` is 60
degrees and a steep berm is another 40; the sum lands past the singularity at
90, where `tan` returns a large *negative* number. Unclamped, the bike snaps
into turning the wrong way at exactly the moment the rider has committed
everything to the corner. At the 80 degree clamp `tan` is already 5.7, far
more turn than any tyre will hold, so nothing real is given up.

### Steps and stairs

A raycast wheel samples the ground at a single point, so it has no footprint
and **cannot roll over an edge**. A riser arrives as an instantaneous jump in
ground height: the suspension compresses in one frame, spikes, and throws the
bike. A whole flight is worse than one step, because the chassis pitch lags
the staircase slope, so the frame ends up driven into a riser two steps ahead
of the front wheel while that wheel is still down on the first tread. Measured
at 8 m/s into 0.20 m risers, the chassis took a single 1717-unit shove along
its own heading and the bike went from 11.2 m/s to a standstill inside one
frame.

**Reshaping the chassis does not fix this**, which is worth stating because it
is the obvious thing to try. Five colliders were measured over a twelve-step
flight, averaged across five entry phases so the result was not one lucky run:

| chassis | 0.15 m rise | 0.18 | 0.20 | 0.25 |
| --- | --- | --- | --- | --- |
| as shipped | 4/5 | 0/5 | 0/5 | 0/5 |
| front pulled inside the wheel | 5/5 | 0/5 | 0/5 | 0/5 |
| front in, belly raised | 0/5 | 0/5 | 1/5 | 0/5 |
| compact, heavily rounded | 5/5 | 0/5 | 0/5 | 0/5 |
| **as shipped, `step_assist` 0.3** | **5/5** | **5/5** | **5/5** | **5/5** |

Pulling the front inside the wheel and raising the belly both made things
*worse* on balance. The chassis has to be moved, not reshaped.

`step_assist` is the tallest step, in metres, the bike will climb. When one is
found it places the chassis on top of it and hands back the pace the jam took.
Two details decide whether this is a fix or a cheat:

- **It moves the body rather than pushing it.** Impulses were tried first and
  do not work: the frame is already pressed into the riser, so the solver
  cancels whatever forward speed is handed to it, and lifting bodily just
  raises the jam along with the bike. This is the same thing a character
  controller does to walk up a stair.
- **It fires before the impact, and only on a discontinuity.** Two probes
  ahead of the *leading* wheel -- at 0.12 m and 0.3 m -- must show flat ground
  and then a rise. A ramp is already climbing under the near probe, so it
  never triggers; measured against 15, 25, 35 and 45 degree ramps, the
  trajectory with the assist on is bit-identical to the one with it off. A
  wall has no top within reach of the far probe, so it stays a wall. Waiting
  for the jam instead meant taking the hit first, which threw the rider and
  made clearing a flight a coin flip.

Probing close matters: treads are short, and looking 0.35 m or more ahead
lands two steps on and reads their combined height as unclimbable.

### Lean is only ever as far as the turn earns

A leaned bike is balancing centripetal acceleration against gravity, so the
honest angle is `atan(v*w/g)`. `lean_target` clamps to exactly that, using the
yaw rate measured *last frame* -- not one derived from the lean, so there is no
circularity -- with a small floor so a turn can still be started from no yaw at
all.

This is inert everywhere except walking pace, which is the whole point. Held at
full lock:

| speed | turn rate | justified lean | shown before | shown now |
| --- | --- | --- | --- | --- |
| 2.0 m/s | 1.62 rad/s | 18 deg | 35 deg | **18 deg** |
| 4.0 | 2.80 | 48 | 60 | **49** |
| 8.9 | 2.52 | 66 | 60 | 60 |
| 17.5 | 2.14 | 75 | 60 | 60 |

From about 4 m/s up the cap is already past `lean_max`, so full lean stays
available and the turn rates are unchanged to two decimal places. Below that
the bike used to be drawn lying over at 35 degrees for a turn that justified
23, which reads as falling over rather than riding.

### Pitch is asked for, never stumbled into

Drive and brake torque act at the contact patch, and on a bike with real grip
they are several times gravity's restoring moment: full throttle alone stood the
bike vertical at 84 degrees and balanced it there. So `bike` **answers the tyres'
own pitch moment in full** every tick, and the only thing that pitches the bike
is the rider.

`weight` back on the throttle wheelies to `wheelie_limit`; `weight` forward on
the front brake stoppies to `stoppie_limit`. Both are a PD on the pitch angle
with gravity's moment through the support wheel fed forward, so the balance
point is the angle asked for. Backstops past each limit stop a loop-out or an
endo whatever the rider does. Pitch is measured **against the contact normal**,
not the world, so a ramp does not read as a wheelie and none of this fights one.

### Preload, and the clutch

`weight` (rider fore/aft) compresses the suspension while in contact, and a
release edge while still in contact converts the stored travel into a launch
impulse scaled by how much was stored. One pop per compression: the latch arms
above 35% travel and fires below 15% remaining rider weight. This is what makes
jump height a skill rather than a function of approach speed.

`clutch` disconnects the drive and spins the engine up against it; releasing it
hands that stored speed straight to the rear wheel and lofts the front. A launch
from rest is grip limited, so a clutch dump cannot make the bike travel further
— what it buys is the front wheel, which is what the clutch is for over a log or
the face of a jump.

### In the air

The body is free, so whips, flips and tabletops are real rotation. Rate targets
keep repeated input controllable: `whip` yaws at up to `whip_rate` and lays the
back end over with it, a held `weight` pitches at `flip_rate` (a backflip is
`2π / flip_rate` seconds), and `lean` rolls for a tabletop.

A **neutral stick does more than damp**. It levels roll toward world up and
swings the nose back toward the direction of travel at `air_level`. That is the
assist that makes a whip landable: send it, let go, and the bike comes back
square. Holding the bars out all the way down is the rider refusing to bring it
back, and it lands as a case.

### Landing

Freestyle landings are the point, so the envelope is deliberately generous:
`landing_roll` 1.05, `landing_pitch` 1.35 and `landing_yaw` 1.1 radians, a
`crash_delta_v` of 24 (a 16 m drop peaks at 15.6), and 0.40 m of suspension
travel to absorb the rest. For `LANDING_ASSIST` seconds after a real air the
roll controller's gain is tripled and the bike is yawed toward its direction of
travel, which plants it rather than letting a few degrees become a slide. What
still bails you is a landing you have actively held wrong — the assists only run
on a neutral stick.

On the airborne-to-contact edge, after `0.25 s` of air, the landing is judged
**against the surface it lands on**, not against world up — landing on a steep
face is fine when the bike matches the face. Roll, pitch and (above walking
pace) the angle between the bike and its direction of travel are checked against
`landing_roll`, `landing_pitch` and `landing_yaw`; past any of them the rider is
handed off through the same `safety` path as a collision, with
`reason: "landing"`. Hard hits keep `crash_delta_v`.

### Controls

`Controls` gains `lean`, `weight`, `whip`, `trick`, `trick_extend` and `clutch`,
all defaulted, so existing Lua `control` tables keep working unchanged. The host
publishes the matching axes from `sdk.vehicle.input()`: left stick is the bike
(`steering`, `weight`, and `whip` sharing the bars axis), right stick is the
rider (`lean`, `rider_y`), plus raw `trigger_l`/`trigger_r` so a mod can split
them into throttle and front brake instead of the kart's combined pedal, and
`trick_a`/`trick_b`/`clutch` buttons. `brake` is the front lever: on a bike it
takes the steered wheel at full and the rear at 0.3, while `handbrake` locks the
rear for a slide. Trick **selection** stays in Lua: the host publishes axes and
buttons, the mod decides what they mean.

`sdk.vehicle.read()` additionally returns `velocity`, `angular_velocity`,
`wheel_contacts`, `airborne` and `wheel_speed` — what a freestyle scorer needs to
measure rotation and tell an air from a landing.

### Geometry a bike needs, that a kart does not

The chassis collider must never reach the ground. A kart's box sits low by
design; a bike on the same numbers rides belly-down on the floor. Size
`half_extents` and `collider_offset` so the underside clears the ground at the
compressed ride height — the headless tests assert 0.25 m of clearance at static
sag — and put `center_of_mass` **high**, around 0.3 m above the chassis origin
for a 250.

Suspension wants about a third of its travel used at rest. Rapier's spring force
is `stiffness × Δlength × chassis_mass`, so static sag is `g / (2 · stiffness)`
and nothing else: at `suspension_stiffness` 25 that is 0.20 m, which on 0.3 m of
travel is two thirds gone before the rider has done anything, and `preload_force`
then bottoms it permanently. 46 over 0.32 m of travel gives 35% sag.

### Engine sound

`engine_audio.profile` selects `generic` (the original harmonic stack) or
`four_stroke_single`. A single cylinder fires once every two crank revolutions,
so the thumper is one hard asymmetric event per cycle with a long decay, not a
harmonic stack. Its attack is fast but finite: decaying straight off a vertical
edge leaves a step at the phase wrap, which clicks and aliases when pitched up.

For a bike, revs come from the **driven wheel**, not chassis speed — the whole
point of a clutch and a rev in the air is that the bike is not moving. Against
the clutch `wheel_speed` reports the engine rather than the stationary wheel.

### Squaring up a bought model

Bike models are usually sold posed rather than neutral, and the pose is almost
always a little steering lock. `prepare_bike.py` cannot see it: it squares the
chassis using the two axle centres, and turning the bars barely moves the front
axle, so the wheelbase it aligns to is already straight while the bars, clamps,
forks, fender, plate and front wheel are all 20-odd degrees off. In game that
reads as a bike permanently trying to turn.

```powershell
python tools/align_bike.py sdk/examples/freestyle-mx/bike.glb --report
python tools/align_bike.py sdk/examples/freestyle-mx/bike.glb --apply --cut 0.15
```

The angle is measured, not eyeballed: the fork legs are the only pair of long
thin parallel tubes on a bike, they are rigidly part of the front end, and on a
straight bike they are mirror images. Turning the bars swings one forward and
the other back, so the angle of the line joining them, seen from above, **is**
the steering angle, and their shared long axis is the steering axis, rake
included. Run it until it converges -- 21 degrees came down to 3.6, then 0.7,
then 0.14 -- and **look at the render**, because the measurement only knows
about the fork legs.

Three things there are worth knowing before pointing it at another model:

* **The rotation origin must stay on the steering axis.** Sliding it sideways
  to move the cut turns the rotation into a rotation plus a translation, and
  the front end comes off the frame.
* **Classify whole connected islands, not vertices.** A plane cannot separate
  swept-back handlebars from a forward-leaning fuel tank, because the bar ends
  reach back past the tank's front edge. Per-vertex, it sheared a third of the
  radiators and a tenth of the tank off while leaving most of the bars behind.
* **Bar-mounted hardware needs its own rule.** Levers, perches, switchgear and
  grip ends sit behind the steering axis, so no cut catches them without also
  taking the tank. Height separates them cleanly instead: on this bike the tank
  tops out at 0.54 and the seat at 0.52, while the lowest bar fitting is 0.58.

`tools/split_kickstand.py` splits the side stand into its own named node, which
`parked_nodes` in the vehicle definition then hides while the bike is ridden --
otherwise it trails along the ground through every corner and jump. It finds the
stand without being told where it is: it is the only part that is on one side of
the centre plane, long and thin, and reaching down to the wheels' own contact
height. The geometry is not duplicated; glTF lets the new node's primitive share
the original attribute accessors and carry only its own index buffer.

Both tools keep a copy of the model **beside** the package rather than inside
it: `package_mod.py` ships every file in the mod folder, and a spare copy of a
5.8 MB model trebled the archive.

### Rider posture

Six optional animation slots — `stand`, `crouch`, `weight_back`,
`weight_forward`, `lean_left`, `lean_right` — are held poses the host blends the
riding pose towards by how much of each the ride is asking for: standing in the
air and under the brakes, crouching through suspension travel past static sag,
weight from `Controls::weight`, and lean from the handling lean. They stack, so
a rider standing on the pegs with his weight back through a lean is all three at
their own weights rather than an authored pose per combination.

### Verification

`crates/skate-vehicles/tests/bike.rs` covers static sag and collider clearance,
upright recovery, lean and carve in both directions, turn radius widening with
speed, preload raising the jump apex and firing exactly once, a wheelie that
lifts the front without looping out, a stoppie, a whip that straightens itself
to land, a backflip on a held stick, a clean landing keeping the rider, sideways
and on-its-side landings bailing, hops off bumps never being judged, no air
authority on a parked bike, cornering without false ejection, the clutch, 7200
ticks of mixed input staying finite, and 60/120 Hz agreement. Sign conventions
are unit-tested directly in `bike.rs`.

As everywhere else in this SDK, none of this establishes handling **feel**. The
values are authored arcade constants, exposed as live mod settings because the
only way to find them is to ride.

### Rider trick poses

`animations.tricks` is a list of clip names indexed by `Controls::trick` minus
one, capped at 32 and requiring `animations.file`. Each entry is a **single held
pose**, not an animation: `vehicle_pose` now takes an ordered layer stack rather
than one optional steering target, so the riding pose is eased towards the trick
by `trick_extend`. That is what makes a half-thrown trick read as half-thrown,
and it means a trick costs one authored frame instead of a clip.

`tools/author_bike_poses.py` generates them against the native rig with no Mixamo
and no calibration profile: the seated stance is fitted onto the vehicle's own
measured grips and pegs by coordinate descent, and each trick sets named limbs to
absolute angles. Limbs a trick does not take over are re-fitted afterwards, which
is what keeps both hands on the bars through a spine twist.

Several things there were measured and contradict the obvious guess.

**The export is a world-space delta, not a bone matrix.** Writing Blender's pose
bone matrices out directly is what produced a rider whose limbs did not join up:
they carry Blender's own bone convention, and the host wants the glTF one. What
transfers is `posed @ rest.inverted()` -- a pure rigid motion -- applied to the
*reference GLB's* rest node, which is what the working kart rider does. Both
sides of the delta carry the same convention, so it cancels. The tool asserts
this by exporting an unposed rig and requiring it to reproduce the reference rest
pose exactly; a silent basis error here looks perfectly fine in the data.

**Poses are written about the seat, not the hips.** The host puts the rider root
at `definition.seat`. Recentring each frame on HIPS instead pins the pelvis
there forever, which makes standing, crouching and weight shifts impossible to
author at all -- and those are most of what a motocross rider does.

**Mesh nodes carry their own transform.** This bike's are scaled 0.989 and
dropped 0.16 m, so raw vertex coordinates are not chassis local. Reading them
directly put the seat 0.24 m too high and left the rider floating above the bike
with his legs through the engine. Measure in the scene, after the import has
composed the transforms; with that done the contacts come out as real motocross
numbers -- seat 0.97 m, pegs 0.41 m, grips 1.11 m above the ground.

**Knees and elbows are hinges.** Given a lateral axis, the solver will happily
reach a peg with the knee swung 48 degrees out to the side: the target is met
and the rider looks broken. Each limb gets three knobs -- the proximal joint
aims, the hinge sets the distance -- which is exactly determined for a point in
space, plus a tiny cost on splay to settle the remaining sign the anatomical way.

**Blender's IK is unusable here** without pole targets -- it put a foot 0.71 m
off the centreline, and it points a limb at an unreachable target rather than
failing, so a wrong pose still looks solved. Coordinate descent is used instead,
seeded from the limb's current angles so a refit after the hips move only has to
correct a good solution, and the residual at each hand and foot is printed.

**Trick angles must be absolute**: the stance is a fitted solution, so a delta on
top of it partly cancels the fit.
