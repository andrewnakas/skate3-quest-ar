use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VehicleDefinition {
    pub version: u32,
    pub model: String,
    pub model_scale: f32,
    pub model_offset: [f32; 3],
    pub model_yaw: f32,
    pub half_extents: [f32; 3],
    pub center_of_mass: [f32; 3],
    pub inertia_half_extents: Option<[f32; 3]>,
    pub collider_offset: [f32; 3],
    pub collider_rounding: f32,
    pub chassis_friction: f32,
    pub engine_audio: EngineAudio,
    pub rider_safety: RiderSafety,
    pub mass: f32,
    pub engine_force: f32,
    pub brake_impulse: f32,
    pub max_speed: f32,
    pub steering_angle: f32,
    pub suspension_length: f32,
    pub suspension_stiffness: f32,
    pub suspension_damping: f32,
    pub tire_grip: f32,
    pub ground_stability: f32,
    pub air_control: f32,
    pub bike: BikeProfile,
    pub wheels: Vec<WheelDefinition>,
    pub seat: [f32; 3],
    pub exit: [f32; 3],
    pub camera_distance: f32,
    pub camera_height: f32,
    /// Named model nodes hidden while someone is riding. A side stand is the
    /// motivating case: correct when the bike is parked, and dragging through
    /// every corner and jump when it is not.
    pub parked_nodes: Vec<String>,
    pub animations: Animations,
}
/// Occupied-seat collision and automatic ejection. Coordinates are chassis-local.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RiderSafety {
    pub enabled: bool,
    pub offset: [f32; 3],
    pub radius: f32,
    pub half_height: f32,
    pub crash_delta_v: f32,
    pub hit_impulse: f32,
    pub inverted_up_y: f32,
    pub inverted_seconds: f32,
    pub eject_up_speed: f32,
}
impl Default for RiderSafety {
    fn default() -> Self {
        Self {
            enabled: true,
            offset: [0., 0.5, 0.],
            radius: 0.25,
            half_height: 0.25,
            crash_delta_v: 6.,
            hit_impulse: 180.,
            inverted_up_y: -0.2,
            inverted_seconds: 0.2,
            eject_up_speed: 2.,
        }
    }
}
/// Single-track (two wheel) handling. Disabled by default, so every existing
/// four-wheel definition keeps the car behaviour in `assists` and `handling`.
///
/// A bike is not a narrow car, and it is not a rigid body that balances either.
/// The chassis body is held upright over the contact line while a wheel is
/// down; **lean is a separate handling state** that steers the bike (a leaned
/// bike carves the radius its lean angle dictates), rolls the visual model
/// about the contact line and drives the rider's pose. That is how the arcade
/// motocross games do it, and it is why the bike can never topple, ground its
/// engine cases in a corner or lose a wheel's ground contact because its
/// raycasts leaned with it -- which is precisely what a physically leaning
/// chassis on centreline raycasts did. In the air the body is free, so whips,
/// flips and tabletops are real rotation, and the landing is judged against
/// the ground it lands on.
///
/// These are authored arcade values, not recovered constants.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BikeProfile {
    pub enabled: bool,
    /// Maximum lean angle, radians. Also the lean the tyres can hold: a bike
    /// leaned to `atan(tire_grip)` is asking for exactly the friction it has.
    pub lean_max: f32,
    /// How fast the lean follows the bars, 1/s. Higher is twitchier; lower
    /// takes the lean up gradually, which is what makes a line adjustable
    /// mid-corner rather than a thing you commit to once.
    pub lean_rate: f32,
    /// Softens the bars around centre, 0..1. The lean a stick asks for is
    /// `x * (1 - e + e * x²)`, so at `e = 0.6` a third of the stick asks for
    /// about a sixth of the lean and full stick still reaches `lean_max`.
    /// Without it the first few degrees of stick are the whole corner and
    /// small corrections are impossible to make.
    pub lean_expo: f32,
    /// How much bar input becomes lean at speed, 0..2.
    pub counter_steer: f32,
    /// Roll authority holding the chassis over the contact line, rad/s² per
    /// radian of error. Damping is derived, so this cannot oscillate.
    pub upright_gain: f32,
    /// Speed-sensitive steering lock divisor. Lower keeps lock at speed.
    pub steer_falloff: f32,
    /// Lateral stiffness correction: two wheels each carry twice a kart's load.
    pub cornering_scale: f32,
    /// Yaw authority toward the turn rate the lean angle dictates, 1/s.
    pub lean_yaw: f32,
    /// Turn rate the bars alone will ask for at speed, rad/s.
    ///
    /// Lean physics caps the turn rate at `g·tan(lean)/v`, which is 1.43 rad/s
    /// at 12 m/s and falls off from there -- about half what an arcade
    /// motocross game turns at. Raising grip to close that gap does not work:
    /// sustaining 2.5 rad/s at 12 m/s needs 3.1 g and the tyres have 1.9, so
    /// the bike just slides and scrubs its speed off. This asks for the rate
    /// directly instead, and `grip_assist` is what makes it real.
    pub steer_rate: f32,
    /// How much of the turn is made by pointing the bike rather than by the
    /// tyres, 0..1. Each grounded tick the velocity is rotated toward the
    /// heading, at constant speed, so a commanded turn actually changes the
    /// direction of travel instead of becoming a slide. 0 is pure physics.
    /// High values feel railed -- the bike stops having weight.
    pub grip_assist: f32,
    /// Rear brake pivot: extra yaw authority while the rear brake is held,
    /// on top of the back end stepping out on its own.
    ///
    /// Defaults to 1 — no extra yaw. Measured, any boost at all sends the
    /// bike past a pivot and into a spin: a 0.8 s tap already swings it 130
    /// degrees, and the slide costs most of the speed whichever way the
    /// rear's grip is tuned. Raise it if you want the bike loose, but it
    /// wants the heading kept on a leash first.
    pub pivot_boost: f32,
    /// How much of a banked surface is added to the rider's lean, 0..2.
    ///
    /// Not 1, and the reason is worth knowing: most of the berm is already
    /// there without this. The suspension pushes along the contact normal,
    /// and on a berm that normal leans toward the turn centre, so the ground
    /// supplies centripetal force for free. Adding the bank to the carve at
    /// full strength counts it twice -- measured on a 26 degree bowl, an
    /// assist of 1 commanded so much yaw that the tyres gave up and the bike
    /// left the corner at 8 m/s where it entered at 14, while an assist of 0
    /// left at 20. Past about 0.45 the bike actually sweeps *less* for more
    /// command, which is the signature of a slide. This trims the turn-in
    /// rather than providing the effect.
    pub berm_assist: f32,
    /// Suspension compression from rider weight, newtons.
    pub preload_force: f32,
    /// Launch impulse when preload is released in contact, newton-seconds.
    pub preload_release: f32,
    /// Airborne yaw authority (whips), rad/s².
    pub air_yaw: f32,
    /// Airborne nose-down authority, rad/s². Scrubs are deliberately asymmetric.
    pub air_pitch_down: f32,
    /// Airborne nose-up authority, rad/s².
    pub air_pitch_up: f32,
    /// Airborne roll authority, rad/s².
    pub air_roll: f32,
    /// Pitch rate at full stick in the air, rad/s. A backflip is a held stick
    /// for `2π / flip_rate` seconds.
    pub flip_rate: f32,
    /// Yaw rate at full bars in the air, rad/s.
    pub whip_rate: f32,
    /// How hard a neutral stick levels roll and brings the nose back round to
    /// the direction of travel in the air, 1/s. This is what lets a whip land.
    pub air_level: f32,
    /// Nose-up pitch the wheelie assist balances at, radians.
    pub wheelie_limit: f32,
    /// Rear-up pitch the stoppie assist balances at, radians.
    pub stoppie_limit: f32,
    /// Engine force multiplier for the half second after a clutch dump.
    pub clutch_boost: f32,
    /// Roll relative to the ground, radians, beyond which a landing is a crash.
    pub landing_roll: f32,
    /// Pitch relative to the ground, radians, beyond which a landing is a crash.
    pub landing_pitch: f32,
    /// Angle between the bike and its direction of travel, radians, beyond
    /// which a landing above walking pace is a crash.
    pub landing_yaw: f32,
    /// Tallest step the bike will hop up when its frame jams against one,
    /// metres. 0 turns the assist off.
    ///
    /// A raycast wheel cannot roll over an edge -- it samples the ground at a
    /// single point, so a riser arrives as an instantaneous jump in ground
    /// height rather than something to climb. On a flight of stairs the
    /// chassis pitch lags the slope and the frame ends up driven into a riser
    /// two steps ahead of the front wheel, which stops the bike dead. No
    /// collider shape avoids that: it was measured across five, and every one
    /// jammed. This lifts the bike over the step instead, and only when a
    /// climbable top is actually found ahead, so it cannot be used to ride up
    /// a wall.
    pub step_assist: f32,
}
impl Default for BikeProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            lean_max: 0.9,
            lean_rate: 9.,
            lean_expo: 0.,
            counter_steer: 1.,
            upright_gain: 60.,
            steer_falloff: 0.03,
            cornering_scale: 2.,
            lean_yaw: 8.,
            steer_rate: 2.5,
            grip_assist: 0.5,
            pivot_boost: 1.,
            berm_assist: 0.2,
            preload_force: 1100.,
            preload_release: 500.,
            air_yaw: 8.,
            air_pitch_down: 10.,
            air_pitch_up: 10.,
            air_roll: 8.,
            flip_rate: 5.5,
            whip_rate: 3.5,
            air_level: 3.,
            wheelie_limit: 0.85,
            stoppie_limit: 0.5,
            clutch_boost: 1.6,
            landing_roll: 0.75,
            landing_pitch: 0.95,
            landing_yaw: 0.9,
            step_assist: 0.,
        }
    }
}
/// Built-in synthesized engine; no external recording is required.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EngineAudio {
    pub enabled: bool,
    pub volume: f32,
    pub idle_pitch: f32,
    pub max_pitch: f32,
    /// Exhaust character: `generic` (the original harmonic stack) or
    /// `four_stroke_single` (a thumper). The host synthesizes both.
    pub profile: String,
}
impl Default for EngineAudio {
    fn default() -> Self {
        Self {
            enabled: false,
            volume: 0.45,
            idle_pitch: 0.7,
            max_pitch: 2.8,
            profile: "generic".into(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WheelDefinition {
    pub position: [f32; 3],
    pub radius: f32,
    pub steering: bool,
    pub driven: bool,
    #[serde(default)]
    pub node: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Animations {
    pub file: Option<String>,
    pub enter: Option<String>,
    pub exit: Option<String>,
    pub drive: Option<String>,
    pub idle: Option<String>,
    pub reverse: Option<String>,
    pub brake: Option<String>,
    pub steer_left: Option<String>,
    pub steer_right: Option<String>,
    /// Rider posture layers, each one held pose the host eases the riding
    /// pose towards by how much of that posture the ride currently asks for:
    /// standing on the pegs, absorbing a landing, weight back or forward, and
    /// hanging off the inside of a lean.
    pub stand: Option<String>,
    pub crouch: Option<String>,
    pub weight_back: Option<String>,
    pub weight_forward: Option<String>,
    pub lean_left: Option<String>,
    pub lean_right: Option<String>,
    /// Clip names for air tricks, indexed by `Controls::trick` minus one. Each
    /// is a single held pose: the host blends the riding pose towards it by
    /// `trick_extend`, so a trick needs one authored frame, not an animation.
    pub tricks: Vec<String>,
}
impl Default for VehicleDefinition {
    fn default() -> Self {
        Self {
            version: 1,
            model: String::new(),
            model_scale: 1.,
            model_offset: [0.; 3],
            model_yaw: 0.,
            half_extents: [0.7, 0.2, 1.1],
            center_of_mass: [0.; 3],
            inertia_half_extents: None,
            collider_offset: [0.; 3],
            collider_rounding: 0.,
            chassis_friction: 0.3,
            engine_audio: EngineAudio::default(),
            rider_safety: RiderSafety::default(),
            mass: 220.,
            engine_force: 1800.,
            brake_impulse: 100.,
            max_speed: 25.,
            steering_angle: 0.5,
            suspension_length: 0.25,
            suspension_stiffness: 30.,
            suspension_damping: 4.,
            tire_grip: 1.3,
            ground_stability: 0.,
            air_control: 0.,
            bike: BikeProfile::default(),
            wheels: vec![],
            seat: [0., 0.25, 0.],
            exit: [1.8, 0., 0.],
            camera_distance: 5.,
            camera_height: 2.,
            parked_nodes: Vec::new(),
            animations: Animations::default(),
        }
    }
}
pub fn package_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.contains([':', '\\', '#'])
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && !path.chars().any(char::is_control)
}
pub fn point(p: &[f32; 3], max: f32) -> bool {
    p.iter().all(|x| x.is_finite() && x.abs() <= max)
}
impl VehicleDefinition {
    pub fn validate(&self) -> Result<(), String> {
        let range = |v: f32, a: f32, b: f32| v.is_finite() && (a..=b).contains(&v);
        if self.version != 1 || !package_path(&self.model) || !self.model.ends_with(".glb") {
            return Err("Vehicle version must be 1 and model a package-relative GLB".into());
        }
        if !range(self.model_scale, 0.001, 100.)
            || !point(&self.model_offset, 100.)
            || !self.model_yaw.is_finite()
            || !self.half_extents.iter().all(|x| range(*x, 0.05, 10.))
            || !point(&self.center_of_mass, 10.)
            || self
                .inertia_half_extents
                .is_some_and(|v| !v.iter().all(|&x| range(x, 0.05, 10.)))
            || !point(&self.collider_offset, 10.)
            || !range(
                self.collider_rounding,
                0.,
                self.half_extents
                    .iter()
                    .copied()
                    .fold(f32::INFINITY, f32::min)
                    * 0.95,
            )
            || !range(self.chassis_friction, 0., 2.)
            || !matches!(
                self.engine_audio.profile.as_str(),
                "generic" | "four_stroke_single"
            )
            || !range(self.engine_audio.volume, 0., 1.)
            || !range(self.engine_audio.idle_pitch, 0.25, 2.)
            || !range(
                self.engine_audio.max_pitch,
                self.engine_audio.idle_pitch,
                5.,
            )
            || !point(&self.rider_safety.offset, 5.)
            || !range(self.rider_safety.radius, 0.1, 1.)
            || !range(self.rider_safety.half_height, 0.05, 1.)
            || !range(self.rider_safety.crash_delta_v, 1., 50.)
            || !range(self.rider_safety.hit_impulse, 10., 10000.)
            || !range(self.rider_safety.inverted_up_y, -1., 0.5)
            || !range(self.rider_safety.inverted_seconds, 0.05, 3.)
            || !range(self.rider_safety.eject_up_speed, 0., 10.)
            || !range(self.mass, 10., 10000.)
            || !range(self.engine_force, 0., 100000.)
            || !range(self.brake_impulse, 0., 10000.)
            || !range(self.max_speed, 1., 100.)
            || !range(self.steering_angle, 0.01, 1.2)
            || !range(self.suspension_length, 0.01, 2.)
            || !range(self.suspension_stiffness, 1., 200.)
            || !range(self.suspension_damping, 0.1, 30.)
            || !range(self.tire_grip, 0.1, 20.)
            || !range(self.ground_stability, 0., 1.)
            || !range(self.air_control, 0., 10.)
            || !range(self.bike.lean_max, 0.05, 1.4)
            || !range(self.bike.lean_rate, 0., 60.)
            || !range(self.bike.lean_expo, 0., 1.)
            || !range(self.bike.counter_steer, 0., 2.)
            || !range(self.bike.upright_gain, 0., 60.)
            || !range(self.bike.steer_falloff, 0., 1.)
            || !range(self.bike.cornering_scale, 0.1, 10.)
            || !range(self.bike.preload_force, 0., 50000.)
            || !range(self.bike.preload_release, 0., 20000.)
            || !range(self.bike.air_yaw, 0., 20.)
            || !range(self.bike.air_pitch_down, 0., 20.)
            || !range(self.bike.air_pitch_up, 0., 20.)
            || !range(self.bike.air_roll, 0., 20.)
            || !range(self.bike.lean_yaw, 0., 40.)
            || !range(self.bike.steer_rate, 0., 8.)
            || !range(self.bike.grip_assist, 0., 1.)
            || !range(self.bike.pivot_boost, 1., 4.)
            || !range(self.bike.berm_assist, 0., 2.)
            || !range(self.bike.flip_rate, 0., 12.)
            || !range(self.bike.whip_rate, 0., 12.)
            || !range(self.bike.air_level, 0., 20.)
            || !range(self.bike.wheelie_limit, 0.1, 1.4)
            || !range(self.bike.stoppie_limit, 0.1, 1.2)
            || !range(self.bike.clutch_boost, 1., 4.)
            || !range(self.bike.landing_roll, 0.1, 3.2)
            || !range(self.bike.landing_pitch, 0.1, 3.2)
            || !range(self.bike.landing_yaw, 0.1, 3.2)
            || !range(self.bike.step_assist, 0., 0.6)
            || !point(&self.seat, 10.)
            || !point(&self.exit, 10.)
            || !range(self.camera_distance, 2., 30.)
            || !range(self.camera_height, 0.5, 15.)
        {
            return Err("Vehicle dimensions/tuning outside supported finite ranges".into());
        }
        if !(2..=8).contains(&self.wheels.len())
            || !self.wheels.iter().any(|w| w.driven)
            || !self.wheels.iter().all(|w| {
                point(&w.position, 10.)
                    && range(w.radius, 0.05, 2.)
                    && w.node
                        .as_ref()
                        .is_none_or(|n| !n.is_empty() && n.len() <= 128)
            })
        {
            return Err("Vehicle needs 2..8 valid wheels and at least one driven wheel".into());
        }
        // Single-track handling assumes one steered front and one driven rear.
        if self.bike.enabled
            && (self.wheels.len() != 2 || !self.wheels.iter().any(|w| w.steering))
        {
            return Err("A bike profile needs exactly two wheels, one of them steering".into());
        }
        if self.parked_nodes.len() > 16
            || self
                .parked_nodes
                .iter()
                .any(|n| n.is_empty() || n.len() > 128)
        {
            return Err("A vehicle supports at most 16 named parked nodes".into());
        }
        if self
            .animations
            .file
            .as_ref()
            .is_some_and(|p| !package_path(p))
        {
            return Err("Invalid vehicle animation path".into());
        }
        if self.animations.tricks.len() > 32
            || self
                .animations
                .tricks
                .iter()
                .any(|n| n.is_empty() || n.len() > 128)
        {
            return Err("A vehicle supports at most 32 named trick poses".into());
        }
        if !self.animations.tricks.is_empty() && self.animations.file.is_none() {
            return Err("Trick poses require an animation file".into());
        }
        for name in [
            &self.animations.enter,
            &self.animations.exit,
            &self.animations.drive,
            &self.animations.idle,
            &self.animations.reverse,
            &self.animations.brake,
            &self.animations.steer_left,
            &self.animations.steer_right,
            &self.animations.stand,
            &self.animations.crouch,
            &self.animations.weight_back,
            &self.animations.weight_forward,
            &self.animations.lean_left,
            &self.animations.lean_right,
        ]
        .into_iter()
        .flatten()
        {
            if self.animations.file.is_none() || name.is_empty() || name.len() > 128 {
                return Err("Animation slots require a file and valid clip names".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VehicleTuning {
    pub engine_volume: Option<f32>,
    pub engine_force: Option<f32>,
    pub max_speed: Option<f32>,
    pub brake_impulse: Option<f32>,
    pub steering_angle: Option<f32>,
    pub tire_grip: Option<f32>,
    pub lean_max: Option<f32>,
    pub lean_rate: Option<f32>,
    pub lean_expo: Option<f32>,
    pub counter_steer: Option<f32>,
    pub preload_release: Option<f32>,
    pub air_yaw: Option<f32>,
    pub lean_yaw: Option<f32>,
    pub steer_rate: Option<f32>,
    pub grip_assist: Option<f32>,
    pub pivot_boost: Option<f32>,
    pub berm_assist: Option<f32>,
    pub step_assist: Option<f32>,
    pub flip_rate: Option<f32>,
    pub whip_rate: Option<f32>,
}
impl VehicleTuning {
    pub fn apply(&self, definition: &VehicleDefinition) -> Result<VehicleDefinition, String> {
        let mut d = definition.clone();
        if let Some(v) = self.engine_volume {
            d.engine_audio.volume = v;
        }
        if let Some(v) = self.engine_force {
            d.engine_force = v;
        }
        if let Some(v) = self.max_speed {
            d.max_speed = v;
        }
        if let Some(v) = self.brake_impulse {
            d.brake_impulse = v;
        }
        if let Some(v) = self.steering_angle {
            d.steering_angle = v;
        }
        if let Some(v) = self.tire_grip {
            d.tire_grip = v;
        }
        if let Some(v) = self.lean_max {
            d.bike.lean_max = v;
        }
        if let Some(v) = self.lean_rate {
            d.bike.lean_rate = v;
        }
        if let Some(v) = self.lean_expo {
            d.bike.lean_expo = v;
        }
        if let Some(v) = self.counter_steer {
            d.bike.counter_steer = v;
        }
        if let Some(v) = self.preload_release {
            d.bike.preload_release = v;
        }
        if let Some(v) = self.air_yaw {
            d.bike.air_yaw = v;
        }
        if let Some(v) = self.lean_yaw {
            d.bike.lean_yaw = v;
        }
        if let Some(v) = self.steer_rate {
            d.bike.steer_rate = v;
        }
        if let Some(v) = self.grip_assist {
            d.bike.grip_assist = v;
        }
        if let Some(v) = self.pivot_boost {
            d.bike.pivot_boost = v;
        }
        if let Some(v) = self.step_assist {
            d.bike.step_assist = v;
        }
        if let Some(v) = self.berm_assist {
            d.bike.berm_assist = v;
        }
        if let Some(v) = self.flip_rate {
            d.bike.flip_rate = v;
        }
        if let Some(v) = self.whip_rate {
            d.bike.whip_rate = v;
        }
        d.validate()?;
        Ok(d)
    }
    pub fn valid(&self) -> bool {
        [
            self.engine_volume.map(|x| (x, 0., 1.)),
            self.engine_force.map(|x| (x, 0., 100000.)),
            self.max_speed.map(|x| (x, 1., 100.)),
            self.brake_impulse.map(|x| (x, 0., 10000.)),
            self.steering_angle.map(|x| (x, 0.01, 1.2)),
            self.tire_grip.map(|x| (x, 0.1, 20.)),
            self.lean_max.map(|x| (x, 0.05, 1.4)),
            self.lean_rate.map(|x| (x, 0., 60.)),
            self.lean_expo.map(|x| (x, 0., 1.)),
            self.counter_steer.map(|x| (x, 0., 2.)),
            self.preload_release.map(|x| (x, 0., 20000.)),
            self.air_yaw.map(|x| (x, 0., 20.)),
            self.lean_yaw.map(|x| (x, 0., 40.)),
            self.steer_rate.map(|x| (x, 0., 8.)),
            self.grip_assist.map(|x| (x, 0., 1.)),
            self.pivot_boost.map(|x| (x, 1., 4.)),
            self.berm_assist.map(|x| (x, 0., 2.)),
            self.step_assist.map(|x| (x, 0., 0.6)),
            self.flip_rate.map(|x| (x, 0., 12.)),
            self.whip_rate.map(|x| (x, 0., 12.)),
        ]
        .into_iter()
        .flatten()
        .all(|(x, a, b)| x.is_finite() && (a..=b).contains(&x))
    }
}
