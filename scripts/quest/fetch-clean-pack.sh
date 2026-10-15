#!/usr/bin/env bash
# Fetch the skate3-cleanroom asset pack (core.pack: regenerated skater model,
# animations, state graphs, tuning, HUD, clean sounds) into the APK's assets.
# The app unpacks it on first launch, so players need no game files.
# Never committed; run once per checkout (and again when the pack changes).
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
