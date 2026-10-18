# Quest 3 build (run on the Mac)

The Quest app is built as an Android library (`crates/skate-quest`) with the
`xr` feature: OpenXR, colour passthrough, Touch controllers. It always starts
in XR mode, as a miniature skater in your real room.

It has two tiers:

- **Clean-room (default, public).** The skater runs on the `sk8` engine
  (`crates/skate-lite-sys`, vendored from skatecore-cpp `clean/`), written
  from scratch for this app: its own physics, flick recogniser, tricks and
  tuning. The skater, board, poses and bails are built in code
  (`crates/skate-game/src/clean/`), and the sounds are CC0 recordings or
  original synthesis. It reads no game data at all, and
  `scripts/cleanroom/audit.py` checks it against a retail tree.
- **Imported disc (opt-in).** The faithful engine port, run on data you
  converted from your own Skate 3 disc (below). It is used only when
  `files/use-retail-data` exists.

## One-time setup

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
export ANDROID_HOME=~/Library/Android/sdk            # wherever the SDK lives
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/<version>  # r26+ recommended
scripts/quest/fetch-openxr-loader.sh                 # Khronos loader -> runtime_libs/
```

The headset must be in developer mode and visible to `adb devices`.

## Build and install

```sh
scripts/quest/build-apk.sh --install
```

The APK carries no game data; the clean tier needs none.

## Imported disc (optional)

Convert your own Skate 3 disc with the desktop engine, copy the transfer
bundle (made by `scripts/quest/make-bundle.ps1` on the PC) and push it after
installing the APK once:

```sh
scripts/quest/push-assets.sh quest-data/assets quest-data/audio-pcm-cache
```

That lands in `/sdcard/Android/data/com.andrewnakas.skate3ar/files/`:
`assets/` (the converted installation) and
`Skate3RustEngine/audio-pcm-cache/` (pre-decoded player sound banks; the
headset has no ffmpeg). `push-assets.sh` also creates `files/use-retail-data`,
so the next launch runs the imported disc. Delete that file (or choose
"Use my imported disc" again from the clean tier's start screen to create it)
to switch:

```sh
adb shell rm /sdcard/Android/data/com.andrewnakas.skate3ar/files/use-retail-data
```

## Run and read logs

Launch "Skate 3 AR" from the headset's library (Unknown Sources), or:

```sh
adb shell am start -n com.andrewnakas.skate3ar/android.app.NativeActivity
adb logcat -s skate3 bevy RustStdoutStderr
```

Every desktop console line (`REPORT_META ...`, errors, panics) appears
under the `skate3` tag.

## Controls (clean-room tier)

| Touch | Action |
|---|---|
| Left stick | Steer; in the air, spin; in a manual, balance (up/down) |
| Right stick | Hold down to crouch, flick to pop and flip (How to play lists every trick) |
| A (hold) | Push |
| B | Brake |
| X + steer | Powerslide |
| Right grip / left grip | Manual / nose manual |
| Triggers, in the air | Grab (left / right hand) |
| Left grip + left trigger | Aim; let go to put the skater there |
| Left menu (tap) | Start screen: skate, build, how to play, SK8 menu, put the skater in front of me |
| Left menu held + both grips | SK8 menu |
| Left menu held 1 s (alone) | Build mode (see below) |

Tricks (hold the right stick down first): through the centre to up = ollie,
up-left = kickflip, up-right = heelflip; sweep along the rim to the left and
let go = pop shove-it, sweep on to up-left = tre flip; start up for nollies.
The in-app How to play page is generated from the engine's own trick table.

### SK8 menu

Live tuning of the clean-room engine's own values (pop and jump, gravity and
speed, spin, flips, bails, grinds, manuals, scoring), plus Practice (game
speed 0.05-2x, pause, frame step, five markers, return to the last marker
after a bail, never bail, bail now), World (park size, world scale, put the
skater in front of me) and Presets. Grips switch tabs, the stick selects and
adjusts (hold the left trigger for fine steps, the right for coarse), A
freezes a value so presets and resets leave it alone, X resets it, B goes
back. Values are saved to `files/sk8-tuning.json`.

## Controls (imported-disc tier)

| Touch | Pad |
|---|---|
| Left / right stick | Left / right stick (flick-it) |
| Triggers | LT / RT |
| Grips | LB / RB |
| A B X Y | A B X Y |
| Stick clicks | LS / RS |
| Left menu held + left stick | D-pad |
| Left menu held + X | Back |
| Left menu held 1 s (alone) | Build mode (see below) |

A menu tap no longer sends Start in XR: it opened the flat pause menu, whose
camera is off in the headset, so the game froze behind an invisible menu.

### Build mode

Hold the left menu button for a second to enter or leave. The skater gets no
input while building:

| Touch | Build action |
|---|---|
| Left stick left/right | Choose a piece (wrist palette): kicker, quarter pipe, half pipe, rail, ledge, fun box, platform, bank, stairs + handrail, spine |
| Left stick up/down | Grow / shrink the whole world (skater and pieces together); saved with the layout |
| Left grip + left stick up/down | Grow / shrink only the park (every placed piece, 0.25-4x, about its own base); the skater keeps its size; saved with the layout |
| Left stick click | Drop a starter park where the right controller points |
| Left trigger | Undo |
| Right controller | Point; a ghost shows where the piece goes |
| Right trigger | Drop it there |
| Right stick left/right / up-down | Turn 15° (snaps to table edges) / resize; pointing at a placed piece, turns / resizes that piece |
| Right stick click on a piece | Copy it (kind, size, turn) |
| Right grip on a piece | Pick it up, release to put it down |
| B on a piece | Delete it |
| A | Keep the detected-planes overlay on outside build mode too |
| Y | Rescan the room (Space Setup) |
| Hold X | Clear every piece |

### Planes

The Quest runtime has no `XR_EXT_plane_detection`, so the app builds its own
from Space Setup: the labelled floor, walls and furniture (`XR_FB_scene`),
plus flat patches it finds in the scanned room mesh (steps, benches, boxes,
shelves Space Setup didn't label). Mesh patches that fill their rectangle
are ridden as flat surfaces with grindable edges. The Scene is re-queried
every 15 s, so a rescan or edited room shows up without a restart.

Build mode shows the detected planes: floor blue, furniture green, walls
grey, mesh-found surfaces orange (yellow when only shown, not ridden). The
overlay also flashes for a few seconds whenever the detected set changes.
The headset asks for one-time Spatial Data consent the first time; without
it only the real floor (y = 0) is used.

### Sound

Both tiers on the Quest play the clean sounds in
`crates/skate-game/clean-audio/`: rolling, grinds (metal on placed rails and
copings), powerslides, pops, flips, catches, landings, pushes, footsteps and
bails. Every sound is spatial: the headset is the listener and the skater
the source, with distance measured in real metres (full volume within 1 m,
a quarter at 2 m), so it pans and fades as the skater rides around the room at
any world scale (`xr/spatial_audio.rs`). 22 are CC0 recordings from freesound.org (credits in
`clean-audio/CREDITS.json`: nolimitkid, Sheyvan, mbezzola, BlackNeon1234,
visionear, neotronable, PHetish, Bruno_Garcez, C-V, bspiller5,
jarrondesnudo, gerardcatala); the other 9 (flips, footsteps, the chime and
the metal roll) are original synthesis from `scripts/cleanroom/synth_sounds.py`.
In the imported-disc tier, `SKATE_AUDIO=retail` switches to the retail
player-audio runtime.

The miniature is 12x smaller than life by default (a 1.8 m skater is 15 cm);
in build mode the left stick rescales it anywhere from life size to 60x smaller.
