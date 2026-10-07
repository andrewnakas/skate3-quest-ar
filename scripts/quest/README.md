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

## Game data (once, and after asset changes)

The APK carries no game data. Copy the transfer bundle from the PC
(`D:\skate3rustdesktopar\quest-data\`, made by `scripts/quest/make-bundle.ps1`)
and push it:

```sh
scripts/quest/push-assets.sh quest-data/assets quest-data/audio-pcm-cache
```

That lands in `/sdcard/Android/data/com.andrewnakas.skate3ar/files/`:
`assets/` (the converted installation) and
`Skate3RustEngine/audio-pcm-cache/` (pre-decoded player sound banks; the
headset has no ffmpeg).

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
| Left menu held 1 s (alone) | Placement mode: the miniature follows your right controller, A sets it down |

A new session starts in placement mode: touch the right controller to the
table where the skater should start and press A. While placing, the app
scans the room for real surfaces (`XR_EXT_plane_detection`) and snaps the
miniature flush onto whichever table or floor you're pointing at, rather
than wherever the controller happens to be tilted. The headset will ask for
one-time Spatial Data consent the first time this runs; until it's granted,
placement falls back to the raw controller pose.

The miniature is 12x smaller than life by default (a 1.8 m skater is 15 cm).
