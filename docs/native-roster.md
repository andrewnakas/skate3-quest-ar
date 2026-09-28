# Native pro and special characters

The Custom models menu includes the 41 character definitions in the owned retail `characters_marquee` table that inherit from `pro_skaters` or `ip` and name a real recipe. It includes Isaac Clarke (`isaak` / `z_legend`), Meat Man (`steak` / `security_03`), Dem Bones, Coach Frank, Dr Pepper Mascot and the retail pros. Props, ambient pedestrians, unused recipe-only assets and absent DLC definitions are not presented as additional playable pros.

`tools/asset_pipeline/native_roster.py` extracts the original LOD0 modular RX2 meshes and recipe materials from `marquee.big`. It uses the existing retail RX2/ABIN scene exporter, retaining native vertices, weights, bone hierarchy and inverse binds. It performs no Mixamo processing, humanoid fitting, automatic weighting or animation baking. Matrix perspective residue below 1e-6 is made exactly affine for glTF compliance. Original diffuse, normal, alpha and specular textures feed the existing renderer's material representation; this does not reproduce every proprietary retail shader effect.

Entries share the persistent model library and hot-swap mechanism. The menu's **Show** filter cycles All / Pros / Specials / Imported. Native metadata is committed with the scene, so failed loads retain both the previous model and its style. Saved selection works for native and imported characters alike. Switching to the stock customiser or an imported model restores the customiser's style preferences.

## Native animation evidence

Observed in Xbox 360 TU3 / retail 1.05, input dump SHA-256 `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`, image base `0x82000000`:

`GetCACSettings`, VA `0x82590BE0..0x82590DBC` (module offsets `0x590BE0..0x590DBC`), compares these character hashes and returns the strings at VA `0x82224820..0x82224864`:

| Character | Hash | Animation selector |
|---|---|---|
| Danny Way | 42545B0938E341C9 | DannyWay |
| Mike Carroll | 146D3B7D88385267 | MikeCarroll |
| PJ Ladd | 1733A97B7064D4C2 | PJLadd |
| Jason Dill | 0DED833242F2F857 | JasonDill |
| Jerry Hsu | 1D939610FEA090EE | JerryHsu |
| Rob Dyrdek | BE3309CE9BA41097 | RobDyrdek |
| Other Marquees | fallback at 82590D88 | Aggressive |

Character hashes independently resolve against the retail VLT names. These exact values are passed through the existing `ProSkater` construction attribute and graph condition, using the retained OnBoard/OffBoard banks. The skin choice does not replace those banks. Native gestures use the retail reset defaults; CAC posture overrides are disabled for these characters. Current stance, equipment tuning and in-progress skating state remain with the player. The authored selector takes effect when the native graph constructs its next animation tree, as with existing style edits; this avoids restarting a trick during a model swap. Not every pro or special character has a unique animation set.

Private provenance and extraction reports live under `Documents/Mixamo-to-Skate/native-roster`. The source archive SHA-256 is `0e12660e22c33239d9f28a708d09b2e3f291276012c0c6296c42ae20b6b41acc`.

## Verification

All 41 exported models pass Khronos glTF validation with no errors. The native-roster test loads every GLB through Bevy, waits for image dependencies and binds every skinned scene against the retained retail animation skeleton. It also checks that the installed animation banks contain the selected ProSkater values and their child trees. These are offline asset tests; no game was launched and live gameplay/visual parity still requires user playtesting.
