#!/usr/bin/env bash
# Fetch the Khronos OpenXR loader for Android into the APK's runtime_libs.
# The loader is not committed; run this once per checkout.
set -euo pipefail
VERSION="${OPENXR_LOADER_VERSION:-1.1.63}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="$ROOT/crates/skate-quest/runtime_libs/arm64-v8a"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
URL="https://repo1.maven.org/maven2/org/khronos/openxr/openxr_loader_for_android/$VERSION/openxr_loader_for_android-$VERSION.aar"
echo "Downloading OpenXR loader $VERSION"
curl -fsSL "$URL" -o "$TMP/loader.aar"
unzip -q -o "$TMP/loader.aar" "jni/arm64-v8a/libopenxr_loader.so" -d "$TMP"
mkdir -p "$DEST"
cp "$TMP/jni/arm64-v8a/libopenxr_loader.so" "$DEST/"
echo "Installed $DEST/libopenxr_loader.so"
