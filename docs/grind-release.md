# Grind release at low speed and rail ends

TU3 reference: default_82000000_011B0000.bin, base82000000,
SHA256 f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4.
Static generated instruction extracts are under logs/biped-<address>.txt.

Observed: PreUpdate82D3F4E8 owns a latched leaving phase. Decision82D40DA0
enters it after more than60 state updates when Processed2652 speed is below
0.15m/s, or on the family exit condition. It dispatches exit forces instead
of the ordinary holding-force update. More than35 leaving updates requests
native reason17. The previous host never ran this phase and continued pinning
and orienting the board while stationary.

Connected Release state and the existing physical body/force/request owners.
50-50 exit82D41DF0 uses100 side/125 lift below investigation kind2, otherwise
10 side/0 lift, capped by outward speed1.0. Helper82D3F850 uses the actual
contact side and support normal. Other static-rail families use their native
exit calls82D428C0/82D420A0/82D42A80/82D41250. Ordinary friction/pinning and
orientation updates do not run during release. GRIND_RELEASE records the
transition during user gameplay.

Endpoint queries already clip segment intersections to[0,1]; losing the last
contact makes the existing selector enter Nonspecific701 and disable the
hook, then choose airborne/ground from actual contacts. No infinite extension
or endpoint teleport was added. The release phase addresses a board that
stalls while its last truck/tip remains supported at an endpoint.

Scope: common slow-speed/explicit release and static-rail exit forces.
Specialized tipslide drop-in and investigation-driven early-exit branches
remain separate existing gaps; this does not claim full retail parity.
Build only, no tests or automated gameplay. Visual checking is left to user.
