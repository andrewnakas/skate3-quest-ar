# Lua modding SDK 1

Start with [the agent instructions](../sdk/AGENTS.md) and [ZIP package guide](mod-packages.md).
Use `PLAY.bat` / `Build.ps1` for the current integrated build;
older named trainer/window launchers are historical snapshots. The project launcher
loads top-level `mods/`, starts paused, and writes `.local/vehicle-sdk/session.log`.
Prepared engine assets are required; Lua itself is embedded.

Open Escape/Start → Mods. New mods start disabled. Each has a red DISABLED or green
ENABLED badge and explicit enable/disable action. Enabled mods have separate movable,
resizable and collapsible settings windows with continuous scrolling and a scrollbar.
The ordinary pause menu stays centred. Minus/plus controls change values; Tab/controller
X cycles focus, arrows/D-pad select/adjust, and strings open the text editor. Changes
are saved outside the package. Window layout is retained for the current process.

## Quick start

Copy `sdk/examples/native-trainer` into a new authoring folder, change its `id` and `name`, then package it with `tools/package_mod.py`. A ZIP contains:

```text
my-mod.zip (archive root)/
  mod.json
  main.lua
  assets/             # optional original text data or authored body clip JSON
```

`SKATE3_MODS` overrides the discovery directory; its default is the top-level `mods` folder **beside the running game executable**, independent of the working directory. The engine creates it if absent and logs its full path; the Mods list has an **Open mods folder** action. Immediate `.zip` files and non-hidden development folders are packages. ZIPs require mod.json at their root and are extracted automatically into a session cache. `SKATE3_MOD_SETTINGS` overrides the writable preferences directory; the default is `settings/mods` beside the engine asset directory. Keep preferences outside packages.

A minimal manifest:

```json
{
  "id": "yourname.first-mod",
  "api": 1,
  "name": "My first mod",
  "version": "1.0.0",
  "author": "Your name",
  "description": "An original example.",
  "entry": "main.lua",
  "settings": {
    "greeting": {
      "type": "string", "label": "Greeting",
      "description": "Text shown on screen.", "default": "Hello skater"
    }
  }
}
```

```lua
local function draw()
    sdk.ui.text('greeting', sdk.settings.greeting)
end
return {
    on_load = draw,
    on_settings = draw,
    on_event = function(event)
        if event.name == 'world_changed' then draw() end
    end
}
```

Edit the greeting in the menu; Lua receives it immediately. Edit and save the script; enabled mods reload after files settle. Disable it and its text disappears. Add `sdk.log('loaded')` for a labelled log line. Use `sdk/skate.lua` as a Lua language-server library (Lua 5.4); do not execute this annotation file or package it as your entry point.

## Manifest and settings reference

Unknown manifest/setting fields are errors, including `dependencies`, `conflicts`, `permissions` and `enabled`. SDK 1 does **not** implement inter-mod dependency resolution or cross-mod imports. Packages must be independent; do not depend on another mod's callback side effects. Conflicting IDs reject all copies, regardless of directory ordering. Conflicting animation slots reject the later installing mod. Packages execute in ascending, case-sensitive ID order.

All manifest fields in the minimal example except `settings` are required. `api` is the integer `1`, an exact compatibility major, not a range. `version` is three unsigned integer components `MAJOR.MINOR.PATCH`. IDs and setting/object keys contain 1–64 ASCII lower-case letters, digits, dot, underscore or hyphen; `.` and `..` alone and Windows device-name stems such as `con` or `aux` are invalid. Use a globally distinctive author prefix. IDs identify saved preferences across directory moves and upgrades. Name and author are at most 120 bytes; name is nonempty. Description is at most 2048 bytes. Entry is a relative UTF-8 Lua source file, at most 256 KiB. Bytecode is not accepted.

`settings` is an object keyed by stable setting ID (at most 24). Rows sort by key. Every schema entry requires `type`, `label`, `description`, and `default`. Label is at most 120 bytes and description at most 512 bytes. Types:

| Type | Value | Schema fields |
| --- | --- | --- |
| `boolean` | JSON/Lua boolean | `default` true or false |
| `number` | finite JSON/Lua number | required finite `min`, `max`, positive `step`, and default in range |
| `string` | string without control characters, at most 128 Unicode characters | `default` string |
| `choice` | string equal to a listed choice | required nonempty `choices`, at most 32 strings, each at most 128 bytes |

`min`, `max`, `step` and `choices` are optional on other types and have no effect there. Number step controls the menu increment; values need not lie on that increment grid. Saved values are type/range validated. Missing, invalid or renamed values use defaults; removed keys are discarded on loading the current schema. Same-key compatible values survive version changes. Renaming a key intentionally resets it. There is no arbitrary Lua migration callback or persisted mutable Lua state in API 1. Changing the package ID creates a distinct preference identity. Reset delivers one `on_settings` notification per setting in key order. Preferences store `{ "enabled": boolean, "values": {...} }` as `<id>.json` through a sibling temporary file and rename. Save errors appear in the menu; the live edit may already have applied. Never distribute these preference files with a mod. Enable preference and actual running status differ after a fault: the error remains visible and the mod stays stopped until reload, a settled edit, or a future launch.

There is no generic gamepad rebinding setting yet. A choice containing supported key names, as an ordinary key-choice schema, is an ordinary validated setting; it does not remap native input.

## Lifecycle, ordering and scheduling

Each package owns a fresh Lua state. Entry code must return a table containing only these optional function fields:

| Callback | Argument | Timing |
| --- | --- | --- |
| `on_load` | nil | after source evaluation; top-level and load commands commit together |
| `on_update` | `{dt:number}` | once per active rendered Update, after engine animation; real seconds clamped to 0.25 |
| `on_fixed_update` | `{dt:number}` | after the existing fixed physics set; fixed time delta in seconds |
| `on_event` | event table below | at the indicated host boundary |
| `on_settings` | `{key:string,value:boolean\|number\|string}` | immediately after validation, with new `sdk.settings` already installed |
| `on_unload` | nil | before dropping a running state, including normal host shutdown; commands emitted here are discarded |

Update/fixed callbacks pause during the pause menu and replay. Discovery, settings, load/unload and world notifications remain available while paused. Fixed callbacks observe the just-completed simulation; a teleport request feeds the existing actor reset path on subsequent input publication. Lua never executes on rendering/GPU or map-worker threads. Mods menu input precedes map publication; maintenance follows publication in PreUpdate. Update callbacks run after `FrameSet::Animation`. There is no deterministic lockstep promise across frame rates, filesystems or machines; use fixed callbacks for tick-count logic. Do not expect several fixed callbacks in one render frame to observe distinct keyboard states.

Within each dispatch, callbacks run by ascending mod ID. Commands are collected at the host boundary and grouped by ascending owner ID, preserving that owner's emission order, with its sole teleport deferred until its reversible commands succeed. At most one teleport per owner batch is accepted; if multiple mods request one, the first accepted request makes later ones fail as busy. Host failures stop the offending mod and retire its current owned resources. Log output cannot be retracted. Already completed teleports are gameplay actions and are not reversed when a mod is disabled later.

No callbacks accumulate across reload: there is one returned lifecycle table and one isolated state per running package. Timers and all Lua locals reset. On disable/reload/fault, owned text, meshes, materials, timers and animation overrides are removed. Old queued actions are discarded. `on_unload` errors cannot prevent cleanup. A Lua error discards the entire failing invocation's queue; source/load errors commit no commands. Prior reversible changes made by that mod are also retired when it stops. Lua memory mutations before an error are immaterial because that state is dropped.

