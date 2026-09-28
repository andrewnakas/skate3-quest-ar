# Riding collision group restoration

## Original behavior

Reference: Skate 3 TU3 image `default_82000000_011B0000.bin`, base
`0x82000000`, SHA-256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.

Observed in the generated PPC source: Ground entry `0x82D37538` calls
SetStandard `0x82C03AF8`, which calls `0x82C091F8` (standard deck angular
drag), `0x82C090C0` (standard collision groups/materials), and `0x82C06ED8`
(volume enable flags). In `0x82C090C0`, `li r8,4` supplies the value written
to assembly+24 and each 96-byte part's +92 collision group.

The imported Ground entry already restored deck drag, but omitted the group
assignment. Offboard board release assigns group 7. Riding skeleton mode 6
restores rider parts to group 5. The single-player table from `0x82765EF0`
culls group 4 against 5, but allows group 7 against 5. Consequently a remount
could leave riding enabled while retaining board/rider self-collisions.

## Evidence and interpretation

User reported the persistent skating fault begins after getting back on the
board. Capture `logs/game-20260908-024133.stderr.log` contains nonzero deck
split-position/orientation contact reactions (for example solver tick 2178),
while the animation hook impulse is zero and the offboard hand drives are no
longer present. The preceding `023133` capture shows continued roll and
three-wheel contact despite neutral steering. These observations support
unwanted board/rider contacts as the cause; the captures did not directly
record the board collision-group value.

Ground entry now restores group 4 at the native SetStandard point. This is
a missing source assignment, not a friction, steering, or balance adjustment.
The source-level omission is confirmed; symptom resolution awaits the user's
remount gameplay check. Compilation passed with `cargo build -p skate-game
--locked`; no automated tests or gameplay tests were run.
