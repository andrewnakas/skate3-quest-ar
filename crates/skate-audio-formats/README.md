# skate-audio-formats

Parsing for Skate 3's audio containers. Byte handling and bounds checks only — codec
arithmetic belongs elsewhere, matching the split the engine's existing `abin` module states.

- `eb` — EA "EB" v3 archives
- `eaac` — EA Audio Core stream headers, block chains, chunk splitting, `.sth` sub-sounds
- `mus` — interactive-music segments

All three classes convert **sample-exact** through this crate's parsing plus the offline
decoder (`docs/xma-transcode.md`). `.mpf` sequencing is the one container gap: the top-level
structure is decoded but sections 0–3, which order the music segments, are not.

`cargo test` runs 33 unit tests. Those use synthetic payloads and prove only self-consistency;
the `examples/verify_*.rs` programs run the same code over real archives and are what
actually find bugs. Point them at your own copy of the game.

Synthetic tests have never once caught a format bug in this project. Real data has caught
every one — four separate times — so run the verify examples against all three asset classes
before believing a format works.
