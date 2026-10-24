#!/usr/bin/env bash
# Build the store app, SK8 AR (crates/sk8-quest: com.andrewnakas.sk8ar): the clean-room game only,
# built without the imported-disc tier (skate-game's `retail` feature), with no storage permission
# and no bundled data. The GitHub build (Skate 3 AR, with the import) is scripts/quest/build-apk.sh.
#   scripts/quest/build-store-apk.sh            # debug-signed for now (a store upload needs a release key)
#   scripts/quest/build-store-apk.sh --install  # also adb install -r
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
: "${ANDROID_NDK_ROOT:=${ANDROID_NDK_HOME:-}}"
: "${ANDROID_NDK_ROOT:?set ANDROID_NDK_ROOT to the Android NDK}"
export ANDROID_NDK_ROOT
if [ ! -f crates/skate-quest/runtime_libs/arm64-v8a/libopenxr_loader.so ]; then
  scripts/quest/fetch-openxr-loader.sh
fi
export RUST_MIN_STACK="${RUST_MIN_STACK:-268435456}"
cargo apk build -p sk8-quest --release --lib
APK="$(ls -t target/release/apk/sk8ar*.apk | grep -v unaligned | head -1)"
# The store build must carry nothing from the retail game: no import code, no Skate 3 name.
if unzip -p "$APK" 'lib/arm64-v8a/libsk8_quest.so' | LC_ALL=C grep -a -q -e 'use-retail-data' -e 'Skate 3 data'; then
  echo "ERROR: $APK still contains the imported-disc tier" >&2
  exit 1
fi
echo "APK: $APK"
if [ "${1:-}" = "--install" ]; then
  adb install -r "$APK"
fi
