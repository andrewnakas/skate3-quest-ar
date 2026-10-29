#!/bin/sh
# Side by side on the Mac: the original (the faithful port, skatecore's web build, running your own
# cleanpack) in Chrome on the left, the clean-room game (sk8, `skate3rust --clean`) on the right,
# both on KenneyPark from the same spawn. Plug in a pad: both read it (the browser only while its
# window is in front - click a window to give it the pad).
#
# Usage: scripts/cleanroom/compare-desktop.sh [--rebuild]
#   PACKDIR   your cleanpack (core.pack + KenneyPark.skate)
#   SKATECORE skatecore-cpp checkout (builds the port's web page and the map dump)
#   OUT       where the web page and the map dump go (~/skate-compare)
#   GAME      the desktop game binary (cargo build --release -p skate-game --bin skate3rust)
set -eu
PACKDIR="${PACKDIR:-/Volumes/FreeAgent GoFlex Drive/MacOffload/Documents/haloskate-ref/cleanpack}"
SKATECORE="${SKATECORE:-$HOME/cleanroom/skatecore-cpp}"
OUT="${OUT:-$HOME/skate-compare}"
GAME="${GAME:-/Volumes/FreeAgent GoFlex Drive/MacOffload/skate3-quest-ar-desktop-target/release/skate3rust}"
PORT=8771

[ -f "$PACKDIR/core.pack" ] || { echo "no core.pack in $PACKDIR (is the drive mounted?)"; exit 1; }

# 1. the original: the port's web page, pointed at your pack (nothing copied, only linked)
site="$OUT/original"
if [ ! -f "$site/skate3.wasm" ] || [ "${1:-}" = "--rebuild" ]; then
	mkdir -p "$site"
	make -C "$SKATECORE" -j6 --no-print-directory s3-wasm S3_WASM_OUT="$site"
	cp "$SKATECORE/web/s3/index.html" "$SKATECORE/web/s3/main.js" "$SKATECORE/web/s3/style.css" "$site/"
fi
mkdir -p "$site/pack"
ln -sf "$PACKDIR/core.pack" "$site/pack/core.pack"
ln -sf "$PACKDIR/KenneyPark.skate" "$site/pack/KenneyPark.skate"
python3 - "$PACKDIR" "$site/pack/pack.json" <<'EOF'
import json, os, sys
d, out = sys.argv[1], sys.argv[2]
size = lambda f: os.path.getsize(os.path.join(d, f))
json.dump({"default": "KenneyPark", "core": {"size": size("core.pack"), "parts": ["core.pack"]},
           "maps": [{"name": "KenneyPark", "size": size("KenneyPark.skate"), "parts": ["KenneyPark.skate"]}]},
          open(out, "w"))
EOF
if ! curl -s -o /dev/null "http://127.0.0.1:$PORT/pack/pack.json"; then
	(cd "$site" && python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &)
	sleep 1
fi

# 2. the same park for the clean game: its collision and grind lines dumped from your map
mesh="$OUT/kenney_mesh.txt"
[ -s "$mesh" ] || "$SKATECORE/build/mapdump" "$PACKDIR/KenneyPark.skate" > "$mesh"

# 3. both windows, side by side
open -na "Google Chrome" --args --new-window "http://127.0.0.1:$PORT/?base=/pack/&map=KenneyPark"
# (a dev build links bevy and the Rust std dynamically: point the loader at them)
sysroot="$("$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc" --print sysroot 2>/dev/null || rustc --print sysroot)"
libs="$sysroot/lib/rustlib/aarch64-apple-darwin/lib:$(dirname "$GAME")/deps:$(dirname "$GAME")"
DYLD_LIBRARY_PATH="$libs" SK8_MESH="$mesh" SK8_SPAWN="13.5,0.1,-1.5,-1.511" "$GAME" --clean &
game=$!
sleep 6
osascript <<'EOF' || true
tell application "Finder" to set b to bounds of window of desktop
set w to (item 3 of b) div 2
set h to item 4 of b
tell application "System Events"
	try
		tell process "Google Chrome" to set {position, size} of front window to {{0, 25}, {w, h - 25}}
	end try
	try
		tell process "skate3rust" to set {position, size} of front window to {{w, 25}, {w, h - 25}}
	end try
end tell
EOF
echo "original: http://127.0.0.1:$PORT/?base=/pack/&map=KenneyPark (left)   clean: skate3rust --clean (right, pid $game)"
wait "$game"
