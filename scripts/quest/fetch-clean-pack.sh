#!/usr/bin/env bash
# DEVELOPMENT ONLY. Fetch the skate3-cleanroom asset pack (core.pack) into the
# APK's assets; the app then unpacks it on first launch instead of using
# imported data. Its model, textures and sounds are regenerated, but its
# animations, state graphs, tuning and player colliders are still retail data,
# so an APK built with it must NOT be published. Public builds bundle nothing
# until the fully clean-room pack exists. Remove apk-assets/core.pack (or set
# CLEAN_PACK_BASE to the clean pack) before building a release.
set -euo pipefail
BASE="${CLEAN_PACK_BASE:-https://andrewnakas.github.io/skate3-cleanroom}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="$ROOT/crates/skate-quest/apk-assets"
mkdir -p "$DEST"
echo "Downloading $BASE/core.pack"
curl -fL "$BASE/core.pack" -o "$DEST/core.pack.part"
head -c 5 "$DEST/core.pack.part" | grep -q SKPK1 || { echo "not an SKPK1 pack"; exit 1; }
mv "$DEST/core.pack.part" "$DEST/core.pack"
ls -la "$DEST/core.pack"
