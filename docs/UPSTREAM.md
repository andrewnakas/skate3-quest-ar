# Upstream engine snapshot

This repository started as a snapshot of the Skate 3 Rust engine working tree:

- Source worktree: `C:\Users\andre\.codex\worktrees\b0ad\Sk8EngineAudio`
- Branch `audio/retail-exact-player-sound`, commit `1415b5d570cd6e4363a04ac99dd958568a5d317e`
- Plus the uncommitted working-tree changes at 2026-09-27 (the signed-off scoring
  work: 25 modified tracked files and the endless-tricks additions).
- Excluded: `target/ bin/ logs/ .local/ settings/ data/`, private maps and all
  game assets (see `.gitignore`), `maps/University-collisiononly.skate.bak`,
  `sdk/examples/bike.glb.before-unspin`.

## Pulling later engine changes

Engine and port histories are unrelated, so carry changes as patches:

```sh
cd <engine worktree>
git diff 1415b5d <new-commit> -- crates vendor tools > /tmp/engine.patch
cd <this repo>
git apply --3way /tmp/engine.patch
```

Update the commit recorded above after each sync.
