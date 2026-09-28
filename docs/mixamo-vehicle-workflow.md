# Mixamo to native vehicle animations

This is the workflow used for the working Mario Kart rider. The game plays a native
Skate skeleton pose on the currently selected compatible character. It does not swap
the rider to the mesh bundled in the Mixamo download. Other characters must first be
converted to the supported native render rig; arbitrary skeletons are not interchangeable.
Different proportions may still need contact adjustments to reach a wheel or pedals.

## 1. Keep a matched calibration pair

Use your owned stock `assets/private/skater.glb` as the native reference. Upload the
same stock character mesh to Mixamo, rig it there, and retain its returned rest/T-pose
with skin and skeleton. Convert that returned reference FBX to GLB without altering
its bind pose (Blender import FBX, export glTF/GLB with skin). This paired stock return
is the calibration source, not a Mario mesh or a posed entry/exit frame.

The converter's `--calibrate` path accepts a GLB. It verifies mesh bounds and records
the reference hash, source hash, scale and Mixamo bind transforms. Example:

```powershell
python tools/mixamo_to_skate/main.py returned-stock.glb --reference assets/private/skater.glb --profile calibration.json --calibrate --noninteractive
```

Keep calibration.json with that exact stock reference. For this local kart, the matched
profile is `your converter’s calibration.json`. Do not replace it
with a profile built from another character. Calibration transfers the change from a
Mixamo bind pose into the original native bind basis; copying raw rotations ignores
bone axes and is what produced the forward shoulders in the earlier attempt.

## 2. Download the animations from the same Mixamo character

Select Entering Car and Exiting Car on that same uploaded stock character. Download FBX
**with skin** for this pipeline, retaining its skeleton. The skin is useful for verifying
the reference and is not used as the in-game rider. Use consistent frame rate and no
keyframe reduction. Preserve root motion; these clips need to move between standing
and seated positions. Keep the original downloads untouched.

The accepted local inputs were `Entering Car (2).fbx` and `Exiting Car (2).fbx` in Downloads.
The earlier `(1)` files used another character and are not the calibrated source.

## 3. Verify the raw transfer before fitting a vehicle

Run Blender in background mode with the stock GLB and matched profile. Use fresh output
directories: the tools refuse to overwrite existing preview folders.

```powershell
& $blender -b --python tools/preview_mixamo_animation.py -- --source 'Entering Car (2).fbx' --reference assets/private/skater.glb --profile calibration.json --output .local/raw-enter
& $blender -b --python tools/preview_mixamo_animation.py -- --source 'Exiting Car (2).fbx' --reference assets/private/skater.glb --profile calibration.json --output .local/raw-exit
```

`$blender` is the path to your Blender executable. Open each `Raw-Skate-Animation.blend`.
Check the first, middle and final frames, especially shoulders, wrists, hips and feet.
The tool checks the calibration hash and source bind compatibility. Do not bypass a
mismatch by forcing rotation offsets into the native rest skeleton. Native rest bones
and skin inverse-bind matrices must remain unchanged. Fix the calibration/source first.

## 4. Fit the approved poses to the kart

```powershell
& $blender -b --python tools/fit_kart_animation_preview.py -- --raw .local/raw-enter/Raw-Skate-Animation.blend --kart sdk/examples/mario-kart/kart.glb --mode enter --output .local/fit-enter
& $blender -b --python tools/fit_kart_animation_preview.py -- --raw .local/raw-exit/Raw-Skate-Animation.blend --kart sdk/examples/mario-kart/kart.glb --mode exit --output .local/fit-exit
```

This example script aligns the seated hips, eases torso lean and solves arms/legs to
wheel and footwell targets. It bakes a separate fitted action, retains the raw action,
checks unchanged rest bones and writes a fit report. Review both .blend files before
export. Feet need clearance over the side pod, hands need a reachable wheel position,
and the last entry pose should match the first exit pose. The seated pose is also
used for the driving loop. Fingers are limited by the native character's hand rig.

Targets in this script use Blender Z-up: seat `(0, .20, .73)`, wheel grips
`(+/-.19, -.36, .90)`, feet `(+/-.17, -.55, .33)`; the preview kart is raised `.55`.
These are **kart example values**. For another vehicle adjust alignment, contact targets,
clearance and torso lean in the authoring script or bake equivalent edits in Blender.
Do not change rest bones, and keep the fitted armature object transform identity.

## 5. Export in the game's exact skeleton order

Obtain the order from the installed native bank (or `sdk.animation.info().bone_names`).
Do not use the larger reference list in the vendor importer: this build's bank has 36
bones, including reparented helpers. Generate the JSON list with the provided utility:

```powershell
cargo run --locked -p skate-data --example vehicle_bone_names -- assets/private/stock/data/anim/OnBoard.abin .local/bone-names.json
& $blender -b --python tools/export_kart_rider.py -- --enter .local/fit-enter/Kart-enter.blend --exit .local/fit-exit/Kart-exit.blend --reference assets/private/skater.glb --bone-names .local/bone-names.json --output sdk/examples/mario-kart/rider.json
```

The exporter preserves the glTF skin bind basis while converting Blender poses into
column-major native matrices. It subtracts the preview seat, exports enter/exit clips,
uses the final entry pose as drive, and generates left/right steering endpoints with
length-preserving arm reach. It renders both endpoints for inspection. The host blends
between them continuously with steering input. For another cockpit, change the exporter
seat, steering-wheel centre/axis and steering angle alongside the fitted scene targets.

The example's game seat is `[0, .18, -.20]`: convert the Blender seat to Y-up and subtract
the preview kart's .55 m lift. Model fitting offsets are not an excuse to alter native
bone axes. Export fps comes from each scene; retain its timing instead of relabelling
frames with a different rate. The output is a full native pose, not additive rotations.

## 6. Package, validate and playtest

Put `rider.json`, the embedded `kart.glb`, `vehicle.json`, `mod.json` and `main.lua` in
`sdk/examples/mario-kart` (or your own authoring folder). Set animations.file to rider.json,
enter/exit to their names, idle/drive/reverse/brake to drive for this example, and
steer_left/steer_right to the corresponding endpoints. Package it with:

```powershell
python tools/package_mod.py sdk/examples/mario-kart mods/mario-kart.zip
```

The archive has mod.json at its root. The game extracts it automatically; do not edit
mods/.cache. See mod-packages.md and vehicle-sdk.md for limits, schema and lifecycle.

The host validates names, frame counts, finite rigid transforms and clip references at
load. The source loader tests cover malformed inputs; local validation also checked the
export against the actual stock bank and matched the final entry pose to drive. The
runtime applies the same native pose to whichever compatible character is selected.

`Build.ps1` stages a build, example ZIPs and documentation. Repackage approved
source changes to update the live ZIP. The game rescans and reloads enabled changed mods;
use Rescan packages to force discovery. Existing compatible settings survive updates.
Use PLAY-MARIO-KART.bat for a manual session. Check entry/exit from multiple vanilla
stances, steering in both directions, ramps, braking, reset, Escape, and mod disable.
Also try characters with different proportions; native-rig compatibility does not
promise exact contact for every body size. Gameplay is never launched by these tools.

Source code and authoring scripts are reusable. The supplied Mario Kart model and
fitted rider.json are committed as the bundled example, including mods/mario-kart.zip.
Raw FBX/Blender authoring files and the stock skater mesh remain local and ignored.
