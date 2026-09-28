# Native session markers

With the original HUD overlay prepared, hold **LB**, press **D-pad Down** to place;
hold **LB + D-pad Up** to return. These are the original `input.cfg` bindings
(`SessionMarkerSet`, `SessionMarkerUse`), including pressed versus held behavior.
An unset marker cannot return. Releasing cancels the hold. A completed hold does
not repeatedly teleport until released. Automatic bail recovery retains its own
checkpoint. Successful map changes clear the manual marker; pause and replay
cancel an active return and require releasing LB before accepting another one.

The branch includes map-transition baseline `4a92ad1`, cherry-picked as `1d89d85`.
No game, recomp, controller harness or GPU gameplay validation was launched for
this task. Gameplay validation remains manual.

## Original evidence

Addresses are TU3 virtual addresses in the owned `default.xex` image, with the
local disassembly image based at `0x82000000`. Local generated PPC and the
community symbol map corroborate function boundaries. No game binary, extracted
art, original shader bytecode, fonts or private research files are committed.

SHA-256 provenance:

```text
Owned default.xex: 1db39496585c521d17a2137804f42cf73ebed2b32cac166ec42dbf772f4dcf7f
TU3 disassembly image: f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4
HUD-corrected private manual executable: 169d6258cb2a62b4e6caa0ad54788ab66f2d9587231a9d056b9dc984169040d4
```

| Native owner | Behavior represented |
| --- | --- |
| `82898FC8` PlayerUI::UpdateSessionMarker | Actions42/43; set/return ordering; retained timer and release latch |
| `82DB6EC0`, `82BE1AE8` | Skeleton output+416 is animation-to-world translation, used for distance to the saved marker |
| `825DF618` FrontEndManager::RenderButton | Full texture-sized quad, minimum corner (-24,-24), rather than the authored 32px placeholder |
| `825D6B68`, `82CA1FD8` | Futura Shadow draws black first; original futuraheavy draws text color at local X+1 |
| `8289B6D8`, `8289B140` | Saved transform, foot-forward and on-board byte; marker initially unset |
| `8289B928`, `8289BAE0` | Ground/biped eligibility; wheel count, deck Up.Y threshold, excluded states |
| `82BFBC18` | Downward10m line from Y+.1; VLT slope/drop/clearance; surface switch |
| `82591E30` | Deck+.2 capture; normalized travel velocity above squared speed.25; cross-product threshold.9 |
| `82592B68`, `82B97388`, `82B970D8` | Foot-forward is natural/relative stance equality, inverted by fakie |
| `82592C08`, `82B97350`, `82B97308`, `82B972A8` | Foot-forward setter and orientation/mirror/relative-stance publication |
| `828977A8` | Flag69, pending wipeout teleport and state702 return gates |
| `825926F8`, `82DB8998` | Actor-reset packet and ordinary board/skeleton/velocity reset; camera cut publication |
| `827A9C60`, `827AAF10`, `827AB790` | `cMsgTeleportEffectAmount` (`FAF37802`) publication |
| `827EDE58` | Progress to `f_NoiseFade` (`F0F1D438`), `(1-p,p,0,0)`; random UV scroll and channel weights |
| `827ED7E8`, `82A8AF10` |64x64 four-channel binary noise texture, native six-word generator |

Return duration is.2s through100m,1s from1000m, and a native fused linear ramp
between them (`3A690453`, `3DE38E39`). The timer adds `3C888889` per UI tick.
The host budgets those ticks from real UI time, once per frame, and consumes
them alongside controller input; physics substeps no longer supply the clock.
Distance uses the animation-root translation, not the board+.2 capture position.
At 100m, 550m and 1000m the strict comparison fires on ticks 13, 36 and 61.
Relocation uses a strict greater-than comparison. Within.5m there is no effect
or relocation; the native timer still accumulates. The trigger tick and next two
UI ticks publish full effect. The shader's recovered noise contribution uses
the original10x5.625 and11x6.625 UV scales and channel interpolation, rather than
a generic random-pixel or scanline effect.

The validation collection is `Hash_12B64C0E804B0853/default`, with fields
`Hash_ADD032CACF6A1C15` (slope.75), `Hash_8ABE098D3806D273` (drop1m),
`Hash_C0526C883AF0ECCA` (sweep length.4), and
`Hash_CEB092E418A5B001` (radius.5). Values load from installed VLT data.
The CTR branch chain at `82BFBE30`, including its zero-case fallthrough, rejects
surface categories5,6,9,12,13; category8 is allowed by this geometry function.

## Original HUD extraction and future integration

`tools/extract_session_marker.py` uses the existing private UI toolkit. Example:

```powershell
python tools/extract_session_marker.py --game OWNER_GAME_DIRECTORY --ui-toolkit UI_TOOLKIT_DIRECTORY --output .local/session-marker/overlay
```

