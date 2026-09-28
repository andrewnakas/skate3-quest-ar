# Character customiser

Open Escape/Start, then Character customiser in the complete Windows package.
First-run setup now prepares the clothing/body library, presets, tattoos, all
owned pro/special characters and their authored lighting. These preparation
steps previously ran only in private feature-test folders.

Character additions have their own release fingerprint. Updating an already
prepared copy can build these assets from its original Xbox source without
reconverting maps. Setup publishes validated character stages and records unavailable extras.
If the base library fails, it keeps a verified previous generation or the stock skater. Existing imported models and
saved profiles are preserved; interrupted preparation can reuse its own cache.
Players do not need Python, Blender or development tools.

The runtime retains the visible skater if a library is missing or incomplete.
Returning from a native/imported character reapplies the saved outfit and binds
its visible rigs again. Native models use their own shader parameter rows and
specular textures, including torso slots absent from the default stock model.
Customiser pieces retain tattoo/hair composition and use character SH/key/rim
lighting on retail maps. Custom maps continue to use their ordinary PBR lights.

## Hair coverage and gameplay verification (V3)

Hair now uses its independent opacity texture's **red channel on TEXCOORD1**.
The owned `cac_hair_defaultVS` copies primary/secondary UVs into XY/ZW, and
`cac_hair_defaultPS` samples diffuse RGB from XY and opacity R from ZW. Output
alpha is opacity R times `i_params.x`. The old combined diffuse-alpha texture
was incorrect: on the moussed-up example only 0.29% of diffuse pixels exceed the
0.5 cutoff, versus 42.18% of the opacity red channel. That discarded most raised
hair geometry. All 31 opacity-bearing hair materials now retain separate maps.
Thin hair cards render from both sides. The colour and depth/shadow passes use
the same corrected coverage; existing PBR lighting formulas remain unchanged.

V3 uses `library-v3.json` when present, with a legacy `library.json` fallback.
For side-by-side preparation set `"library_index":"library-v3.json"` in the
private preparation config. V2's index, executable and launcher are retained.

Offline checks use the same `apply_preferences` function called by the menu,
then the real stock actor publication, processed physics input and consumers:

- Truck tightness reaches processed field 2760. At identical steering input,
  loose/default/tight produce tilt 0.08955247 / 0.07074646 / 0.062686734. The
  tight/loose ratio is the loaded native `tight_trucks_scalar`, 0.7.
- Wheel hardness reaches processed field 2764. At identical sideways travel,
  soft/default/hard produce side force -23.31247 / -15.153105 / -11.656235.
  Ratios match `physicswheels/default/SoftestWheelSideFrictionScalar` from the
  extracted collection data. Straightening torque also changes, from -1.113991
  to 1.1697853 at the endpoints. These are controlled test samples, not universal
  steering angles, grip percentages or speed ratings.
- Standard/Loose/Gonzo/Aggressive select distinct stock push clips through the
  `PROSKATER` selector: default, Hsu, Gonzales and PJ respectively. Evaluated bone
  poses differ. Stiff/Slouch/Buff add their respective authored posture poses
  and change the evaluated result. Switching natural stance changes its basis
  and switching back restores it.

The stock riding idle is shared between styles. Style differences appear in
specific authored movements (including pushes and selected tricks), and posture
is applied on eligible motion-tree construction. Wheel/truck merchandise models
are appearance choices; **Wheel hardness** and **Truck tightness** are the native
handling controls. No invented brand-dependent statistics are added.

Shader validation covers colour, depth/shadow and normal prepass paths, each
with and without secondary UVs. Tests run without a game window or GPU. The hair
fix still needs the user's visual playtest; no game process was launched here.

## Menu and controls

Right stick left/right rotates the character preview through 360 degrees. A
24% dead zone prevents stick drift; rotation accelerates with stick deflection
up to 2 radians/second. It uses real frame time while gameplay is paused and
keeps the existing automatic close-up framing. The viewing angle resets when
opening the customiser and is not saved into the character profile. V4 retains
the V3 hair and gameplay changes and uses the same prepared library.

Four main sections: **Body, Clothes, Board, Style**. Item lists are alphabetical,
fit the available panel height, and can be filtered by typing. Backspace edits a
search; Escape/B clears it before going back. Arrows/D-pad/left stick browse,
left/right or visible minus/plus buttons adjust, and Enter/A chooses. Mouse wheel
and page buttons move through lists. Back returns one level. **Done**, present
only at the root, writes the profile and resumes skating. There are no repeated
Save buttons or material submenus.

