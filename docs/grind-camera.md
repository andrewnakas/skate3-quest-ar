# Grind camera publication

The camera adapter previously published Grinds0 as world X and Grinds96 as
world origin even during active grinds. The native movement compass82DF3AC0
uses Grinds0 directly while grinding, and GrindAnchor82DF2C78 consumes Grinds96.
This explains a heading snap followed by an unchanging direction, with an
incorrect positional anchor as well.

The live adapter now supplies:

- Directed primitive tangent:82D87460 calls82D37048, flips against velocity,
  and preserves the previous direction when absolute along-speed is below
  .1 and the direction dot is below -.9. These literals are820641A8/822F8E94.
- Contact target:82DF0640 initializes on entry, extrapolates the previous two
  targets on changes to grind type or primitive midpoint, then decays the
  resulting offset by .95 each physical tick. Constructor82DF24CC loads this
  coefficient from82072818. Inactive ticks invalidate target history.

This is contact discontinuity conditioning, not a general camera latency
filter. Existing camera compass, anchors and authored camera graph remain the
consumers. Straight authored/imported primitives supply their actual endpoints.

Evidence: local TU3 native image
`Skate3Research/research/reverse-engineering/XexDump/default_82000000_011B0000.bin`,
base82000000, SHA256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`.
Instruction listings were extracted from the generated PPC source into
`logs/biped-82D87460.txt`, `logs/biped-82D37048.txt`,
`logs/biped-82DF0640.txt`, and `logs/biped-82DF2130.txt`; coefficients were
cross-checked directly against image bytes. Behavior above is observed static
code; the reported visual symptom is consistent with the missing publications.
No reference gameplay trace or automated gameplay testing was performed.
