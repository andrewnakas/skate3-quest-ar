//! The clean tier's skater and board, built in code: a jointed figure of
//! simple segments (own proportions and naming) posed procedurally from the
//! engine's state every frame, and a deck with trucks and wheels.
//!
//! Posing works in the rider's frame: origin on the deck top between the
//! feet, +Y up, +Z the way the chest faces (the board's toe side), +X toward
//! the tail. The feet are placed on the deck (or the ground, when pushing)
//! and the legs solved with two-bone IK; the body and arms come from a few
//! continuous pose parameters that are eased toward targets read from the
//! state, so every transition blends on its own.
use super::sim::CleanSkate;
use bevy::prelude::*;
use skate_lite_sys::{Mode, State};

// ---- figure proportions (metres) ----
const DECK_TOP: f32 = 0.105;
const THIGH: f32 = 0.43;
const SHIN: f32 = 0.415;
const ANKLE_UP: f32 = 0.07;
const HIP_HALF: f32 = 0.095;
const TORSO: f32 = 0.5;
const NECK_TO_HEAD: f32 = 0.145;
const SHOULDER_HALF: f32 = 0.19;
const SHOULDER_DROP: f32 = 0.07;
const UPPER_ARM: f32 = 0.29;
const FOREARM: f32 = 0.27;
const FOOT_LEN: f32 = 0.27;
/// Feet on the deck, along the rider's X (front foot toward the nose, -X).
const FRONT_FOOT_X: f32 = -0.21;
const BACK_FOOT_X: f32 = 0.24;

// ---- board (metres, board frame: +Z nose, origin under the wheels) ----
const DECK_LEN: f32 = 0.8;
const DECK_WIDTH: f32 = 0.205;
const DECK_THICK: f32 = 0.013;
const KICK_LEN: f32 = 0.13;
const KICK_ANGLE: f32 = 0.33;
const WHEEL_R: f32 = 0.027;
const AXLE_Z: f32 = 0.235;
const DECK_CENTRE: f32 = 0.09;

/// How fast the pose eases toward its target, 1/s.
const EASE: f32 = 14.0;

#[derive(Component)]
pub(crate) struct RiderRoot;
#[derive(Component)]
pub(crate) struct BoardRoot;
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Part {
    Pelvis,
    Torso,
    Head,
    Cap,
    UpperArm(Side),
    Forearm(Side),
    Hand(Side),
    Thigh(Side),
    Shin(Side),
    Foot(Side),
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Side {
    Front,
    Back,
}

/// The eased pose parameters.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct Pose {
    hip_height: f32,
    hip_x: f32,
    hip_z: f32,
    bend: f32,
    side_lean: f32,
    twist: f32,
    arms_out: f32,
    reach_front: f32,
    reach_back: f32,
    front_lift: f32,
    back_lift: f32,
    tumble: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            hip_height: 0.86,
            hip_x: 0.02,
            hip_z: 0.0,
            bend: 0.22,
            side_lean: 0.0,
            twist: 0.0,
            arms_out: 0.2,
            reach_front: 0.0,
            reach_back: 0.0,
            front_lift: 0.0,
            back_lift: 0.0,
            tumble: 0.0,
        }
    }
}

impl Pose {
    fn ease(&mut self, target: &Pose, k: f32) {
        let f = |a: &mut f32, b: f32| *a += (b - *a) * k;
        f(&mut self.hip_height, target.hip_height);
        f(&mut self.hip_x, target.hip_x);
        f(&mut self.hip_z, target.hip_z);
        f(&mut self.bend, target.bend);
        f(&mut self.side_lean, target.side_lean);
        f(&mut self.twist, target.twist);
        f(&mut self.arms_out, target.arms_out);
        f(&mut self.reach_front, target.reach_front);
        f(&mut self.reach_back, target.reach_back);
        f(&mut self.front_lift, target.front_lift);
        f(&mut self.back_lift, target.back_lift);
        f(&mut self.tumble, target.tumble);
    }
}