Names remove exporter terms and distinguish new/worn finishes. Board graphics,
trucks and wheels are individually named entries instead of hundreds of hidden
variants behind one model. The camera frames the selected area, and turns around
for back tattoos. Tattoo previews temporarily use the authored viewing outfit;
this does not change the saved clothes.

## Implemented controls

- Both bodies use their authored fallback outfits and retail rigs.
- Seven native skin colours, native hair colours, hairstyles and facial hair.
- Body weight/definition and all 17 individual facial controls, with native
  `0..0.5` ranges and unchanged defaults.
- Ten native face presets per body, decoded by target hash and normalized range.
- Upper/lower tattoos: 170 owned designs; left/right limbs plus chest/back
  placement for upper-body stamps. Tattoos can be removed and are saved.
- Hats, T-shirts, shirts, hoodies, jackets, sweaters, pants/shorts, shoes, socks,
  accessories, decks, trucks and wheels. Authored variants and primary clothing
  colours are selectable. Hat/hair, sleeves, inner tops, legs/socks and accessory
  removal dependencies are resolved together.
- All 37 gestures in each of the four direction slots, regular/goofy stance,
  Standard/Loose/Gonzo/Aggressive skating styles, and native posture options.
- Truck tightness and wheel hardness retain `0..1`, step `0.1`, default `0.7`.

Physical/animation preferences apply to the running actor. Posture and skating
style are consumed by the appropriate native motion construction when skating
resumes; selecting them does not fabricate a separate preview animation.

## Immediate appearance updates

The Python converter is no longer invoked by the game. Preparation creates 480
resident part GLBs and shared textures. Visible choices also prefetch their
resolved sleeve, leg, hair and skin dependencies. Selection changes visibility
and material handles as one transaction after dependencies are ready. Morph
weights update directly on the GPU. Existing parts stay visible during initial
asset loading. First-time GPU/texture loading can still take frames; no measured
zero-latency or frame-rate claim is made without gameplay testing.

Each visible rig is bound to the existing stock pose. Shared joint entities are
bound once. The untouched original character remains in use until the customiser
is opened or a valid saved profile is found. The profile lives at
`settings/character.json` beside the test assets and is written through a temporary
file and rename. Legacy profiles without a gender field are interpreted as male.
Map restarts reuse the same asset/settings root and reload the saved profile.

## Asset preparation and validation

Retail payloads, catalogues, converted parts and machine paths remain private.
The source archive and installed stock assets are reused; no downloads, Blender,
ISO prompts, or map reconversion are needed. The non-geometric Misc model is the
stamp catalogue, not a missing renderable part. The female face's extra XML
material group is retained in metadata; its high-LOD source has one drawable mesh.

Preparation commands, using a private configuration with the existing source and
asset roots:

```text
python -m tools.asset_pipeline.customisation_library <private-worker.json>
python -m tools.asset_pipeline.customisation_profiles <private-customisation-directory>
```

The old worker remains an offline compatibility tool. It is not the runtime.

Validation: 21 Python tests and eight Rust customiser tests pass. Owned-data checks
cover all 480 GLBs, indices, skin weights, 22 GPU target slots, texture references,
both default outfits, all selectable model/material combinations against those
outfits, presets, search, colour selection, JSON round trips, and temporary tattoo
outfits. Shader composition is parsed and validated offline with Bevy's shader
cache, with and without secondary UVs; this creates no window or GPU device.
Release compilation also passes. Interactive layout, controller feel and visual
appearance still require the user's game test.

This is a sandbox catalogue, with owned items available irrespective of career
unlock state. The runtime now shares its retail character lighting with the
customiser; full multipass retail parity and every native colour-zone effect
remain outside this adapter's implementation.

## Additional native evidence for V2

- `GetCACSettings`, `0x82590B50`, including its complete tail: regular/goofy,
  four gesture indices, posture, and style selectors Standard/Loose/Gonzo/Aggressive.
- `ResetGestureSet`, `0x824FA730`: marks all 37 gesture entries available and
  initializes the first four in table order.
- `InitializeSkaterAnim`, `0x82B97E38`: stance basis and high flag bits. Changing
  natural stance preserves the actor's relative stance.
- `cac_presets`: 32-byte MorphPreset records, target hash at byte 8, normalized
  float at byte 24; invert `0x8253CD18` normalization using native target ranges.
- `cacstamp_skin_defaultVS`: transforms TEXCOORD1 with the two i_customGraphic
  rows. `cacstamp_skin_defaultPS`: multiplies skin albedo by
  `1 + (decal.rgb - 1) * decal.a`, before lighting. The V2 extension adds this
  composition and retains Bevy's existing PBR/post-lighting entry point.
