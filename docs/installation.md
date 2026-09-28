# Windows installation

`scripts/Build-Release.ps1` produces `target/skate3rust-windows-x64.zip`. Keep its
`support` directory beside `skate3rust.exe`. The game links Bevy and the MSVC
runtime statically; the setup helper bundles Python, NumPy, Pillow and Tcl/Tk.
It also bundles a small Rust RefPack decoder. Users need no compiler.

On first launch and after an extractor update, setup asks for a local Xbox 360
Skate 3 `default.xex` or ISO. Selecting `default.xex` uses the surrounding
folder, which must still contain the full game data. Extracted copies skip
ISO extraction and need no tool download. For ISO input, setup downloads
the hash-pinned extract-xiso utility from XboxDev. Asset conversion runs in
the bundled setup helper. Blender is neither required nor downloaded.
No game files are downloaded or included in the package.

The conversion pipeline:

1. Extracts the disc into a new installation workspace.
2. Preserves native animation banks and state graphs from `miscload.big`,
   and the controller mapping from `miscboot.big`.
3. Reads the big-endian 64-bit AttribSys schema and collections from `db.big`.
   Settings are decoded from the disc, with numeric hashes retained for names
   absent from debug strings. The JSON export serves the fields consumed by
   this engine; it is not a general VLT editor or portable array serialization.
4. Builds the default modular skater from the CAC archive, including textures,
   morph settings, the native rig and its four board-relative IK targets.
   Writes GLB directly with retail skin weights, inverse-bind matrices and
   the bone-local basis used by the runtime. Gameplay animations are evaluated
   from ABIN by the Rust runtime, without offline animation baking.
5. Converts each `worldDIST_*.big` directly into a v14 `.skate` map. Native
   collision archives and grind splines are embedded; collision triangles are
   decoded from that archive by the game rather than duplicated in the file.
   University uses the existing starting location. Other
   districts select a broad upward collision face near the district origin.
6. Runs the game's asset and map loaders before publishing `installation.json`.
   This validates loading, not gameplay. Conversion intermediates are removed
   after their output succeeds. Failed workspaces retain logs for diagnosis.

Data lives in `data` beside this portable copy's executable. `installation.json` points
to the completed installation. Conversion tools are cached separately, so
retrying does not download them again. `setup-error.log` reports setup errors;
each installation also has `setup.log` with converter output.

For an already extracted disc, developers can use `tools/prepare_assets.py`
with `--game-root`, `--output` and `--game-exe`. The release helper can run the
same entry point with `--task tools/prepare_assets.py`.

The Escape menu discovers installed `.skate` files. Loading a map starts a new
game process and closes the current session so physics and render resources
are recreated. A fresh launch defaults to University. Development checkouts
with `assets/private/game.json` retain their existing local assets and default
test world; `--assets DIRECTORY` also selects a prepared asset set explicitly.

## Conversion checks

The direct converter was checked against an extracted Xbox 360 disc containing
the three main districts, Skate School, and six separately stored parks:
Black Box, Downtown Skate Park, Industrial Skate Park, Maloof, Mega Park and
Start Park. All ten outputs passed the runtime's asset, spline and collision
load checks. This does not establish visual or gameplay parity, and the
original ISO extraction step still needs checking with an ISO.

Two retail spline details needed corrections. Downtown contains duplicate
knots and sub-millimetre chords. The native cubic records remain intact;
only unusable contact primitives are omitted. Skate School's first spline
header word is `0001 0002`, meaning two rails, not 65,538. TU3 function
`82C1EEF0` reads the count with `lhz +2` (also at `82C1EFBC`). The reader now
uses that halfword and retains the other halfword in the map metadata.

## Setup performance

On the development PC, a full packaged conversion of the same extracted disc
dropped from approximately 13 minutes to 81 seconds. This includes skater
preparation, all ten maps, runtime load checks and cleanup; ISO extraction is
excluded. Timing depends on CPU, available memory and storage.

Setup uses native RefPack decompression, batched NumPy texture decoding,
uncompressed temporary geometry/texture caches and faster lossless final
compression. The spawn search evaluates triangles in batches and rejects
distant collision bounds before decoding them. Up to three isolated map
workers run together, limited by available RAM, with the largest districts
scheduled first. Per-map conversion and load logs remain in the installation.

