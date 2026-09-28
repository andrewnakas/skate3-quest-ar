# .skate maps

Drag a `.skate` file onto `PLAY.bat`. Command-line equivalent:

```powershell
.\scripts\Launch.ps1 -Map 'C:\path with spaces\park.skate'
```

The original test environment remains available through PLAY.bat without a map.
Maps are parsed before initializing graphics; invalid files produce a readable
error and do not silently load the default level instead.

## Reader and adapter are different capabilities

| Data | Current support |
| --- | --- |
| SKATE01–15 records | Reader supports raw, zlib and Zstandard blocks, plus v15 compressed materials, filtered RGBA, vertex streams, delta indices, indexed collision and backward texture references. |
| SKATE15+ | Rejected with a version error. V15's additional storage transforms are not implemented. |
| Visual geometry | Authored positions, triangle winding, normals, UV0/UV1 and material groups. Packed v12+ tangent frames are reconstructed. |
| Embedded materials | Albedo, normal, ORM, emissive, alpha mask/blend and indirect lightmaps. Lightmap RGB is decoded as `encodedÂ² * 4` into half-float textures. |
| Collision | Separate authored collision mesh, reference 1 mm vertex welding, adjacent-edge and coplanar-vertex metadata. No collision inferred from visible triangles. |
| Surface types | Exact audio/physics/pattern packing from `EncodeRwSurfaceId`. Authored friction is retained; the original imported static-world contact combine remains in charge. |
| Spawn | Package position interpreted as the wheel-ground anchor, with the supplied wheel/body offset. Heading rotates about Y; coordinates stay Y-up metres. |
| Point/spot lights | Position, direction, color, intensity, range and cone settings are passed to Bevy. Renderer brightness parity is not established. |
| Sky/day-night | Metadata retained. Currently uses the package horizon as the clear color and existing game directional/ambient lighting. No full sky/celestial adapter yet. |
| Grind rails | Authored polylines and native spline words retained. Supplied game's `NoGrindEdges` path does not support grind engagement. A startup warning reports this. |
| Doors | Full records and geometry parsed. Launch rejected if present: no hinged rigid-body controller exists in the imported game. |
| NPC routes / area lights | Parsed and retained, but no runtime adapter. Startup warnings report them. |
| Retail shader definitions | Retained as original definition records. Portable material fields render; original shader families/decal channels are not recreated. |
| Native collision edge codes | Decoded with the recovered TU3 formula and convex-edge/disabled-corner flags. Native codes take precedence over generated adjacency. |
| Embedded `RWCM` collision | Schema 1 `RWCMSET1` archives take precedence over portable collision. Original triangle coordinates, surface/group IDs, sidedness and cluster order are retained, without welding. Float, unsigned 16-bit offsets with signed bases, and signed 32-bit vertices are supported. Currently supports triangle units; quad/list units report an explicit error. |
| Extensions | Decoded and retained. WMET/WCFG/BMAT report their limitations. SKYB schema 1 uses the map horizon with an explicit warning; the retail sky shader is not implemented. Other extensions reject launch because they may contain required world geometry/controllers. |
| Empty texture placeholders | Parsed, but launch requires embedded textures. |

This is a map-loading integration, not a claim of full SK8 Engine renderer or
Skate 3 gameplay parity. In particular, maps can be readable without all their
runtime features being supported. Nothing bypasses the imported game's existing
`AddRunoutAttribs` failure, documented in the main README.

## Maps supplied locally

- `format-demo.skate`: original 60 m square checker floor with an embedded
  albedo and indirect lightmap; reproducible with `python tools/make_skate_demo.py`.
- `private/transition_grind_park.skate`: existing v8 transition map, copied from
  the user's research folder. Contains 510 visual / 398 collision triangles.
- `private/skate_parity_grid.skate`: existing v8 textured floor, copied from
  the same folder. Contains 2 visual / 2 collision triangles.

The user's v14 `blender_bake_showcase.skate` also parses (15,072 visual / 1,028
collision triangles), but contains a hinged door and is not a supported playable
map yet. It has not been modified to remove that feature.

### New San Vanelona embedded collision fix

