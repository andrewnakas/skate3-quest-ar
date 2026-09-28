# Mods

Native Trainer and Mario Kart are included here as committed, ready-to-install ZIPs.
Mario Kart includes the prepared model and fitted rider animations. Both remain
disabled by default for new users; enable them through Escape > Mods.

Drop community `.zip` packages here. PLAY-ENDLESS-TRICKS.bat loads this project-root folder;
Escape → Mods lets you enable, disable, reload and configure them. Open mods folder
returns here. ZIPs must have mod.json at their root; the game extracts them automatically.

Editable example sources live in `sdk/examples/`. Rebuild after editing:

```powershell
python tools/package_mod.py sdk/examples/native-trainer mods/native-trainer.zip
python tools/package_mod.py sdk/examples/mario-kart mods/mario-kart.zip
```

For a new mod, point your agent at **sdk/AGENTS.md**. Full structure and rules are in
**docs/mod-packages.md**; API signatures are in **sdk/skate.lua**.

Development folders are also supported. Do not install duplicate IDs (including a
folder and its ZIP). Ignore .cache: it is managed by the loader. Settings live outside
packages. Standalone executables default to a mods folder beside the executable;
SKATE3_MODS is the explicit override used by this project's launcher.

## Multiplayer

Use matching enabled ZIPs and the same integrated build on all peers. World objects
and vehicles synchronize through the SDK; Lua shared rules use `sdk.net`.
See [Multiplayer mod SDK](../docs/multiplayer-mods.md) for ownership, collisions,
late joins and shared-state examples.
