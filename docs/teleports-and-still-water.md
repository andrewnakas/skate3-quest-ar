# Travel menu and still-water follow-up

## Travel

Esc → Teleport lists the original English frontend location names for usable
destinations on the currently loaded map. A continuous list supports mouse-wheel
scrolling, a draggable scrollbar and keyboard selection that scrolls into view.
There are no pages or cross-map destinations in this menu. The installer
now extracts `private/teleports.json` by joining `fe_locations`, the shipped label
and English language tables, and global RX2 locator records. Names are joined by
their label hashes, not string-table order. The special skate wordmark byte is
represented as `skate.` in plain text.

The current owned installation yields 42 frontend destinations, 41 with unique
authored transforms. `Stadium - Monster Park` remains in the catalog but is hidden
from the menu because it is unavailable:
its reference `vert_skp5_01_challengelocator_01` was not found in the global
locators or the inspected mission payload. No substitute position is invented.
Frontend templates and unnamed park-layout aliases are excluded.

Same-map travel queues the existing player-input teleport/reset service and
updates the checkpoint. The `--teleport ID` startup adapter also checks that the
destination belongs to the loaded map; menu travel does not restart the session.
Authored height and orientation are preserved. Runtime arrivals and menu layout
have not been checked in a running game.

Evidence: TU3 `cGlobalFileLocationManager::LoadGlobalLocations` at `0x828A3D50`
loads `data/content/global_locators/*.rx2`. The RX2 `0xEB0009` section contains
128-byte records with the transform at +0, name pointer at +104, tag pointer at
+108 and identifier at +112. The parser validates record and string boundaries.
All 41 resolved positions have collision surfaces beneath their authored X/Z in
the exported maps; vertical offsets range from approximately -0.008 to 4.536 m.
This is an offline placement check, not a gameplay arrival test.

## Rendering

Still-water materials (`water.default`, `water.alpha`, `water.skatepark`) now
have a dedicated family. Retained shader names upgrade older map packages at
load time. The shader implements the decoded two-layer PCA normal reconstruction,
UV scrolling, refraction, four-tap lightmap sampling and literal shadow floor.
Flowing-water materials keep their existing family. Still water requires the
private ocean PCA sidecar; automatic extraction of that sidecar from a fresh
installation remains outstanding, and missing data uses the existing fallback.

The default and alpha pixel shaders have 128 bytes of literal constants before
their instructions. SHA-256 of their instruction streams:

- Default: `B5F39F3080C89A220250E87643C5AD15F4A94B42B2C3DCB6BAF090E036FE3B5A`
- Alpha: `81D9905165B28B03C3623D18887E7E9D9303D3FFDA072993B8C183D24BD3A547`

An offline register evaluator compared 600 normal/refraction cases across all 30
owned PCA frames with the implemented algebra (maximum error 2.22e-16). This
does not establish full pixel-shader or GPU visual parity. Private disassembly
and validation output are under `logs/renderer-work/shader-inspect`.

Exposure extraction now follows each map's render-location reference and its
inherited exposure settings into `private/exposure-profiles.json`. Runtime
selects the active map's profile, with the existing global fallback. The ten
current profiles resolve to the same values, so selection alone need not produce
a visible change.

## Remaining parity work

- Spatial lighting/colour transition volumes and their native controller.
- Exact colour grading: profiles reference `cc_financial.rgb`, which was not
  found in the inspected content. The available `cc_petes.rgb` is not evidence
  that this different lookup table should be applied.
- Native bloom composition, console shadow-atlas filtering and the per-frame
  shadow-colour controller.
- Fresh-install ocean PCA extraction and in-game water/teleport validation.

Movable-object work is deferred at the user's request. Hair is unchanged.

## Checks

Release build, four filtered Rust menu tests, eight Python extraction/environment
tests, and composed world/depth shader validation passed. All ten map packages
passed the offline reader; their geometry/collision tails remain byte-identical
to the installed inputs. No game or recomp was launched. Owned assets and private
inspection artifacts are not committed.


## Integration with the current engine

The Teleport option is a separate scrollable pause-menu panel alongside character
customisation, multiplayer and updates. Its list refreshes after in-process map
changes and contains only destinations in the active map. Travel uses the existing
manual teleport reply so it does not replace automatic bail checkpoints.

Scenery supplements and authored skies are prepared through the asynchronous map
asset staging path, then retired with the map. Character shadow casters use layer
28, separate from the scoring HUD on layer 31. Map changes reload character lighting
and exposure. These integration changes require manual visual checks across a map
switch, a destination teleport and the scoring/character menus.
