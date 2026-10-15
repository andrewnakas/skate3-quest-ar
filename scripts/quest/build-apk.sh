#!/usr/bin/env bash
# Build the Quest APK (run on the Mac). Needs: Android SDK + NDK
# (ANDROID_HOME / ANDROID_NDK_ROOT), `rustup target add aarch64-linux-android`
# and `cargo install cargo-apk`.
#   scripts/quest/build-apk.sh            # debug-signed release-optimised APK
#   scripts/quest/build-apk.sh --install  # also adb install -r
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
if [ ! -f crates/skate-quest/apk-assets/core.pack ]; then
  scripts/quest/fetch-clean-pack.sh
fi
# The workspace compiles at opt-level 3 and rustc needs a large stack for
# the game crate (see the desktop build notes).
export RUST_MIN_STACK="${RUST_MIN_STACK:-268435456}"
cargo apk build -p skate-quest --release --lib
APK="$(ls -t target/release/apk/*.apk | head -1)"
echo "APK: $APK"
if [ "${1:-}" = "--install" ]; then
  adb install -r "$APK"
fi