- RX2 format `0x002C23A5` is Xenos format 37, FLOAT2, not SHORT4N. The secondary
  UV decoder now reads two big-endian float32 values. Retail rectangle order is
  top/bottom/left/right; conversion to downward texture V is corroborated by
  artwork pixel bounds and exposed-limb UV regions. Left limb positions also
  agree with the authored LeftArm bind positions.

## Native evidence

Addresses below are virtual addresses in the inspected image whose dump base
is `0x82000000`; subtract that base for a dump-relative offset. Symbol names
from the prior Ghidra export are leads, corroborated with instructions and
authored data. The original IDA database was opened read-only; symbol import
and exploratory disassembly used a private copy.

Source identities:

- Owned `default.xex` SHA-256:
  `1db39496585c521d17a2137804f42cf73ebed2b32cac166ec42dbf772f4dcf7f`.
- TU3 raw image SHA-256:
  `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
- CAC XML SHA-256:
  `be2f23ab6d2f29b5ac5e143133788e72e1bebd073adedb343effd1fd7e94f4f1`.
- `skatercollections.vlt` SHA-256:
  `3b7dbd062bb1c906a085514355afff35cfa22f486ae70820c5ad1a42a7aab25b`.
- The character archive identity is recorded in the existing fallback manifest.

Observed native behaviors:

| Consumer | Evidence and implication |
| --- | --- |
| `FrontEndState_CaC::ReceiveInput`, `0x825FC7E8` | Cases 47/48 adjust recipe fields `+0x2004/+0x2008` by signed `0.1`, clamped to `[0,1]`. Constants: `0x820641A8=0.1`, `0x82165A10=0`, `0x8231A844=1`, `0x8216DEE0=-1`. These are truck tightness/wheel hardness preferences, distinct from merchandise IDs. |
| `GetIntegerValue`, `0x825FFB18` | Cases 47/48 read those same floats and convert to display ticks with the scale at `0x821963E4=10` and rounding offset `0x8209975C=0.5`. |
| `CASMorphParams` initialization, `0x8253CA60` | Uses the `cac_morph_params` layout's target order at `+4`, target prefix at `+0`, and parameter triples at `+12/+16/+20` or `+24/+28/+32`. Each target's working record is 28 bytes. |
| Morph normalization, `0x8253CD18` | Clamps to that target's minimum/maximum then computes `(value-min)/(max-min)`. The UI's normalized value is not the mesh's direct delta weight. |
| `GetTargetOrder`, `0x8253CF50` | Matches target-name prefixes against 19 entries; clothing `fat_*`/`thin_*` must follow the corresponding native mapping. |
| `CASPlayerInitializer::InitializePlayer`, `0x8253D470` | Copies each working record's default at `+8` into the 19-value customisation container; chooses default parts through `CACPartDB`. |
| `CASAssembler::Correct`, `0x8253F648` | Coordinates hair, rostral, torso, organ, legs/socks, other accessories, removals, colours and stamp information. XML flags are inputs to this sequence, not sufficient proof of a replacement assembly algorithm. |
| `CorrectRemovals`, `0x8253FD40` | Walks the native slots, resolves component names in comma-separated removal strings, and clears the referenced slots. |
| `CorrectRostral`, `0x825406E0`; `CorrectTorso`, `0x82540828` | Further select material/part combinations from native colour and clothing relationships. |
| `GetCACSettings`, `0x82590B50` | Reads saved stance, four gestures and posture-related profile fields. Existing Rust posture consumers already preserve the pending-tree application semantics. |

Observed `cac_morph_params` values in this bank are `min=0`, `max=0.5` for
all 19 targets. Face defaults are `0.25`; fat and thin defaults are zero.
Target order is **fat=0, thin=1**. The existing fallback parser's descriptive
`skinniness/fatness` names must not be used to infer that ordering merely
because both default values happen to be zero. Nose height/length have swapped
target-order and UI-zone indices; the export retains both.

Do not equate low-level constructors with gameplay defaults.
`RecipeData` constructor `0x82DDE700` initializes equipment values to zero,
while the current Rust actor profile uses `0.7`, following its documented
actor settings path. That current behavior is unchanged here. Saved/default
CAC profile initialization must be traced before changing it.

## Integration with map switching and checkpoint recovery

The customiser plugin coexists with the in-process map menu and session-marker
HUD. Controller navigation is sampled before menu input. Profile preferences
are applied after map-transition publication and before gameplay input, so a
fresh skater receives the saved stance, style and equipment settings before its
first physical tick. Character entities and the resident library remain alive
across map changes. Native checkpoint stance requests remain deferred through
the teleport graph; profile edits do not replace that recovery path.

The combined release compiles. Runtime/visual checks remain user-run.
