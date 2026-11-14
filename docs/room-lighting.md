# Room lighting (Roomtone)

Status: tested on the desktop (the estimate, its conversion to Bevy's lights, depth from the
room scan, the settings) and run once on a Quest 3 (Horizon OS 2.10), where the permission
prompt, the camera capture and the lighting worked. The brightness has had one pass by eye
(about 80% on the slider looked right) and is not measured against passthrough.

AR settings > **Roomtone room lighting** lights the skater, the board and the park's pieces by the
player's real room instead of the clean tier's fixed sun: a dim warm room gives a dim warm
skater, a window on the left lights the left side, and a ramp beside a lamp is brighter than
one across the room. Off (the default) nothing changes.

Quest has no light estimation API. The estimate is made by
[Roomtone](https://github.com/rhythmerc/roomtone) (`crates/roomtone-sys`, MIT, plain C++17,
no GPU work) from the passthrough camera's frames, each reduced to a 32x32 grid of colour with
its exposure and the head's pose. It runs on its own thread, five frames a second.

## What it needs

- Quest 3 or 3S on Horizon OS v74 or later (Meta's Passthrough Camera API). On other headsets
  the setting says the cameras can't be used and the skater keeps its sun.
- `horizonos.permission.HEADSET_CAMERA`, in both apps' manifests and asked for at runtime the
  first time the player turns the setting on. Frames are reduced to the grid on the capture's
  thread and never stored or sent; a store listing needs to declare the camera use.
- For light that depends on where things are: a room scan (Space Setup), which the app
  already loads. Free roam gets the room's light without placement.

## How it works (`crates/skate-game/src/xr/room_light/`)

| File | |
|---|---|
| `camera.rs` (Android) | Streams the left passthrough camera through the NDK's Camera2 at its smallest square size and lowest frame rate. The camera's id comes from the entry point (`CameraHooks::passthrough_camera`): Meta marks it with vendor tags only Java can look up by name. |
| `reduce.rs` | A YUV frame to the 32x32 grid of mean linear colour and the share clipped. |
| `depth.rs` | A 64x64 depth image ray cast from the room's surfaces (`ArWorld::room`: Scene planes, boxes and the room mesh) at each frame's head pose, in `XR_META_environment_depth`'s convention. Roomtone places what the camera saw with it, without the Depth API's GPU readback. |
| `feed.rs` | What the game shares with the capture's thread: the head's pose, the surfaces, whether it should run. |
| `shade.rs` | Roomtone's relative units to Bevy's photometric ones, the ambient cubes and their texture, the key lights. |
| `mod.rs` | The plugin: where the light is made, reading it, and showing it. |

Everything is shown through Bevy's own lighting, so every `StandardMaterial` takes it (the
rider, the board, placed pieces) with no shader changes:

- **Soft light** (Roomtone's L2 spherical harmonics) becomes an `IrradianceVolume`: one
  ambient cube while the capture has no depth, and with a room scan a grid over the room (a
  point about every 1.5 m, at most 3x2x3), padded so the skater stays lit past the scan's edge.
- **The two brightest lights** become a `PointLight` each when depth gave them a place
  (inverse-square from where the lamp is, scaled with the miniature), a `DirectionalLight`
  when it didn't. No shadows (the eye views have no budget for them).
- **Brightness** follows passthrough's: the camera shares its auto exposure. `shade::Scale`
  maps Roomtone's level to scene luminance through the camera's `Exposure`;
  `REFERENCE_WHITE` and the settings' brightness slider are where to tune it.

While it is on and a light has been made, the tier's own `DirectionalLight`s and
`GlobalAmbientLight` fade out over a second, and back when it is turned off.

## Trying it without a headset

```sh
SK8_ROOM_LIGHT=lamp      cargo run -p skate-game --no-default-features --features xr -- --clean
SK8_ROOM_LIGHT=lamp-room ...   # the same bulb in a room with known walls: placed by depth, a grid of probes
SK8_ROOM_LIGHT=window    ...   # daylight from the left, no placement
SK8_ROOM_LIGHT=<file>    ...   # a session recorded with Roomtone's Recorder
```

`cargo test -p roomtone-sys` and `cargo test -p skate-game --no-default-features --features xr room_light`
cover the bindings, the unit conversion, the volume texture's layout, the depth ray caster and
the frame reduction.

## Not done

- Tuning on the headset: the default brightness against passthrough (`REFERENCE_WHITE`), the
  irradiance volume's cost on the Adreno, and a run on a Quest 3S.
- The imported-disc tier on a headset. In AR it has no map, so its skater is drawn with Bevy's
  standard lighting and takes the room's light as the clean tier's does (seen on the desktop's
  test world with `SK8_ROOM_LIGHT`); it has not been run on a Quest. With a retail map loaded
  the skater uses the character shader's authored probes (`retail_character.rs`) instead, and
  the room's light does not reach it.
- Updating the vendored Roomtone: `scripts/roomtone/sync.sh`.
