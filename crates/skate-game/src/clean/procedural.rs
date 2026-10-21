//! Motion layered on the clean tier's clips after they are sampled (PostUpdate, before transforms
//! propagate), so it sees this frame's pose and nothing lags a frame behind:
//!
//! - the body leans into carves and its hips spring down on a landing (knees take it);
//! - the chest lags the board's accelerations a little (overlapping action);
//! - the board is placed under the feet (lifted with them in an ollie tuck, its tail snapped on the
//!   pop), then both feet are pinned to the deck with two-bone IK so blended poses never slide or
//!   float off it.
use super::sim::CleanSkate;
use super::skater::{self, Avatar, BoardRoot, RiderRoot};
use bevy::math::Affine3A;
use bevy::prelude::*;
use skate_lite_sys::{EventKind, Mode, State};

/// Bones the layer moves, found by name once the skater is bound.
pub(crate) struct Bones {
    hips: Entity,
    chest: Entity,
    /// leg, knee, ankle; front (left) foot first.
    legs: [[Entity; 3]; 2],
    wrists: [Entity; 2],
    /// Everything `SK8_ANIM_TRACE` watches, with names.
    watched: Vec<(Entity, &'static str)>,
}

impl Bones {
    pub(crate) fn find(names: &Query<(Entity, &Name)>) -> Option<Self> {
        let by = |n: &str| names.iter().find(|(_, name)| name.as_str() == n).map(|(e, _)| e);
        const WATCH: [&str; 15] = [
            "hips", "back_mid", "chest", "skull", "leg.L", "knee.L", "ankle.L", "leg.R", "knee.R", "ankle.R", "arm.L",
            "elbow.L", "arm.R", "elbow.R", "base",
        ];
        Some(Self {
            watched: WATCH.iter().filter_map(|n| by(n).map(|e| (e, *n))).collect(),
            hips: by("hips")?,
            chest: by("chest")?,
            legs: [
                [by("leg.L")?, by("knee.L")?, by("ankle.L")?],
                [by("leg.R")?, by("knee.R")?, by("ankle.R")?],
            ],
            wrists: [by("wrist.L")?, by("wrist.R")?],
        })
    }
}

/// Body lean into a full carve, radians.
const CARVE_ROLL: f32 = 0.2;
/// The deck rolls a little on its trucks with the carve, radians.
const BOARD_ROLL: f32 = 0.06;
/// Hips drop per m/s of landing speed (initial velocity of the spring), and the spring itself.
const LAND_KICK: f32 = 0.13;
const HIPS_HZ: f32 = 2.6;
const HIPS_DAMPING: f32 = 0.55;
/// Chest pitch per m/s² of forward / vertical acceleration (radians), and its spring.
const CHEST_GAIN: f32 = 0.012;
const CHEST_MAX: f32 = 0.22;
const CHEST_HZ: f32 = 3.0;
const CHEST_DAMPING: f32 = 0.6;
/// A foot this far above the deck (m, sole) is meant to be off it and is left alone.
const PIN_REACH: f32 = 0.09;
/// Seconds for a foot pin to take hold or let go.
const PIN_BLEND: f32 = 0.06;
/// The foot stays on the deck: at most this far off its centre line, and inside the trucks' span.
const PIN_HALF_WIDTH: f32 = 0.07;
const PIN_HALF_LEN: f32 = 0.36;
/// Room left between the feet and a flipping board (m).
const FLIP_ROOM: f32 = 0.09;
/// The pushing foot: seconds to take hold of the ground at the plant, and to lift off it (rising
/// this far, m) and ease back into the clip's swing; it lets go early past this share of the leg's
/// reach. A foot leaving the ground keeps going back with the ground's speed, losing it in LIFT_CARRY s.
const LIFT_CARRY: f32 = 0.05;
const PLANT_IN: f32 = 0.07;
const LIFT_TIME: f32 = 0.14;
const LIFT_HEIGHT: f32 = 0.05;
const PLANT_REACH: f32 = 0.96;

/// The pushing foot's hold on the ground.
#[derive(Default, Clone, Copy)]
enum Ground {
    #[default]
    Off,
    /// Standing at `at` (world), taking hold over PLANT_IN.
    Planted { at: Vec3, w: f32 },
    /// Lifting off from `rel` (rider frame, so it rides along with the board), `t` s ago, still
    /// carried back at `vel` (rider frame) as it leaves the ground, slowing.
    Lifting { rel: Vec3, vel: Vec3, t: f32 },
}

#[derive(Default)]
pub(crate) struct Motion {
    hips: f32,
    hips_vel: f32,
    chest: Vec2,
    chest_vel: Vec2,
    last_vel: Option<Vec3>,
    air_vy: f32,
    pin: [f32; 2],
    ground: Ground,
    /// This stroke's foot has planted already (one plant per stroke).
    planted: bool,
    /// 0..1: how much the board is held in the hands (grabs where both feet leave it).
    held: f32,
    /// Where the ground was when a hippie jump left it (the board rolls on along it).
    hippie_ground: Option<f32>,
    /// 0..1: how far a tilted grab (method, rocket) has turned the deck, and which one.
    tilt: f32,
    tilt_style: u32,
    /// How far the board rides up under the feet in the air (m), and its rate.
    lift: f32,
    lift_vel: f32,
    /// The board after a bail: flung on from the rider's speed, tumbling, bouncing to rest.
    loose: Option<Loose>,
    trace: Option<Trace>,
}

struct Loose {
    at: Vec3,
    vel: Vec3,
    rot: Quat,
    spin: Vec3,
    ground: f32,
}

impl Loose {
    fn step(&mut self, dt: f32) {
        self.vel.y -= 9.8 * dt;
        self.at += self.vel * dt;
        let s = self.spin * dt;
        self.rot = (Quat::from_scaled_axis(s) * self.rot).normalize();
        if self.at.y < self.ground {
            self.at.y = self.ground;
            if self.vel.y < 0.0 {
                self.vel.y *= -0.3;
            }
            // Rolling and scraping to a stop, settling wheels-down (or on its back).
            self.vel.x *= (1.0 - 3.0 * dt).max(0.0);
            self.vel.z *= (1.0 - 3.0 * dt).max(0.0);
            self.spin *= (1.0 - 6.0 * dt).max(0.0);
            let up = self.rot * Vec3::Y;
            let flat = if up.y >= 0.0 { Vec3::Y } else { Vec3::NEG_Y };
            let settle = Quat::from_rotation_arc(up, flat);
            self.rot = Quat::IDENTITY.slerp(settle, (8.0 * dt).min(1.0)) * self.rot;
        }
    }
}

/// `SK8_ANIM_TRACE=1`: how fast every watched bone turns (degrees per 1/60 s, rider frame) and how
/// far a pinned foot slides on the deck, flagging spikes with the mode they happened in.
#[derive(Default)]
struct Trace {
    /// Frames traced so far: the first half second (the rest pose giving way to the clips) is skipped.
    frames: u32,
    last: Vec<Quat>,
    last_local: Vec<Quat>,
    feet: [Option<Vec3>; 2],
    worst: (f32, &'static str, String),
    slip: f32,
    /// How far a planted pushing foot ends up from its spot on the ground (m).
    ground: f32,
    since_report: f32,
}

const SPIKE_DEG: f32 = 14.0;

/// Above this a one-frame turn is logged with the clips playing (SK8_JUMP_DEG, default 42).
fn jump_deg() -> f32 {
    std::env::var("SK8_JUMP_DEG").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0 * SPIKE_DEG)
}
const SLIP_M: f32 = 0.012;

/// Damped spring towards `target`, semi-implicitly (stable at frame rates down to ~20 Hz for these
/// frequencies).
fn spring(x: &mut f32, v: &mut f32, target: f32, hz: f32, damping: f32, dt: f32) {
    let w = std::f32::consts::TAU * hz;
    *v += (-w * w * (*x - target) - 2.0 * damping * w * *v) * dt;
    *x += *v * dt;
}

/// `e`'s transform in the space the rider and board live in (walking up to the rider root).
fn in_space(e: Entity, rider: Entity, q: &Query<(&mut Transform, Option<&ChildOf>), Without<BoardRoot>>) -> Option<Affine3A> {
    let mut m = Affine3A::IDENTITY;
    let mut cur = e;
    loop {
        let (t, parent) = q.get(cur).ok()?;
        m = t.compute_affine() * m;
        if cur == rider {
            return Some(m);
        }
        cur = parent?.parent();
    }
}

fn rot(m: &Affine3A) -> Quat {
    m.to_scale_rotation_translation().1
}

/// Give `e` the rotation `want` (in rider space) by changing its local rotation only.
fn set_rotation(e: Entity, parent: &Affine3A, want: Quat, q: &mut Query<(&mut Transform, Option<&ChildOf>), Without<BoardRoot>>) {
    if let Ok((mut t, _)) = q.get_mut(e) {
        t.rotation = (rot(parent).inverse() * want).normalize();
    }
}

/// Replace `e`'s transform so it ends up at `want` (rider space).
fn set_space(e: Entity, parent: &Affine3A, want: Affine3A, q: &mut Query<(&mut Transform, Option<&ChildOf>), Without<BoardRoot>>) {
    if let Ok((mut t, _)) = q.get_mut(e) {
        let scale = t.scale;
        let mut local = Transform::from_matrix(Mat4::from(parent.inverse() * want));
        local.scale = scale;
        *t = local;
    }
}

/// Bend a leg (hip `a`, knee, ankle) so the ankle reaches `target`, keeping the knee in the plane
/// it already points in and the foot's own orientation (turned by `foot_turn`).
fn two_bone(
    chain: [Entity; 3],
    target: Vec3,
    foot_turn: Quat,
    rider: Entity,
    q: &mut Query<(&mut Transform, Option<&ChildOf>), Without<BoardRoot>>,
) -> Option<()> {
    let [leg, knee, ankle] = chain;
    let leg_m = in_space(leg, rider, q)?;
    let knee_m = in_space(knee, rider, q)?;
    let ankle_m = in_space(ankle, rider, q)?;
    let parent_of = |e: Entity, q: &Query<(&mut Transform, Option<&ChildOf>), Without<BoardRoot>>| {
        let (_, p) = q.get(e).ok()?;
        in_space(p?.parent(), rider, q)
    };
    let hip_parent = parent_of(leg, q)?;
    let (a, b, c) = (leg_m.translation.into(), knee_m.translation.into(), ankle_m.translation.into());
    let (a, b, c): (Vec3, Vec3, Vec3) = (a, b, c);
    let (l1, l2) = (b.distance(a), c.distance(b));
    let to = target - a;
    let d = to.length().clamp(1e-3, (l1 + l2) * 0.999);
    let dir = to.normalize_or(Vec3::NEG_Y);
    // The knee keeps bending the way it does now.
    let bend = (b - a) - dir * (b - a).dot(dir);
    let bend = if bend.length_squared() > 1e-8 { bend.normalize() } else { dir.any_orthonormal_vector() };
    let along = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let knee_at = a + dir * along + bend * (l1 * l1 - along * along).max(0.0).sqrt();

    let turn_leg = Quat::from_rotation_arc((b - a).normalize(), (knee_at - a).normalize());
    let leg_rot = turn_leg * rot(&leg_m);
    set_rotation(leg, &hip_parent, leg_rot, q);
    let leg_new = in_space(leg, rider, q)?;
    let ankle_moved = a + turn_leg * (c - a);
    let turn_knee = Quat::from_rotation_arc((ankle_moved - knee_at).normalize(), (target - knee_at).normalize());
    let knee_rot = turn_knee * turn_leg * rot(&knee_m);
    set_rotation(knee, &leg_new, knee_rot, q);
    let knee_new = in_space(knee, rider, q)?;
    set_rotation(ankle, &knee_new, foot_turn * rot(&ankle_m), q);
    Some(())
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn layer(
    time: Res<Time<Real>>,
    sim: Res<CleanSkate>,
    avatar: Res<Avatar>,
    clock: Res<skater::Clock>,
    mut motion: Local<Motion>,
    riders: Query<Entity, With<RiderRoot>>,
    mut bones: Query<(&mut Transform, Option<&ChildOf>), Without<BoardRoot>>,
    mut board: Query<&mut Transform, With<BoardRoot>>,
) {
    let dt = if sim.paused { 0.0 } else { time.delta_secs().min(0.1) * sim.time_scale };
    let (a, b, t) = (&sim.previous, &sim.current, sim.alpha());
    let s: &State = b;
    let pos = Vec3::from_array(a.position).lerp(Vec3::from_array(b.position), t);
    let rider_rot = Quat::from_array(a.rider_rot).slerp(Quat::from_array(b.rider_rot), t);
    // A board is the same either way round: when the engine snaps a shove-it's half turn back on
    // landing, blend from the equivalent orientation instead of sweeping half a turn in one frame.
    let (qa, qb) = (Quat::from_array(a.board_rot), Quat::from_array(b.board_rot));
    let qa = if ((qa.inverse() * qb) * Vec3::Z).z < -0.5 { qa * Quat::from_rotation_y(std::f32::consts::PI) } else { qa };
    let board_rot = qa.slerp(qb, t);
    let lean = a.lean + (b.lean - a.lean) * t;
    let up = rider_rot * Vec3::Y;
    let fwd = rider_rot * Vec3::Z;
    let vel = Vec3::from_array(a.velocity).lerp(Vec3::from_array(b.velocity), t);

    if sim.events.iter().any(|e| e.kind == EventKind::Respawn) {
        // The trace starts over too: a cut is not a spike.
        let trace = motion.trace.take().map(|t| Trace { last: Vec::new(), feet: [None; 2], ..t });
        *motion = Motion { trace, pin: [1.0; 2], ..default() };
    }

    // Springs.
    if s.mode == Mode::Air {
        motion.air_vy = vel.dot(up).min(0.0);
    }
    if sim.events.iter().any(|e| e.kind == EventKind::Land) {
        motion.hips_vel -= LAND_KICK * (-motion.air_vy).min(8.0);
    }
    let (mut h, mut hv) = (motion.hips, motion.hips_vel);
    spring(&mut h, &mut hv, 0.0, HIPS_HZ, HIPS_DAMPING, dt);
    (motion.hips, motion.hips_vel) = (h.clamp(-0.14, 0.04), hv);
    let accel = match motion.last_vel {
        Some(last) if dt > 0.0 => (vel - last) / dt,
        _ => Vec3::ZERO,
    };
    if dt > 0.0 {
        motion.last_vel = Some(vel);
    }
    // Speeding up tips the chest back, slowing down (or a landing) throws it forward.
    let grounded = s.mode != Mode::Air && s.mode != Mode::Bail;
    let chest_goal = if grounded {
        Vec2::new((-accel.dot(fwd) - accel.dot(up) * 0.5) * CHEST_GAIN, 0.0).clamp_length_max(CHEST_MAX)
    } else {
        Vec2::ZERO
    };
    for i in 0..2 {
        let (mut x, mut v) = (motion.chest[i], motion.chest_vel[i]);
        spring(&mut x, &mut v, chest_goal[i], CHEST_HZ, CHEST_DAMPING, dt);
        motion.chest[i] = x;
        motion.chest_vel[i] = v;
    }

    let (Some(set), Ok(rider)) = (avatar.bones.as_ref(), riders.single()) else { return };
    let rider_at = pos + up * skater::DECK_TOP;

    // The body: lean into the carve about the line between the feet, and the landing spring.
    let carving = matches!(s.mode, Mode::Ride | Mode::Slide) || (s.mode == Mode::Manual);
    let roll = if carving { -lean * CARVE_ROLL } else { 0.0 };
    if let (Some(hips_m), Some(parent)) = (
        in_space(set.hips, rider, &bones),
        bones.get(set.hips).ok().and_then(|(_, p)| p).and_then(|p| in_space(p.parent(), rider, &bones)),
    ) {
        let pivot = rider_at;
        let lean_q = Quat::from_axis_angle(fwd, roll);
        let want = Affine3A::from_translation(pivot + up * motion.hips)
            * Affine3A::from_quat(lean_q)
            * Affine3A::from_translation(-pivot)
            * hips_m;
        set_space(set.hips, &parent, want, &mut bones);
    }
    if let (Some(chest_m), Some(parent)) = (
        in_space(set.chest, rider, &bones),
        bones.get(set.chest).ok().and_then(|(_, p)| p).and_then(|p| in_space(p.parent(), rider, &bones)),
    ) {
        let side = rider_rot * Vec3::X;
        let pitch = Quat::from_axis_angle(side, motion.chest.x);
        set_rotation(set.chest, &parent, pitch * rot(&chest_m), &mut bones);
    }

    // The board, from this frame's feet.
    let ankles: Vec<Vec3> = set.legs.iter().filter_map(|l| in_space(l[2], rider, &bones)).map(|m| m.translation.into()).collect();
    let flipping = s.flip_progress.is_some_and(|p| p < 0.8);
    let mut at = pos + up * skater::DECK_CENTRE;
    // Where the clips assume the deck is: flat under the rider, no lift, snap, roll or flip.
    let rest_inv = Transform::from_translation(at).with_rotation(rider_rot).compute_affine().inverse();
    // In the air the board rides up under the feet (a tuck brings it with them); while it flips it
    // keeps a little room to turn. The height springs, so a flip starting or ending never snaps it.
    use skate_lite_sys::grab_style as gs;
    let tilting = s.mode == Mode::Air && matches!(s.grab_style, gs::METHOD | gs::ROCKET);
    if tilting {
        motion.tilt_style = s.grab_style;
    }
    if dt > 0.0 {
        // turned over the grab's reach; flat again at once on the ground (no landing on a tilted deck)
        let k = 1.0 - (-dt / if s.mode == Mode::Air { 0.09 } else { 0.02 }).exp();
        motion.tilt += ((tilting as u8 as f32) - motion.tilt) * k;
    }
    let lift_goal = if s.mode == Mode::Air {
        // a tilted deck turns about its centre, which sits between the feet, not under the lower one
        let heights: Vec<f32> = ankles.iter().map(|p| (*p - rider_at).dot(up)).collect();
        let mean = heights.iter().sum::<f32>() / heights.len().max(1) as f32;
        let low = heights.iter().copied().fold(f32::MAX, f32::min);
        let lowest = low + (mean - low) * motion.tilt;
        let room = if s.flip_progress.is_some_and(|p| p < 0.85) { FLIP_ROOM } else { 0.0 };
        if lowest < 1.0 { (lowest - skater::ANKLE - room).max(0.0) } else { 0.0 }
    } else {
        0.0
    };
    if dt > 0.0 {
        // Exact critically damped follow: a stiff explicit spring here diverged at 60 Hz.
        let (l, lv) = skater::smooth_cd(motion.lift, motion.lift_vel, lift_goal, 0.035, dt);
        (motion.lift, motion.lift_vel) = (l.max(0.0), lv);
    }
    if s.mode == Mode::Air {
        at += up * motion.lift;
    } else {
        // Landed: the board is on the ground at once (the spring only smooths the air).
        motion.lift = 0.0;
        motion.lift_vel = 0.0;
    }
    // No Foot and Superman: both feet leave the board, which hangs from the hands on its edges.
    // on foot, or planted on an edge: the board is in the back hand
    let walking = s.mode == Mode::Walk || (s.mode == Mode::Grind && s.grind_kind >= 15);
    let hand_held =
        walking || (s.mode == Mode::Air && matches!(s.grab_style, gs::NO_FOOT | gs::SUPERMAN | gs::CHRIST));
    // which hands hold it: both, or the back hand alone (Christ air out to the side, carried on foot)
    let one_hand = s.grab_style == gs::CHRIST || walking;
    if dt > 0.0 {
        let k = 1.0 - (-dt / 0.08).exp();
        motion.held += ((hand_held as u8 as f32) - motion.held) * k;
    }
    if motion.held > 1e-3 {
        let wrists: Vec<Vec3> = set.wrists.iter().filter_map(|e| in_space(*e, rider, &bones)).map(|m| m.translation.into()).collect();
        if wrists.len() == 2 {
            let grip = if one_hand { wrists[1] - up * 0.06 } else { (wrists[0] + wrists[1]) * 0.5 - up * 0.04 };
            at = at.lerp(grip, motion.held);
        }
    }
    // Hippie jump: the board stays down on the ground under the jump, rolling on (the feet are off it)
    let hippie = s.mode == Mode::Air && clock.hippie;
    if sim.events.iter().any(|e| e.kind == EventKind::Pop) {
        motion.hippie_ground = Some(pos.dot(up));
    }
    if hippie {
        if let Some(g) = motion.hippie_ground {
            // under the rider, back on the ground: none of the air's lift to the feet
            at = pos + up * skater::DECK_CENTRE - up * (pos.dot(up) - g).max(0.0);
        }
    }
    let mut deck = board_rot;
    // carried on foot: on its edge at the side, nose ahead
    if walking {
        deck = rider_rot * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    }
    if carving {
        deck = Quat::from_axis_angle(fwd, -lean * BOARD_ROLL) * deck;
    }
    if motion.tilt > 1e-3 {
        // method: heel edge up behind (about the deck's length); rocket: nose up into the hands
        let turn = if motion.tilt_style == gs::METHOD {
            Quat::from_rotation_z(skater::METHOD_ROLL.to_radians() * motion.tilt)
        } else {
            Quat::from_rotation_x(-skater::ROCKET_PITCH.to_radians() * motion.tilt)
        };
        deck = deck * turn;
    }
    if s.mode == Mode::Air {
        let (shift, pitched) = skater::pop_snap(deck, clock.since_pop, clock.nollie);
        at += shift;
        deck = pitched;
    }
    if s.mode == Mode::Bail {
        if motion.loose.is_none() {
            // Kicked out ahead and to the toe side, flipping over.
            motion.loose = Some(Loose {
                at,
                vel: vel * 0.9 + up * 1.6 - rider_rot * Vec3::X * 0.8,
                rot: deck,
                spin: fwd * 9.0 + up * 4.0,
                ground: pos.y,
            });
        }
        if let Some(l) = motion.loose.as_mut() {
            // The ground where the rider's body is (the engine keeps it on the surface).
            l.ground = pos.y + skater::DECK_CENTRE;
            l.step(dt);
            at = l.at;
            deck = l.rot;
        }
    } else {
        motion.loose = None;
    }
    let deck_tf = Transform::from_translation(at).with_rotation(deck);
    if let Ok(mut tf) = board.single_mut() {
        *tf = deck_tf;
    }

    // Feet onto the deck.
    // Feet attach to the deck as the rider sees it: during a flip (catch included) that is an
    // un-flipped deck in the same place, or the feet would twist with the spinning board and, after a
    // shove-it, land on the wrong ends of it.
    let pin_tf = if s.flip_progress.is_some() {
        Transform::from_translation(at).with_rotation(rider_rot)
    } else {
        // A board is the same either way round: a deck turned more than a quarter turn from the
        // rider (after a shove-it) is used half a turn back, so the feet keep their ends.
        let rel = rider_rot.inverse() * deck;
        let nose = rel * Vec3::Z;
        let rel = if nose.z < 0.0 { rel * Quat::from_rotation_y(std::f32::consts::PI) } else { rel };
        // upside down (a darkslide): the feet stand on its underside, which faces up
        let up_side = (rider_rot * rel * Vec3::Y).dot(up);
        let rel = if up_side < 0.0 { rel * Quat::from_rotation_z(std::f32::consts::PI) } else { rel };
        Transform::from_translation(at).with_rotation(rider_rot * rel)
    };
    let pusher = if s.push_mongo { 0 } else { 1 };
    let deck_m = pin_tf.compute_affine();
    let deck_inv = deck_m.inverse();
    let sole = skater::DECK_TOP - skater::DECK_CENTRE;
    // Pinned feet turn with the deck (manual pitch, carve roll, powerslide yaw, the pop).
    let tilt = pin_tf.rotation * rider_rot.inverse();
    for (i, chain) in set.legs.iter().enumerate() {
        let Some(ankle) = ankles.get(i).copied() else { continue };
        let height = deck_inv.transform_point3(ankle).y - sole - skater::ANKLE;
        // The foot rides with the deck: its spot on the deck the clip meant, carried to where the
        // deck really is (so a powerslide, manual or pop turns the feet with it instead of sliding).
        let mut local = rest_inv.transform_point3(ankle);
        let wanted = match s.mode {
            Mode::Bail | Mode::Walk => false,
            Mode::Grind if s.grind_kind >= 15 => false,
            // The board is in the hands: the feet are off it.
            Mode::Air if motion.held > 0.5 || hippie => false,
            Mode::Air => !flipping,
            // The pushing foot is on the ground (the back foot, or the front one pushing mongo), and
            // lets go of the deck as it steps off.
            Mode::Ride if s.push_phase.is_some() || s.push_start.is_some_and(|k| k > 0.4) => i != pusher,
            _ => true,
        } && height < PIN_REACH
            // only over the deck: a foot stepping back on from beside the board isn't dragged across
            && local.x.abs() < PIN_HALF_WIDTH + 0.03
            && height > -0.2;
        let k = 1.0 - (-dt / PIN_BLEND).exp();
        motion.pin[i] += ((wanted as u8 as f32) - motion.pin[i]) * if dt > 0.0 { k } else { 0.0 };
        let w = motion.pin[i];
        if w < 1e-3 {
            continue;
        }
        local.x = local.x.clamp(-PIN_HALF_WIDTH, PIN_HALF_WIDTH);
        local.z = local.z.clamp(-PIN_HALF_LEN, PIN_HALF_LEN);
        // on the deck's real surface: concave across, rising up the nose and tail
        let u = local.x / (super::board::DECK_WIDTH * 0.5);
        local.y = super::board::deck_top(local.z, u) + skater::ANKLE;
        let target = ankle.lerp(deck_m.transform_point3(local), w);
        two_bone(*chain, target, Quat::IDENTITY.slerp(tilt, w), rider, &mut bones);
    }

    // The pushing foot on the ground: from the plant it stays where it touched down while the board
    // rolls on (the stroke gets longer the faster the board goes), then lifts off from wherever it
    // got to and eases back into the clip's swing. The clip's own stroke is one fixed length, which
    // would skate along the ground at every speed but one.
    let phase = s.push_phase.filter(|_| s.mode == Mode::Ride);
    if phase.is_none_or(|p| p < skate_lite_sys::PUSH_PLANT) {
        motion.planted = false;
    }
    let in_drive = phase.is_some_and(|p| (skate_lite_sys::PUSH_PLANT..skate_lite_sys::PUSH_RELEASE).contains(&p));
    let chain = set.legs[pusher];
    let mut ground_err = None;
    if let (Some(hip), Some(knee), Some(ankle)) =
        (in_space(chain[0], rider, &bones), in_space(chain[1], rider, &bones), in_space(chain[2], rider, &bones))
    {
        let (hip, knee, ankle): (Vec3, Vec3, Vec3) = (hip.translation.into(), knee.translation.into(), ankle.translation.into());
        let reach = (hip.distance(knee) + knee.distance(ankle)) * PLANT_REACH;
        let ground_at = |p: Vec3| p - up * (p - pos).dot(up) + up * skater::ANKLE;
        let inv = rider_rot.inverse();
        let lift_from = |at: Vec3| Ground::Lifting { rel: inv * (at - rider_at), vel: -(inv * vel), t: 0.0 };
        motion.ground = match motion.ground {
            Ground::Off if in_drive && !motion.planted && dt > 0.0 => {
                motion.planted = true;
                Ground::Planted { at: ground_at(ankle), w: 0.0 }
            }
            Ground::Planted { at, .. } if !in_drive || hip.distance(at) > reach => lift_from(at),
            Ground::Planted { at, w } => Ground::Planted { at, w: (w + dt / PLANT_IN).min(1.0) },
            Ground::Lifting { t, .. } if t >= LIFT_TIME => Ground::Off,
            Ground::Lifting { rel, vel, t } => {
                Ground::Lifting { rel: rel + vel * dt, vel: vel * (-dt / LIFT_CARRY).exp(), t: t + dt }
            }
            g => g,
        };
        // A bail, a pop or the end of pushing lets go at once.
        if phase.is_none() {
            motion.ground = Ground::Off;
        }
        let target = match motion.ground {
            Ground::Planted { at, w } => Some((at, skater::smooth01(w))),
            Ground::Lifting { rel, t, .. } => {
                let k = (t / LIFT_TIME).clamp(0.0, 1.0);
                let at = rider_at + rider_rot * rel + up * LIFT_HEIGHT * (std::f32::consts::PI * k).sin().max(0.0) * (1.0 - k * 0.5);
                Some((at, 1.0 - skater::smooth01(k)))
            }
            Ground::Off => None,
        };
        if let Some((at, w)) = target {
            if w > 1e-3 {
                two_bone(chain, ankle.lerp(at, w), Quat::IDENTITY, rider, &mut bones);
            }
            if let (Ground::Planted { at, w }, Some(m)) = (motion.ground, in_space(chain[2], rider, &bones)) {
                if w >= 1.0 {
                    ground_err = Some(Vec3::from(m.translation).distance(at));
                }
            }
        }
    }

    if motion.trace.is_none() && std::env::var_os("SK8_ANIM_TRACE").is_some() {
        motion.trace = Some(Trace::default());
    }
    if dt > 0.0 {
        let pin = motion.pin;
        if let (Some(tr), Some(root)) = (motion.trace.as_mut(), in_space(rider, rider, &bones)) {
            let to_rider = rot(&root).inverse();
            let now: Vec<Quat> =
                set.watched.iter().map(|(e, _)| in_space(*e, rider, &bones).map_or(Quat::IDENTITY, |m| to_rider * rot(&m))).collect();
            let local: Vec<Quat> =
                set.watched.iter().map(|(e, _)| bones.get(*e).map_or(Quat::IDENTITY, |(t, _)| t.rotation)).collect();
            let r2 = |v: Option<f32>| v.map(|p| (p * 100.0).round() / 100.0);
            let mode = format!("{:?} flip={:?} push={:?} start={:?} mongo={} v={:.1} t={:.2}", s.mode, r2(s.flip_progress),
                r2(s.push_phase), r2(s.push_start), s.push_mongo, s.speed, tr.frames as f32 / 60.0);
            tr.frames += 1;
            if tr.last.len() == now.len() && tr.frames > 30 {
                for (k, (q, last)) in now.iter().zip(&tr.last).enumerate() {
                    let deg = q.angle_between(*last).to_degrees() / (dt * 60.0);
                    if deg > SPIKE_DEG {
                        info!("SK8 anim spike {} {:.1} deg/frame in {}", set.watched[k].1, deg, mode);
                    }
                    if deg > jump_deg() {
                        // A jump this big in one frame is a discontinuity: show what was playing.
                        let mut w: Vec<(f32, usize)> =
                            avatar.weights.iter().enumerate().map(|(i, w)| (*w, i)).filter(|(w, _)| *w > 0.01).collect();
                        w.sort_by(|a, b| b.0.total_cmp(&a.0));
                        let playing: Vec<String> = w
                            .iter()
                            .map(|(w, i)| format!("{}={:.2}@{:.2}", skater::CLIP_NAMES[*i], w, avatar.times[*i]))
                            .collect();
                        info!("SK8 anim jump clips: {}", playing.join(" "));
                        if let Some(prev) = tr.last_local.get(k) {
                            info!("SK8 anim jump local {} {:?} -> {:?} ({:.0} deg)", set.watched[k].1, prev, local[k],
                                prev.angle_between(local[k]).to_degrees());
                        }
                    }
                    if deg > tr.worst.0 {
                        tr.worst = (deg, set.watched[k].1, mode.clone());
                    }
                }
            }
            tr.last = now;
            tr.last_local = local;
            for (i, chain) in set.legs.iter().enumerate() {
                let foot = in_space(chain[2], rider, &bones).map(|m| deck_inv.transform_point3(m.translation.into()));
                if let (Some(f), Some(prev)) = (foot, tr.feet[i]) {
                    if pin[i] > 0.95 && s.mode != Mode::Air {
                        let d = f.distance(prev);
                        tr.slip = tr.slip.max(d);
                        if d > SLIP_M {
                            info!("SK8 anim slip foot {} {:.3} m in {}", i, d, mode);
                        }
                    }
                }
                tr.feet[i] = foot;
            }
            if let Some(e) = ground_err {
                tr.ground = tr.ground.max(e);
                if e > SLIP_M {
                    info!("SK8 anim ground slip {:.3} m (planted foot off its spot) at speed {:.1}", e, s.speed);
                }
            }
            tr.since_report += dt;
            if tr.since_report > 2.0 {
                info!("SK8 anim worst {:.1} deg/frame ({} in {}), max pinned slip {:.3} m, ground slip {:.3} m",
                    tr.worst.0, tr.worst.1, tr.worst.2, tr.slip, tr.ground);
                tr.ground = 0.0;
                tr.since_report = 0.0;
                tr.worst = (0.0, "", String::new());
                tr.slip = 0.0;
            }
        }
    }
}
