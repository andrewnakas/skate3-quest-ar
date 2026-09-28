# Difficulty selection and parity boundary

Escape offers Easy, Normal and Hardcore. Changes select the native physics-mode
index for the next simulation tick, without resetting bodies, the current
animation/trick, input history, equipment preferences or the camera. The choice
saves to `settings/gameplay.json`. Missing settings default to Easy. For a test
launch, `--difficulty easy|normal|hardcore` overrides the saved choice.

## Data and code evidence

The runtime already contains converted stock collection data in the private
`assets/private/stock/skater-collections.json`. This change uses its inherited
`physics_mode/easy`, `normal` and `hardcore` records through the existing typed
reader. It does not substitute hand-tuned force, jump or gravity multipliers.
The field values below are rounded for readability; the runtime reads the
original float bits.

| Stock field | Easy | Normal | Hardcore |
| --- | ---: | ---: | ---: |
| MaxPushDVStart / MaxPushDVEnd | 1.5 / 1.5 | 0.6 / 0.5 | 0.5 / 0.25 |
| JumpMinHeight / JumpMaxHeight | 1.71 / 1.71 | 1.33 / 1.71 | 1.21 / 1.71 |
| JumpHeightOverrideEnabled | true | true | false |
| MaxAutoBodySpinSpeed | 8 | 8 | 0.5 |
| EasyBodySpins / PerfectBodyFlips | true / true | false / false | false / false |
| UnintentionalPumpScalar | 1 | 0.7 | 0 |
| AutoPushEnabled | true | false | false |
| GrindLockDist | 1.35 | 0.9 | 0.15 |

These are inputs to the recovered calculations, not promises of a particular
measured jump height or speed. Source collection SHA-256 identities:

- `physics_mode/easy.xml`: `4ee489b21c0e8f70616d0bf42f39a32604f3bec6ee5932671be7f9e5b9d6711c`
- `physics_mode/normal.xml`: `6054c99fd49265da916e647574566f8242adbf4f1d3e42bce2d3f8039ba25344`
- `physics_mode/hardcore.xml`: `44008a66b6f275a21e067ee46bf53be2927379904c97bfc5572ebc71f647aa68`

Native reference: the user's Skate3Research TU3 memory image
`research/reverse-engineering/XexDump/default_82000000_011B0000.bin`,
SHA-256 `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
Addresses here are Xbox guest addresses, image base `0x82000000`.
The research generated instruction translation is supporting evidence; no
gameplay code or constants were copied from the older clone implementation.

- Player constructor `82DB18A0`, variant setup `82DB1B08..82DB1B78`: native order
  Easy=0, Normal=1, Hardcore=2, Motorized=3, Test=4. Only the first three are
  exposed to the player.
- Actor packet publication `825937EC` and PlayerInput `82DB4048`: carry the
  selected index into processed input `+2528` and collection reference `+2548`.
- SurfacePhysics constructor `82D85D08`: five surface collections. The existing
  native selector normalizes surface types according to mode field `+60`.
  Ground force settings now follow that processed selection instead of always
  reading the startup `smooth` collection.
- Existing gesture recognizer uses its native Hardcore branch for strength
  response. It now receives the same selection as the physical/animation packet.

## Connected runtime settings

Previously, some systems already indexed all five native tables, while ground
forces, jump-attribute override and auto-push retained their startup values.
Now ground settings are preloaded for all five native modes and all five
surfaces. Selection is an immutable shared handle, with no file parsing per
tick. Jump-override selection happens after PlayerInput chooses the mode and
before Skeleton processes animation attributes. Auto-push reads that same
processed mode. Pumping, ground jumps, air spins/flips, wipeout checks and
trajectory grind-lock distance already consume indexed tables.

## What is not yet full Skate 3 parity

This is mode selection for the implemented native systems, not a claim that the
entire game now matches every retail mode. The current world adapter still has
`NoGrindEdges`, and physical grind-state updates remain incomplete. Changing
GrindLockDist does not make those missing controllers work.

Six otherwise unused mode fields lie in the grind-state portion of the native
layout: `+84/+88`, `+92/+96`, `+100/+104`. Direct instruction inspection finds
interpolating consumers at `82D42720`, `82D40D60`, and `82D41CE8`, respectively,
using processed scalar `+2624`. Nearby native diagnostic strings identify grind
states. These consumers are observed in the binary; assigning new gameplay
semantics to the hashed field names would require more evidence. They have not
been replaced by invented assists or declared implemented merely because their
data can be read.

No matched retail replay was run for this change, so complete behavioral parity
is not established. Remaining unsupported game states are still reported by the
existing runtime rather than simulated by substitute behavior.

## Validation

All 70 game tests passed with private fixtures enabled. The private integration
test switches all three modes during production ticks,
checks that mode selection preserves bodies and pose history, and compares all
15 player-mode/surface combinations with the exact stock push, friction and
wobble values. Legacy Normal-mode fixtures now initialize both physics and
skater as Normal instead of mixing an Easy packet with Normal cached settings.

Run the complete game suite with private assets:

```powershell
$env:SKATE3_ASSET_ROOT = Join-Path $PWD 'assets'
$env:SKATE3_MOTION_INVENTORY = Join-Path $PWD 'logs/motion-inventory.csv'
$env:SKATE_MAP_TEST_PATH = Join-Path $PWD 'maps/Skate_2_New_San_Vanelona.skate'
cargo test -p skate-game --locked -- --include-ignored
```
