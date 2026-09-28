# Mod SDK

**Start here: [AGENTS.md](AGENTS.md).** Give this file to your coding agent when asking
it to make a mod. It links the package format, complete API, examples and validation.

Editable sources: `examples/`. Player-installable output: project-root `mods/*.zip`.
Use `tools/package_mod.py` to validate and package. The current integrated game launcher
is `PLAY-ENDLESS-TRICKS.bat`; older feature-specific launchers are historical builds.

## Multiplayer

Use matching enabled ZIPs and the same integrated build on all peers. World objects
and vehicles synchronize through the SDK; Lua shared rules use `sdk.net`.
See [Multiplayer mod SDK](../docs/multiplayer-mods.md) for ownership, collisions,
late joins and shared-state examples.
