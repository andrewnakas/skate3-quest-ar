"""Split a retail recomp audio capture into small per-subject files.

The capture comes from the recomp's `src/skate3_audio_capture.cpp` (enabled with the environment
variable SKATE3_AUDIO_CAPTURE). Each line is `<host_ms> <mixer_frame> <TAG> ...`. This keeps the
local skater's rows only and writes, under `<out>/`:

  state.tsv      frame, ms, then the 160 audio-state words at +192 (ST, local state only)
  posts.tsv      every PO line (all objects)
  updates/<object>.tsv   every UP line for that object
  releases.tsv   every RL line
  board.tsv      the local board's BD lines
  picks.tsv      every GP line
  loads.tsv      every GL line
  vf.tsv         every VF line whose `this` is a local player component (by controller)
  mixmap/<ctrl>.tsv   MC lines for the controllers the local components read

Usage: python tools/audio_capture_extract.py <capture.log> <out-dir> [local-state] [local-board]
"""
import os
import sys
from collections import defaultdict


def main():
    capture, out = sys.argv[1], sys.argv[2]
    local_state = sys.argv[3] if len(sys.argv) > 3 else "40C10E20"
    local_board = sys.argv[4] if len(sys.argv) > 4 else "40C3B020"
    os.makedirs(os.path.join(out, "updates"), exist_ok=True)
    os.makedirs(os.path.join(out, "mixmap"), exist_ok=True)
    files = {}

    def w(name, line):
        f = files.get(name)
        if f is None:
            f = files[name] = open(os.path.join(out, name), "w", encoding="utf-8")
        f.write(line)

    # First pass: which controllers do local components read?
    controllers = set()
    with open(capture, encoding="utf-8", errors="replace") as f:
        for line in f:
            parts = line.split(" ", 6)
            if len(parts) > 5 and parts[2] == "VF":
                controllers.add(parts[5])
            elif len(parts) > 4 and parts[2] == "ST" and parts[3] == local_state:
                controllers.add(parts[4])
            elif len(parts) > 4 and parts[2] == "BD" and parts[3] == local_board:
                controllers.add(parts[4])

    with open(capture, encoding="utf-8", errors="replace") as f:
        for line in f:
            parts = line.split(" ", 5)
            if len(parts) < 4:
                continue
            ms, frame, tag = parts[0], parts[1], parts[2]
            rest = line.split(" ", 3)[3] if len(parts) > 3 else ""
            if tag == "ST":
                if parts[3] == local_state:
                    words = line.split("|", 1)[1].split()
                    w("state.tsv", "\t".join([frame, ms] + words) + "\n")
            elif tag == "PO":
                w("posts.tsv", f"{frame}\t{ms}\t{rest}")
            elif tag == "UP":
                name = parts[4].replace("/", "_")
                w(os.path.join("updates", name + ".tsv"), f"{frame}\t{ms}\t{rest}")
            elif tag == "RL":
                w("releases.tsv", f"{frame}\t{ms}\t{rest}")
            elif tag == "BD":
                if parts[3] == local_board:
                    w("board.tsv", f"{frame}\t{ms}\t{rest}")
            elif tag == "GP":
                w("picks.tsv", f"{frame}\t{ms}\t{rest}")
            elif tag == "GL":
                w("loads.tsv", f"{frame}\t{ms}\t{rest}")
            elif tag == "VF":
                w("vf.tsv", f"{frame}\t{ms}\t{rest}")
            elif tag == "MC":
                ctrl = parts[3]
                if ctrl in controllers:
                    w(os.path.join("mixmap", ctrl + ".tsv"), f"{frame}\t{ms}\t{rest}")
    for f in files.values():
        f.close()
    print(f"controllers read by components: {len(controllers)}")
    for name in sorted(files):
        print(name, os.path.getsize(os.path.join(out, name)))


if __name__ == "__main__":
    main()
