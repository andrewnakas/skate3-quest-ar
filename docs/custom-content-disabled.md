# Local custom content disabled

The current private custom assets are parked under their original directory,
`assets/private/custom`, with `.disabled` appended to their filenames:

- `crouch-treflip.json.disabled`
- `climbing.json.disabled`
- `360flip.json.disabled`
- `riding.json.disabled`

The animation replacement loader only reads `crouch-treflip.json`; its existing
missing-file behavior uses stock clips. The climbing loader only reads
`climbing.json`; without it, both climbing attachment and the approach pose
overlay return without taking control. The other two files were not referenced
by the current Rust loaders, but are parked with the rest of the custom content.

No custom-content support code was removed. To reactivate an asset, restore its
original `.json` filename and restart the game. These private files are ignored
by Git; this document records the local configuration change. The practice
block remains ordinary world geometry.
