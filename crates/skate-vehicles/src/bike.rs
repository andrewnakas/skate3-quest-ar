//! Single-track handling the way the arcade motocross games do it.
//!
//! The chassis body is **held upright over the contact line** whenever a wheel
//! is down. Lean is not body roll: it is a handling state that steers the bike
//! (a leaned bike carves the radius its lean angle dictates), rolls the visual
//! model about the contact line and drives the rider's posture. A physically
//! leaning chassis on centreline raycasts could never work: Rapier casts each
//! wheel ray along the chassis' own down axis, so a body rolled 50° read its
//! suspension as extended, sank, and ground its collider on the floor.
//!
//! In the air the body is free. Whips, flips and tabletops are real rotation,
//! a neutral stick levels and straightens the bike for the landing, and the
//! landing itself is judged against the ground it lands on.
//!
//! Same contract as [`crate::assists`]: apply impulses through the rigid body,
//! never overwrite its transform and never add lift. Authored arcade values,
//! not recovered constants.
//!
//! ## Sign conventions, stated once
//!
//! The chassis frame is +Y up, +Z forward, +X driver-left, and `steering` is
//! positive-left (`docs/vehicle-sdk.md`). From that:
//!
//! - A **positive local-Z** rotation tilts the up axis toward -X, so it leans the
//!   bike **right**. Leaning left — the way you turn left — is **negative Z**.
//!   The `lean` state uses the same sign: negative is left.
//! - A **positive local-X** rotation tilts forward toward -Y, so it is **nose
//!   down**. Nose up, the wheelie direction, is negative X.
//! - A **positive local-Y** rotation sends forward toward +X, so it yaws **left**.
use crate::rapier3d::utils::AngularInertiaOps;
use crate::{Vehicle, rapier3d::prelude::*};

/// Speed at which the bars fully command lean rather than steering lock. Low
/// on purpose: a motocross bike is expected to come round in its own length in
/// a corner, and gating lean behind road-bike speeds makes it feel like it
/// refuses to turn.
const LEAN_SPEED: f32 = 3.5;
/// Preload must reach this fraction of travel before a release will pop.
const ARM_FRACTION: f32 = 0.35;
/// Below this much remaining rider weight, an armed preload fires.
const RELEASE_FRACTION: f32 = 0.15;
/// A hop shorter than this is a bump, not an air: no landing judgement.
const MIN_AIR: f32 = 0.25;
/// Landings slower than this are never judged on heading: you can land a
/// bike sideways at walking pace.
const YAW_JUDGEMENT_SPEED: f32 = 4.;
/// Hard ceiling on how fast the bike will come round, rad/s. Only a slow,
/// steeply leaned corner ever reaches it; it exists so a walking-pace lean
/// cannot spin the bike on the spot, which steering lock does instead.
const CARVE_LIMIT: f32 = 4.5;
/// Effective lean is clamped here before the tangent that turns it into a turn
/// rate. 80 degrees, safely short of the singularity at 90.
const CARVE_MAX_LEAN: f32 = 1.40;
/// How long a clutch dump boosts the engine, seconds.
const BOOST_SECONDS: f32 = 0.5;
/// How long the bike is helped to plant itself after a real air, seconds.
/// Freestyle landings are the point of the whole mod: coming down square
/// enough should stick, not spit the rider over the bars because one wheel
/// touched a degree out.
const LANDING_ASSIST: f32 = 0.45;

#[derive(Default)]
pub(crate) struct Bike {
    /// Stored suspension compression from rider weight, 0..1.
    preload: f32,
    /// One pop per compression: set while loaded, cleared when it fires.
    armed: bool,
    /// Handling lean, radians. Negative leans left.
    pub(crate) lean: f32,
    /// No wheel has touched since the last contact.
    pub(crate) airborne: bool,
    /// Seconds since the last wheel left the ground.
    pub(crate) air_time: f32,
    /// Chassis-local height of the contact line, the axis the model leans about.
    pub(crate) contact_line: Option<f32>,
    /// Suspension compression of the most loaded wheel, 0..1 of travel.
    pub(crate) compression: f32,
    /// Engine speed held against the clutch, rad/s at the driven wheel.
    pub(crate) revs: f32,
    /// Seconds of clutch-dump boost remaining.
    boost: f32,
    /// Seconds of post-touchdown planting assist remaining.
    pub(crate) landing: f32,
    /// Forward pace carried into this frame, m/s, decaying.
    carry: f32,
    /// Seconds before another step hop is allowed.
    hop: f32,
}

impl Bike {
    pub(crate) fn preload(&self) -> f32 {
        self.preload
    }
}

/// Soften an axis around centre while keeping its full range at the stop.
/// A linear stick spends most of its travel past the lean anyone wants to
/// hold, so the usable part of a corner lives in the first few millimetres and
/// nothing smaller than a whole lane change can be asked for.
pub(crate) fn expo(x: f32, amount: f32) -> f32 {
    x * (1. - amount + amount * x * x)
}

/// Lean the bars and rider ask for, radians, negative left.
pub(crate) fn lean_target(d: &crate::BikeProfile, c: &crate::Controls, speed: f32, yaw: f32) -> f32 {
    let speed_factor = (speed.abs() / LEAN_SPEED).clamp(0., 1.);
    // Bars are the main lean input at speed; rider weight hangs off on top.
    // Positive steering and positive lean both mean left, which is negative Z.
    let stick = (c.steering * d.counter_steer + c.lean * 0.4).clamp(-1., 1.);
    let want = -expo(stick, d.lean_expo) * d.lean_max * speed_factor;
    // Never lie the bike further over than the turn it is actually doing can
    // carry. A leaned bike is balancing centripetal acceleration against
    // gravity, so the honest angle is `atan(v*w/g)` -- and `w` here is the
    // yaw rate measured last frame, not one derived from the lean, so there
    // is no circularity.
    //
    // This only bites at walking pace. Above about 4 m/s the cap already
    // exceeds `lean_max` and the term is inert, which is what the
    // measurements showed: the shown lean tracked the justified angle within
    // a few degrees from 4 m/s up, but at 2 m/s the bike was drawn lying over
    // at 35 degrees for a turn that justified 23.
    let cap = (speed.abs() * yaw.abs() / 9.81).atan().max(0.12);
    want.clamp(-cap, cap)
}

