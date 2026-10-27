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
# (Android won't let adb push create folders inside another app's files: make them first)
(cd "$ASSETS" && find . -type d | sed 's|^\./||;s|^\.$||') | while IFS= read -r d; do
  [ -n "$d" ] && printf 'mkdir -p "%s/assets/%s"\n' "$DEST" "$d"
done > /tmp/sk8-mkdirs.sh
adb push /tmp/sk8-mkdirs.sh /data/local/tmp/sk8-mkdirs.sh >/dev/null
adb shell sh /data/local/tmp/sk8-mkdirs.sh
adb shell rm /data/local/tmp/sk8-mkdirs.sh
adb push --sync "$ASSETS/." "$DEST/assets/"
if [ -n "$CACHE" ]; then
  adb push --sync "$CACHE/." "$DEST/Skate3RustEngine/audio-pcm-cache/"
fi
# The app runs the clean-room tier unless this marker exists.
adb shell touch "$DEST/use-retail-data"
echo "Pushed to $DEST; the next launch uses this imported data (rm $DEST/use-retail-data for the clean-room tier)"
