<p align="center">
  <img src="docs/images/skating-crab.png" alt="Rust crab riding a skateboard" width="480">
</p>

# Skate 3 AR for Meta Quest 3 (alpha)

A skater in your real room in passthrough AR on Quest 3: ride your floor, tables and furniture,
and build a park of ramps, rails, ledges, quarter pipes and stairs with the Touch controllers.
Ollies and flip tricks, grinds and slides, manuals, grabs, body flips, spins and big airs.

**[Download the latest APK](https://github.com/andrewnakas/skate3-quest-ar/releases/latest)**
and install it with `adb install -r skate3ar.apk` (or SideQuest). Open **Skate 3 AR** from the
headset's Library > Unknown Sources.

**It plays straight away with no game files.** The default skater is our own clean-room
engine: original physics, model, animations and CC0 sounds, with nothing taken from EA.

**Own Skate 3 for Xbox 360? Import it for the full retail tier** (retail skater, animations,
controls and sounds), converted from your own disc. Four steps, one Windows PC:

1. Run the [Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine/releases/tag/experimental)
   once on your ISO and skate for a minute.
2. Put `MAKE-QUEST-ZIP.bat` + `make-bundle.ps1` (from the release) next to `skate3rust.exe`
   and double-click the `.bat`: it writes `Skate3-Quest.zip`.
3. Copy the zip to the Quest's `Download/Skate3` folder.
4. In the app: menu > Skate 3 data > Import, then reopen the app.

Step-by-step with pictures of where everything is: **[docs/IMPORT-SKATE3.md](docs/IMPORT-SKATE3.md)**.
No EA assets are in this repo or the APK.

### Controls

| Touch | |
|---|---|
| Right stick | Hold down to crouch, flick up to ollie; flick directions for flip tricks |
| Left stick | Steer; in the air, spin; pushed back to a diagonal: powerslide |
| A / X | Push / mongo push (tap a light push, hold a full one) |
| B / Y | Brake / step off the board |
| Triggers (air) | Grab; **grab + left stick forward/back: front flip / back flip** (add a spin for flip 360s, 540s) |
| Right stick on a rail | Up/down: nosegrind / 5-0; right out to a side and held: turns into crooked, salad, feeble, smith, boardslide, lipslide |
| Left menu (tap / hold 1 s) | Menu / build mode (A places, B deletes, right stick turns and scales the piece) |

More in [scripts/quest/README.md](scripts/quest/README.md) and the in-app How to play page.

**Handing off to Snap Spectacles?** Start with [docs/HANDOFF.md](docs/HANDOFF.md): how this
maps onto the C++ port [skatecore-cpp](https://github.com/andrewnakas/skatecore-cpp).

# Skate 3 Rust Engine

A Rust and Bevy skating project built from Skate 3 reverse-engineering research.
Includes skating, tricks, grinds, offboard movement, difficulty settings and
`.skate` map support. Gameplay parity is still a work in progress.

## Mods and agent authoring

Drop mod ZIPs into top-level `mods/` and enable them in the Mods menu. **Point your coding agent at [sdk/AGENTS.md](sdk/AGENTS.md)**
to make a mod. See [package structure](docs/mod-packages.md), [Lua API](docs/lua-modding.md),
[vehicle API](docs/vehicle-sdk.md), [multiplayer mod SDK](docs/multiplayer-mods.md) and [Mixamo workflow](docs/mixamo-vehicle-workflow.md).
Editable examples live in `sdk/examples/`; `tools/package_mod.py` validates and packages them.
[Endless Tricks](docs/endless-tricks.md) is a bundled mod that extends the flip ladder past
retail's quad; run it with `PLAY-ENDLESS-TRICKS.bat`.

## Play

[Download Experimental](https://github.com/SK8-ENGINE/skate-3-rust-engine/releases/tag/experimental).
Successful `main` builds replace this prerelease. Choose **Latest** in Updates
for experimental updates; **Stable** is the default.

Extract the Windows release ZIP and run `skate3rust.exe`. Select your Skate 3
Xbox 360 ISO, or select `default.xex` in an extracted game folder. Keep its
`data` folder alongside it. Setup prepares the skater, animations and all disc maps, then
starts University. The original scoring and session-marker HUD assets are also
exported automatically during setup. No Blender, Python or Rust installation is needed.
ISO extraction needs internet access. The first conversion can take a while.

Use an XInput controller to play. Escape opens graphics, difficulty and map
settings. Maps can be switched without restarting the game.

**Skate 3 assets are not included.** Your converted files stay in
the `data` folder beside your executable. Each freshly unpacked copy runs its
own setup; it does not adopt another installation. In-place updates refresh
only changed asset groups.

## Build

Requires Windows, Rust with the MSVC toolchain, and LLVM installed in its default
location. Run `BUILD.bat` to build, then `PLAY.bat` to launch the test world.
`PLAY.bat` opens your saved map (University by default); use the in-game menu to switch maps, or drag a `.skate` file onto `PLAY.bat`. An XInput controller is required for gameplay;
Escape opens difficulty and graphics settings.

Development builds use a prepared asset set in `assets/private/` or the
installed asset directory. `scripts/Build-Release.ps1` builds the portable Windows
package and requires Python 3.13. GitHub Actions builds `main` automatically;
numbered releases are published separately.

Custom animations and climbing support remain available, but no custom clips
are shipped. The included format-demo map is original procedural content.

Implementation notes are in [`docs/`](docs/). Patched Bevy dependencies and
their licenses are in [`vendor/`](vendor/). This is an unofficial project,
not affiliated with EA.

## Advanced diagnostics

Windows builds support opt-in [performance timeline capture](docs/performance-tracing.md)
through the `--trace` CLI option, including optional GPU pass diagnostics.

## License

GPL-3.0, the same as the upstream
[Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine).
Clean skate sounds are CC0 (credits in `crates/skate-game/clean-audio/CREDITS.json`).
