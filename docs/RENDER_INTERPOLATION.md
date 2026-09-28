# Render interpolation

The Bevy presentation layer retains two completed simulation snapshots and
interpolates them with `Time<Fixed>::overstep_fraction()` in `Update`. Capture
runs after the physics system on every fixed tick, including catch-up ticks.
Frames without a new simulation tick still advance the interpolation fraction.

Snapshots contain the animation root, final physical skin hierarchy (including
the board/trucks/wheels), and camera transform/FOV. Bone transforms are converted
to skin-local transforms at each endpoint before translation/scale lerp and
quaternion slerp. The root and camera use that same fraction. These rendered
transforms are not read back by the native simulation or camera controllers.

Initialization, camera discontinuities, the existing camera-reset flag and
simulation-period changes collapse the history to the newest snapshot. There
is no boxcar camera filter, additional smoothing window, or extrapolation.
Interpolation introduces one fixed interval of presentation delay (normally
about 16.7 ms). This is a renderer feature, not a claim about retail behavior.

Reference: Bevy's official fixed-timestep presentation example:
https://bevy.org/examples-webgpu/movement/physics-in-fixed-timestep/

Validation: `Build.ps1` succeeded; no automated or gameplay tests were run,
as requested. Gameplay smoothness remains for the user to assess.
