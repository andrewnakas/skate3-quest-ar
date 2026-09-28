# Retail renderer

The world material adapter follows Alex's native renderer at
[`f6e0ae8`](https://github.com/mchughalex/skate3recomp/tree/f6e0ae87fdfecbadb5c1e36c55d66a744187a3cd/src/native/shaders).
The reference files are `scene.hlsl`, `scene_common.hlsli`,
`scene_frame_cb.hlsli`, and `hdr.hlsl`. This is a Bevy implementation of
those shading equations, not a port of the game hooks or the whole renderer.

Implemented:

- World families 1–13: diffuse/lightmap squaring, the family lighting terms,
  decal art, macro overlay, normal/detail lighting and specular masks.
- Reflective-family cube sampling with all six faces and a complete mip chain.
- The shared HDR tone curve, including environmentdiffuse's reduced curve.
  The final output accounts for Bevy's sRGB conversion instead of applying
  gamma twice. The HUD is drawn after this pass.
- Alpha-tested world materials use the reference's 30/255 threshold in both
  colour and depth passes. Foliage is two-sided.
- Lightmaps remain bilinear, clamped and fixed at mip zero. Repeating world
  textures have mipmaps.
- All ten installed worlds resolve sky geometry/texture paths through their
  authored `world` collection inheritance. The parks inherit the default
  panorama; Downtown and Industrial override it. Sky X/Z follow the camera;
  Y comes from the selected `render_locations` chain, not camera height.
- The sky uses its material's diffuse and specular GUID bindings. The latter
  is the 512x16 radial sun gradient: sine-angle lookup, squared RGB divided
  by saturated alpha + 0.01, then the material multiplier and scene exposure.
  `material_sky/default.m_params[0]` supplies scale 0.75 and multiplier 0.35;
  these are decoded inputs, not constants copied from one frame. This follows
  `scene.hlsl`'s sky branch and `skate3_native_scene.cpp`'s sky-bank layout.
- The selected render location supplies the normalized sun vector used by
  the sky and existing world tangent-sign terms. This adds no directional
  light energy or shadow map to imported worlds.

The converter reads scalar material channels at record offset 0x10.
Earlier exports incorrectly read their string pointer and saved empty strings.
Reconvert maps to obtain those constants and complete reflection cubes.
Old packages still load; missing constants disable the relevant layer and
single-face environment textures are not treated as cubes.

The VLT converter now adds `array` to array fields, retaining the existing
`type` and `data` unchanged. `array.items` contains complete big-endian element
hex strings without padding; `capacity`, `element_size` and `alignment`
describe the source layout. Text arrays additionally retain decoded
`text_items` before the binary bank is discarded. The layout follows
VaultLib's `VLTArrayType`: four big-endian u16 header lanes followed by
individually aligned elements. Alignment in the schema is a base-two exponent.

Skies can be refreshed independently of map/character conversion:

```text
python -m tools.asset_pipeline.sky --game-root OWNED_EXTRACTED_GAME --assets PRIVATE_ASSET_ROOT
```

Use a private staging asset directory for experiments. This reads only the
database pair and sky resources; it does not require an ISO or Blender.
Legacy sky metadata retains its previous multiplier and has no sun gradient.

## Treeline investigation

The missing forest in the supplied University / Super Ultra Mega Park
comparison is not resolved by this change. The camera-relative
`WorldPresEntityOptimesh` anchor from `sub_82792900` is consistent with the sky
placement above, but the decoded University panorama has mountains and city
scenery, not a dense forest layer. Its RX2 contains one 380-triangle dome.
The separate `DIST_MegaPark` archive contains 102 mesh parts and no tree mesh;
it is not a source for additional trees at University's park.

The main University export retains 479 model assets, 8,546 mesh parts and 86
named TreeWall parts. Those named walls are outside the immediate park area;
their absence at that location does not prove geometry is missing. Nearby
proxy geometry demonstrably overlaps the main trees: in cell 350/-750,
451 of 456 proxy long-tree triangles match triangles in the 912-triangle
main tree after quantization to one millimetre, independent of winding.
The five nonmatching triangles are not evidence for an extra forest.
The main tree's diffuse and transparent channels resolve the same DXT5
texture, with nonconstant alpha, and its baked lightmap is retained. The
renderer already uses the retail cutoff in colour/depth and two-sided foliage.

No proxy geometry has been added. `skate3_draw_distance.cpp` documents the
ProxyWorld/full-detail exchange; rendering both whole sets is not a valid
fix. Identifying the specific missing silhouette now requires matched-camera
reference evidence connecting it to a submitted mesh/material or a streaming
collection. The two supplied images have different viewpoints. Native mip
selection and regional/streaming state also remain unverified.

## Remaining differences

The current pass enables authored distance fog, GPU exposure adaptation, flowing
water, scrolling emissive signs, the global reflective water sheets and additional
hair strand coverage. See [visual-parity-status.md](visual-parity-status.md) for
provenance, validation and the distinction between native equations and adapters.

This is not pixel parity. Native regional environment selection is not ported;
static map selection supplies the fog/sun and the default exposure tuning. The
exposure meter uses portable HDR luminance samples instead of the native resolve
surface. Shadow colour/filtering and CAC hair colour-register composition still
have documented approximations. Ocean PCA wave reconstruction now uses the owned 30-frame table; its native
phase/reset controller and the separate non-flowing water shader remain unported. SSAO, SSR, bloom and volumetric lighting
are also outside the currently implemented renderer. Missing parameters retain
fallback materials; these effects are not silently labelled complete.

The shaders are embedded in the executable. Sky and map payloads are extracted
from the user's game and must never be committed or bundled with releases.

### Treeline investigation and diagnostic (2026-09-08)

The user's mega-park screenshots refer to the bowl/cliffs in University, not
DIST_MegaPark (the separate create-a-park venue). A read-only decode of the installed
University SKATE14 package found 8,546 materials, 1,645,617 render triangles,
1,180 tree-family mesh materials and all 86 TreeWall meshes from the source cache.
TreeWall diffuse texture 0x2c70170a00053a88 retains its RGBA cutout, and the tree
materials retain masked alpha and two-sided flags. A TOC audit of all 479 full
models found no extra vertex or face buffers omitted by the parser.

Proxy comparisons must resolve mesh material GUIDs with
`_bind_material_groups_by_guid`; `mdl_parser.Mesh.name` is in material-table order
and can label proxy terrain as trees. With GUID bindings, the 14 proxy foliage
meshes intersecting x=200..550, z=-850..-600 contain 3,878 distinct triangles;
3,813 match full geometry after unordered-vertex quantization to 1 mm. The small
remainder does not establish absent trees (different triangulation/rounding also
changes these keys). DMO_University and DMO_Global contain movable props, not a
second background forest. These checks do not prove visual parity or a fix.

For a user-run discriminating capture, `SKATE_DEBUG_FOLIAGE=1` renders tree-wall
cards cyan and other tree-family surfaces magenta, opaque and unlit. Geometry,
camera, depth testing and occlusion remain authored. This bypasses texture alpha
in both colour and depth passes without adding proxy geometry. Return to the
reported viewpoint: solid foliage appearing in the missing area implicates the
normal shading/alpha path; an empty area requires checking placement, camera or
source selection. Unset the variable to restore normal rendering. This diagnostic
has been prepared without launching the game.

### Missing global tree-wall backdrop resolved (2026-09-08)

The user-run opaque-foliage diagnostic left the same empty background. The
missing surfaces are in `world/models/DIST_Water_University.rx2`, not in
`DIST_University` or its proxy stream. Despite its name, this global model has
both an ocean plane and a separate tree material: mesh 1 contains 1,572 vertices
and 786 tree-wall triangles. World fields `951898F6C0FA6856` and
`CA5A157A65E75934` select this model and its texture resource. The global
Industrial model likewise contains 716 foliage triangles.

`tools.asset_pipeline.backdrop` follows those world references and exports only
the tree-family presentation meshes into `private/native-backdrops/<map>.skate`.
Mesh-to-material and material-to-texture selection use binary GUIDs: texture
name suffixes differ from the global texture resource GUIDs. Original positions,
indices, UVs, masked alpha, two-sided rendering and baked lightmaps are retained.
At that stage only tree-family meshes were selected. The later visual pass also
retains the authored global reflective water sheets; no synthetic placements are added.

The renderer loads this supplemental package after the district and before sky
setup, through the existing retail mesh/material path. `parse_render_only` is
an explicit data-reader entry point for presentation packages; ordinary playable
map loading still requires collision. Presentation loading rejects collision,
rails, doors, lights, routes and extensions other than WMET. This does not alter
the playable map's collision or gameplay state. Future installation runs prepare
the supplement automatically; existing assets can be updated with:

```powershell
python -m tools.asset_pipeline.backdrop --game-root <owned-game> --assets <assets-root>
```

Static verification: the University supplement contains exactly the source's
786 triangles and matching vertex positions/indices/UVs. Zero of these triangles
match the 86 existing district TreeWall meshes after unordered-vertex 1 mm
quantization. Both University and Industrial supplements pass the runtime data
reader's offline inspector. Synthetic GUID tests and map-reader regression tests
pass. The executable was compiled but not launched; visual confirmation remains
for the user. The normal launcher clears SKATE_DEBUG_FOLIAGE.

For integration with staged map switching, prepare this supplement alongside the
district, before applying sky lighting. Track its entities and assets in the
same map lifetime. The current adapter is in `retail_backdrop.rs`.

### Black texture/lightmap bands (2026-09-08)

The University bark rings and mega-park shark logo expose an exporter defect in
the shared Xbox texture decoder. `XGAddress2DTiledX/Y` returns coordinates in the
padded physical tile. `_untile360` flattened `y * logical_width + x` before
rejecting columns outside the logical width. Padding columns consequently aliased
onto later valid rows, overwriting colour blocks with padding or other tile data.
The NumPy implementation reproduced the scalar bug, including last-write wins.
Both implementations now reject `x >= logical_width` before writing.

The logo's diffuse is `0x2c70170a001d01bb`; its 64x64 DXT1 lightmap is
`0x8b068e5c4f70dd7a`. Re-decoding that lightmap removes the black stripes while
the logo diffuse remains unchanged. Nine lightmaps bound to the spring/fall-tree
bark diffuse textures `0x00007e2903e3870a` and `0x00007dc403e3870a` have the same
corruption. The shared decoder affects colour textures as well: the 64x128 palm
fringe texture `0x00007e3103e3870a` also loses artificial horizontal bands.
This establishes an asset-decoding cause independently of shader lighting.

A separate colour-palette error was corrected in both decoders: DXT3/DXT5 colour
blocks must retain four interpolated colours even when their endpoints are in
ascending order. The old code reused DXT1's three-colour/black branch and merely
made that black opaque. DXT1's transparent palette remains intact. See Microsoft's
[block compression reference](https://learn.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-block-compression)
for the format layouts and separate BC2/BC3 alpha blocks.

The private University repair re-decodes the original RX2 sources and changes
723 of 2,046 texture payloads: 720 DXT1 (logical widths 16/32/64), two DXT5 and
one A8R8G8B8. It preserves the exporter's dedicated B5G6R5 decoder output. Every
original payload was checked against its source cache before replacement;
material definitions and the entire geometry/collision/metadata tail remain
byte-identical. The repaired package passes the offline SKATE reader.

Regression tests cover narrow/wide tile layouts at 2/4/8/16 bytes per unit,
DXT1 transparency, and DXT3/DXT5 reversed colour endpoints with independent alpha,
using both scalar and NumPy paths. All 16 asset-pipeline tests pass. The game was
not launched; visual confirmation remains user-run. Existing exported maps need
their textures regenerated from RX2, since changing the decoder cannot repair
already-baked PNG/RGBA cache files. Fresh conversions use the fix across maps.

Player-shadow stability follow-up: the ambient floor is now a smoothed shared
16-byte GPU buffer, updated without rebuilding world materials. The player-only
source uses one 24 m cascade and reduced receiver bias. Native colour/filter
parity and runtime visual/performance verification remain outstanding; see the
character lighting investigation. The private launcher now captures crash output.