`Skate_2_New_San_Vanelona.skate` is v12 and includes `WMET`, `SKYB`, and
`RWCM`. Its former `.spawn-collision.rwcmset` sidecar is already embedded:
101,642,678 decoded bytes, 792 meshes, 21,503 clusters and 4,628,459 triangles.
The portable block has 4,615,663 triangles and is not substituted for that archive.
No separate sidecar is needed; external sidecar discovery is not implemented.

The adapter builds query ranges from the native clusters. Conservative bounds
filter wheel, camera, foot, trajectory and primitive-contact candidates while
preserving triangle order, narrow-phase tests and solver behavior. The native
serialized KD-tree is not executed; host cluster bounds provide the acceleration.

Validation on 2026-09-06: the actual package builds its complete collision
world and a spawn query finds supporting ground. The rebuilt staged executable
rendered the map and skater and completed 175 physics ticks with 25 contacts.
Local evidence: `logs/san-van-collision-test.log`, `logs/san-van-startup.png`
and `logs/san-van-startup.input.txt`. This is a startup check, not a complete
gameplay/performance or physics-parity verdict. Retail sky and grind engagement
remain the separate limitations listed above.

Edge decoding was checked against `ClusteredMesh::GetUnitVolumes` at
`82AC8A68` and its PPC instruction listing. Each low-five-bit exponent decodes
as `1 - f32::from_bits(0x411de9e7) / (8 << exponent)` with separate f32 divide
and subtract. Bits 5 and 6 map to convex edges and disabled vertices;
bit 7 is not published as a triangle feature flag. Evidence is retained in
`logs/edge-decode.json` and `logs/edge-evidence.json`; the latter records the
reference input hash. Reader layout also follows the extraction tool's
`retail_collision_mesh.py` and the runtime's `LoadRetailCollisionArchive`.

#### Correction: compressed vertex signedness

The first integration copied an incorrect signed-halfword interpretation from
`retail_collision_mesh.py`. Native `GetVertex` at `82AC79D0` actually uses
`vmrghh` with a zero vector (`82AC7A2C`), then signed saturating addition to
the base (`vaddsws`, `82AC7A30`). The offset is therefore **unsigned u16**;
values `0x8000..0xffff` must not become negative. Rust now follows those
operations, including conversion to f32 before multiplication by granularity.

An audit of this map found 131,616 affected compressed vertices used by
211,699 triangles. The incorrect decode put 38,311 vertices outside their
source mesh bounds; the corrected decode puts none outside (0.01 m audit
tolerance). At this map's 1 mm granularity, sign extension displaced an
affected coordinate by approximately 65.536 m, producing stretched or
misplaced collision faces. The embedded archive does not need re-extraction
for this defect. The separate Python extractor still has the signed-offset
bug and should be corrected before regenerating portable geometry.

Evidence: `logs/collision-vertex-native.json` records original instruction
bytes and the reference binary hash. `logs/collision-signedness-audit.json`
records counts and before/after coordinates. Regression coverage includes
offsets `0x7fff`, `0x8000`, `0xffff`, negative bases, and signed saturation.
The earlier startup capture only established that the map loaded; it did not
detect this geometry defect.

## Source evidence

Layout, versions and compression were checked against the user's local:

- `Skate3Research/Skate3CustomEngineLayer/tools/blender_owned_map/SKATE_FORMAT.md`
- `Skate3Research/Skate3CustomEngineLayer/owned/world/src/owned_map_package.cpp`
- `SK8R15/Source/tools/blender_owned_map/SKATE_FORMAT.md`
- `SK8R15/Source/owned/world/src/owned_map_package.cpp`

Collision surface packing, welding and adjacency reference:
`owned/world/include/skate/world/rw_collision_mesh.h` and
`owned/world/src/rw_collision_mesh.cpp`. Spawn ground-anchor reference:
`src/skate3_native_collision.cpp`, native collision placement block.
All reference files remain outside this repository and were read only.

## Nonvisual checks

```text
cargo test -p skate-data --test skate_map --locked
cargo test -p skate-game skate_world::tests --locked
cargo test -p skate-data --test retail_collision --locked
cargo test -p skate-core --lib board_world --locked
cargo run -p skate-data --example inspect_skate -- path/to/map.skate
```

