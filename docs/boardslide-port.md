# Boardslide contact integration

The live grind owner now supports paired-truck 50-50 and centre-deck boardslide
contacts. The ordinary authored default-world splines and imported splines use
the same primitive collection. This is an incremental playable port, not all
retail grind families or established 1:1 parity.

Native TU3 source boundaries:

- 82C1FB98: longitudinal deck rectangle, TestDepthEpsilon592, TestDepth596,
  DeckCenterToTruck636, first-triangle precedence and closest valid hit.
- 82D889A8: upright dot>.65, squared deck depth<.0036, low-speed ground gate,
  projected rail/deck clearance and truck-width exclusion.
- 82D875A8: current-state preference followed by fallback contact paths.
  Only the implemented truck and centre-deck paths are currently connected.
- 82D418B8/82D40890: right-axis alignment for boardslides, compared with
  forward-axis alignment for50-50. The native matrix interpolation weight is
  .1; the separate random orientation perturbation is not implemented here.
- 82D419A0/82D3FD08: translation25, lateral damping20, off-centre force10,
  mass-scaled60Hz opposing-velocity correction and exit force201.
- 82D41848: friction strengths55/55/60.
- 82D41CE8: interpolated mode100/104 pop values, distinct from50-50 mode92/96.
- 82D41D18: Grinds136 publishes kind1. Selector maps kind1 to state400.

Geometry investigation now retains six native probe indices. Native82C20C08
reads the raised cross line at result5 when deciding the blocked geometry kind;
the older ground-only helper incorrectly inspected result4 there. The new
boardslide investigation does not reuse that incorrect classification shortcut.
The existing ground-only caller has not been changed in this increment.

Still incomplete: other concrete grind state owners, full chromosome/name
production (the existing approach-name convention remains), all material and
engagement branches, and airborne GrindAirAdjust. The new trajectory admission,
publication and landing-normal leaves remain disconnected from live airborne
selection until that owner is complete. No unconditional snapping was added.

Validation: compilation only. The user has reserved all gameplay and stability
playtesting for themselves; no automated gameplay or test suite was run.
