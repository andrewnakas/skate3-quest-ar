#!/usr/bin/env bash
# Copy game data to the headset (run after the APK is installed once, so the
# app's external files directory exists). Nothing here is ever committed.
#   scripts/quest/push-assets.sh <assets-dir> [<pcm-cache-dir>]
# <assets-dir>    a converted installation's assets folder, e.g. a copy of
#                 C:\s3\installations\<id>\assets
# <pcm-cache-dir> the PC's %LOCALAPPDATA%\Skate3RustEngine\audio-pcm-cache
#                 (the headset has no ffmpeg; player sound banks come from it)
set -euo pipefail
ASSETS="${1:?usage: push-assets.sh <assets-dir> [<pcm-cache-dir>]}"
CACHE="${2:-}"
PKG=com.andrewnakas.skate3ar
DEST=/sdcard/Android/data/$PKG/files
adb shell mkdir -p "$DEST/assets" "$DEST/Skate3RustEngine/audio-pcm-cache"
adb push --sync "$ASSETS/." "$DEST/assets/"
if [ -n "$CACHE" ]; then
  adb push --sync "$CACHE/." "$DEST/Skate3RustEngine/audio-pcm-cache/"
fi
echo "Pushed to $DEST"