Decoded map geometry, material data, texture pixels, native collision archives,
rail payloads and the character GLB were compared with the earlier outputs.
Final map storage uses a different compression level, without reducing quality.

## Separate portable installations and asset refresh

Each copy uses `data/installation.json` beside its executable. A freshly unpacked
ZIP has no record and always asks for an Xbox source. Startup does not search the
working directory, another package, or `%LOCALAPPDATA%` for prepared game assets.
Once that copy is set up, ordinary launches reuse its own data. `--assets` remains
an explicit developer override and bypasses setup and asset-refresh management.
Do not ship a local `data` folder in release ZIPs.

The release manifest includes fingerprints for core data, HUDs, character,
environment and maps. They are derived from the packaged extractor sources and
shared dependencies, not the game build number. An in-place program update keeps
`data` intact. Next startup offers an asset refresh only when these fingerprints
change. A HUD extractor edit refreshes HUDs without converting maps; changes to
core VLT/animation decoding invalidate dependent groups too. Shared map/material
parsers can invalidate both maps and environment assets. New converter modules
must be assigned to their consuming groups in `asset_pipeline/versions.py`.

Refresh uses the same `Select ISO or default.xex` file picker as initial setup;
the player never needs to select an intermediate extraction folder. ISO sources
must be unpacked again when a refresh needs disc files; unchanged asset groups
are reused without conversion. The chosen Xbox executable
must match the original edition. Source paths stay in local installation records
and are never included in published release metadata.

Updates run only affected exports. Core/HUD/character/environment/map recipes
have separate dependency identities; setup UI and transaction changes do not
invalidate every map. Explicit, exact old/new fingerprint equivalences cover
verified unchanged exports when migrating the earlier broad fingerprint scheme.
Unrecognized versions still refresh normally.

Core refreshes create a new installation before publication. Unchanged prepared
maps and immutable character generations use hardlinks (copy fallback on filesystems without support); mutable user files and
outputs being rebuilt have independent storage. Customiser-only refreshes keep
the current core installation and publish a new character generation. Custom
models, profiles, settings, selections and mods are retained. Core gameplay inputs and at least one map must pass validation before the
installation record changes. Missing optional content is recorded separately;
fatal failures preserve the previous record.
A kernel lock prevents concurrent setup and releases automatically on process
exit, including a crash; the harmless `setup.lock` file can remain on disk.

Character catalog, library, menu, lighting and roster stages have separate
source/dependency identities and SHA-256 output receipts. Completed stages can
be reused across generations or resumed after an interruption; partial stages
are rebuilt before any existence-based converter sees them. Setup verifies
checksums before reuse. Startup checks receipt paths and file sizes, avoiding a
full map checksum scan on every launch. Same-size corruption is detected during
setup validation, rather than ordinary startup. Old generations remain for
recovery; this still requires space for genuinely changed output.

See [character preparation measurements](setup-preparation-performance.md) for
benchmark conditions, stage timings and output equivalence checks.

Installations from releases that used the old global asset store need one setup
in the new per-copy layout. There is deliberately no automatic global migration.
Manually unpacking a ZIP over the same folder retains that folder's `data` and
behaves like an in-place update; unpack into a new folder for a fresh setup.

## Missing content

Setup isolates missing or malformed optional items. Clothing variants and tattoos
that cannot be decoded are excluded from the customiser. Missing pro characters,
lighting, HUD banks, environment extras and movable props do not block core setup.
Scoring and session-marker artwork are prepared independently. Verified previous
HUDs and customiser generations are kept when a replacement fails; otherwise the
stock skater and available features remain usable.

Each district converts independently. A failed or absent source district can use
its previous converted map only after checksum and engine validation. Initial
setup can select another available district if University is absent. Missing core
animation, physics, the stock skater, or every playable map still requires repairing
the source. Missing original artwork cannot be recreated by ignoring the error.

Setup displays a notice and writes `setup-report.json` in the active installation,
with unavailable items and error details. Availability records participate in
output receipts so acknowledged missing extras do not cause a setup loop. A new
extractor fingerprint requests preparation again. Disk and permission failures
in in-process conversion still abort rather than publishing partial writes.
