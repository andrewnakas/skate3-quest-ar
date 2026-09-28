# Character lighting investigation

Status: position-dependent character lighting implemented; user-run visual validation pending.

Observed in the local reference renderer (`skate3_native_scene.cpp`, CaptureCharLighting,
and `scene_char.hlsli`): defaultcharacter/CAC shading consumes a directional key,
nine RGB spherical-harmonic rows, ambient multiplier, exposure, rim colour/direction,
key/rim specular colours and powers, material multiplier, and shadow visibility.
The character material path differs from the baked world-lightmap path. Hair uses
its own wrapped diffuse/ambient/rim response. Skin needs its dedicated specular
mask; opaque diffuse alpha is not a substitute. The player currently uses generic
StandardMaterial, with imported-world directional lighting disabled.

Observed in TU3 generated PPC source:
- 0x827E7280 reads nine Vector3 elements from field F45DFCAB6EC195CD, copies them
  into 16-byte rows at manager +16, and retains a default-probe mode byte.
- Collections with class hash 6A2B5946514B8490 have default/dlc/cas/freeskate rows
  containing this complete nine-element array. Class 1FFDC8E3ACA07C1F also has it.
- 0x827E7348 computes squared position differences against records at +160 and
  copies nine lighting vectors from a selected record. This supports a spatial
  lighting lookup, but does not yet establish the complete sampling/blending,
  registration, or fallback rule.
- `light_probes` (class 0E902F3F038A483C) contains global directional-light structs,
  ambient, and arrays of cSimpleLightData. Do not assume these are the same as
  the spatial baked SH samples just because of the class name.

Initial investigation checklist (superseded by the implementation status below):
1. Recover the spatial irradiance resource/registration and sampling path and
   resolve default probe selection from world/render state.
2. Extract character frame inputs and per-material parameters through native
   references, retaining texture/normal/spec-mask semantics.
3. Add a skinned character material adapter that evaluates native SH, key, rim,
   and highlights, updated from character position. Keep world materials and
   gameplay/physics unchanged.
4. Supply character-only shadow visibility separately from the world's disabled
   additive sun/shadow rendering; establish the native source before choosing
   its renderer implementation.
5. Validate in exposed and shaded locations and with skin/clothing/hair. No
   game launches are authorized by this investigation; prepare user launchers.

Reference caveat: community symbol labels are hypotheses. In particular the
82E3B548 label must not be treated as a verified prototype. No copied renderer
capture's lighting constants should become a universal outdoor preset.

## Implementation, 2026-09-08

`tools.asset_pipeline.irradiance` extracts RX2 type `0xEB0024` from the district's
other presentation assets. Its section starts with count and relative array
offset; each 160-byte record contains nine float4 SH rows, XYZ at byte 144 and a
runtime link word at 156. The `SHP1` sidecar preserves groups, values and original
world coordinates, converting only endianness. University contains 17,493 samples
in 64 resources. Future map conversions write `<map>.irradiance` beside the map.

TU3 `0x827E6E28` registers these resources into a 41-by-41 spatial grid;
`0x827E6AC8` selects the nearest sample in three dimensions from the XZ
neighborhood and falls back to the default coefficients. `0x827E7348` checks a
six-entry MRU cache with a strict squared-distance threshold of 6.25. Raw binary
constant `0x820BD16C` is 15.0 (query margin); `0x822F8F38` is 6.25. The evidence
is the generated PPC instructions corroborated with resource contents and the
TU3 dump with SHA-256
`F4AA113EB541BFBA03DBC108CF5AB43F58C965B20FA3B82F9C40938A0AD841C4`.
Addresses use the dump's `0x82000000` base.

`retail_irradiance.rs` retains the native nearest-sample/cache behavior. Its
static-world adaptation searches all loaded groups using expanded bounds and an
XZ 15-metre neighborhood. It does not reproduce the original 12-resource
streaming limit or grid-cell boundary quantization. The freeskate collection
supplies the nine fallback rows; live changes of the manager's default mode are
not implemented. This is not trilinear probe interpolation.

