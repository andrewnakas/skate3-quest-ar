# 360 flip through the stock graphs

The ordinary on-board 360-flip route is wired for user visual testing.
PLAY.bat and PLAY-MAP.bat use the staged build. The supplied graphs and
animation banks remain the source of animation selection, transitions,
postures, timing, and physical animation attributes.

## Input and playback

- Right-stick magnitude and angle publish `AnticMag` and `AnticAngle`.
  The existing anticipation states choose and blend the crouch/scoop poses.
- The seven supplied joystick `.pat` files now run through the recovered
  recognizer. There is no trick hotkey, synthetic launch impulse, or new clip
  selected outside the graphs.
- Recognized events pass through the original listener gates, then publish
  the gesture name, `Trick`, and `GestureSpeed` into the ActionGraph.
- `CreateTrickIntentFromGesture` uses all 270 recovered mapping records,
  including native bucket order, mirrored mappings, overrides, dark-catch
  categories, and the four hold successors. A non-mirrored `360Flip` gesture
  maps to `360Flip`; when mirrored, that gesture maps to `Laserflip`, and the
  opposite gesture maps to `360Flip`, as in the original mapping.
- The stock ground path selects `B_360FLIP_G`, then sequences into
  `B_360FLIP_A`. `SetTrickHeight` consumes the preceding animation's
  `AnticStrength`, gesture speed, and the stock jumping settings. Its
  `JumpHeightOverride` packet enters the existing skeleton-input handler.
- Trick construction identity, scoring metadata, underflip/dark-catch
  requests, and nose-weight anticipation now have their original handlers.

The stock scoop coordinates for the non-mirrored 360-flip gesture correspond
to left/slightly down, down-left, down/slightly right, then an up-right flick
on the physical right stick. The recognizer permits holding the initial
point without weakening the eventual gesture. Direction changes with the
stock mirrored mapping; this is not an extra control setting.

## Evidence

TU3 instruction bytes were checked in a disposable IDA database against
the recompiled PPC instruction listings. Relevant boundaries:

| Behavior | Native source |
| --- | --- |
| PAT loading and push-front coordinate storage | 82696B18, 82699738 |
| Recognition, competition, strength, refractory frame | 82697168, 826972B8, 826974A8 |
| Axis sign and component deadzone | 82696030, 826962D8, 82F75F60 |
| Anticipation inputs | 82599AD0–82599C4C, 8259AD0C–8259AD40 |
| Pattern/held listeners and intent publication | 8259B878, 8259B9D0, 8259B1F0–8259B7D8 |
| Gesture mappings and graph lifecycle | 82B98F70, 82BA07F0, 82BA1C30–82BA2228 |
| Trick height | 82BAED78, 82BAEE90 |
| Trick construction identity | 82BB5C20, 82BB5CA0 |
| Scoring metadata | 82BBFA88, 8258FA20 |
| Underflip requests | 82BBE098, 82BBE0A8, 82BBE358 |
| Trick stair gate and physical reset | 82BA6930, 82DE3F38 |
| Nose-weight anticipation | 82BB2670, 82BB26D0, 82B97110 |

Factory names were verified from registration strings. In particular,
the community label on 82BC7848 incorrectly calls it `ScoringTrick`; the
actual registration is `SetTrickAttr`. Scoring's factory is 82BCC030.
Local research outputs and the re-extracted mapping records are under
`logs/`; original private binaries and animation data are not committed.

## Verification and user checks

Nonvisual checks pass for 36 core input tests, the PAT parser, the gesture
mapping/ActionHost lifecycle, and two private-stock fixtures. Those fixtures
recognize the authored scoop through all seven files and exercise the
stock crouch, scoop, takeoff, and air-animation handlers in both stances,
including finite decoded poses and required attributes. The fixture also
checks the ordinary route and ancestor entry/transition conditions for
unsupported factories. These are data/code
checks, not a claim of visually verified gameplay parity. No gameplay or
visual test was automated. The existing game instance was closed to stage
the new build; restarting it for the user is separate from these checks.

Please check:

1. Hold the right stick to crouch while stationary and while rolling.
2. Perform a 360 flip; check preparation, pop, board flip/spin, and foot motion.
3. Land and push away, then repeat in the opposite stance.

The first user-input run exposed the missing `OkToDoTrickOnStairs` gate.
It now reads the retained PhysOutAnimation byte166. Native reset82DE3F38
clears this byte; ordinary Ground Fill82D3A388 and common
ProcessOutput82DB6EC0 do not override it. This is the original packet
lifecycle, not a constant-success condition.

These nonvisual checks cover the ordinary ground route, not every trick in the newly
connected catalog. Advanced manual/grind-out branches still contain the
formerly unsupported `JumpInto` behavior (now implemented; see
[grind-out fix](grind-out-jump-into.md)), and the existing separate
`AddRunoutAttribs` gap remains. Those handlers were not bypassed. The current
test does not certify dark catches or the rest of the trick catalog visually.

The subsequent user run reached `ManualOutTimerIsActive` at tick3854.
This condition now reads the existing retained timer (>0, native82BA78B0
and8258F978). `SetManualOutTimer` arms it on End (82BB9158/8258F990),
using the authored length; UpdateManualOutTimer already owns decay/reset.
The regression fixture covers arming, hold during manuals, expiry, and
sibling takeoff preconditions evaluated before FromAntic selection.
