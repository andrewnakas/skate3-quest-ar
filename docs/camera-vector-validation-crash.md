# Camera validation crash after the archive merge

`game-20260907-225258.stderr.log` exited at tick1016 in state/category500.
The imported camera publication check rejected the skeleton root translation
`[5.0928864, -1.535, -4.822083, -inf]`. All position and basis XYZ components
were finite; only the translation's fourth stored lane was non-finite.

The native-shaped camera vectors retain all four lanes, but camera distances,
dot products, direction tests and final rendered positions use XYZ. See
`skate-core/src/camera/vector_tracker.rs`, `compass.rs` and `subject_pose.rs`.
The merge introduced validation of every stored lane, incorrectly making the
unused lane fatal even though the existing camera accepts this representation.

Publication validation now checks XYZ independently for each root column and
each position/direction/motion vector. Invalid XYZ remains an error. No source
vector is sanitized, and no offboard simulation or camera tracking is changed.
The root check also no longer allocates a temporary flattened vector each tick.

Validation: source inspection and compilation; no automated gameplay/tests run.
