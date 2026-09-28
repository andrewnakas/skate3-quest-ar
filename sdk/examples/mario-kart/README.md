# Mario Kart vehicle mod
Version 1.5 adds arcade rollover resistance and airborne left-stick control.
In air, stick left/right rolls, forward pitches the nose down, and back pitches
the nose up. Keyboard: A/D roll, Up/Down arrows pitch. Neutral input damps rotation.
No lift is added. Aerial inversion is allowed; sustained grounded inversion still
ejects. Minor bumps are more forgiving; severe collisions still eject the rider.
The matching updated host executable is required.
Enable this mod in Mods, resume, then press F10 to spawn the kart in front of you.
E (controller Y) enters/exits within 4 metres (slow below 3 m/s to exit). WASD drives; Space brakes;
left Shift is the handbrake; R or right-stick click rights/resets the kart. The controller bind can be changed to left-stick click in the mod settings. F10 replaces the kart.

Controller: RT/LT accelerate/reverse, left stick steers, A brakes, B handbrakes.
Version 1.4 uses the updated host's physical tire model: progressive steering,
Ackermann wheel angles, torque-driven wheel spin, shared braking/cornering grip,
and contact-point weight transfer. Brake overrides throttle; the opposing pedal
brakes before engaging reverse. Tire friction defaults to 1.3. The old arcade
grip preference is replaced by this new coefficient; other settings are retained.
This package requires the matching updated game executable. It remains a
simplified raycast-wheel kart, without a gearbox or ABS.
The enabled mod shows keyboard and controller controls on screen, including the current reset bind. Toggle Show driving HUD in its settings to hide/show them, even before spawning.

The GLB is prepared from your supplied ZIP with tools/prepare_mario_kart.py and is
committed with the mod. To rebuild it: python tools/prepare_mario_kart.py PATH_TO_ZIP sdk/examples/mario-kart
The script requires NumPy. The prepared kart.glb and fitted rider.json are included in source control and in mods/mario-kart.zip.

Edit vehicle.json for dimensions, wheels, suspension, seat, exit and camera offsets.
The bundled rider.json contains fitted entry, exit, seated and left/right steering poses.
The host blends steering with stick input and eases between vanilla and vehicle poses
using the native local-transform interpolation routine. The skateboard is hidden while
seated and restored during the exit hand-off. Camera changes are eased too.

Rebuild rider.json with Blender and tools/export_kart_rider.py, passing --enter and
--exit fitted .blend previews, --bone-names a JSON list from the stock bank hierarchy,
--reference the stock skater.glb and --output the local
rider.json path. Raw FBX files, Blender projects and the stock character mesh remain local; only the prepared kart model and fitted rider clips are bundled.
See docs/vehicle-sdk.md for the animation format and lifecycle details.

Ramp tuning uses a shorter rounded chassis and a lower centre of mass, with mass
distribution independent of the collider. The shared engine_audio definition enables
throttle/speed-responsive synthesized engine sound; Engine volume in the mod window
controls it live (zero mutes). Full authoring guide: docs/mixamo-vehicle-workflow.md.

Author here, then run `python tools/package_mod.py sdk/examples/mario-kart mods/mario-kart.zip`.
The live ZIP loads automatically; do not edit mods/.cache. See sdk/AGENTS.md.

Hard crashes, strong head/torso hits and sustained inversion now eject the rider into
native bail physics with carried momentum. An occupied-seat capsule protects the torso
against map geometry. Tune rider_safety in vehicle.json; see docs/vehicle-sdk.md for
threshold units, vehicle_bailed events and the native/Rapier collision boundary.

## Multiplayer

Use matching enabled ZIPs and the same integrated build on all peers. World objects
and vehicles synchronize through the SDK; Lua shared rules use `sdk.net`.
See [Multiplayer mod SDK](../../../docs/multiplayer-mods.md) for ownership, collisions,
late joins and shared-state examples.
