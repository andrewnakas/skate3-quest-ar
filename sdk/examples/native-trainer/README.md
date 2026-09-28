# Native Trainer Showcase 2.1

One original mod demonstrating the actual SDK, with no game assets. Enable it in
Escape -> Mods. Its independent draggable settings window opens on Escape.
The eleven physics controls start at 1x stock. Disable the mod to restore stock
parameters and remove all its HUD and world objects.

## Native controls

Pop height, grind pop, push speed, push power, brake strength, steering response,
speed wobble, off-board jump height, ground wheel/slide grip, heading turn force,
and manual balance drag. These scale existing inputs/settings, not simulation
speed or arbitrary body velocities. Most controls range 0.25x-4x; wobble is 0x-2x.
Current difficulty, native state gates and surface rules still apply.

## Showcase features

- Compact/full/off telemetry: board speed, peak, approximate travelled distance,
  root position, bail count, grind entries and active grind time.
- F5 saves a trainer checkpoint; F6 requests return through the native reset path.
  This does not replace the game's own session marker. Save on valid clear ground.
- F7 starts/stops the active-play stopwatch; its clock pauses with Escape/replay.
- F8 clears session statistics, stopwatch, checkpoint and all visual markers.
- F9 places a coloured visual beacon. Eight keyed beacons are reused in a ring.
- Optional breadcrumb trail reuses 24 tiny cubes at configurable distance spacing.
- Colour choices, units, HUD label, help hints and configurable keyboard shortcuts.
- Temporary notifications expire using owned timers. The log reads help.txt using
  the package-relative file API. Edit source to see debounced hot reload in action.

Markers have no collision and are not grind rails. Statistics are local sampled
observations, not native career scores: travel rejects single-frame jumps of 50m
or more but is not replay-grade odometry. Board velocity is not walking speed.
The full HUD uses separate owned lines to avoid overlapping other notifications.

All transient state resets on script reload/disable or map change. Persisted
settings survive. The shortcut choice lists let authors try rebinding; use distinct
keys. If bindings coincide, actions execute in save/return/timer/clear/beacon order.
A native teleport rejection stops this mod and reports the error, as with any
SDK host-command failure; recover via Reload. No invincibility, general gravity
edit, physics collision insertion, arbitrary animation slot or score API is claimed.

## Source tour

main.lua deliberately separates apply (validated native tuning), update (input,
HUD, visual objects), on_event (map/bail/grind observations), and timer cleanup.
Use this as a practical starting point for training tools, navigation helpers,
telemetry overlays and original visual extensions. The engine's full API reference
is docs/lua-modding.md and language-server annotations are sdk/skate.lua.

Hold fakie stance disables automatic fakie stance switching; normal steering and slides still work. Off by default. Escape settings windows can be resized by dragging the bottom-right // grip.

Authoring folder: sdk/examples/native-trainer. Package with tools/package_mod.py into
mods/native-trainer.zip. See sdk/AGENTS.md and docs/mod-packages.md.

## Multiplayer

Use matching enabled ZIPs and the same integrated build on all peers. World objects
and vehicles synchronize through the SDK; Lua shared rules use `sdk.net`.
See [Multiplayer mod SDK](../../../docs/multiplayer-mods.md) for ownership, collisions,
late joins and shared-state examples.