/// Where the pose wants to be for this state.
fn target_pose(s: &State, since_pop: f32) -> Pose {
    let mut p = Pose::default();
    let lean = s.lean.clamp(-1.0, 1.0);
    p.side_lean = 0.22 * lean;
    p.hip_z = 0.05 * lean;
    let crouch = s.crouch;
    p.hip_height = 0.86 - 0.24 * crouch;
    p.bend = 0.22 + 0.38 * crouch;
    match s.mode {
        Mode::Ride => {
            if let Some(phase) = s.push_phase {
                // The back foot comes off and kicks along the ground.
                p.back_lift = -1.0 - phase; // negative = ground push, see `back_foot`
                p.hip_height = 0.8;
                p.bend = 0.35;
                p.twist = -0.25;
            }
        }
        Mode::Manual => {
            p.hip_x = if s.balance >= 0.0 { 0.12 } else { -0.13 };
            p.bend = 0.12;
            p.arms_out = 0.7;
            p.side_lean += 0.25 * s.balance;
        }
        Mode::Slide => {
            p.hip_height = 0.7;
            p.bend = -0.15;
            p.twist = 0.5 * lean.signum();
            p.arms_out = 0.6;
        }
        Mode::Air => {
            if since_pop < 0.12 {
                // Snap up off the tail.
                p.hip_height = 0.93;
                p.bend = 0.1;
                p.back_lift = 0.05;
                p.front_lift = 0.12;
            } else {
                p.hip_height = 0.68;
                p.bend = 0.45;
                p.arms_out = 0.45;
            }
            if let Some(f) = s.flip_progress {
                let lift = (f * std::f32::consts::PI).sin();
                p.front_lift = 0.24 * lift + 0.04;
                p.back_lift = 0.18 * lift + 0.03;
                p.hip_height = 0.72;
            }
            if s.grab & 1 != 0 {
                p.reach_front = 1.0;
                p.hip_height = 0.58;
                p.bend = 0.75;
            }
            if s.grab & 2 != 0 {
                p.reach_back = 1.0;
                p.hip_height = 0.58;
                p.bend = 0.75;
            }
        }
        Mode::Grind => {
            p.hip_height = 0.74;
            p.bend = 0.32;
            p.arms_out = 0.95;
            p.side_lean += 0.3 * s.balance;
        }
        Mode::Bail => {
            p.tumble = 1.0;
            p.hip_height = 0.5;
            p.bend = 0.9;
            p.arms_out = 1.0;
        }
    }
    p
}

// ---------------------------------------------------------------- solving

/// Joint positions in the rider frame.
struct Joints {
    hips: [Vec3; 2],
    knees: [Vec3; 2],
    ankles: [Vec3; 2],
    toes: [Vec3; 2],
    pelvis: Vec3,
    neck: Vec3,
    head: Vec3,
    shoulders: [Vec3; 2],
    elbows: [Vec3; 2],
    hands: [Vec3; 2],
}

/// Middle joint of a two-bone chain from `root` toward `target`, bending
/// toward `pole`.
pub(crate) fn two_bone(root: Vec3, target: Vec3, l1: f32, l2: f32, pole: Vec3) -> (Vec3, Vec3) {
    let to = target - root;
    let reach = (l1 + l2) * 0.999;
    let d = to.length().clamp(1e-4, reach);
    let dir = to.normalize_or(Vec3::NEG_Y);
    let end = root + dir * d;
    let a = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - a * a).max(0.0).sqrt();
    let side = (pole - dir * pole.dot(dir)).normalize_or(Vec3::Z);
    (root + dir * a + side * h, end)
}

