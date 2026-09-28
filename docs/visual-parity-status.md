# Visual parity status – 2026-09-08

Follow-up: the hair changes below were rolled back at the user's request.
See [dynamic-props-and-grime.md](dynamic-props-and-grime.md) for the current
hair rollback, grime coordinate correction and authored prop placement status.

Implemented and compiled offline; game/recomp not launched. User-run checks are
still needed for the previously reported crash, shadow silhouette and performance.
No physics, collision, gameplay or animation logic was changed.

## Authored distance fog

**Observed:** TU3 `0x828012D0` loads the fog collection at manager +180. Schema
`material_fog` offsets are colour=0, power=16, near=20, max=24, far=28. The routine
writes ramp `(1/(far-near), -near/(far-near), power)` and colour
`(colour.rgb*max, -max)`. The negative-one vector at `0x830BD4A0` is initialized by
`0x82F825F0` from float -1 at `0x8216DEE0`; its zero bytes in a static dump must
not be interpreted as the initialized value. Colour is not squared here.

The reference world shader applies `f=pow(saturate(distance*scale+bias),power)`
and `linear*(1-max*f)+colour.rgb*max*f`, before exposure. The exporter retains
both the authored inputs and these derived rows. All ten private sky metadata
files are refreshed. Regional transitions remain unimplemented.

## Exposure

**Observed:** `0x827F0D00` reads the GPU measurement, scales the weighted mean by
2.515 (`0x821A01E8`), and evaluates
`clamp(previous*(1 + damping*(target-measured)), min, max)`.
The tuning schema independently confirms offsets 120/124/128/132 for
`auto_exposure_target_luminance/min/max/damping`. The owned default collection
contains target .25, min .75, max 2.5, damping .5. `0x827F0B78` uses squared
`1-abs(x*y)` centre weighting. The comparison renderer normally pins exposure at
zone max because its resolve readbacks are disabled; that is not console adaptation.

**Adapter:** one 256-thread compute group meters a 16x16 bilinear sample grid of
linear HDR using Rec.709 luminance, native centre weighting and the native update
formula. It uses a 30 Hz time basis and caps hitch contribution at 50 ms. This is
not the native packed, resolved measurement surface or regional tuning selection.
The compute result stays on the GPU; the existing final tone pass consumes it.
There is no synchronous CPU readback and no per-material exposure update.
`SKATE_FIXED_EXPOSURE=1` retains 2.5 for comparison. Missing metadata also retains
fixed exposure. No claim is made that this meter's measured values match console.

## Water and other world materials

The material adapter now retains its shader key and loads authored VLT parameter
arrays from `private/render-parameters.json`. Additional supported families:

- 30, `water.flowing*`: two time-scrolled normal taps, mapped normal, refracted
  lightmap coordinates, player-shadow minimum clamp, sun specular, cube reflection
  and native alpha floor. Follows reference `scene_water.hlsli`'s flowing shader.
  Texture row/UV flips are accounted for in scale, scroll and refraction offsets.
- 14, `incandescent.backlituvscroll`: authored `uAnimationSpeed/vAnimationSpeed`,
  squared diffuse and material multiplier, followed by fog/exposure.
- 32, `ocean.reflection`: shader support for squared horizon colour and the authored
  height fade. This is not the animated ocean-surface shader (family 31).

Global model export also includes authored `environment.reflective_simple` water
sheets previously excluded by the foliage-only filter. Their original GUID-bound
textures and mesh positions are retained in render-only supplements. These have
no collision and never enter physics. Three maps now have global supplements.

**Ocean PCA follow-up:** `0x827905B0` initializes a 30-frame table with means at
`0x830118D8` (3 floats/frame) and weights at `0x83011A40` (24 floats/frame).
`0x82790858` selects frames at 1/30-second intervals, scales mean/weights by
1/255, and publishes the seven rows. The reference shader binding swizzles mean
X/Z/Y and weight pairs R/B/G. The build-checked `ocean_pca` extractor preserves
those tables in a private sidecar. Family 31 now reconstructs normals from both
PCA component textures and these animated rows, then evaluates Fresnel cube
reflection, squared lightmap and Ward sun specular from the native shader.
The adapter selects the authored frame by elapsed time; native phase/reset and
hitch advancement are not reproduced. It requires the supported owned mapped
image for extraction; missing/invalid PCA metadata retains the prior fallback.
It does not invent procedural waves.

**Still unported:** the distinct non-flowing `water.default`/`water.alpha` shader.
The local reference also uses an empirical fallback for this family, so its exact
shader contract needs separate recovery.

## Character and shadows

The native nearest-probe selection/cache remains intact. Displayed SH coefficients
now ease between samples using the same bounded .35-second adapter transition as
the shadow floor. All ten models retain their existing skinning and poses.

The GLB exporter now preserves the hair's raw second UV set. RX2 declaration
`0x002C23A5` is format 37 (big-endian float2), confirmed against the native
vertex decoder. The old skeleton parser incorrectly read it as four normalized
shorts. That decoder is corrected, and glTF accessor shape checks now reject
four-component data declared as VEC2. The character
sidecar references the original coverage texture; the shader samples its red
channel on that second UV, scales it by `m_params[6].x` (CAC hair c15.x), and uses
blended strand coverage. Its depth path retains strand holes with the world
30/255 threshold, an adapter choice. Existing dye/rim composition remains an
approximation: live CAC colour-register derivation is still outstanding.

The previous player-only shadow source and smooth shared shadow floor remain.
Its shared storage buffer is now 144 bytes to carry animation time and ocean PCA; neither
world materials nor their bind groups are rebuilt as these values change. Exact
console atlas/filtering and the per-frame shadow-colour controller remain pending.

## Existing maps

Freshly decoded RX2 caches refreshed 2,019 of 7,204 texture payloads across ten
private maps. Each map passes the offline reader, and its entire geometry,
collision and metadata tail is byte-identical to the installed input. Material
prefixes are preserved. Original installed maps are untouched. New refresh tooling
publishes atomically and refuses in-place input overwrite.

## Evidence and validation

PPC source and raw dump identity: base `0x82000000`, SHA-256
`F4AA113EB541BFBA03DBC108CF5AB43F58C965B20FA3B82F9C40938A0AD841C4`.
Local native renderer reference commit
`f6e0ae87fdfecbadb5c1e36c55d66a744187a3cd`, shaders `scene.hlsl`,
`scene_water.hlsli`, `scene_char.hlsli`, plus `skate3_native_render.cpp`.
Private evidence lives under `logs/renderer-work`: `exposure-evaluator.txt`,
`sub_828012D0-fog-candidate.txt`, per-map texture reports and
`visual-refresh-validation.json`. Owned data is not committed.

Offline checks cover shader composition, exporter fog boundaries, invalid refresh
rollback and existing asset regressions, plus the windowless Rust rendering tests.
These do not validate GPU execution, visual parity, frame time or the prior crash.

Final offline validation: 20 Python regression tests, six filtered Rust tests,
three render batching tests, six composed shaders and the release build passed.
All ten refreshed maps passed the offline reader and preserved geometry tails.
