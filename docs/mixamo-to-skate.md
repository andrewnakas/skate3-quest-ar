# Mixamo to Skate converter

This offline converter produces a GLB for the existing stock-pose renderer. It
does not install assets, implement a menu library, change physics, or launch the
game. Export a standard Mixamo humanoid **with skin**, preferably in T-pose.
Use owned/local assets as the stock reference. Blender is not required.

## Player workflow

In the Windows game package, use **Custom models → Import model...**. The
importer and FBX2glTF are embedded in `support/skate3setup.exe`; the game
provides the current copy's prepared stock reference. Successful imports become
saved thumbnail cards and equip automatically. No additional software is needed.

Output publication is transactional: failure leaves
existing characters and source files unchanged. No assets are uploaded.

## Source use

Install the Python requirements into your own tool environment. Obtain
FBX2glTF 0.9.7 from its official release and put the Windows executable in
`tools/mixamo_to_skate/tools/FBX2glTF.exe`, or pass `--fbx-tool` explicitly.

```text
python tools/mixamo_to_skate/main.py model.fbx --reference path/to/private/skater.glb --noninteractive
python tools/mixamo_to_skate/main.py model.glb --reference path/to/private/skater.glb --no-board --noninteractive
```

The input GLB option is for an already normalized Mixamo conversion, not an
arbitrary rig. Arguments are passed directly to FBX2glTF without a shell.
Embedded FBX media extraction goes to a disposable temporary directory.
The in-game library also accepts an already converted stock-rig GLB when it
passes validation against the active installation's stock reference.

The complete game release is built with `scripts/Build-Release.ps1`, including
a frozen importer smoke check. The following optional standalone developer
package is separate from the in-game release path:

```text
python -m PyInstaller --onedir --console --name "Mixamo to Skate" --paths tools/mixamo_to_skate --distpath <output>/app --workpath <output>/build --specpath <output>/build tools/mixamo_to_skate/main.py
```

Keep the resulting executable and `_internal` directory together. Add
`tools/FBX2glTF.exe` next to the executable. The official 0.9.7 Windows binary
used here has SHA-256
`8d90fb5e0a8d186a3d9a7ff8c75eaee541c3975ce4df0d80351f20092ae0877f`.
Copy dependency license files into the package. A local private calibration
may be placed beside the executable; it is excluded from redistributable builds.

## What conversion means

This is **stock-proportion fitting**, not runtime motion retargeting. The
converter reads bind matrices, estimates the character's body axes and scale,
fits its bind geometry, and remaps/merges weights. The output uses the owned
stock hierarchy, stock bone axes and inverse binds (with negligible floating
point affine-row residue cleared). Local transforms are exported as TRS after
removing tiny stock matrix roundoff shear, retaining the authored inverse binds. All output
primitives share one skin, including the optional stock board. The engine
therefore retains its existing physics, contacts, motion and camera behavior.

Mixamo Spine/Spine1/Spine2 map to stock SPINE/SPINE1/SPINE3; the unused stock
SPINE2 and NECK1 remain in the hierarchy. Missing target finger/end bones fold
into their nearest mapped ancestor. Missing toe joints are permitted. No
individual finger animation is created. Foot/hand contact quality and fitted
proportions require visual review; success is a structural validation result,
not a guarantee that every skating motion looks correct.

Generic fitting aligns limb directions and positions to the stock rig. An
optional private `calibration.json` (legacy in-game location:
`support/custom-models/calibration.json`, or explicit CLI `--profile`) improves fitting
using the user's round-trip pair: the original stock GLB and a Mixamo-rigged
copy of the same neutral geometry, normalized to GLB by FBX2glTF. Generate it
with `--calibrate --profile calibration.json` and the normalized source GLB.
It is locked to the reference SHA-256. It contains derived bind frames, not
meshes. Do not bundle private calibration or stock assets in public releases.
The converter works without it, with an explicit generic-fit warning.

## Limits

- Standard Mixamo body chain required; duplicate names and unrelated rigs fail.
- Stock-sized output: original unusual proportions are not preserved reliably.
- No automatic rigging, animation export, facial rigging, cloth or hair physics.
- Morph targets are rejected rather than silently lost. Source clips are omitted.
- Supports triangles, normals, UV0/UV1, vertex colours, embedded PNG/JPEG and
  basic glTF materials. Texture/material extensions and external GLB resources
  are rejected. FBX conversion diagnostics are retained in the report.
- Up to eight source influences; merge then retain four. More than 10% lost
  weight on any vertex is an error. Any smaller loss is reported.
- Up to 512 MiB input and one million output character vertices. These are
  import limits, not recommended game budgets.
- The board is copied from the local stock reference by default so the existing
  combined-character presentation does not lose it. `--no-board` emits only the
  character for a future modular importer. Stock outfit/profile integration and
  gameplay installation are separate work.

## Dependencies and verification

FBX parsing uses Facebook's FBX2glTF 0.9.7 (BSD-3-Clause project, Autodesk FBX
SDK dependency). The Python conversion uses NumPy. Packaged Python builds use
PyInstaller and Tcl/Tk for the file chooser. Retain dependency notices when
packaging; no game content belongs in the redistributable tool folder.

Primary sources:
- https://github.com/facebookincubator/FBX2glTF/releases/tag/v0.9.7
- https://github.com/facebookincubator/FBX2glTF
- https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html

Run `python -m unittest discover -s tools/mixamo_to_skate -p "test_*.py"`.
Tests use procedural fixtures, not retail models. Engine compilation and game
launches are not necessary for this standalone converter. Rendered gameplay
validation remains a later user-run step.

Validation during development: nine procedural checks cover paired geometry,
unit/axis changes, hand-weight collapse and physical-pose skinning, generic
fitting, invalid joints, failed-output cleanup, existing-output protection,
reference mismatch, missing bones, morph rejection and truncated input. The
owned round-trip FBX was also converted and checked with Khronos glTF Validator
2.0.0-dev.3.10: zero errors; stock board normal maps retain the original
engine-generated tangent-space warnings. No gameplay tests were run.