fn solve(p: &Pose, time: f32) -> Joints {
    let pelvis = Vec3::new(p.hip_x, p.hip_height, p.hip_z);
    let body = Quat::from_rotation_y(p.twist) * Quat::from_rotation_x(p.side_lean);
    let hip_axis = body * Vec3::X;
    let hips = [pelvis - hip_axis * HIP_HALF, pelvis + hip_axis * HIP_HALF];
    // Feet: on the deck, lifted for flips, or the back one pushing.
    let front = Vec3::new(FRONT_FOOT_X, ANKLE_UP + p.front_lift.max(0.0), 0.02);
    let back = if p.back_lift < -0.5 {
        // -1 .. -2 = push phase 0 .. 1: the foot sweeps from the front truck
        // to behind the tail on the ground beside the toe edge, with a lift
        // at each end.
        let phase = (-p.back_lift - 1.0).clamp(0.0, 1.0);
        let x = -0.12 + 0.6 * phase;
        let lift = 0.06 * (1.0 - (phase * std::f32::consts::PI).sin());
        Vec3::new(x, ANKLE_UP - DECK_TOP + lift, 0.2)
    } else {
        Vec3::new(BACK_FOOT_X, ANKLE_UP + p.back_lift.max(0.0), 0.0)
    };
    let ankles = [front, back];
    let pole = Vec3::new(0.0, 0.0, 1.0);
    let (k0, a0) = two_bone(hips[0], ankles[0], THIGH, SHIN, pole + Vec3::NEG_X * 0.3);
    let (k1, a1) = two_bone(hips[1], ankles[1], THIGH, SHIN, pole + Vec3::X * 0.3);
    let toe = Vec3::new(0.0, -ANKLE_UP * 0.8, FOOT_LEN * 0.8);
    // Torso: bends toward the chest side, leans with the carve.
    let spine = body * (Quat::from_rotation_x(p.bend) * Vec3::Y);
    let wobble = if p.tumble > 0.5 { (time * 9.0).sin() * 0.3 } else { 0.0 };
    let neck = pelvis + spine * TORSO;
    let head = neck + (spine + Vec3::Z * 0.15).normalize() * NECK_TO_HEAD;
    let across = body * Vec3::X;
    let shoulders = [
        neck - across * SHOULDER_HALF - spine * SHOULDER_DROP,
        neck + across * SHOULDER_HALF - spine * SHOULDER_DROP,
    ];
    // Arms hang, spread along the board for balance, or reach for a grab.
    let mut hands = [Vec3::ZERO; 2];
    let mut elbows = [Vec3::ZERO; 2];
    for (i, sh) in shoulders.iter().enumerate() {
        let out = if i == 0 { -1.0 } else { 1.0 };
        let hang = *sh + Vec3::new(out * (0.1 + 0.42 * p.arms_out), -0.5 + 0.35 * p.arms_out + wobble * out, 0.08);
        let reach = if i == 0 { p.reach_front } else { p.reach_back };
        let grab = Vec3::new(out * 0.06, 0.03 - DECK_TOP * 0.2 + p.front_lift.max(p.back_lift.max(0.0)), 0.11);
        let target = hang.lerp(grab, reach);
        let (e, h) = two_bone(*sh, target, UPPER_ARM, FOREARM, Vec3::new(out * 0.5, 0.0, -1.0));
        elbows[i] = e;
        hands[i] = h;
    }
    Joints {
        hips,
        knees: [k0, k1],
        ankles: [a0, a1],
        toes: [a0 + toe, a1 + toe],
        pelvis,
        neck,
        head,
        shoulders,
        elbows,
        hands,
    }
}

/// A segment from `a` to `b` for a mesh built along +Y with unit length.
fn segment(a: Vec3, b: Vec3) -> Transform {
    let d = b - a;
    let len = d.length().max(1e-4);
    Transform {
        translation: (a + b) * 0.5,
        rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
        scale: Vec3::new(1.0, len, 1.0),
    }
}

fn part_transform(part: Part, j: &Joints) -> Transform {
    let i = |s: Side| if s == Side::Front { 0 } else { 1 };
    match part {
        Part::Pelvis => Transform::from_translation(j.pelvis),
        Part::Torso => segment(j.pelvis, j.neck),
        Part::Head => Transform::from_translation(j.head),
        Part::Cap => Transform::from_translation(j.head + Vec3::Y * 0.035),
        Part::UpperArm(s) => segment(j.shoulders[i(s)], j.elbows[i(s)]),
        Part::Forearm(s) => segment(j.elbows[i(s)], j.hands[i(s)]),
        Part::Hand(s) => Transform::from_translation(j.hands[i(s)]),
        Part::Thigh(s) => segment(j.hips[i(s)], j.knees[i(s)]),
        Part::Shin(s) => segment(j.knees[i(s)], j.ankles[i(s)]),
        Part::Foot(s) => {
            let (a, t) = (j.ankles[i(s)], j.toes[i(s)]);
            Transform {
                translation: (a + t) * 0.5,
                rotation: Quat::from_rotation_arc(Vec3::Z, (t - a).normalize_or(Vec3::Z)),
                scale: Vec3::ONE,
            }
        }
    }
}

// ---------------------------------------------------------------- spawning

fn capsule(radius: f32) -> Mesh {
    // Unit length along Y (scaled per frame); the hemispheres are squashed
    // with it, which reads fine at this size.
    Capsule3d::new(radius, 1.0).mesh().rings(2).latitudes(6).longitudes(10).into()
}

