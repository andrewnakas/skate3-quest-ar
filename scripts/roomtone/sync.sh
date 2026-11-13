#!/bin/sh
# Copy Roomtone (https://github.com/rhythmerc/roomtone) over crates/roomtone-sys/vendor.
# Usage: scripts/roomtone/sync.sh [path to a roomtone checkout]   (default ~/projects/roomtone)
# Files Roomtone has since dropped are left behind: delete those by hand.
set -eu
src="${1:-$HOME/projects/roomtone}"
dest="$(cd "$(dirname "$0")/../.." && pwd)/crates/roomtone-sys/vendor"
rev="$(git -C "$src" rev-parse --short HEAD)"
dirty="$(git -C "$src" status --porcelain include src | head -1)"
mkdir -p "$dest/include/roomtone" "$dest/src"
cp "$src"/include/roomtone/* "$dest/include/roomtone/"
cp "$src"/src/*.cpp "$dest/src/"
cp "$src/LICENSE" "$dest/LICENSE"
printf 'roomtone at %s%s\n' "$rev" "${dirty:+ (with local changes)}" > "$dest/REVISION"
echo "vendored roomtone $rev into $dest"
