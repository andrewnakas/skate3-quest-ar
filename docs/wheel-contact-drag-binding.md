# Wheel contact drag binding

The user capture `logs/game-20260907-230543.stderr.log` begins with normal
four-wheel riding. Later neutral-input samples have zero turn, brake, balance,
and slide, with truck targets decaying to zero. Samples 4320–5160 repeatedly
report three wheel contacts. This rules out persistent controller steering in
that interval; it does not establish which contact or force caused the symptom.

Static inspection found an independent, definite adapter mismatch:

- TU3 `Skateboard::UpdatePostPhysics` (`82C07D20`), at `82C08634`, stores the
  per-wheel contact-dependent drag at inertia offset **36**.
- `RigidBody::DynamicUpdate` (`82AE6590`) reads offset 36 for angular damping
  and offset 32 for linear damping. The Rust inertia adapter likewise packs
  `linear_drag` at word 8 and `angular_drag` at word 9.
- The game previously assigned the wheel result to `linear_drag`. It now
  assigns `angular_drag`, and the retained field is named accordingly.

Values, contact gates, update order, and reset lifetime are unchanged. The
original constant ground torque was also inspected and is retained.

Evidence image: `Skate3Research/research/reverse-engineering/XexDump/default_82000000_011B0000.bin`,
base `82000000`, SHA-256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
Instruction extractions: `logs/biped-82C07D20.txt` and
`logs/biped-82AE6590.txt`; generated reference source is in the research
project's `Skate3CustomEngineLayer/generated` directory.

Validation: compile only; gameplay confirmation remains with the user.
Optional `skate_game::riding_trace=debug` logging now includes board body
states, contact reports, and queued forces to distinguish remaining physical
faults if the correction does not resolve the complete symptom.
