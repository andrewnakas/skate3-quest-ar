# Installer performance and fidelity

Setup keeps the same asset conversion formats, lossless compression settings,
map validation, and publication checks. The optimizations remove repeated work:

- SFIL padding checks use a view of the input instead of copying the remaining
  stream for each asset. District cells share the same ATOC lookup table.
- Map preparation creates its output directories once. Direct installation
  avoids temporary model/texture RX2 copies which the writer never read and
  setup previously deleted. Standalone extraction still writes these sources
  by default; simulation and irradiance sources remain available in both modes.
- Spawn selection consumes collision meshes during their existing decode pass,
  retaining triangle order, floating-point operations, and tie behavior.
- Model serialization reads each mesh's arrays once. Prop transforms only read
  arrays belonging to the selected mesh.
- The installation builds and validates its shared prop catalog once, then
  passes a JSON snapshot to map workers. It lives inside the current conversion
  workspace and is removed after successful installation.
- Two bounded texture-packing threads overlap lossless zlib compression, with
  the original texture order and compression level. Map-worker limits retain
  the existing CPU and available-memory checks.

Map entries in `maps.json` now include `phase_seconds` for extraction,
preparation, collision packaging, map writing, prop export, validation, and
hashing/cleanup. These are diagnostic timings, not runtime asset inputs.

## Verification

A Windows x64 comparison on 2026-09-09 used packaged installers, the same
extracted owned disc, all ten maps, three map workers, and the same runtime
executable. A full installation took 117.8 seconds before and 91.5 seconds after
the changes. A subsequent baseline run took 103.0 seconds, putting the observed
reduction at 11–22%, depending on the baseline run. These are local warm-cache
measurements; ISO unpacking and downloading the ISO tool were outside them.

The comparison used the same output path for both runs and checked every file
under `assets`, `maps`, and `settings`: no missing or extra files; 2,231 files
were byte-identical, including all ten maps. Five prop packages had identical
binary payloads and decoded metadata, with differences only in JSON object key
order. Prop texture metadata is now sorted to make its order deterministic.
Timing records and logs were excluded from asset equality checks.

Both complete installations passed every existing runtime asset check. The
asset-pipeline suite passed 48 tests, with one unrelated owned-customisation
fixture skipped because it is opt-in. New fixtures cover serial/parallel texture
byte equality, cubemap orientation, compressible and incompressible textures,
worker failures, spawn ties, catalog matrix precision, selective mesh reads,
and malformed or incomplete SFIL input.