/// Bank of the surface under the bike, radians, in `lean`'s own sign: negative
/// means the ground is tilted the way a left turn wants it.
///
/// `lean` is measured against the *surface*, because the chassis is held
/// square to the contact normal. That is what makes a berm free: the bank is
/// simply lean the rider did not have to ask for, and it adds to theirs.
pub(crate) fn bank_angle(normal: Vector, forward: Vector) -> f32 {
    let flat = Vector::new(forward.x, 0., forward.z);
    if flat.length_squared() < 1e-6 {
        return 0.;
    }
    // The bike's left, horizontal. A normal leaning that way is a surface
    // banked to support a left turn, and a left turn is a negative lean.
    let left = Vector::Y.cross(flat).normalize_or_zero();
    -normal.dot(left).clamp(-1., 1.).asin()
}

/// Yaw rate a bike carves at this speed, rad/s, positive left.
///
/// Steady cornering balances gravity against centripetal acceleration:
/// `v²/R = g·tan(lean)`, so the turn rate is `v/R = g·tan(lean)/v`. Faster is
/// wider for the same lean, exactly as on a real bike.
///
/// On a banked surface the same balance holds about the *bank*, so the two
/// angles simply add — a berm banked `b` under a bike leaned `l` corners as
/// though leaned `b + l`. That one term is the whole berm effect, and it also
/// gives off-camber for nothing: a bank of the wrong sign subtracts, the bike
/// pushes wide, and riding straight across a camber pulls you downhill the way
/// a real bike does.
pub(crate) fn carve_rate(lean: f32, bank: f32, speed: f32) -> f32 {
    // Clamp the sum *before* the tangent, and not as tidiness: `lean_max` is
    // 60 degrees and a 35 degree berm is another 35, so a committed rider
    // reaches 95 — past the singularity, where `tan` returns a large negative
    // number and the bike would snap into turning the wrong way exactly when
    // it is being asked for everything. At the 80 degree clamp `tan` is 5.7,
    // already far more turn than the tyres can hold, so nothing real is lost.
    let effective = (lean + bank).clamp(-CARVE_MAX_LEAN, CARVE_MAX_LEAN);
    // A berm is not the case the rate limit exists for — that is a walking
    // pace lean spinning the bike on the spot — so the bank raises it.
    let limit = CARVE_LIMIT * (1. + 1.5 * bank.abs());
    // No speed gate here. `lean_target` already fades the lean out as the bike
    // slows, and gating the carve as well squared that fade: at 2 m/s the bike
    // kept eleven percent of its turning authority and felt like it would not
    // come round at all. A lean that exists should carve what it is worth.
    (-9.81 * effective.tan() / speed.max(3.)).clamp(-limit, limit)
}

/// Airborne pitch/yaw/roll acceleration, rad/s², in the chassis frame.
///
/// Rate targets keep repeated input controllable. A neutral axis does more
/// than damp: it levels roll toward world up and swings the nose back toward
/// the direction of travel, which is what lets a whip land. `heading_error`
/// is the signed yaw from the forward axis to the horizontal velocity,
/// positive when the velocity is to the left; `roll_error` is the roll that
/// would stand the bike up, positive when the bike is leaning left.
pub(crate) fn air_acceleration(
    d: &crate::BikeProfile,
    c: &crate::Controls,
    local_omega: Vector,
    heading_error: f32,
    roll_error: f32,
) -> Vector {
    let rate = |target: f32, current: f32, authority: f32| {
        ((target - current) * 3.).clamp(-authority, authority)
    };
    // Stick back is nose up, which is negative X. A held stick is a flip.
    let pitch_target = -c.weight * d.flip_rate;
    let pitch = rate(
        pitch_target,
        local_omega.x,
        if pitch_target >= local_omega.x {
            d.air_pitch_down
        } else {
            d.air_pitch_up
        },
    );
    let whipping = c.whip.abs() > 0.05;
    let leaning = c.lean.abs() > 0.05;
    // The bars in the air are a whip: yaw, with the back end laid over.
    let yaw_target = if whipping {
        c.whip * d.whip_rate
    } else {
        (heading_error * d.air_level).clamp(-d.whip_rate, d.whip_rate)
    };
    let roll_target = if whipping || leaning {
        -(c.whip * 0.5 + c.lean * 0.6) * d.whip_rate
    } else {
        (roll_error * d.air_level).clamp(-d.whip_rate, d.whip_rate)
    };
    Vector::new(
        pitch,
        rate(yaw_target, local_omega.y, d.air_yaw),
        rate(roll_target, local_omega.z, d.air_roll),
    )
}

