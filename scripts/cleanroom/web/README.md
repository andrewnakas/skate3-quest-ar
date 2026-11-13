# Headless browser side-by-side

The original (the faithful port's web page, on your own cleanpack) and the clean game (sk8 compiled
to wasm, `skatecore-cpp/clean/web`) run on KenneyPark in one headless Chrome. They get **the same
pad input, tick by tick**, starting from the same spawn and heading. Each run prints both games'
tricks and bails, and saves side-by-side frames, a report page and a per-tick CSV.

```sh
scripts/cleanroom/web/serve.sh            # build both pages + serve them on 127.0.0.1:8771 (PACKDIR = your cleanpack)
node scripts/cleanroom/web/run.mjs --demo # pushes, ollie, kickflip, carve
node scripts/cleanroom/web/run.mjs --fuzz 941 --ticks 160      # edge/fuzz.py's random input for a seed
node scripts/cleanroom/web/run.mjs my.txt --ticks 300 --every 5 # any probe script
for s in $(seq 900 919); do node scripts/cleanroom/web/run.mjs --fuzz $s --every 0 | grep -E "^(MATCH|DIFFER)" | sed "s/^/$s /"; done
```

- Output goes to `~/skate-compare/runs/<name>/`: `index.html` (the frames side by side), `ticks.csv`
  (input, state, trick, speed and position of both games every tick), and `o####.png` / `c####.png`.
- Script lines use the probes' format: `<from> <to> lx ly rx ry buttons [lt rt]`. Sticks run -1..1
  with +y up. Buttons are Xbox bits: A 0x1000 push, B 0x2000 brake, X 0x4000 mongo, Y 0x8000 off,
  RB 0x200. sk8 reads them in the app's layout (`--host-map`).
- The exit code is 0 on a match and 1 on a difference.
- Run one at a time: it's one headless Chrome with swiftshader, about 25 s per 280 ticks with frames.
- Without `capture=1` both pages are playable in a normal browser, with the same keys or a pad.
- Nothing from the pack is copied: the pages link to it, and the park is dumped from your map at
  run time.
