# Native grind families

The live Rust state dispatcher now admits all six TU3 grind owners. Contacts are
queried from the existing authored spline primitives; this change does not
regenerate maps or use proximity-only attachment.

| TU3 state | Contact family | Native queries and update |
| --- | --- | --- |
| 401 | 50-50 | 82C1FDC0, 82D89150, 82D41D70 |
| 400 | Boardslide/lipslide | 82C1FB98, 82D889A8, 82D41848 |
| 403 | Single truck (5-0, nosegrind, crooked/Smith/feeble variants) | 82D89898, 82D42808 |
| 402 | Nose/tail slide and blunt variants | 82C20130, 82D89428, 82D42030 |
| 404 | Backslash contact | 82D883E0, 82D429F8 |
| 405 | Inverted deck/darkslide | 82C1F968, 82D88E88, 82D41138 |

The additional controllers use the native contact geometry, support investigation,
orientation targets, pin strengths, friction-family parameters and conditioned
animation controls. The truck pitch path is also connected for 50-50s, allowing
that contact to change into a single-truck grind. Loaded-end publication318
feeds the existing processed bit9 used by the tip query on subsequent ticks.
Tipslide pop heights use stock physics_mode84/88; ordinary truck and boardslide
heights retain their separate stock fields.

## Animation variations

The chromosome producer consumes world animated-board orientation, effective
physical board orientation, physical foot directions, approach history, contact
end and rail support. It indexes the native 2 x 2 x 2 x 2 x 4 x 6 table, with
separate animation/scoring stability counters (82DEF518). This includes the
native frontside/backside and backwards variants, rather than labeling every
non-50-50 contact BOARD.

The 384 canonical entries have NUL-separated SHA256
`8fa7ccbca90f773dfcc1277aca278e6ed572d82a80e532f8b24281ca7b24d317`.
The table was cross-referenced with the existing clone research; the behavior
producers were read from TU3 generated PPC. A canonical-name index is not a
scorable-object ID, so the latter remains unassigned.

## Evidence and limits

Source: TU3 image `default_82000000_011B0000.bin`, base82000000, SHA256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`,
and the matching Skate3CustomEngineLayer generated PPC. Function-level instruction
extracts are kept locally under logs/biped-*.txt.

All six contact families and their named variations are connected for a manual
playability pass. This is not verified retail parity. Existing host limitations
remain: specialized tipslide drop-in exit handling, full native
material/engagement feedback, and the air439 history
snapshot are not completely connected. Common pop/exit behavior still supplies
those families in this adapter. The ground history supplies approach naming;
short approaches and rapid re-grinds particularly need visual comparison.

Only compilation and source review were performed, as requested. Gameplay and
animation behavior are for the user's visual test on the default map.

Trajectory-driven magnetic acquisition is now connected; see [magnetism](grind-magnetism-difficulty.md).