/// Lift the bike over a step its frame has jammed against.
///
/// A raycast wheel samples the ground at a single point, so it cannot roll
/// over an edge: a riser arrives as an instantaneous jump in ground height.
/// Climbing a flight is worse than one step, because the chassis pitch lags
/// the staircase slope and the frame ends up driven into a riser two steps
/// ahead of the front wheel while that wheel is still on the first tread.
/// That was measured -- the chassis takes a single large shove along its own
/// heading and the bike goes from 11 m/s to a standstill in one frame.
///
/// Reshaping the collider does not help. Five shapes were measured over a
/// twelve-step flight, from the shipped box to a compact heavily-rounded one,
/// and every one of them jammed on anything above a 0.15 m rise. The chassis
/// has to be lifted instead.
///
/// Two conditions gate it, and together they are what stop this becoming a
/// cheat. The frame must actually be against a near-horizontal face, which a
/// ramp never does -- the wheels ride a slope and the chassis never touches
/// it. And a climbable top has to be found by probing ahead, so a wall, which
/// has no top within reach, is still a wall.
pub(crate) fn step_up(v: &mut crate::Vehicle, world: &mut PhysicsWorld, dt: f32) {
    let d = v.definition.bike;
    if !d.enabled || d.step_assist <= 0. {
        return;
    }
    v.bike.hop = (v.bike.hop - dt).max(0.);
    let c = v.controls;
    let body = &world.bodies[v.body];
    let rotation = *body.rotation();
    let up = rotation * Vector::Y;
    let forward = rotation * Vector::Z;
    let flat = Vector::new(forward.x, 0., forward.z).normalize_or_zero();
    let velocity = body.linvel();
    let speed = Vector::new(velocity.x, 0., velocity.z).dot(flat);
    // Remember the pace from just before a jam. By the time the jam is
    // visible the solver has already taken the speed away, and the hop has to
    // give it back or every step costs the bike all of its momentum.
    v.bike.carry = (v.bike.carry - v.bike.carry * 3. * dt).max(speed);
    // The rider asking to go is what makes this an assist rather than a
    // trampoline: nothing happens to a bike being left alone against a wall.
    let asking = c.throttle.abs() > 0.1;
    if v.bike.hop > 0. || !asking || speed.abs() > 9. || up.y < 0.5 || flat == Vector::ZERO {
        return;
    }
    // A near-horizontal face against the frame. Deliberately *not* gated on
    // the impulse: that is large only on the frame of the crash, and a moment
    // later the bike is simply resting against the riser going nowhere, which
    // is exactly the state that needs rescuing.
    let blocked = body.colliders().iter().any(|&col| {
        world.narrow_phase.contact_pairs_with(col).any(|pair| {
            pair.manifolds.iter().any(|m| {
                let n: Vector = m.data.normal.into();
                n.y.abs() < 0.5
                    && n.dot(flat).abs() > 0.3
                    && m.points.iter().any(|p| p.dist < 0.02)
            })
        })
    });
    // Probe from the *leading* wheel. Taking the lowest contact instead reads
    // the rear wheel, which on a staircase is still down on the flat several
    // steps behind, where the ground ahead is flat too -- so nothing is ever
    // found to climb.
    let Some(here) = v
        .controller
        .wheels()
        .iter()
        .map(|w| w.raycast_info())
        .filter(|r| r.is_in_contact)
        .map(|r| r.contact_point_ws)
        .max_by(|a, b| a.dot(flat).total_cmp(&b.dot(flat)))
    else {
        return;
    };
    let reach = d.step_assist + 0.2;
    let ground = |ahead: f32| {
        let nose = here + flat * ahead + Vector::Y * (d.step_assist + 0.15);
        world
            .cast_ray(
                &Ray::new(nose, -Vector::Y),
                reach,
                true,
                QueryFilter::only_fixed(),
            )
            // Nothing within reach is an edge to drop off, not to climb.
            .map(|(_, toi)| (nose.y - toi) - here.y)
    };
    // Probe close. What has to be climbed is the *next* riser, and stair
    // treads are short -- looking 0.35 m or more ahead lands two steps on,
    // reads their combined height and rejects it as unclimbable.
    let (near, far) = (ground(0.12), ground(0.3));
    let Some(rise) = far.filter(|r| *r > 0.03 && *r <= d.step_assist) else {
        return;
    };
    // Step over it *before* being stopped by it. Waiting for the jam means
    // taking the hit first, and that hit is what threw the rider and made
    // clearing a flight a coin flip.
    //
    // The pair of probes is what tells a step from a ramp, and without it
    // this would haul the bike up every slope in the park: a ramp rises
    // steadily, so the near probe is already climbing, while a step leaves it
    // flat right up to the riser.
    let ahead = near.is_some_and(|n| n < 0.05) && rise > 0.08;
    if !blocked && !ahead {
        return;
    }
    // Place the chassis on the step rather than trying to drive or bounce it
    // there. Impulses do not work here: the frame is already pressed into the
    // riser, so the solver cancels whatever forward speed is handed to it,
    // and lifting bodily just raises the jam along with the bike. Moving it
    // is the same thing a character controller does to walk up a stair, and
    // it is the only version of this that measured as working.
    let body = &mut world.bodies[v.body];
    let lifted = body.translation() + Vector::Y * (rise + 0.04);
    body.set_translation(lifted, true);
    // Hand back the pace the jam took, with a floor so a bike that has been
    // sat against the step long enough for `carry` to decay can still walk
    // itself up rather than being stranded.
    let keep = (v.bike.carry * 0.9).max(3.);
    let mut next = body.linvel();
    next.y = next.y.max(0.);
    if Vector::new(next.x, 0., next.z).dot(flat) < keep {
        next = Vector::new(flat.x * keep, next.y, flat.z * keep);
    }
    body.set_linvel(next, true);
    v.bike.hop = 0.05;
}

/// Turn rate the bars alone ask for, rad/s, positive left.
///
/// Lean physics caps the rate at `g·tan(lean)/v`, which is about half what an
/// arcade motocross game turns at once you are moving: 1.43 rad/s at 12 m/s
/// against the ~2.5 it wants. Grip cannot close that gap -- 2.5 rad/s at
/// 12 m/s needs 3.1 g and the tyres have 1.9 -- so the rate is asked for
/// directly and `redirect` below is what stops it becoming a slide.
///
/// The falloff is deliberately gentle. `g·tan/v` falls as `1/v`; this falls
/// far slower, which is exactly the part that was missing at speed.
pub(crate) fn steer_rate(d: &crate::BikeProfile, c: &crate::Controls, speed: f32) -> f32 {
    let stick = (c.steering + c.lean * 0.25).clamp(-1., 1.);
    // Comes in from a standstill over the same span lean does, so a parked
    // bike is not spun on the spot by the bars alone.
    let moving = (speed.abs() / LEAN_SPEED).clamp(0., 1.);
    let falloff = 1. / (1. + speed.abs() * 0.025);
    expo(stick, d.lean_expo) * d.steer_rate * falloff * moving
}

