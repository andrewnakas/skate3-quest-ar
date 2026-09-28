# ZIP mod packages (SDK 1)

For agents, start at [sdk/AGENTS.md](../sdk/AGENTS.md). For function signatures, use
[sdk/skate.lua](../sdk/skate.lua); for callbacks/settings use [Lua SDK](lua-modding.md).

## Installation and updates

Drop a `.zip` directly into top-level `mods/` and launch `PLAY.bat`.
Open Escape → Mods, select it and enable it. New IDs start disabled. Open mods folder
opens this same directory. SKATE3_MODS overrides discovery; a standalone executable
defaults to its adjacent mods/. No manual extraction is needed.

The game scans approximately every 500 ms. Changed enabled packages reload after a
750 ms debounce; Rescan forces discovery. Replace the ZIP to update; remove it to
uninstall. Stable IDs preserve compatible settings, stored outside the ZIP. Reloading
restarts Lua and removes/recreates owner objects. Removing a driving vehicle releases
its rider. Disabling a mod keeps its package available for later use.

The loader supports editable folders too, but **never install a folder and ZIP with
the same mod ID**. All copies of a duplicate ID are rejected. Hidden directories are
not packages. The two editable examples are under sdk/examples; live mods/ contains
native-trainer.zip and mario-kart.zip. Archives remain local and untracked.

## Exact archive structure

```text
my-mod.zip
  mod.json                 REQUIRED: manifest at archive root
  main.lua                 REQUIRED if manifest.entry is main.lua
  README.md                recommended controls and credits
  vehicle.json             optional vehicle definition
  assets/
    kart.glb               optional embedded vehicle model
    help.txt               optional package text data
  animations/
    rider.json             optional native vehicle clips
```

Do not wrap this in a my-mod/ parent folder. `entry` may name a different relative
Lua file. Optional directory names are conventions, not magic. Paths in vehicle.json
are **package-root relative**, even if the definition itself lives in a subdirectory.
For the above layout, model=`assets/kart.glb`, animations.file=`animations/rider.json`.
Lua uses `sdk.vehicle.spawn('kart','vehicle.json',position,heading)` and
`sdk.read_text('assets/help.txt')`. A ZIP's filename does not determine its mod ID.

Include only files your mod needs. Vehicle GLBs must embed geometry, buffers and textures.
Rider JSON must match the installed native skeleton exactly; FBX and .blend are authoring
inputs, not runtime animation files. Engine audio is currently synthesized through the
vehicle definition; adding arbitrary WAV/MP3 files does not make them playable.
No executable binaries or external Lua modules are loaded.

## Minimal complete mod

mod.json:

```json
{
  "id": "yourname.hello", "api": 1, "name": "Hello Skater",
  "version": "1.0.0", "author": "Your name", "description": "Example HUD mod.",
  "entry": "main.lua",
  "settings": {
    "message": { "type": "string", "label": "Message",
      "description": "Text shown on screen.", "default": "Hello skater" }
  }
}
```

main.lua:

```lua
local function draw()
    sdk.ui.text('hello', sdk.settings.message)
end
return {
    on_load = draw,
    on_settings = draw,
    on_event = function(event)
        if event.name == 'world_changed' then draw() end
    end,
}
```

The host removes owner visuals on disable/reload. Number settings require finite min,
max, positive step and an in-range default. Choice settings require choices. Unknown
manifest fields are rejected; there is no permissions or dependency declaration.
Read the full setting and callback reference before extending this example.

## Package and check

From the repository:

```powershell
python tools/package_mod.py sdk/examples/your-mod mods/your-mod.zip
cargo run --locked -p skate-mods --example check_mod -- mods/your-mod.zip
```

Requires Python 3.9+ and Rust/Cargo. The packer uses Python's standard ZIP library and
runs the authoritative Rust checker on both the source and ZIP before replacing the
output. It writes a temporary non-ZIP file first so discovery cannot see a half-written
archive. Optional `--target-dir .local/vehicle-tests` selects the Cargo cache.
The checker does not execute Lua callbacks. Passing it is not a gameplay test.

`Build.ps1` builds the host, packages both example sources and stages a
standalone copy with docs. Older mod folders in its private staging directory are
preserved under hidden .legacy-* names to avoid duplicate IDs. It does not delete
personal mod edits. Both installable example ZIPs are committed, along with the
Mario Kart model/rider source assets, so a fresh checkout can use or rebuild both mods.
ZIP metadata is deterministic; an unchanged source folder produces identical package bytes.

## Limits and extraction rules

- At most 128 immediate packages; 512 ZIP entries and 512 extracted files/directories.
- At most 64 MiB compressed and 64 MiB expanded per ZIP/package.
- ZIP Store or Deflate compression. No encryption, links or special file entries.
- Relative forward-slash paths only, at most 240 bytes for ZIP paths. No traversal,
  absolute/drive paths, Windows device names, forbidden characters or trailing dots/spaces.
- Duplicate paths, including case collisions, are rejected. No overwrite extraction.
- mod.json ≤64 KiB; Lua entry ≤256 KiB of UTF-8 source, no bytecode.
- Vehicle definition ≤128 KiB; model ≤32 MiB; vehicle clips ≤16 MiB.
- Text reads ≤256 KiB; authored vanilla body replacements ≤4 MiB. Other Lua quotas
  and native validation rules are described in lua-modding.md and vehicle-sdk.md.

The loader checks/decompresses within bounds, then writes to a new session directory
under mods/.cache. Assets use this private directory transparently. Never edit or package
.cache. Changed ZIPs get new cache paths, so assets cannot silently reuse the old model.
Session caches are removed on normal shutdown; a crash may leave cache directories,
which may be deleted with the game closed. Do not delete live session caches.
A rejected ZIP shows diagnostics in the Mods UI/log and does not continue running an
outdated copy indefinitely. Cache contents do not become independent installed mods.

## Further examples

- [Native Trainer](../sdk/examples/native-trainer/README.md): tuning, HUD, markers, timers.
- [Vehicle SDK](vehicle-sdk.md): physics, controller input, ramp tuning and engine audio.
- [Mixamo workflow](mixamo-vehicle-workflow.md): calibrated native-rig animation authoring.

## Multiplayer

Use matching enabled ZIPs and the same integrated build on all peers. World objects
and vehicles synchronize through the SDK; Lua shared rules use `sdk.net`.
See [Multiplayer mod SDK](multiplayer-mods.md) for ownership, collisions,
late joins and shared-state examples.
