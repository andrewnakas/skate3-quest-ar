# Grind-out JumpInto crash

Observed user-run failure: `logs/game-20260907-213845.stderr.log`, tick528,
after BS_50_50 entry, reports MotionGraph behavior2228 unsupported JumpInto.
The preceding run also reached behavior2335 at tick468.

Native reference: TU3 image `default_82000000_011B0000.bin`, base82000000,
SHA-256 f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4.
Registration82F88B80 points to factory82BC7A58; the factory reads `attribute`
and installs vtable8231FBDC. Instance allocation82BB5930 initializes byte8
true. Begin/End are empty82B61BB8. Update82BACDC0 consumes the byte once,
queries the authored attribute with mask31, and, if found, seeks playback to
its begin-time field. Missing markers consume the latch without seeking.
Static instruction extracts are retained at `logs/biped-<address>.txt`.

The host now parses JumpInto, allocates a per-activation pending flag, and
performs this query on the live playback tree during Update. It uses the
existing tree query and absolute set_time paths, including transition and
blend-tree routing. It does not synthesize a physics impulse or substitute
the attribute scalar for its timestamp.

Validation: compilation only; user performs gameplay testing. This closes
the logged unsupported-behavior exit, not a claim of verified retail parity.