pub(crate) fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mat = |m: &mut Assets<StandardMaterial>, c: Color| {
        m.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: 0.85,
            ..default()
        })
    };
    let hoodie = mat(&mut materials, Color::srgb(0.16, 0.55, 0.52));
    let sleeve = mat(&mut materials, Color::srgb(0.13, 0.46, 0.44));
    let pants = mat(&mut materials, Color::srgb(0.24, 0.24, 0.3));
    let skin = mat(&mut materials, Color::srgb(0.78, 0.6, 0.47));
    let shoe = mat(&mut materials, Color::srgb(0.92, 0.9, 0.84));
    let cap = mat(&mut materials, Color::srgb(0.85, 0.36, 0.2));
    let deck = mat(&mut materials, Color::srgb(0.95, 0.75, 0.25));
    let grip = mat(&mut materials, Color::srgb(0.08, 0.08, 0.09));
    let metal = mat(&mut materials, Color::srgb(0.7, 0.72, 0.75));
    let urethane = mat(&mut materials, Color::srgb(0.96, 0.95, 0.9));

    let rider = commands
        .spawn((RiderRoot, Transform::default(), Visibility::default()))
        .id();
    let mut add = |part: Part, mesh: Mesh, material: &Handle<StandardMaterial>| {
        let child = commands
            .spawn((part, Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone()), Transform::default()))
            .id();
        commands.entity(rider).add_child(child);
    };
    add(Part::Pelvis, Cuboid::new(0.3, 0.15, 0.19).into(), &pants);
    add(Part::Torso, capsule(0.14), &hoodie);
    add(Part::Head, Sphere::new(0.105).mesh().ico(2).unwrap(), &skin);
    add(Part::Cap, Cylinder::new(0.112, 0.07).into(), &cap);
    for side in [Side::Front, Side::Back] {
        add(Part::UpperArm(side), capsule(0.05), &hoodie);
        add(Part::Forearm(side), capsule(0.042), &sleeve);
        add(Part::Hand(side), Sphere::new(0.045).mesh().ico(1).unwrap(), &skin);
        add(Part::Thigh(side), capsule(0.072), &pants);
        add(Part::Shin(side), capsule(0.056), &pants);
        add(Part::Foot(side), Cuboid::new(0.105, 0.075, FOOT_LEN).into(), &shoe);
    }

    // Board: root at the deck centre so flips turn about it.
    let board = commands
        .spawn((BoardRoot, Transform::default(), Visibility::default()))
        .id();
    let mut piece = |mesh: Mesh, material: &Handle<StandardMaterial>, t: Transform| {
        let child = commands
            .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone()), t))
            .id();
        commands.entity(board).add_child(child);
    };
    let flat = DECK_LEN - 2.0 * KICK_LEN;
    let y = DECK_TOP - DECK_CENTRE - DECK_THICK * 0.5;
    piece(Cuboid::new(DECK_WIDTH, DECK_THICK, flat).into(), &deck, Transform::from_xyz(0.0, y, 0.0));
    piece(
        Cuboid::new(DECK_WIDTH * 0.98, 0.002, flat).into(),
        &grip,
        Transform::from_xyz(0.0, y + DECK_THICK * 0.5 + 0.001, 0.0),
    );
    for end in [1.0f32, -1.0] {
        let hinge = Vec3::new(0.0, y, end * flat * 0.5);
        let rot = Quat::from_rotation_x(-end * KICK_ANGLE);
        let centre = hinge + rot * Vec3::new(0.0, 0.0, end * KICK_LEN * 0.5);
        piece(
            Cuboid::new(DECK_WIDTH * 0.96, DECK_THICK, KICK_LEN).into(),
            &deck,
            Transform::from_translation(centre).with_rotation(rot),
        );
        let axle = WHEEL_R - DECK_CENTRE;
        piece(
            Cuboid::new(0.16, 0.035, 0.05).into(),
            &metal,
            Transform::from_xyz(0.0, (axle + y) * 0.5, end * AXLE_Z),
        );
        for x in [-1.0f32, 1.0] {
            piece(
                Cylinder::new(WHEEL_R, 0.032).into(),
                &urethane,
                Transform::from_xyz(x * 0.085, axle, end * AXLE_Z)
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
            );
        }
    }
}

// ---------------------------------------------------------------- per frame

#[derive(Default)]
pub(crate) struct Tracker {
    since_pop: f32,
    bail_dir: Vec3,
}

/// Rider stance yaw: the chest faces the board's toe side (+X of the board).
fn stance() -> Quat {
    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
}