Polling occurs at most every 500 ms and hashes bounded package contents (not just timestamps). Changed valid files must retain the same hash for at least 750 ms, so usual effective latency is 1–1.5 seconds. Invalid manifests get a 750 ms grace interval before the old package is retired and diagnostics replace its listing. Deleted directories retire immediately at the next poll. New packages are discovered disabled unless their ID already has a saved enable preference. Explicit Rescan/Reload bypasses debounce: finish saving before using it. A failed reload does not retain the previous version; it remains visibly stopped. Fix/save or explicitly reload to recover. Polling runs on the main thread, so large packages or slow storage can hitch; no zero-hitch guarantee is made. A root-directory I/O error reports diagnostics; a missing root retires its packages.

Successful world changes remove **all** mod objects and animation overrides before `world_changed` is delivered. Lua locals, settings and timers survive this event. Rebuild world-specific objects in the callback, cancel any world-specific timers, and clear stored positions when retaining world-specific positions. `sdk.snapshot.map.generation` identifies the current world, including reloading the same map. Failed map preparation retains the previous world and emits no world-change event.

## SDK API

The callable names/signatures are also in [sdk/skate.lua](../sdk/skate.lua).
Vehicle commands, collider/mass/audio configuration and rider slots are documented
in [vehicle-sdk.md](vehicle-sdk.md). Trainer tuning is documented below.

All vectors are 1-based Lua arrays `{x,y,z}` in engine world coordinates, metres, Y up. All API functions below use dot calls, not colon calls. Mutable snapshot/settings tables are per-state copies: editing them does not mutate the engine. Retained snapshots are stale; query again inside callbacks.

| Function/value | Return and effect |
| --- | --- |
| `sdk.api_version` | integer `1` |
| `sdk.settings` | current validated setting-key → value table |
| `sdk.snapshot` | current observation table described below |
| `sdk.log(text:string)` | nil; queue labelled INFO diagnostic, maximum 2048 UTF-8 bytes |
| `sdk.read_text(path:string)` | UTF-8 string; synchronously read a package-relative file, at most 256 KiB; raises on missing/invalid/traversing/non-UTF8 path or file |
| `sdk.player.read()` | player snapshot table, never a live object |
| `sdk.player.teleport(position:Vec3, heading:number, on_board:boolean)` | nil; queue native manual actor reset; radians about Y, zero facing +Z; preserves the separate bail and session-marker checkpoints |
| `sdk.input.down(key:string)` | boolean; whether Bevy physical key name is held at snapshot time; unrecognized/unpressed names return false |
| `sdk.input.action(id:integer)` | number from published native gameplay action 64–81; errors outside range |
| `sdk.ui.text(key:string,text:string)` | nil; create/update owned plain screen text, max 1024 bytes; stable top-left rows sorted by owner/key, 19px font, 28px row spacing |
| `sdk.scene.cube(key:string,position:Vec3,size:Vec3,color:Vec3)` | nil; create/update owned visual cuboid; no collision, grind attachment or rigid body |
| `sdk.scene.remove(key:string)` | nil; remove owned cube/text; absent key is a no-op; cannot address another mod's objects |
| `sdk.time.elapsed` | active Update seconds since state creation, advanced after `on_update` |
| `sdk.time.after(key:string,seconds:number,callback:function)` | nil; replace keyed one-shot timer; delay 0–86400 finite seconds, key 1–64 bytes, at most 64 timers |
| `sdk.time.cancel(key:string)` | nil; remove timer; absent key is a no-op |
| `sdk.animation.info()` | `{bone_names:string[],slots:table<string,{fps:number,frame_count:integer}>}`; authoring metadata for installed supported body slots |
| `sdk.animation.replace(path:string)` | nil; queue installation of package-relative body-clip JSON, at most 4 MiB; replaces this mod's previous set; conflicting owners or invalid clips stop the mod |

