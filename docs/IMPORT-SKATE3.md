# Play with your own Skate 3 (optional)

Skate 3 AR works out of the box with no game files: the default **clean-room** skater is
our own (own physics, model, animations and CC0 sounds). If you own Skate 3 for Xbox 360 you
can also switch the app to the **full Skate 3 tier**: the retail skater, animations, controls
and sounds, run from data converted from **your own disc**. Nothing from EA is in this repo or
the APK, and nothing is downloaded: you bring the disc.

You need a Windows PC once (to convert the disc) and about 5 GB free on the Quest.

## 1. Convert your disc on a Windows PC (once)

1. Download the Skate 3 Rust Engine for Windows:
   [skate-3-rust-engine releases](https://github.com/SK8-ENGINE/skate-3-rust-engine/releases/tag/experimental)
   and extract the ZIP.
2. Run `skate3rust.exe` and select your **Skate 3 Xbox 360 ISO** (or `default.xex` in an
   extracted game folder). Let the setup finish (the first time takes a while and needs
   internet for ISO extraction). The game starts when it's done.
3. **Skate around for a minute.** That decodes the sound banks the Quest needs (the headset
   can't decode them itself). Then quit.

## 2. Make the Quest zip

1. From the [Skate 3 AR release](https://github.com/andrewnakas/skate3-quest-ar/releases/latest)
   download `MAKE-QUEST-ZIP.bat` and `make-bundle.ps1`, and put both **in the same folder as
   `skate3rust.exe`**.
2. Double-click `MAKE-QUEST-ZIP.bat`. It finds your converted data by itself and writes
   **`Skate3-Quest.zip`** next to it (a few GB; it takes a few minutes).

If it says it can't find your data, run `skate3rust.exe` with your ISO first (step 1). If it
says "Sound banks: none yet", skate in the desktop game for a minute and run it again
(without them the Quest has no retail sounds).

## 3. Put it on the Quest

1. Install Skate 3 AR (see the main README) and open it once, then close it.
2. Plug the Quest into the PC. In the headset, allow access to files when it asks.
3. In File Explorer open **Quest 3 > Internal shared storage > Download**, make a folder named
   **`Skate3`**, and copy **`Skate3-Quest.zip`** into it.

(Or with adb: `adb push Skate3-Quest.zip /sdcard/Download/Skate3/`)

## 4. Import in the headset

1. Open Skate 3 AR, tap the **left menu** button, go to **Skate 3 data > Import**.
2. The first time, the Quest opens an "All files access" page for the app: switch it **on**,
   come back to the app and press **Import** again.
3. When it says it's done, choose **Close the app now**, then open Skate 3 AR again.
   It now starts in the full Skate 3 tier.

Switch back any time: **menu > Skate 3 data > Play the clean-room game from the next launch**. You can delete
`Download/Skate3/Skate3-Quest.zip` after importing to free the space.

## Troubleshooting

- **No sound in the Skate 3 tier:** the zip had no sound banks. Skate in the desktop game for a
  minute, make the zip again, re-import.
- **Import can't see the zip:** it must be in `Download/Skate3/` and the app needs "All files
  access" (Settings > Apps > Skate 3 AR > Permissions), or run
  `adb shell appops set com.andrewnakas.skate3ar MANAGE_EXTERNAL_STORAGE allow`.
- **From a Mac/Linux with adb instead:** run `MAKE-QUEST-ZIP.bat -NoZip` on the PC to get a
  `quest-data` folder, copy it over, then `scripts/quest/push-assets.sh quest-data/assets quest-data/audio-pcm-cache`.