`tools.asset_pipeline.character_lighting` resolves each selected model's
AttribulatorMaterialName through VLT inheritance and exports its nine complete
material parameter rows. Skin/face specular textures preserve the original red
channel separately; the GLB's grayscale inverse-roughness conversion is not used
as the native specular mask. Source assets remain private.

`retail_character.rs` replaces the ten named retail GLB materials while retaining
Bevy's skinned vertex path. It updates the SH rows from PlayerRoot's position.
The fragment shader evaluates the native SH basis, authored directional key,
ambient multiplier, rim response, key/rim highlights, normal maps and material
multiplier. Diffuse sampling accounts for the GLB's sRGB texture format before
the retail gamma-two decode. Output remains linear HDR for the existing shared
tone pass. The separate depth shader retains alpha rejection.

A zero-lux directional shadow source provides world/self occlusion to the
character's key and key specular terms. The baked world shader does not sample
this shadow map; remaining StandardMaterials receive zero additive sunlight.
This uses Bevy cascades/filtering/biases, not the console's exact cascade layout
and 9-tap shadow filter. It does not add a skater-shadow receiver to baked world
surfaces. No gameplay or physics code changes are involved.

Hair now uses the native wrapped diffuse/ambient/fresnel response, with the
authored material colours and the existing GLB dye/coverage composition. The
live CAC colour-register derivation and separate second-UV blended strand pass
are not reconstructed; hair keeps the installed cutout mode. This is an explicit
adapter limitation, not a claim of exact hair parity.

Scene exposure remains the existing 2.5 day-capture value shared with the world;
the regional exposure controller, water and fog are still unfinished. Missing
lighting sidecars leave the existing GLB material path active and log the reason.

Validation: 13 synthetic Python extraction/inheritance/GUID tests, two spatial
selection/cache tests, and the windowless world-render adapter test pass. Character
colour, shadow and normal-depth shader variants pass offline Naga composition
against the vendored Bevy 0.18 shader modules. Release compilation passes. The
game/recomp was not launched. User visual checks should cover exposed terrain,
sheltered spots, skin/clothing highlights, hair coverage and body/board shadows.

## Player shadow receiver, 2026-09-08

The baked-world receiver is now connected. Native `scene.hlsl` evaluates
`min(lightmap * lightmap, shadow_visibility + shadow_colour)` before diffuse,
normal/detail lighting and specular. This is a component-wise minimum cap, not
an extra multiplicative darkening pass. The local native reference's caster path
is corroborated by TU3 `0x827EBEF0` (CSM setup), `0x827F82F8` (CSM render) and
the character-family shadow shader names in the executable. The animated mesh
and board are the caster geometry; no ground decal or circular blob is used.

Two zero-lux shadow sources separate the receiver contracts. The existing source
includes world and player casters, and is sampled by the character. The new
source includes only the ten skinned retail player/board pieces on reserved
render layer 31. Camera views include that layer; ordinary world geometry stays
on layer zero. World families 1–8 and 13 sample the player-only source and apply
the recovered minimum clamp. This prevents terrain/building shadows from being
applied a second time over the baked map. The source's lightmapped-diffuse flag
identifies its role in the GPU light table, so entity ordering does not determine
which shadow map the shaders select. Both lights have zero illuminance.

This reproduces the projected-mesh and lightmap-combination method, **not full
console parity**. Shadow map projection/filtering/biases still use Bevy cascades.
The original per-frame c8 shadow-colour controller has not been recovered: the
adapter currently uses the local irradiance probe's constant RGB coefficient as
the ambient shadow floor. At University's starting point that is approximately
(0.071711, 0.095925, 0.127945), read from the source sample, not a global preset.
The floor updates when the selected probe changes. Native caster exclusions,
atlas coverage encoding/blur, shadow colour and pixel matching remain separate
parity work. Unsupported world material families still have no custom receiver.

Offline validation: world, character and character depth shaders pass Naga
composition/validation; a windowless ECS regression test verifies the caster-layer
separation and zero illuminance. Release compilation passes. The prepared user
launcher retains the texture-band repairs and prior character lighting. No game
or recomp was launched, and no physics/animation implementation changed.