Timers run after `on_update`, within its same instruction budget and transaction. Due timer keys sort lexicographically. A timer is removed before invocation. Newly scheduled timers normally wait for the next Update (replacing an already due, not-yet-delivered key can affect that key's pending delivery). Pausing stops the timer clock. Errors disable the owner. `sdk.time.elapsed` and snapshot fields should be treated as read-only by authors.

Cube positions and teleport positions must be finite, each component within ±100000. Size components must be finite in `(0,100]`. RGB components must be finite in `[0,1]`, interpreted as sRGB. Heading must be finite. Text and cubes share an owner-local key namespace: switching type replaces the old object. At most 64 visual objects per mod; update existing keys to avoid churn. At most 128 commands per Lua invocation. Invalid arguments raise Lua errors; exceeding limits stops the mod. Commands return nil on successful **queuing**, not proof of engine completion. Host rejection appears in the mod's error panel and log. Teleports reject map loading, replay and busy actor-reset conditions. This is a free-position command, not a terrain-safe spawn finder: authors must select valid destinations. It does not set velocities directly or bypass the native deferred reset/camera-cut sequence.

### Snapshot and event payloads

`sdk.snapshot` contains:

```lua
{
  player = {
    position = {x,y,z},       -- current animation-to-world root translation
    velocity = {x,y,z},       -- physical board velocity; not biped walking speed
    on_board = boolean,      -- physical category != 500
    state = integer, category = integer, -- native identifiers, not a trick name
    bailing = boolean,
    grind = {active=boolean,name=string,kind=integer,distance=number}
  },
  map = {name=string,generation=integer},
  tick = integer,            -- engine physics tick, may reset on map replacement
  keys = { [physical_key_name]=true },
  actions = {number,...},    -- 18 values, array slot 1 is native action 64
  paused = boolean, replay = boolean,
  animation = {bone_names={...},slots={...}}
}
```

Keyboard names follow Bevy 0.18 `KeyCode` debug names, e.g. `KeyA`, `KeyJ`, `F6`, `Space`, `ArrowUp`, `ShiftLeft`. Held-state reads do not consume input or inject native actions. Track the previous held state in a Lua local for edge detection. Native actions expose the current published mapping: 64/65 left stick axes, 66 left trigger, 67/68 right stick axes, 69 right trigger, 70/71 stick clicks, 72/73 shoulders, 74–77 face buttons, 78/79 Start/Back, 80/81 D-pad horizontal/vertical as provided by the existing expression map. These are observations, not a binding editor. Consult `crates/skate-core/src/input/gameplay_map.rs` for the retained source mapping; no new native input semantics are introduced.

`on_event` receives one of:

| `name` | Additional fields | Boundary |
| --- | --- | --- |
| `world_changed` | `map={name,generation}` | PreUpdate after successful map publication and mod resource retirement |
| `bail_changed` | `bailing:boolean` | after fixed physics when observed bail flag changes |
| `player_state_changed` | `previous:integer,state:integer` | after fixed physics when native state identifier changes |
| `grind_changed` | `grind={active,name,kind,distance}` | after fixed physics when accepted grind active flag changes |

Fixed event order is bail, state, grind, then `on_fixed_update`. These are sampled transitions, not a complete native event bus: multiple transitions within one engine tick can be coalesced. Initial observations can emit state changes from the host's initial zero/false baseline. No trick-completed, combo or score event is promised by SDK 1; a native state number must not be relabelled as proof of a scored trick. The separate scoring owner can add a source-backed API in a future compatible revision.

### Body-animation asset format

This reuses `animation_pose/authored_clips.rs`, not a parallel animation system. `sdk.animation.info()` supplies exact installed names and timing; author original global joint transforms with your own tools and write:

```text
{
  "version": 1,
  "bone_names": [exact installed skeleton order],
  "clips": {
    "supported slot name": {
      "fps": exact installed slot fps,
      "frames": [ [ [16 floats for bone 0], [16 floats for bone 1], ... ], ... ]
    }
  }
}
```

Matrices are affine, column-major 4×4 global joint transforms. Every frame must contain every bone, in exact name order. The existing parser requires finite matrices, positive determinants, scale components between 0.9 and 1.1, homogeneous last row approximately `(0,0,0,1)`, and exact stock frame count/FPS. It reconstructs board and helper targets and only substitutes its supported body range, leaving stock trajectory, board, timing, graph and channel metadata authoritative. Empty `clips` removes that owner's effective substitutions. Supported slots are `R_ANTIC_OLLIE_N_0_INTO`, `R_ANTIC_OLLIE_N_0_CYC`, `R_ANTIC_360SHUVIT_N_0_CYC`, `360FLIP_D_HIGH_G`, `360FLIP_D_HIGH_A`, `360FLIP_D_LOW_G`, `360FLIP_D_LOW_A`. No other slot is accepted. Call replacement at load and after world changes; parsing is synchronous and should not be repeated per frame. The pre-existing optional private replacement loader remains beneath mod overrides. This SDK does not activate or rename any disabled private clip or climbing file.

## Trust, assets, compatibility and limits

Only install mods from authors you trust. Isolation is an engine fault-containment boundary, **not** an OS sandbox or a promise to safely execute hostile code. There is no raw world/entity access, pointer API, FFI, raw socket/network access, writable Lua filesystem API, `io`, `os`, `debug`, `package`, `require`, coroutine library, `load`, `dofile`, `loadfile`, `pcall`, `xpcall`, or Lua GC controls. Standard base operations plus table/string/math/UTF8 are available. Removing protected calls prevents catching and endlessly retrying quota errors. Scripts must be UTF-8 source returning a callback table. Errors are reported rather than caught by mods.

Each Lua allocator is limited to 8 MiB and each entry evaluation/invocation to approximately 101000 VM instructions (hook every 1000 instructions). A long-running C library operation does not execute VM hooks; parsing, garbage collection, file I/O and host work are not hard wall-clock bounded. Aggregate mod load can still lower frame rate. There is no process isolation, hostile-filesystem race defense, hard timeout for native operations or guarantee against implementation bugs in dependencies. Package paths reject traversal and absolute paths, canonicalize within the root and reject escapes. Symlink entries are rejected during scanning; do not use junctions/links in mods. Do not mutate package trees maliciously while reading them.

Discovery inspects at most 128 sorted immediate packages (ZIPs or development folders).
Packages are limited to 512 files/directories and 64 MiB; ZIP compressed/expanded bounds
and path rules are in mod-packages.md. Lua allocator quotas exclude host asset storage,
which is bounded separately. `sdk.read_text` reads UTF-8 package data; there is no general
JSON decoder in Lua. Vehicle definitions and native animation JSON use dedicated host
parsers. Vehicles support embedded GLB models and synthesized engine audio. Arbitrary
scene/texture/font/audio-bank loading is not exposed. Text uses the existing engine font.


No camera-controller override, native audio command, customiser-profile mutation, physics force injection, collision-geometry insertion, climbing controller installation, map-selection command, score manipulation, arbitrary network messaging, cross-mod messages or persistent Lua save store is exposed in API 1. Existing character customisation, native audio integration and private climbing loaders retain their own lifetimes. Broad engine-internal access is deliberately absent. Useful current extension types include HUDs, timers, route/training challenges, native-reset navigation tools, visual world annotations, grind/bail instrumentation and constrained original body animation replacement.

The runtime pins `mlua = 0.12.1`, with `lua54`, `vendored`, `serde`, `send`; Cargo.lock pins transitive build inputs. `send` supports Bevy Resource storage, but host scheduling keeps execution on the main thread. Lua is compiled into the application using the vendored static build; no player-side DLL/Lua install is required. The dependency has no Bevy coupling and compiles against the engine's Bevy 0.18.1/local patches. Release uses Windows MSVC with `-C target-feature=+crt-static` and no default game features. Primary selection references: [mlua source/readme](https://github.com/mlua-rs/mlua), [Lua memory and hook API](https://docs.rs/mlua/0.12.1/mlua/struct.Lua.html). A lockfile entry for the optional LuaJIT source builder does not enable LuaJIT: the selected language feature is Lua 5.4.

Distribute your package directory with source, original permitted data, a license/readme and engine/API requirements. Ship no preferences, local paths, dependency cache, executable or game assets. New compatible SDK functions may be added under API 1; removing/changing existing signatures requires a new API major. Unknown callback names and manifest fields are rejected now, so adding them to an old host fails visibly. Lua/package errors are visible under the selected mod; directory/schema errors are on the list page. If a hot edit fails, fix it and save or Reload. Reload and settings changes are intentionally usable while gameplay is paused.

## Validation and build

```powershell
cargo test --locked -p skate-mods -p skate-vehicles
cargo run --locked -p skate-mods --example check_mod -- path/to/mod.zip
python tools/package_mod.py sdk/examples/native-trainer mods/native-trainer.zip
```

The checker validates package bounds, manifest/settings and Lua syntax. It does not
execute callbacks, launch gameplay, or prove native animation/handling correctness.
ZIP integration tests cover asset reads, replacement, deletion, duplicate IDs, invalid
paths and expansion limits. Validate in-game behaviour manually after the headless checks.

## Native trainer and examples

`sdk.trainer.apply(tuning)` installs one owner's reversible native tuning. Only one mod
can own it at a time. Numeric fields default to 1 on each call; `hold_fakie` defaults false.
Supported multipliers are pop, grind_pop, push_speed, push_power, braking, steering,
offboard_jump, grip, turn_power and manual_drag (0.25..4), and wobble (0..2).
`hold_fakie=true` suppresses the automatic stance switch. Unknown/nonfinite values fail.
`endless_flips=true` (default false) is **not retail**: it lets a held kickflip or heelflip keep
cycling past Skate 3's authored quad while the pop has the air for another rung, and
`endless_flip_max` (1..12, default 12) caps the extra rungs, so 12 reaches a 16x. Flat ground
still stops at a double -- the authored air-time gates are untouched. See docs/endless-tricks.md.
Disable/reload/fault restores defaults; native contact/state conditions still apply.
These are parameter multipliers, not guarantees of measured speed or height.

`sdk/examples/native-trainer` demonstrates tuning, HUD text, checkpoints, timers,
breadcrumbs, beacons and configurable shortcuts. Defaults: F5 save checkpoint, F6 return,
F7 stopwatch, F8 clear transient data, F9 beacon. Its text data stays in the package.
`sdk/examples/endless-tricks` is the smallest tuning example: it owns nothing but the two
endless-flip fields and a status line.
`sdk/examples/mario-kart` demonstrates vehicles, controller binds, driving HUD, fitted
rider animations, steering, ramp collision tuning and engine audio. Both complete
example ZIPs are committed in top-level mods/, including the prepared kart model and rider clips.

All owner-created visuals are removed on disable, reload or fault. Native skating and
vehicle physics remain host-controlled; Lua does not receive memory pointers or World.

## Multiplayer

The integrated launcher includes native multiplayer. Matching enabled mod packages
automatically replicate world cubes and vehicles. Trainer/animation/teleport results
follow native player replication; UI and input remain local. `sdk.net.info()`,
`sdk.net.publish(key,value)` and `sdk.net.read(peer,key)` expose bounded, owner-scoped
latest state for custom Lua rules. This is not an exactly-once event API.

Read [Multiplayer mod SDK](multiplayer-mods.md) before authoring shared scores,
races or world objects. It specifies exact matching, string player IDs, limits,
collision authority, cleanup, host changes and supported versus local-only features.


## Combined engine integration

The Mods menu sits alongside Updates, Teleport, Multiplayer and both character
menus. Trainer grind-pop tuning applies to the current grind launch controller;
normal settings retain native behavior. Existing map-change cleanup and crash
reporting remain enabled. Protocol 5 is required for multiplayer mod state, so
all peers must update together. The bundled trainer and kart packages are included;
private authoring sources and per-feature development launchers are excluded.
