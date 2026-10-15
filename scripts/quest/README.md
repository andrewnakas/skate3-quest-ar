# Quest 3 build (run on the Mac)

The Quest app is the same game as the desktop `skate3rust`, built as an
Android library (`crates/skate-quest`) with the `xr` feature: OpenXR,
colour passthrough, Touch controllers. On the headset it always starts in
XR mode on the built-in test course, as a miniature you place on a table.

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

## Game data

Nothing to push. The APK carries `core.pack`, the
[skate3-cleanroom](https://github.com/andrewnakas/skate3-cleanroom) asset pack
(regenerated skater model, textures and sounds; animations, graphs and tuning
kept as facts), fetched into `crates/skate-quest/apk-assets/` by
`scripts/quest/fetch-clean-pack.sh` (`build-apk.sh` runs it the first time).
On the first launch of each build the app unpacks it into
`/sdcard/Android/data/com.andrewnakas.skate3ar/files/clean/` (about 1 s;
log line `SKATE_ANDROID unpacked the clean pack`).

To run on a converted retail installation instead, push it as before and
create the marker file:

```sh
scripts/quest/push-assets.sh quest-data/assets quest-data/audio-pcm-cache
adb shell touch /sdcard/Android/data/com.andrewnakas.skate3ar/files/use-retail-data
```

## Run and read logs

Launch "Skate 3 AR" from the headset's library (Unknown Sources), or:

```sh
adb shell am start -n com.andrewnakas.skate3ar/android.app.NativeActivity
adb logcat -s skate3 bevy RustStdoutStderr
```

Every desktop console line (`REPORT_META ...`, errors, panics) appears
under the `skate3` tag.

## Controls

| Touch | Pad |
|---|---|
| Left / right stick | Left / right stick (flick-it) |
| Triggers | LT / RT |
| Grips | LB / RB |
| A B X Y | A B X Y |
| Stick clicks | LS / RS |
| Left menu (tap) | Start |
| Left menu held + left stick | D-pad |
| Left menu held + X | Back |
| Left menu held 1 s (alone) | Build mode (see below) |

### Build mode

Hold the left menu button for a second to enter or leave. The pad is neutral
while building:

| Touch | Build action |
|---|---|
| Left stick left/right | Choose a piece (wrist palette): kicker, quarter pipe, half pipe, rail, ledge, fun box, platform, bank, stairs + handrail, spine |
| Left stick up/down | Grow / shrink the whole world (skater and pieces together); saved with the layout |
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

The Quest build plays clean skate sounds (`crates/skate-game/clean-audio/`,
from the skate3-cleanroom web build, extracted by
`tools/extract_clean_audio.py`): rolling, grinds (metal on placed rails and
copings), powerslides, pops, flips, catches, landings, pushes, footsteps and
bails. Every sound is spatial: the headset is the listener and the skater
the source, with distance measured in real metres (full volume within 1 m,
a quarter at 2 m), so it pans and fades as the skater rides around the room at
any world scale (`xr/spatial_audio.rs`). 22 are CC0 recordings from freesound.org (credits in
`clean-audio/CREDITS.json`: nolimitkid, Sheyvan, mbezzola, BlackNeon1234,
visionear, neotronable, PHetish, Bruno_Garcez, C-V, bspiller5,
jarrondesnudo, gerardcatala); the rest are resynthesised. `SKATE_AUDIO=retail`
switches back to the retail player-audio runtime.

The miniature is 12x smaller than life by default (a 1.8 m skater is 15 cm);
in build mode the left stick rescales it anywhere from life size to 60x smaller.
