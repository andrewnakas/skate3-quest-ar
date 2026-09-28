# Repository content

Private runtime assets, extracted maps, custom animation clips, build output,
settings and logs stay local. They are excluded by `.gitignore`.

The two project-owned binary fixtures are generated without game assets:

- `maps/format-demo.skate`: `python tools/make_skate_demo.py`
- `crates/skate-data/tests/fixtures/retail-collision.rwcmset`:
  `python tools/make_collision_fixture.py`

Run these commands from the repository root. Both generators reproduced the
tracked fixtures byte for byte during publication preparation.

The README artwork in `docs/images/skating-crab.png` was supplied by the project
owner for publication. Other binary resources belong to the vendored Bevy dependencies, whose license
files remain alongside their source. Do not add extracted game content to
these directories or force-add ignored private files.

The pre-publication audit inspected all reachable historical blobs for private
paths, matches against local private-file hashes, unexpected binary files and
common credential patterns. No matches requiring removal were found.