/// Rotate a horizontal velocity toward `forward` by at most `limit` radians,
/// keeping its speed. Returns the new velocity.
///
/// This is the arcade mechanic the bike was missing. A real bike changes
/// direction only as fast as the tyres can push it sideways, and asking for
/// more than that does not turn harder -- it slides, and the slide scrubs the
/// speed off, which is measurably *less* turn for more command. Rotating the
/// velocity costs no grip at all, so the turn is real and the speed survives.
pub(crate) fn redirect(velocity: Vector, forward: Vector, limit: f32) -> Vector {
    let flat = Vector::new(velocity.x, 0., velocity.z);
    let aim = Vector::new(forward.x, 0., forward.z);
    let speed = flat.length();
    if speed < 0.5 || aim.length_squared() < 1e-6 || limit <= 0. {
        return velocity;
    }
    let aim = aim.normalize();
    let heading = flat / speed;
    // Only ever close the gap, never overshoot past the heading.
    let error = heading.dot(aim).clamp(-1., 1.).acos();
    if error < 1e-4 {
        return velocity;
    }
    // Never redirect a bike that is travelling backwards into a spin.
    if heading.dot(aim) < 0. {
        return velocity;
    }
    let turn = limit.min(error);
    let side = if Vector::Y.cross(heading).dot(aim) > 0. { 1. } else { -1. };
    let (sin, cos) = (turn * side).sin_cos();
    let rotated = Vector::new(
        heading.x * cos + heading.z * sin,
        0.,
        -heading.x * sin + heading.z * cos,
    );
    rotated * speed + Vector::Y * velocity.y
}

/// Signed yaw from `forward` to `velocity` in the ground plane, positive when
/// the velocity is to the left of the nose. Zero below walking pace.
fn heading_error(forward: Vector, velocity: Vector) -> f32 {
    let f = Vector::new(forward.x, 0., forward.z);
    let v = Vector::new(velocity.x, 0., velocity.z);
    if v.length() < YAW_JUDGEMENT_SPEED || f.length_squared() < 1e-6 {
        return 0.;
    }
    // Left of the nose is +X in the chassis frame, so left of `f` in world
    // space is the direction `Y × f`.
    let left = Vector::Y.cross(f);
    v.dot(left).atan2(v.dot(f))
}

