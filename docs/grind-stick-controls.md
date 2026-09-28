# Grind stick publication

The controller listener omitted the four grind intents consumed by the imported
ActionGraph and MotionGraph. Neutral defaults therefore reached the existing
grind twist, translation, stability and nose/tail physics controls regardless of
stick movement.

`input::grind_intentions::produce` now translates the TU3 listener emissions from
`Fill825999F0`:

| Intent | Derived controller value | Native emission region |
| --- | --- | --- |
| GrindBalanceX | minus left X | 8259ACF0..8259AD0C |
| PhysGrindTranslation | clamp(left X + right X, -1, 1) | 8259AD40..8259AD5C |
| PhysGrindStabilityNudge | left X | 8259AD5C..8259AD78 |
| PhysGrindUpDown | right Y | 8259AD78..8259AD94 |

Derived input words 7, 9 and 10 carry those axes. The listener saves right Y at
stack offset 88 before reloading it into f21 for publication. Neither an extra
dead zone nor a grinding-state gate is added here; the native listener emits
these records whenever their individual value is nonzero. The action-intent map
is rebuilt each tick, so releasing a stick removes its record.

The stock ActionGraph forwards the physics intents to the MotionGraph and applies
its own mirror filter to GrindBalanceX in the grinding state. The MotionGraph
attaches the physics attributes and feeds GrindControlFade. Existing consumers
include boardslide translation, tip-slide stability nudge, and the 82D89F58 truck
control feeding 82D40290 deck orientation. Nose/tail response depends on the grind
family and current contact, rather than applying an unrestricted rotation to all
grinds. No physical tuning changes are part of this patch.

Evidence: generated PPC from Skate3CustomEngineLayer and the imported stock graph
assets. Executable image `default_82000000_011B0000.bin`, base 82000000, SHA256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
The sum clamp constants at 8231A844 and 8216DEE0 are +1 and -1. Existing truck
orientation constants at 822F91E4 and 8208ECD8 were checked as -0.68 and +0.68.

Validation is compilation and source tracing only. In-game feel, transitions and
stance combinations remain for the user's visual testing; this change does not
establish complete behavioral parity of the surrounding grind implementation.
