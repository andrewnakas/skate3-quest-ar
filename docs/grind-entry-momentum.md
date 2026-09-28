# Airborne grind entry momentum

Native source: TU3 `default_82000000_011B0000.bin`, base82000000,
SHA256 f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4.
Static instruction extracts are retained in logs/biped-82D86F00.txt,
logs/biped-82D872B8.txt and logs/biped-82D40AF8.txt.

Observed in source: airborne entry82D86F00 calculates candidate80 from
Processed608 (air/reckoning velocity), sets candidate412 bit30, and retains
the velocity difference for impact admission82D872B8. Native state update
82D40AF8 consumes that flag through82C04168, writing all seven board bodies,
and supplies Skeleton16112/16416 prediction before Ground continuation.
This branch skips ordinary grind forces for that tick.

The previous host omitted this handoff. KnownAir exit restores board velocity
only for ordinary Ground100, so entering a grind had no equivalent velocity
restoration. Losing approach momentum to the landing contact is the resulting
plausible explanation for the user's immediate-stop report; gameplay outcome
is not measured by this source inspection.

The host now retains and consumes the airborne entry velocity once. Across-rail
correction fractions are native family values: 0/3=.4,1/5=.2,2/4=.8. Along-rail
momentum is retained from the airborne source. The native steep-entry, delta
velocity and horizontal-impact gates feed the existing wipeout-request owner.
The stock near-grind impact threshold is physics_wipeout offset256 (8m/s),
not an invented speed cap. All families now investigate their actual support
surface, including 50-50, for the impact side test.

Ground-category entry's separate slope-graph blend is not added in this change.
Existing friction parameters are unchanged. GRIND_ENTRY passively records the
board, airborne and applied velocities during user play. Build only; no tests
or scripted gameplay. User visual confirmation remains necessary.
