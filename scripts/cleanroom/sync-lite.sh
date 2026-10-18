#!/bin/sh
# Copy the clean-room engine (skatecore-cpp clean/) into crates/skate-lite-sys/vendor.
# Usage: scripts/cleanroom/sync-lite.sh [path to skatecore-cpp]   (default ~/cleanroom/skatecore-cpp)
set -eu
src="${1:-$HOME/cleanroom/skatecore-cpp}"
dest="$(cd "$(dirname "$0")/../.." && pwd)/crates/skate-lite-sys/vendor"
rev="$(git -C "$src" rev-parse --short HEAD)"
dirty="$(git -C "$src" status --porcelain clean | head -1)"
rm -rf "$dest"
mkdir -p "$dest"
cp -R "$src/clean/include" "$src/clean/src" "$dest/"
cp "$src/LICENSE" "$dest/LICENSE"
printf 'skatecore-cpp clean/ at %s%s\n' "$rev" "${dirty:+ (with local changes)}" > "$dest/REVISION"
echo "vendored clean engine $rev into $dest"