#[allow(clippy::type_complexity)]
pub(crate) fn animate(
    time: Res<Time<Real>>,
    sim: Res<CleanSkate>,
    mut pose: Local<Pose>,
    mut track: Local<Tracker>,
    mut rider: Query<&mut Transform, (With<RiderRoot>, Without<BoardRoot>, Without<Part>)>,
    mut board: Query<&mut Transform, (With<BoardRoot>, Without<RiderRoot>, Without<Part>)>,
    mut parts: Query<(&Part, &mut Transform), (Without<RiderRoot>, Without<BoardRoot>)>,
) {
    let dt = time.delta_secs().min(0.1);
    let (a, b, t) = (&sim.previous, &sim.current, sim.alpha());
    let pos = Vec3::from_array(a.position).lerp(Vec3::from_array(b.position), t);
    let rider_rot = Quat::from_array(a.rider_rot).slerp(Quat::from_array(b.rider_rot), t);
    let board_rot = Quat::from_array(a.board_rot).slerp(Quat::from_array(b.board_rot), t);
    let up = Vec3::from_array(b.up);

    if sim.events.iter().any(|e| e.kind == skate_lite_sys::EventKind::Pop) {
        track.since_pop = 0.0;
    } else {
        track.since_pop += dt;
    }
    let target = target_pose(b, track.since_pop);
    pose.ease(&target, 1.0 - (-EASE * dt).exp());

    let bail = b.bail_time;
    if bail == Some(0.0) || (bail.is_some() && track.bail_dir == Vec3::ZERO) {
        track.bail_dir = Vec3::from_array(b.velocity).with_y(0.0).normalize_or(rider_rot * Vec3::Z);
    }
    if bail.is_none() {
        track.bail_dir = Vec3::ZERO;
    }

    if let Ok(mut tf) = rider.single_mut() {
        let mut rot = rider_rot * stance();
        let mut at = pos + up * DECK_TOP;
        if let Some(bt) = bail {
            // Tumble forward over the board, then lie in a heap.
            let fall = (bt / 0.7).min(1.0);
            let axis = track.bail_dir.cross(Vec3::Y).normalize_or(Vec3::X);
            rot = Quat::from_axis_angle(axis, -fall * 1.5) * rot;
            at = pos + Vec3::Y * (DECK_TOP * (1.0 - fall)) + track.bail_dir * 0.5 * fall;
        }
        *tf = Transform::from_translation(at).with_rotation(rot);
    }
    if let Ok(mut tf) = board.single_mut() {
        let mut rot = board_rot;
        let mut at = pos + (rider_rot * Vec3::Y) * DECK_CENTRE;
        if let Some(bt) = bail {
            // The board shoots out ahead and settles upside down.
            let k = (bt / 0.9).min(1.0);
            at += track.bail_dir * 1.4 * k + Vec3::Y * (0.6 * (k * std::f32::consts::PI).sin());
            rot = Quat::from_axis_angle(track.bail_dir, k * std::f32::consts::PI) * rot;
        }
        *tf = Transform::from_translation(at).with_rotation(rot);
    }
    let joints = solve(&pose, time.elapsed_secs());
    for (part, mut tf) in &mut parts {
        *tf = part_transform(*part, &joints);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bone_keeps_lengths_and_reaches() {
        let root = Vec3::new(0.0, 1.0, 0.0);
        let target = Vec3::new(0.1, 0.2, 0.05);
        let (mid, end) = two_bone(root, target, 0.43, 0.415, Vec3::Z);
        assert!((mid.distance(root) - 0.43).abs() < 1e-3);
        assert!((mid.distance(end) - 0.415).abs() < 1e-3);
        assert!(end.distance(target) < 1e-3);
        assert!(mid.z > 0.0, "knee bends toward the pole");
        // Out of reach: straightened, pointing at the target.
        let (_, end) = two_bone(root, Vec3::new(0.0, -5.0, 0.0), 0.43, 0.415, Vec3::Z);
        assert!((end.distance(root) - 0.845).abs() < 2e-3);
    }

    #[test]
    fn standing_pose_puts_feet_on_the_deck() {
        let j = solve(&Pose::default(), 0.0);
        for a in j.ankles {
            assert!((a.y - ANKLE_UP).abs() < 0.02, "ankle {a:?}");
        }
        assert!(j.head.y > 1.4 && j.head.y < 1.76, "head height {}", j.head.y);
    }
}