The toolkit directory contains the `skate3_ui_extract` Python package. Its
existing APT/GEO/RX2/font readers perform extraction; this adapter compiles only
the marker subtree. Re-running updates its source cache incrementally.

Sources: `hud2/hudphonelist`, imported `controls/button_item2`, the Xbox360
`button_DPad_Up_hud.Texture`, `button_DPad_Down_hud.Texture` and `button_B_hud.Texture`,
original Futura Shadow and futuraheavy atlases/metrics and English language labels.
The compiler emits fifteen triangle meshes and seven private RGBA textures.
`futuraheavy` declares its bitmap font family as `Futura Std Medium`.
The root `hudintro` endpoint14,
quick-menu `maximized` endpoint49 and three-item label endpoint18 supply the actual
placements. No replacement typeface or redrawn panel is used. Source texture
and extraction-manifest hashes remain in the private compiled manifest.
Textures copy the extracted RGBA payload directly except for shadow coverage.
Following the retail/current comparison screenshots, the host linear-light
compositor uses shadow alpha `1-(1-a)^2.2` and panel opacity 0.75. These are
**screenshot-calibrated presentation adjustments**, not recovered native
constants or proof of native gamma equivalence. The shadow's original atlas
footprint and RGB, foreground font, icon bytes and all geometry are preserved.
Both source and output texture hashes are recorded in the private manifest.
Object Dropper is shown as
unavailable; its 0.3 opacity is a host presentation choice, not recovered
ActionScript behavior. This change does not add Object Dropper functionality.

Runtime accepts `SKATE3_SESSION_MARKER_OVERLAY`, falling back to
`ASSET_ROOT/private/session-marker`. This is the narrow interface for a future
shared HUD extraction/setup owner: emit `hud.json` schema1 and its referenced
RGBA files there. This task does not install another whole frontend pipeline.
The launcher uses `.local/session-marker/assets.path`, or `SKATE3_ASSETS`.

Audio integration emits `FrontEndSoundRequest(u64)` with the original GlobalFEPlaySound
IDs: place `0D6C88A3B91C828F`, rejected place `66B3AFE3B602918C`, return
`7F135F9FD28F7F21`. The player-audio worker consumes them: each id resolves through the vault's
`fe` class (`cellphone_place_marker` / `cellphone_marker_error` / `cellphone_goto_marker`) to a
sample and gain, and plays out of `sk8_menu.bnk` on the Splice one-shot path
(`player_audio/frontend.rs`).

## Verification and limits

Release compilation passed with the existing static-CRT Windows target flags:

```text
cargo build --release --locked --target x86_64-pc-windows-msvc -p skate-game --no-default-features
```

When sharing a Cargo target directory across worktrees, link the final executable
to a checkout-specific output path and verify that artifact before staging it.
A previous build copied a stale executable from the shared cache.

Four standalone hold-state tests passed (cancellation/latching, unset/nearby/
blocked returns and tail, distance ramp, actual distance-dependent trigger ticks).
The WGSL previously passed offline Naga27 parsing
and validation. Original HUD manifest counts, triangle structure and every RGBA
payload size passed data checks. These checks initialize no game or GPU device.
A CPU-only rendering of the compiled HUD was inspected for font layering,
icon size and the three-row panel. The corrected release was rebuilt and staged
with artifact/hash checks.
The subsequent shadow/panel adjustment changes only the extracted HUD overlay;
the same executable reads it on launch. Offline checks verified all output
hashes, unchanged foreground/icon bytes, unchanged shadow footprint and
increased shadow coverage. In-game comparison remains manual.

Manual validation remains necessary: place/replace, return from riding and
biped states, switch/fakie stance, interrupted holds, bail recovery, camera cuts,
replay/pause and successful/failed map changes. No gameplay result is claimed.

This is not a claim of complete frontend/render parity. The native region
membership service (`82BFBB48`), challenge/online mode restrictions and disabled
row ActionScript alpha behavior have no corresponding owner here yet. The HUD
uses the recovered timeline endpoints, not a general ActionScript runtime.
The noise stream starts from the original static generator words but is isolated
from unrelated original renderer callers, so its exact pixel sequence differs.
The compositor implements the marker noise contribution; general TV/fisheye and
vignette passes are outside this feature. Native render scheduling/color-space
equivalence and stance/reset timing still need manual observation. Map marker
invalidation is a deliberate host lifecycle rule, not a recovered serialization
claim. These gaps are explicit integration work, not synthetic substitutes.

## Bail-recovery integration

Manual marker returns and automatic bail checkpoints share Actor82592B68's
leading-foot query and the deferred Actor82592C08/82B97350 stance request.
The teleport motion graph applies the saved stance after ResetSkaterAnimation;
manual return no longer edits animation flags immediately before that reset.
Marker placement and automatic checkpoint history remain separately owned.