pub(crate) fn apply(v: &mut Vehicle, bodies: &mut RigidBodySet, dt: f32) {
    let d = v.definition.bike;
    let c = v.controls;
    let mut contacts = 0;
    let mut normal = Vector::ZERO;
    let mut contact_line = 0.;
    let mut compression: f32 = 0.;
    let mut rear_contact = None;
    let mut front_contact = None;
    let mut contact_point = Vector::ZERO;
    let mut tire_torque = Vector::ZERO;
    let centre = bodies[v.body].center_of_mass();
    for (wheel, def) in v.controller.wheels().iter().zip(&v.definition.wheels) {
        let info = wheel.raycast_info();
        if info.is_in_contact {
            contacts += 1;
            normal += info.contact_normal_ws;
            contact_point += info.contact_point_ws;
            contact_line += def.position[1] - info.suspension_length - def.radius;
            compression = compression.max(
                (v.definition.suspension_length - info.suspension_length)
                    / v.definition.suspension_length.max(1e-3),
            );
            if def.steering {
                front_contact = Some(info.contact_point_ws);
            } else {
                rear_contact = Some(info.contact_point_ws);
            }
            // The moment `handling::tires` just applied by driving or braking
            // this wheel at its contact patch. Drive at the back lifts the
            // nose; the front brake drives it down. On a bike with real grip
            // both are far stronger than gravity's restoring moment, so full
            // throttle alone stands the bike vertical unless this is answered.
            // Lateral force is applied at the mass centre for a bike and so
            // contributes nothing here.
            let n = info.contact_normal_ws;
            let side = (wheel.axle() - n * wheel.axle().dot(n)).normalize_or_zero();
            let along = n.cross(side).normalize_or_zero();
            tire_torque += (info.contact_point_ws - centre).cross(along * wheel.forward_impulse);
        }
    }
    v.bike.compression = compression.clamp(0., 1.);
    if contacts > 0 {
        v.bike.contact_line = Some(contact_line / contacts as f32);
    } else if v.bike.contact_line.is_none() {
        v.bike.contact_line = v
            .definition
            .wheels
            .first()
            .map(|w| w.position[1] - v.definition.suspension_length - w.radius);
    }
    // Clutch and boost, ahead of the body borrow: `handling` reads
    // `engine_scale` on the next tick.
    let rear_radius = v
        .definition
        .wheels
        .iter()
        .find(|w| w.driven)
        .map_or(0.33, |w| w.radius);
    let redline = v.definition.max_speed / rear_radius;
    if c.clutch {
        v.bike.revs += (c.throttle.max(0.) * redline - v.bike.revs).clamp(-4. * redline * dt, 3. * redline * dt);
    } else {
        if v.bike.revs > 0.5 * redline {
            v.bike.boost = BOOST_SECONDS;
            // Dumping the clutch hands the stored engine speed straight to the
            // wheel. That is the launch: the tyre is instantly asking for far
            // more than it can hold, so it lights up and drives rather than
            // waiting for the engine to spin up from rest.
            v.launch = v.bike.revs;
        }
        v.bike.revs -= v.bike.revs.min(4. * redline * dt);
    }
    v.bike.boost = (v.bike.boost - dt).max(0.);
    v.engine_scale = if v.bike.boost > 0. { d.clutch_boost } else { 1. };

    let rotation = *bodies[v.body].rotation();
    let local_omega = rotation.inverse() * bodies[v.body].angvel();
    let forward = rotation * Vector::Z;
    let up = rotation * Vector::Y;
    let velocity = bodies[v.body].linvel();
    let speed = velocity.dot(forward);
    let mut acceleration = Vector::ZERO;
    let normal = normal.normalize_or_zero();
    // One wheel down is still grounded: a bike spends real time on the front or
    // rear alone, which is exactly why `ground_stability`'s two-contact gate
    // does not suit it.
    let upright = up.dot(normal) > 0.3;
    let grounded = contacts >= 1 && upright;
    if contacts >= 1 && v.bike.airborne {
        // Touchdown. Judge it against the surface, not world up: landing on a
        // steep face is fine when the bike matches the face.
        if v.bike.air_time > MIN_AIR && v.occupied {
            let n = rotation.inverse() * normal;
            let roll = n.x.atan2(n.y).abs();
            let pitch = n.z.atan2(n.y).abs();
            let yaw = heading_error(forward, velocity).abs();
            if roll > d.landing_roll || pitch > d.landing_pitch || yaw > d.landing_yaw {
                let body = &bodies[v.body];
                v.ejection = Some(crate::safety::ejection(
                    &v.definition,
                    body,
                    body.angvel(),
                    "landing",
                ));
            }
        }
        if v.ejection.is_none() {
            v.bike.landing = LANDING_ASSIST;
        }
        v.bike.airborne = false;
        v.bike.air_time = 0.;
    }
    // Toppling acceleration per radian of roll, m*g*h / I_roll, with h
    // measured live from the mass centre to the mean contact point so it
    // tracks suspension travel instead of assuming a ride height. The
    // suspension pushes up at the contact patch, which stays on the
    // centreline, so any roll puts that force off to one side and the moment
    // grows with the roll. Without cancelling it the upright controller is
    // fighting a load that grows faster than it does, and `upright_gain` stops
    // meaning anything you can tune by eye: below about 84 the bike slowly
    // lies down, above it the number is arbitrary.
    let [hx, hy, _] = v
        .definition
        .inertia_half_extents
        .unwrap_or(v.definition.half_extents);
    let roll_inertia = ((hx * hx + hy * hy) * v.definition.mass / 3.).max(1e-3);
    let topple = if contacts > 0 {
        let height = (bodies[v.body].center_of_mass() - contact_point / contacts as f32)
            .dot(up)
            .clamp(0., 3.);
        v.definition.mass * 9.81 * height / roll_inertia
    } else {
        0.
    };
    let body = &mut bodies[v.body];
    if grounded {
        let target = lean_target(&d, &c, speed, local_omega.y);
        v.bike.lean += (target - v.bike.lean) * (1. - (-d.lean_rate * dt).exp());
        // Hold the chassis over the contact line. Damping is derived from the
        // gain, so tuning the gain cannot tune the bike into an oscillation.
        //
        // Two different roll errors, and using one for both jobs is a bug that
        // only shows up off the flat. The controller steers toward the
        // *surface*; gravity pulls toward *world up*. On level ground they are
        // the same vector and cancelling one cancels the other, which is why
        // this read as correct for so long. On a cambered slope they diverge,
        // and cancelling gravity along the surface error leaves the real
        // toppling moment untouched: the bike settled about 55% of the way
        // into a 30 degree camber and barely rolled at all on a slope that
        // also climbed. Feed gravity forward along its own lever instead.
        let roll_to_surface = (rotation.inverse() * up.cross(normal)).z;
        let planted = 1. + 2.5 * (v.bike.landing / LANDING_ASSIST).clamp(0., 1.);
        // Cancel the roll moment the ground reaction makes about the mass
        // centre, by the same construction the wheelie controller uses for
        // pitch: the support pushes at the contact patch, and once the bike is
        // leaned that lever is off to one side. Feeding it forward is what
        // lets the PD's target angle be the angle it settles at.
        //
        // The support acts along the **contact normal**, not along world up,
        // and that distinction is the whole difference on a cambered slope.
        // A bike already square to the camber has its contact patch directly
        // beneath its mass centre along that normal, so the real moment is
        // zero -- while a feed-forward written against world up computes
        // `m*g*h*sin(camber)` there and injects a torque that drags the bike
        // back toward world-vertical. That is why it used to take up only
        // about half of a camber and almost none of one that also climbed.
        // On the flat the normal *is* world up, so nothing here changes.
        let gravity_roll = (rotation.inverse()
            * (contact_point / contacts as f32 - centre)
                .cross(normal * v.definition.mass * 9.81))
        .z / roll_inertia;
        acceleration.z = roll_to_surface * d.upright_gain * planted
            - gravity_roll
            - local_omega.z * 2. * (d.upright_gain * planted).sqrt();
        // Carve: the lean angle and the bank under it dictate the turn rate,
        // the tyres supply the force, and the friction circle in `handling`
        // decides whether the back end holds or steps out.
        let bank = bank_angle(normal, forward) * d.berm_assist;
        let carve = carve_rate(v.bike.lean, bank, speed);
        // The bars ask for a rate of their own, which does not fall away with
        // speed the way the lean's does. Whichever is asking for more wins, so
        // a slow corner and a berm still ride on their own physics and this
        // only fills the hole at speed.
        let bars = steer_rate(&d, &c, speed);
        // Backing it in: the rear brake trades grip for rotation.
        let pivot = if c.handbrake { d.pivot_boost } else { 1. };
        let want = if bars.abs() > carve.abs() { bars } else { carve } * pivot;
        acceleration.y = (want - local_omega.y) * d.lean_yaw;
        // Point the bike where it is aimed, rather than waiting for the tyres
        // to get it there. Held back while the rear brake is down, which is
        // what makes the pivot slide instead of rail.
        let assist = if c.handbrake { d.grip_assist * 0.15 } else { d.grip_assist };
        if assist > 0. && speed > 1. {
            let rate = want.abs().max(local_omega.y.abs()) * assist;
            let redirected = redirect(body.linvel(), forward, rate * dt);
            body.set_linvel(redirected, true);
        }
        // Just landed: square the bike up under the rider rather than letting
        // a few degrees of yaw become a slide, and let the roll controller
        // above pull harder for the same window.
        if v.bike.landing > 0. {
            let home = heading_error(forward, velocity);
            acceleration.y += home * d.air_level * 2.;
            v.bike.landing = (v.bike.landing - dt).max(0.);
        }

        // Wheelies and stoppies are asked for, never stumbled into. Pitch is
        // measured against the ground rather than the world, so a ramp does
        // not read as a wheelie and this controller does not fight one.
        //
        // Positive `nose_up` is a wheelie and positive `acceleration.x` is
        // nose down, so with `t = nose_up` the target dynamic is
        // `t_dotdot = k(target - t) - c*t_dot`, where `t_dot = -omega.x`.
        let nose_up = forward.dot(normal).clamp(-1., 1.).asin();
        // Rider weight back on the throttle, or a dumped clutch: the clutch-up
        // is how you loft the front over a log or the face of a jump without
        // needing the run-up a weight shift alone would take.
        let clutched = if v.bike.boost > 0. { 0.8 } else { 0. };
        let wheelie = (c.weight.clamp(0., 1.) * (c.throttle * 3.).clamp(0., 1.))
            .max(clutched)
            * (speed - 1.).clamp(0., 1.);
        let stoppie =
            (-c.weight).clamp(0., 1.) * (c.brake * 2.).clamp(0., 1.) * (speed - 3.).clamp(0., 1.);
        let lift = wheelie.max(stoppie);
        let target = if wheelie >= stoppie {
            c.weight.max(0.).max(clutched) * d.wheelie_limit
        } else {
            c.weight.min(0.) * d.stoppie_limit
        };
        // Reaching a commanded angle wants real authority; a planted bike
        // needs only enough to stay planted.
        let gain = 10. + 30. * lift;
        acceleration.x -= (target - nose_up) * gain + local_omega.x * 2. * gain.sqrt();
        // Gravity's restoring moment through the support wheel, fed forward so
        // the balance point is the angle asked for rather than wherever the
        // PD happens to settle.
        let [_, hy, hz] = v
            .definition
            .inertia_half_extents
            .unwrap_or(v.definition.half_extents);
        let pitch_inertia = ((hy * hy + hz * hz) * v.definition.mass / 3.).max(1e-3);
        let support = if wheelie >= stoppie {
            rear_contact
        } else {
            front_contact
        };
        if let Some(p) = support.filter(|_| lift > 0.) {
            let moment = (rotation.inverse()
                * (p - centre).cross(Vector::Y * v.definition.mass * 9.81))
            .x / pitch_inertia;
            acceleration.x -= moment * lift;
        }
        // Loop-out and endo backstops, regardless of what the rider asks.
        if nose_up > d.wheelie_limit + 0.2 {
            acceleration.x += (nose_up - d.wheelie_limit - 0.2) * 120.;
        } else if -nose_up > d.stoppie_limit + 0.2 {
            acceleration.x -= (-nose_up - d.stoppie_limit - 0.2) * 120.;
        }
        // Answer the tyres own pitch moment in full. Drive and brake torque at
        // the contact patch are several times gravity's restoring moment on a
        // bike with real grip, so left alone, opening the throttle is a
        // wheelie and touching the front brake is an endo, neither of which
        // the rider chose. With this, pitch is exactly what was asked for and
        // nothing else -- and terrain still pitches the bike freely, because
        // `nose_up` is measured against the ground rather than the world.
        let local_tire = rotation.inverse() * tire_torque;
        body.apply_torque_impulse(rotation * Vector::new(-local_tire.x, 0., 0.), true);
    } else {
        // Airborne, or down on its side. The lean state has no meaning here
        // and fades so the model comes back to the body.
        v.bike.lean -= v.bike.lean * (1. - (-d.lean_rate * dt).exp());
        v.bike.landing = 0.;
        if contacts >= 1 && !upright {
            // On its side with a wheel still touching. Pick it back up: the
            // grounded roll controller cannot, because it is gated on the
            // chassis already being over its wheels, and without this the
            // bike lies there until `rider_safety` calls it an inversion.
            let roll = (rotation.inverse() * up.cross(normal)).z;
            acceleration.z =
                roll * (d.upright_gain + topple) - local_omega.z * 2. * d.upright_gain.sqrt();
            let _ = topple;
        }
    }
    if contacts >= 1 {
        // Rider weight compresses the suspension. The asymmetric rates are the
        // point: loading is slower than the snap that releases it.
        let want = c.weight.max(0.);
        v.bike.preload += (want - v.bike.preload).clamp(-8. * dt, 4. * dt);
        if v.bike.preload > ARM_FRACTION {
            v.bike.armed = true;
        }
        if v.bike.armed && want < RELEASE_FRACTION {
            // Stored travel becomes a pop. This is what makes jump height a
            // skill rather than a speed lookup.
            body.apply_impulse(normal * v.bike.preload * d.preload_release, true);
            v.bike.armed = false;
            v.bike.preload = 0.;
        } else {
            body.apply_impulse(-normal * v.bike.preload * d.preload_force * dt, true);
        }
    } else {
        v.bike.airborne = true;
        v.bike.air_time += dt;
        // Airborne preload decays rather than surviving the whole jump.
        v.bike.preload -= v.bike.preload.min(4. * dt);
        v.bike.armed = false;
        if v.occupied {
            let roll_error = (rotation.inverse() * up.cross(Vector::Y)).z;
            acceleration = air_acceleration(
                &d,
                &c,
                local_omega,
                heading_error(forward, velocity),
                roll_error,
            );
        }
    }
    if acceleration.length_squared() > 0. {
        let angular_impulse = body.mass_properties().effective_world_inv_inertia.inverse()
            * (rotation * acceleration * dt);
        body.apply_torque_impulse(angular_impulse, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BikeProfile, Controls};

    fn profile() -> BikeProfile {
        BikeProfile {
            enabled: true,
            ..Default::default()
        }
    }

    /// The whole module hangs off the sign table in the header comment. These
    /// pin it down directly, because a flipped sign here is far cheaper to catch
    /// than to diagnose from a bike that leans out of its turns.
    #[test]
    fn a_left_turn_commands_a_left_lean_and_a_left_carve() {
        let d = profile();
        let left = lean_target(
            &d,
            &Controls {
                steering: 1.,
                ..Default::default()
            },
            LEAN_SPEED,
            4.,
        );
        assert!(left < -0.5, "left steer should lean left (negative), got {left}");
        let right = lean_target(
            &d,
            &Controls {
                steering: -1.,
                ..Default::default()
            },
            LEAN_SPEED,
            4.,
        );
        assert!((left + right).abs() < 1e-5, "lean must be symmetric");
        // A left lean carves left, which is positive yaw.
        assert!(carve_rate(left, 0., 10.) > 0.3);
        assert!(carve_rate(right, 0., 10.) < -0.3);
    }

    #[test]
    fn the_same_lean_carves_wider_at_speed_and_not_at_all_at_rest() {
        let slow = carve_rate(-0.6, 0., 8.);
        let fast = carve_rate(-0.6, 0., 24.);
        assert!(slow > fast && fast > 0., "{slow} vs {fast}");
        // A stopped bike does not carve, but that is `lean_target`'s job: it
        // fades the lean out below `LEAN_SPEED`, so there is no lean left to
        // carve with. `carve_rate` used to gate on speed as well, which
        // squared the fade and left the bike with a ninth of its authority at
        // walking pace -- the reason it felt like it would not come round.
        assert_eq!(lean_target(&profile(), &Controls { steering: 1., ..Default::default() }, 0., 0.), 0.);
    }

    #[test]
    fn rider_weight_leans_without_steering_and_adds_to_it() {
        let d = profile();
        let lean_only = lean_target(
            &d,
            &Controls {
                lean: 1.,
                ..Default::default()
            },
            LEAN_SPEED,
            4.,
        );
        assert!(lean_only < -0.1);
        let both = lean_target(
            &d,
            &Controls {
                lean: 1.,
                steering: 1.,
                ..Default::default()
            },
            LEAN_SPEED,
            4.,
        );
        assert!(both <= lean_only, "steer and lean should agree in sign");
    }

    /// A bike lies over to balance a turn, so the angle has to be one the
    /// turn earns. Held at walking pace the bike used to be drawn at 35
    /// degrees for a turn that justified 23, which reads as falling over.
    #[test]
    fn the_lean_never_exceeds_the_angle_the_turn_actually_justifies() {
        let d = profile();
        let full = Controls {
            steering: 1.,
            ..Default::default()
        };
        for (speed, yaw) in [(2., 1.6), (4., 2.8), (6., 2.7)] {
            let lean = lean_target(&d, &full, speed, yaw).abs();
            let justified = (speed * yaw / 9.81).atan();
            assert!(
                lean <= justified + 1e-3,
                "at {speed} m/s turning {yaw}/s the bike leans {lean:.2} for a justified {justified:.2}"
            );
        }
        // And it is inert where it should be: at riding speed the cap is past
        // `lean_max`, so full lean is still available and nothing changed.
        assert!(
            (lean_target(&d, &full, 12., 2.4).abs() - d.lean_max).abs() < 1e-5,
            "the cap must not bite at riding speed"
        );
    }

    /// Turning has to be able to start. The cap is built from the yaw rate
    /// measured last frame, which is zero the instant the bars are turned, so
    /// without a floor the bike could never lean and so could never carve.
    #[test]
    fn a_turn_can_still_be_started_from_no_yaw_at_all() {
        let d = profile();
        let lean = lean_target(
            &d,
            &Controls {
                steering: 1.,
                ..Default::default()
            },
            6.,
            0.,
        );
        assert!(lean < -0.1, "no lean available to start a turn: {lean}");
    }

    #[test]
    fn a_stopped_bike_commands_no_lean_however_the_stick_is_held() {
        let d = profile();
        let a = lean_target(
            &d,
            &Controls {
                lean: 1.,
                steering: 1.,
                ..Default::default()
            },
            0.,
            0.,
        );
        assert_eq!(a, 0.);
    }

    #[test]
    fn air_whip_yaws_toward_the_requested_side_and_lays_the_bike_over() {
        let d = profile();
        let left = air_acceleration(
            &d,
            &Controls {
                whip: 1.,
                ..Default::default()
            },
            Vector::ZERO,
            0.,
            0.,
        );
        // Positive yaw is left; the back end lays over to the left, negative Z.
        assert!(left.y > 1.);
        assert!(left.z < -0.5);
    }

    #[test]
    fn neutral_air_input_straightens_and_levels_for_the_landing() {
        let d = profile();
        // Yawing left with no input: the rate target damps it back down.
        let damping = air_acceleration(&d, &Controls::default(), Vector::new(0., 3., 0.), 0., 0.);
        assert!(damping.y < -1.);
        // Velocity is off to the left of the nose: yaw left to meet it.
        let straighten = air_acceleration(&d, &Controls::default(), Vector::ZERO, 0.5, 0.);
        assert!(straighten.y > 0.5);
        // Leaning left (a positive correction stands it up): roll right.
        let level = air_acceleration(&d, &Controls::default(), Vector::ZERO, 0., 0.4);
        assert!(level.z > 0.5);
        assert_eq!(
            air_acceleration(&d, &Controls::default(), Vector::ZERO, 0., 0.),
            Vector::ZERO
        );
    }

    #[test]
    fn stick_back_in_the_air_is_a_backflip_rate() {
        let d = profile();
        let flip = air_acceleration(
            &d,
            &Controls {
                weight: 1.,
                ..Default::default()
            },
            Vector::ZERO,
            0.,
            0.,
        );
        // Nose up is negative X.
        assert!(flip.x < -1.);
        let scrub = air_acceleration(
            &d,
            &Controls {
                weight: -1.,
                ..Default::default()
            },
            Vector::ZERO,
            0.,
            0.,
        );
        assert!(scrub.x > 1.);
    }

    /// A surface banked to help a left turn has to read with the same sign as
    /// a left lean, or the two would cancel instead of adding.
    #[test]
    fn a_bank_reads_in_the_same_sign_as_the_lean_it_stands_in_for() {
        let forward = Vector::Z;
        // Normal tilted toward the bike's left (+X): banked for a left turn.
        let left_bank = Vector::new(0.5, 0.866, 0.).normalize();
        assert!(
            bank_angle(left_bank, forward) < -0.4,
            "a left-hand berm should read negative like a left lean, got {}",
            bank_angle(left_bank, forward)
        );
        let right_bank = Vector::new(-0.5, 0.866, 0.).normalize();
        assert!(bank_angle(right_bank, forward) > 0.4);
        // Flat ground banks nothing, whichever way the bike points.
        assert!(bank_angle(Vector::Y, forward).abs() < 1e-6);
        assert!(bank_angle(Vector::Y, Vector::X).abs() < 1e-6);
    }

    /// A berm turns harder for the same stick; off-camber pushes wide. This is
    /// the whole feature in one assertion pair.
    #[test]
    fn a_berm_turns_harder_and_off_camber_pushes_wide() {
        let lean = -0.5;
        let flat = carve_rate(lean, 0., 12.);
        let berm = carve_rate(lean, -0.5, 12.);
        let off = carve_rate(lean, 0.5, 12.);
        assert!(berm > flat * 1.5, "a berm should bite: {berm} vs flat {flat}");
        assert!(off < flat * 0.5, "off-camber should wash out: {off} vs {flat}");
        // Even with no lean at all, a berm still turns the bike.
        assert!(carve_rate(0., -0.5, 12.) > 0.3);
    }

    /// The guard that makes the feature shippable. `lean_max` is 60 degrees
    /// and a steep berm is another 40, which lands past the tangent's
    /// singularity at 90. Unclamped, `tan` goes large and negative there and
    /// the bike would snap into turning the wrong way under a rider who has
    /// just committed everything to the corner.
    #[test]
    fn a_committed_rider_on_a_steep_berm_never_turns_the_wrong_way() {
        for speed in [6., 12., 25.] {
            for bank in [-0.4_f32, -0.6, -0.9, -1.4] {
                let rate = carve_rate(-1.05, bank, speed);
                assert!(
                    rate > 0.,
                    "lean -1.05 on a {bank} bank at {speed} m/s turned {rate}"
                );
                assert!(rate.is_finite(), "non-finite carve at bank {bank}");
            }
        }
    }

    #[test]
    fn switching_the_assist_off_reproduces_flat_ground_exactly() {
        // `berm_assist` scales the bank at the call site, so zero means the
        // bank never reaches the carve and today's numbers are unchanged.
        let with_none = carve_rate(-0.5, 0., 12.);
        assert_eq!(carve_rate(-0.5, 0. * -0.6, 12.), with_none);
    }

    /// The whole point of the arcade term: lean physics caps the turn rate at
    /// `g*tan(lean)/v`, which halves every time you double your speed. This
    /// has to still be asking for a real rate where that one has given up.
    #[test]
    fn the_bars_keep_asking_for_a_turn_where_lean_physics_has_given_up() {
        let d = profile();
        let full = Controls { steering: 1., ..Default::default() };
        for speed in [12., 18., 24.] {
            let physics = (9.81 * d.lean_max.tan() / speed).abs();
            let bars = steer_rate(&d, &full, speed).abs();
            assert!(
                bars > physics,
                "at {speed} m/s the bars ask {bars:.2} and lean physics {physics:.2}"
            );
        }
        // And it still fades in from a standstill, so a parked bike is not
        // spun on the spot by the handlebars alone.
        assert_eq!(steer_rate(&d, &full, 0.), 0.);
    }

    #[test]
    fn steering_is_symmetric_and_softened_around_centre() {
        // The profile default is linear; softening is opt-in, and the mod sets
        // it. Ask for it here rather than assume the default carries it.
        let d = crate::BikeProfile { lean_expo: 0.3, ..profile() };
        let at = |x: f32| steer_rate(&d, &Controls { steering: x, ..Default::default() }, 12.);
        assert!((at(1.) + at(-1.)).abs() < 1e-6, "must be symmetric");
        // Expo: a quarter of the stick asks for well under a quarter of the turn.
        assert!(at(0.25).abs() < at(1.).abs() * 0.25);
        // ...and a linear profile is exactly proportional.
        let linear = profile();
        let lin = |x: f32| steer_rate(&linear, &Controls { steering: x, ..Default::default() }, 12.);
        assert!((lin(0.25).abs() - lin(1.).abs() * 0.25).abs() < 1e-6);
    }

    /// Redirection is the mechanic that makes a commanded turn real instead of
    /// a slide. It must never add or remove speed -- only point it somewhere
    /// else -- or it becomes a hidden accelerator.
    #[test]
    fn redirection_turns_the_velocity_without_changing_its_speed() {
        let velocity = Vector::new(6., -2., 6.);
        let forward = Vector::Z;
        let out = redirect(velocity, forward, 0.1);
        let flat = |v: Vector| Vector::new(v.x, 0., v.z).length();
        assert!(
            (flat(out) - flat(velocity)).abs() < 1e-4,
            "speed changed: {} -> {}",
            flat(velocity),
            flat(out)
        );
        assert_eq!(out.y, velocity.y, "vertical motion must be left alone");
        // It moved toward the heading, and not past it.
        let before = Vector::new(velocity.x, 0., velocity.z).normalize().dot(forward);
        let after = Vector::new(out.x, 0., out.z).normalize().dot(forward);
        assert!(after > before, "should have turned toward the heading");
        assert!(after <= 1.0001);
    }

    #[test]
    fn redirection_never_overshoots_or_spins_a_reversing_bike() {
        // A limit far bigger than the error lands exactly on the heading.
        let out = redirect(Vector::new(5., 0., 5.), Vector::Z, 10.);
        assert!(out.x.abs() < 1e-3, "overshot past the heading: {out:?}");
        // Travelling backwards, it must not haul the bike round.
        let back = Vector::new(0., 0., -6.);
        assert_eq!(redirect(back, Vector::Z, 1.), back);
        // Stationary, and with nothing asked for, it is inert.
        assert_eq!(redirect(Vector::ZERO, Vector::Z, 1.), Vector::ZERO);
        let v = Vector::new(3., 0., 3.);
        assert_eq!(redirect(v, Vector::Z, 0.), v);
    }

    #[test]
    fn heading_error_is_signed_left_and_silent_at_walking_pace() {
        let forward = Vector::Z;
        // Velocity to driver-left (+X) is a positive error.
        assert!(heading_error(forward, Vector::new(5., 0., 5.)) > 0.5);
        assert!(heading_error(forward, Vector::new(-5., 0., 5.)) < -0.5);
        assert_eq!(heading_error(forward, Vector::new(1., 0., 1.)), 0.);
    }
}