Reader tests cover versions 1â€“14 and all three supported compression methods,
every truncated prefix of the v8 fixture, invalid references, non-finite input,
oversized counts, wrong byte order and unknown versions. Adapter tests check
separate collision ownership, packed surfaces, flat seam handling and embedded
lightmap decoding without creating a window. Gameplay and visual checks belong
to the user.

## Rebuilding a retail district from the owned disc (2026-09-24)

`worldDIST_<district>.big` is an ordinary **EB v3** archive — the same container the audio archives
use (`crates/skate-audio-formats/src/eb.rs`), so no external unpacker is needed. University holds
647 members: 231 `cSim_*.xsf` (simulation/collision), 232 `cPres_*.xsf`, 172 `cTex_*.xsf` and the
stream tables.

Working sequence, all in-repo (UTT is vendored at `tools/vendor/utt`, which
`prepare_university.py` wants as `--utt-root`):

1. Extract the `.big` to `<stream-dir>/DIST_University/`.
2. `prepare_university.py --stream-dir … --output … --utt-root tools/vendor/utt`
   → 1,133,649 collision triangles, 4,201 grind rails, zero unmatched textures.
3. `blender --background --python blender/import_university.py -- MANIFEST OUT.blend`
   → 8,546 objects, 1,645,617 render triangles, 2,046 textures.
4. `blender --background OUT.blend --python blender/prepare_university_owned.py -- OWNED.blend`
   → groups, and the `OW_SPAWN` marker the exporter turns into the map's spawn.
5. `SKATE_EXPORT_COMPAT14=1 blender --background OWNED.blend --python …/exporter.py -- OUT.skate --force`
6. `embed_retail_collision_archive.py OUT.skate ARCHIVE.rwcmset maps/<District>.skate`

**Collision only, without the visual half.** If all that is wanted is exact collision,
`tools/build_university_collision_manifest.py` writes the `simulation_assets` records
`build_retail_collision_archive.py` consumes, using only `skate3_streams.py` and
`retail_collision_mesh.py`. That path needs no UTT, no Blender and no textures.

### Three defects this hits

**The exporter defaults to SKATE15; the runtime wants v14.** `exporter.py:38` picks the magic from
`SKATE_EXPORT_COMPAT14`, and its own comment says "The Rust runtime requests the existing v14
storage contract". Export without that variable and the collision embedder refuses the package
("not a SKATE v12-v14"), which is the first sign the version is wrong.

**Rail names do not match what the runtime validates.** `import_hawaiian_dream.py:395` names each
curve `GRIND_{asset_id_without_0x}_{rail_index:04d}`, while
`crates/skate-game/src/grind_world/provider.rs:110` requires
`{asset_id}_{section_index}_{rail_index}` and otherwise refuses the map with "Stock rail
provenance/name mismatch". `tools/rename_university_grinds.py` renames the curves in a prepared
scene (4,201 renamed, 0 missing on University).

**The rail list and its provenance are ordered differently — still open.** The runtime pairs them
positionally (`records.iter().zip(&map.rails)`), but the exporter builds `map.rails` from
`_objects_from_collections`, and Blender's `collection.all_objects` is **name-sorted**, while the
`WMET` payload is `bpy.data.texts["SKATE3_RETAIL_MANIFEST"]` written out verbatim in the
*extraction manifest's* order. On University those differ: manifest record 0 is
`0xB43DB3E887200D32` while rails[0] is the `011DA…` rail, which sorts first. Every rail is then
checked against another rail's provenance and the map is refused. Renaming the curves does not fix
this; it predates the rename.

Reordering the manifest inside Blender (`tools/align_university_grind_manifest.py`) is the obvious
fix but is **not viable as written**: the manifest is 45 MB in a single-line text datablock, and
`Text.as_string()` / `Text.write()` on that did not complete in 40 minutes. The practical fix is
to reorder the `WMET` `grind_splines` array **inside the exported package**, to the package's own
rail order — `export_skate2_skybox._find_extension_offset` already walks and reads every rail name,
and `embed_retail_collision_archive.py` already shows how to rewrite an extension payload. Until
that is done, a rebuilt district loads its visuals and collision but is refused at rail
validation.
