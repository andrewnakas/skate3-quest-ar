# Custom model library

Open the pause menu, choose **Custom models**, then **Import model...**. The native file picker accepts Mixamo FBX and self-contained GLB characters. Successful imports generate textured thumbnail cards and equip automatically. Click any saved card to swap, or **Use stock skater** to restore the regular customiser. Selection persists between sessions. The three-column grid scrolls vertically with the mouse wheel. Keyboard: arrow keys and Enter; controller: D-pad and A; Escape/B returns to the pause menu. Keyboard/controller selection scrolls into view. The Show filter switches between all models, native pros, native specials, and imports.

The game runs `support/skate3setup.exe --character-import` as a hidden child process; its file picker remains visible. The Windows release bundles the importer, Python, NumPy, Pillow, Tcl/Tk, FBX2glTF and its Visual C++ runtime inside that executable. No Blender, Python installation, development tools or import-time download is required. The game passes its active installation's stock skater reference explicitly. Conversion instructions and restrictions are in `mixamo-to-skate.md`.

The existing updater already hashes and replaces the setup executable alongside the game. This also delivers the importer to older releases whose updater only knows those original program files. No separate helper tree needs installing or updating. Importer-only changes do not change owned-disc extractor fingerprints or require map conversion. User library entries, selection, settings and optional legacy `support/custom-models/calibration.json` are not updater targets. That calibration remains optional and is never included in public packages.

The library lives in `%LOCALAPPDATA%/Skate3RustEngine/custom-characters`. Each content-addressed entry contains `manifest.json`, `character.glb`, `source.glb`, `preview.png`, and a conversion report when converted. Imports publish through an atomic directory rename, so failed imports never appear as completed entries. Saved assets do not depend on the original FBX location. Import failures/cancellation preserve the existing character.

Fresh setup also prepares the 41 pro/special definitions supplied by the owned
retail character table, including the special/cheat-code models. These native
entries live in this copy's prepared character generation, not the personal
import library. The menu combines both sources and prefers the current native
entry when a legacy native entry has the same ID. Existing saved selections
continue to resolve; no personal library entries are removed or overwritten.
The fuller database decoder uses the disc's schema report to recover roster
names, instead of feeding hashed gameplay-only names into the roster builder.

The game loads each candidate as a hidden scene, waits for its dependencies and scene instance, validates its stock animation bone bindings, and then commits the visibility/binding switch. The same player entity, physics, animation evaluator, camera and replay state remain in use. Stock modular scene visibility is restored when leaving the custom character. Converted characters include the reference board; the stock board customisation returns with the stock character. Custom selections are local visuals and are not transferred to multiplayer peers.

The paired calibration reproduces the supplied stock character's bind geometry; arbitrary humanoids use the converter's rig fitting and may need adjustment for unusual proportions. This is not a promise of original weight recovery or identical deformation on every model.

Build the complete package with `scripts/Build-Release.ps1`. It obtains the pinned, checksum-verified FBX2glTF binary and stages the redistributable runtime from the build machine's Visual Studio installation. Packaging fails unless the frozen helper can import a procedural character, generate its thumbnail and report invalid FBX input without changing existing library data. Checks run without the game or a graphical window.

Manual release checks: open Custom models and import a supported skinned Mixamo FBX; verify the picker opens, the thumbnail appears and the character equips. Repeat with a supported self-contained GLB. Cancel the picker and try an unsupported model; the current character and saved cards should remain. Restart and check selection persistence. Switch to a native pro/special and back to the stock skater; verify animation, board, outfit customisation and contacts. Finally update an older portable copy and repeat import while checking existing models and settings remain. These visual/gameplay checks must be performed by the player.

Verification: procedural Python conversion/library tests, Rust library discovery and candidate binding tests, release compilation, packaged importer conversion of the supplied FBX, thumbnail inspection, and independent Khronos glTF validation. No game launch or gameplay testing was performed; menu rendering and live visual swapping still need user playtesting.
