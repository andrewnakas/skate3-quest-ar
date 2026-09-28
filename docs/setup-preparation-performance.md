# Character setup preparation measurements

Measured 2026-09-10 on Windows 11 Pro, Ryzen 7 9800X3D (8 cores / 16 threads),
32 GB RAM, Crucial P3 4 TB NVMe system drive. The owned Xbox source and all test
outputs were on that drive. No maps were extracted, copied or converted for
these measurements, and no game was launched.

## Release-dependency measurement

Python 3.13.9, NumPy 2.2.6 and Pillow 11.3.0, matching `requirements-setup.txt`.
Both versions used the same freshly compiled native RefPack library from this
checkout. The baseline Python sources were taken from commit `c8219a5`.

| Stage | Before | After |
| --- | ---: | ---: |
| Catalog | 0.98 s | 1.18 s |
| Clothing/body/material library | 426.43 s | 97.27 s |
| Authored clothing lighting | 5.83 s | 2.04 s |
| Menu | Not timed separately | 0.02 s |
| All 41 pro/special characters | Not timed separately | 140.59 s |

The new complete staged customiser run took **245.62 s**, including source
hashing, all five stages, output receipts and publication. There is no measured
old full-customiser total, so the library comparison must not be advertised as
a full-setup speed multiplier. The new stage timings include receipt creation;
the old timings surround the corresponding converter function.

Each cold-output run started with no generated character library, decoded
textures or models. Only the two required, already prepared stock inputs were
reused: OnBoard.abin and the female fallback recipe. This is **not** a cold OS
cache test: filesystem caches were not flushed, and this was a development PC
with other checks running during parts of the measurements. These are single
observations, not controlled repeated trials or guarantees for other hardware.

A subsequent generation reused matching, checksummed stages in **19.93 s** total:
catalog 0.30 s, library 17.29 s, menu 0.01 s, lighting 0.08 s, roster 0.64 s.
This included checksum verification, generation-relative JSON reference updates
and hardlinking immutable output. It demonstrates validated reuse across a
setup-code change with unchanged export identities; it is not a fresh extraction.

## Profile and diagnosis

Before editing converters, the full library and smaller model/texture samples
were profiled. The source-only profile used Python 3.13.9, NumPy 2.4.3 and Pillow
12.1.1 with the Python RefPack fallback. Full-library wall time was 805.53 s
before and 288.83 s after. Profiling overhead and overlapping development work
mean these figures should not be compared directly with the release-dependency
numbers above.

The old profile spent 424.37 s in 2,920 texture decodes and 171.97 s in 480 GLB
conversions, including 83.33 s repeatedly constructing the animation source.
It parsed RX2 geometry 6,240 times. The new profile spent 66.04 s decoding the
same textures. The source fallback's archive decompression then dominated;
release packaging already supplies a native decoder, so a freshly built native
library was used for the second measurement.

The character-only path loaded `rx2_parser.py` by filename without making its
sibling `rx2_fast.py` importable. It silently selected scalar pixel loops;
other setup stages could incidentally make the fast decoder available. Loading
the matching sibling explicitly removes that import-order dependency. A
12-texture diagnostic sample went from 0.595 s to 0.158 s.

The library now retains one read-only animation source and reference skeleton
per invocation, and passes each item's already parsed mesh and morphs into its
GLB conversion. Item-local data is discarded each iteration, bounding memory.
Lighting resolves authored shader parameters once per model instead of once per
material variant; each variant still receives its own texture and conflict check.
No extra concurrency was added: eliminating duplicated work and ensuring the
existing vectorized decoder is used produced the measured improvement without
additional worker memory or contention.

## Correctness and recovery evidence

- Source-profile comparison: all 4,189 GLB/PNG outputs were byte-identical,
  and library metadata was equal: 480 models, 2,427 materials, 170 tattoos.
- Release-dependency comparison: all 4,218 GLB/PNG outputs, including specular
  masks, were byte-identical. Base library, final lit library and native lighting
  metadata matched after normalizing the generation-relative path prefix.
- The complete new pipeline published all 41 pro/special characters successfully.
  Native roster output was not separately compared against an old full roster.
- Owned-output checks verified geometry bounds, finite buffers, skin weights,
  all 22 live morph targets, secondary UVs, hair opacity and referenced textures.
- Synthetic recovery tests cover default.xex validation, failed finalization,
  preservation of maps/user profiles/mods, crash-released locking, incomplete
  stages, same-size corruption, dependency invalidation, and a crash between
  publishing a generation and deleting its pending record.
- `cargo check --locked -p skate-game --bin skate3rust --no-default-features`
  passed; no game was automatically launched. Gameplay remains user-tested.

## Reproducing stage measurements

With setup dependencies installed, run the character helper against an explicitly
chosen prepared asset root containing its stock inputs:

```text
python -m tools.asset_pipeline.customiser_setup --game PATH_TO_DEFAULT_XEX --assets PATH_TO_ASSETS
```

Stage timings and reuse flags are written inside the selected private character
generation as `timings.json`. To measure fresh output, use a separate owned test
asset root with the required stock inputs, rather than deleting working user
assets. To profile, run the same module through Python's `cProfile`. Keep derived
game content and machine-specific benchmark paths out of commits.

`pipeline-equivalence.json` contains exact old/new pairs for the unchanged core,
HUD, default-character, environment and map exports at `c8219a5`, covering LF and
CRLF package source conventions. The export recipe split preserves the original
operations. Character converter reuse and decoder dispatch were checked by the
byte comparisons above; map exporters were not modified. New fingerprints
normalize text line endings. Each equivalence is bound to its specific new
fingerprint, so later exporter edits cannot inherit an old bypass. Unknown
historical fingerprints still require conversion; no general stale-version
exception exists.
