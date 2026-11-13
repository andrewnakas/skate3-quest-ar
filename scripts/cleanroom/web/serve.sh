#!/bin/sh
# Build and serve both games for headless browser testing, on one local origin:
#   http://127.0.0.1:8771/original/  the faithful port (skatecore's web build) on your own cleanpack
#   http://127.0.0.1:8771/clean/     sk8, the clean-room engine (skatecore-cpp clean/web), same park
# Then drive them with run.mjs (same script, tick by tick, both pages headless).
#
# Usage: scripts/cleanroom/web/serve.sh [--rebuild]
#   PACKDIR   your cleanpack (core.pack + KenneyPark.skate)
#   SKATECORE skatecore-cpp checkout
#   OUT       where the pages go (~/skate-compare)
set -eu
PACKDIR="${PACKDIR:-/Volumes/Untitled/MacOffload/Documents/haloskate-ref/cleanpack}"
SKATECORE="${SKATECORE:-$HOME/cleanroom/skatecore-cpp}"
OUT="${OUT:-$HOME/skate-compare}"
PORT="${PORT:-8771}"
[ -f "$PACKDIR/core.pack" ] || { echo "no core.pack in $PACKDIR (is the drive mounted?)"; exit 1; }
command -v em++ >/dev/null || { echo "needs Emscripten (em++) on the PATH"; exit 1; }

# the original: the port's page, pointed at your pack (linked, never copied)
site="$OUT/original"
mkdir -p "$site/pack"
make -C "$SKATECORE" -j4 --no-print-directory s3-wasm S3_WASM_OUT="$site" >/dev/null
cp "$SKATECORE/web/s3/index.html" "$SKATECORE/web/s3/main.js" "$SKATECORE/web/s3/style.css" "$site/"
ln -sf "$PACKDIR/core.pack" "$site/pack/core.pack"
ln -sf "$PACKDIR/KenneyPark.skate" "$site/pack/KenneyPark.skate"
python3 - "$PACKDIR" "$site/pack/pack.json" <<'PY'
import json, os, sys
d, out = sys.argv[1], sys.argv[2]
size = lambda f: os.path.getsize(os.path.join(d, f))
json.dump({"default": "KenneyPark", "core": {"size": size("core.pack"), "parts": ["core.pack"]},
           "maps": [{"name": "KenneyPark", "size": size("KenneyPark.skate"), "parts": ["KenneyPark.skate"]}]},
          open(out, "w"))
PY

# the clean game: sk8 to wasm, and the same park's collision + grind lines dumped from your map
make -C "$SKATECORE" --no-print-directory sk8-wasm SK8_WASM_OUT="$OUT/clean" >/dev/null
make -C "$SKATECORE" --no-print-directory build/mapdump >/dev/null 2>&1 || true
mesh="$OUT/kenney_mesh.txt"
{ [ -s "$mesh" ] && [ "${1:-}" != "--rebuild" ]; } || "$SKATECORE/build/mapdump" "$PACKDIR/KenneyPark.skate" > "$mesh"

if ! curl -s -o /dev/null "http://127.0.0.1:$PORT/clean/index.html"; then
	(cd "$OUT" && nohup python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &)
	sleep 1
fi
echo "original: http://127.0.0.1:$PORT/original/?base=/original/pack/&map=KenneyPark"
echo "clean:    http://127.0.0.1:$PORT/clean/?mesh=/kenney_mesh.txt&spawn=13.5,0.1,-1.5,3.14159"
