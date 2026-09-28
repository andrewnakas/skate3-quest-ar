# Original HUD installation

The runtime reads both caches from the selected `--assets` directory:

| HUD | Default manifest | Optional development override |
| --- | --- | --- |
| Trick scoring | `private/hud/runtime/trickdisplay.json` | `SKATE_SCORING_HUD_ROOT` |
| Session marker | `private/session-marker/hud.json` | `SKATE3_SESSION_MARKER_OVERLAY` |

Merging HUD code does not install ignored, extracted game assets. Earlier feature
launchers used private caches through environment overrides; the combined build
with only `--assets` could not find either cache. Setup now prepares both with
the existing extractors. The marker extractor defaults to the vendored UI toolkit;
the external `--ui-toolkit` argument remains supported.

The original installer calls `prepare_runtime_huds.py` after VLT conversion.
It exports only the scoring movie and marker subtree needed by the runtime,
validates both, and installs their manifests and referenced RGBA payloads.
Intermediate extraction files remain in the existing conversion workspace and
are removed by the installer's normal cleanup. No full-disc or audio extraction
stage is added, and the original map/skater/environment setup flow is retained.

For an existing installation, reuse prepared caches without reconverting maps:

```powershell
python tools/install_prepared_hud.py --assets <installed-assets> --scoring-cache <prepared-scoring-cache> --marker-cache <prepared-marker-overlay>
```

This validates both manifests and referenced texture sizes before copying any
files, checks available marker texture hashes, and verifies byte-identical copies.
It refuses differing installed files unless `--replace` is supplied. It installs
only the runtime files, preserving authored geometry, fonts, images and actions.
No copyrighted assets are committed or bundled with the source.

If no prepared caches exist, prepare only the two HUDs from the owned game folder:

```powershell
python tools/prepare_hud.py --game <owned-game> --output <installed-assets>/private/hud --collections <installed-assets>/private/stock/skater-collections.json
python tools/extract_session_marker.py --game <owned-game> --output <installed-assets>/private/session-marker
```

Startup logs report the loaded paths or a load failure. Check the executable and
asset paths in the launcher as well: `PLAY.bat` delegates to `scripts/Launch.ps1`, which
selects `bin/skate3rust.exe`, not a feature build elsewhere in `bin`.

Session-marker prompts appear while holding LB during gameplay; they are not an
always-visible panel. Scoring follows the original trickdisplay movie. Its camera
renders layer 31 offscreen before the presentation UI; the marker camera renders
layer 29 at order 1 after presentation. The existing scoring startup dependency
on `PresentationSetup` must be retained. Neither HUD uses the world mesh layers.

The installation repair and release compilation do not establish GPU visibility
or gameplay parity. Manual checks remain: resume gameplay, hold LB for the marker
panel, and perform and land a trick for the scoring display.
