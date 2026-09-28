# Native grind assist

The live acquisition and airborne board-adjustment path is now connected.
See [difficulty and integration details](grind-magnetism-difficulty.md) and
[grind families](grind-families.md).

Core trajectory leaves82D6A398/82D6A168/82D6A840 are supplied with the real spline
collection and indexed collision investigation. Accepted targets pass through
selector scoring and KnownAir into GrindAirAdjust82D712E0. Difficulty uses the
stock physics_mode80 field, not a new snapping radius.

Reference: TU3 image SHA256
f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4,
matching local Skate3CustomEngineLayer generated PPC. Static source translation
and compilation only; gameplay validation is reserved for the user.
