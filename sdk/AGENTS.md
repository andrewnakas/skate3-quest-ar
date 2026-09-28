# Agent guide: make a mod for this game

Use this file as the starting point when asked to make a mod. The current SDK is
embedded Lua 5.4, API major 1. Deliver a validated `.zip` in the project-root `mods/`.
Do not invent API calls or assume this is an unrestricted Lua/native-code plugin system.

## Read these sources in order

1. `docs/mod-packages.md`: ZIP structure, packaging and installation.
2. `sdk/skate.lua`: callable functions, argument types and bounds; annotations only.
3. `docs/lua-modding.md`: manifest, settings, lifecycle, quotas, callbacks and examples.
4. `docs/multiplayer-mods.md`: automatic replication, matching packages, ownership and shared state.
5. For vehicles, `docs/vehicle-sdk.md` and `sdk/examples/mario-kart/vehicle.json`.
6. For rider animations, `docs/mixamo-vehicle-workflow.md`. For character conversion,
   `docs/mixamo-to-skate.md` and `docs/custom-models.md`.

Paths are relative to the repository root. Authoritative implementations are
`crates/skate-mods/src/{schema,vm}.rs`, `crates/skate-vehicles/src/definition.rs`, and
`crates/skate-game/src/modding/`. If a requested feature has no API, explain the gap
or implement/test a shared host API; do not fake an unsupported call in the mod.

## Workflow

- Create an editable folder outside live mods/, e.g. `sdk/examples/your-mod`.
  Copy the smallest suitable example, not the whole repository. Use a unique ID
  like `author.feature` and keep that ID stable across updates.
- Write mod.json with api=1, name, version, author, description and entry. Each
  setting needs type, label, description and default, plus bounds/options for its type.
- Return a callback table from main.lua. Use owner-local stable keys for visuals,
  read settings via sdk.settings, edge-detect held inputs, and clean/reset transient
  state on world_changed. Refresh vehicle controls from on_fixed_update.
- Decide which player owns each shared rule. Cubes and vehicles replicate automatically;
  UI/input stay local. Use sdk.net.publish/read for Lua state and retain player IDs as strings.
  Reuse stable keys; do not broadcast one-frame events or independently award shared scores
  on every client. Consider host changes, late joins and mod disable cleanup.
- Keep assets inside the package and use relative forward-slash paths. The entry
  and vehicle filenames need not be the example names, but their references must match.
  The game loads one Lua entry file; do not assume require/dofile or arbitrary file I/O.
- Build and validate:

```powershell
cargo run --locked -p skate-mods --example check_mod -- sdk/examples/your-mod
python tools/package_mod.py sdk/examples/your-mod mods/your-mod.zip
```

- For code/API changes, also run `cargo test --locked -p skate-mods -p skate-vehicles -p skate-net`.
  Check the game crate and build with `Build-VehicleSDK.ps1` when the host changed.
  The helper packages the two bundled examples; package a new mod separately.
- The checker checks structure, limits, schema and Lua syntax without running the mod.
  It does not prove callbacks, render results, rig compatibility or physics behaviour.
  Run appropriate headless tests. Do not launch the game automatically in this workspace;
  provide `PLAY-MARIO-KART.bat` for the user to test and state what remains unverified.
- Report the ZIP path, controls/settings, validation results and relevant limitations.
  The Native Trainer and Mario Kart ZIPs are intentionally committed, as are the
  prepared kart.glb and rider.json source assets. Keep those ZIPs in sync with their
  editable examples. Other imported assets/raw authoring files remain ignored.

## Available building blocks

- Player/input snapshots, native teleport requests, log/text, coloured cubes, timers,
  package text reads and supported authored animation replacement slots.
- Native Trainer tuning: pop, push, braking, steering, grip, fakie hold and related
  multipliers. Only one mod can own trainer tuning at a time.
- Vehicles: spawn/read/control/tune/enter/exit/reset/remove, Rapier map and vehicle
  collision, raycast wheels/suspension, collider offsets/rounding, independent inertia
  and centre of mass, throttle-responsive engine synthesis and live engine volume.
  Rider safety adds an occupied capsule, crash/inversion ejection, carried momentum
  into the native bail state and vehicle_bailed events; configure rider_safety.
- Native-rig rider enter/exit/drive/idle/reverse/brake/steering slots. The host handles
  steering blending, vanilla hand-offs, board hiding and shared motion interpolation.
- Automatic per-mod draggable/resizable/collapsible settings windows and enable badges.

No native DLL/EXE loading, arbitrary network/filesystem access, external Lua imports,
inter-mod dependencies, custom audio recordings or arbitrary rig playback are exposed.
Vehicle replay recording is not implemented. Multiplayer and car/player collision
bridging are available; see docs/multiplayer-mods.md for ownership and approximation
limits. Assets can be bundled but only documented asset types have loaders.