## Shadow stability follow-up, 2026-09-08

User testing reported thin/warped limbs in the shadow, heavy lag, a crash, and
abrupt changes in darkness. The previous implementation changed every world
material when nearest-probe selection changed. This both stepped the ambient
floor and invalidated all those material assets. It is a confirmed code defect
and a plausible source of the reported stalls; no contemporaneous crash dump or
Application error record was found, so the crash itself is not diagnosed.

World materials now bind one persistent 16-byte storage buffer. An extracted
resource supplies the shadow floor, and the render queue writes it in place
after asset preparation. Neither the storage asset nor world materials are
mutated during probe changes; existing bind groups remain valid. The floor uses
an exponential 0.35-second transition, with each frame's contribution capped at
50 ms to avoid a jump after a hitch. These are adapter settings, not recovered
native constants. The initial sample is applied directly.

The player-only source now uses one 24-metre cascade instead of four extending
to 100 metres. This removes its internal cascade transitions and three shadow
views. Its receiver normal offset is zero and depth bias is 0.002 metres; the
previous defaults were 1.8 texels and 0.02 metres. World-plus-character self
shadowing retains its original source and bias. The new settings target the
reported silhouette distortion, but visible improvement is not confirmed offline.
A single cascade trades distant coverage for stability; shadows outside its
24-metre camera range are intentionally unavailable. Exact console parity is
still pending the native colour controller and filtering work above.

Offline world/character/depth shader validation and six filtered unit tests pass,
including smooth probe transitions, hitch limiting, caster isolation and cascade
configuration. Release compilation passes. The updated private University
launcher records stdout/stderr, backtraces and the exit code in
`logs/renderer-work/player-shadow-runtime.log`. It has not been executed.


## Later visual pass

See [visual-parity-status.md](visual-parity-status.md) for the subsequent SH
transition smoothing, second-UV blended hair coverage, GPU exposure meter and
remaining shadow/CAC fidelity gaps. Earlier implementation sections are historical.

## Stock surface restoration, September 9

Returning from an imported/native model reapplies the modular stock outfit. Its
geometry-only GLBs have normal maps in the material library but no tangent
attributes, so the StandardMaterial input path silently omitted their detail.
The customiser shader also used alpha after `alpha_discard` as the clothing
specular mask; opaque coverage had already replaced that authored mask with 1.

Both original and modular retail shaders now share the derivative normal basis
and gamma-two albedo decode. Modular materials retain the source diffuse alpha
for highlights, independently of opacity and tattoos, and use the dedicated skin
specular mask where authored. Profile updates retain texture handles and apply
colour tint separately. This shared material path covers local and remote stock
outfits without changing imported PBR materials or globally lowering gloss.

Fourteen focused tests pass with the prepared library, including offline shader
compilation, stock restoration and per-profile material preservation. No game was
launched; visual verification of stock -> imported/pro -> stock remains manual.

## Pro hair family classification, September 9

Benny Fairfax's prepared `Retail_Hair` uses `character.default_hair`; other pro
hair also uses `character.default_hair_ropa`. The renderer previously recognised
only the `character.hair` prefix, so those pro materials entered clothing shading
and received bright, untinted specular highlights. A shared material-family check
now recognises all four authored hair names for local/native and modular/remote
materials. The existing hair response and original textures are preserved.

Fifteen focused tests pass, including a native hair binding regression and checks
that skin/clothing are not classified as hair. No asset conversion is required;
Benny's appearance still needs the user-run visual check.

Stock-hair follow-up: all 31 modular stock hair materials use `character.hair`
or `character.hair_ropa`; the original stock material also uses `character.hair`.
The prepared-library regression passes a colour change/restore for every stock
hair material, preserving hair shading, authored tint, normal/diffuse/opacity
textures, specular masks and roughness. This rules out the same family-selection
and tested profile-cache failure, not every possible visual lighting issue.
No runtime change or new executable was needed for this audit.
