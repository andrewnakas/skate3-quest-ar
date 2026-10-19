//! The clean tier's skateboard, built in code to standard proportions: a symmetric deck with rounded
//! nose and tail kicked up at the ends and concave across, gripped on top, on two trucks and four
//! wheels. Board space: X across, Y up, Z along (nose +Z), origin at the deck's centre height
//! ([`DECK_CENTRE`] above the ground under the wheels).
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::f32::consts::FRAC_PI_2;

/// Deck top at the centre of the concave, above the ground (m).
pub(crate) const DECK_TOP: f32 = 0.105;
/// The board root's height above the ground.
pub(crate) const DECK_CENTRE: f32 = 0.09;
pub(crate) const DECK_LEN: f32 = 0.8;
pub(crate) const DECK_WIDTH: f32 = 0.205;
const DECK_THICK: f32 = 0.012;
/// Where nose and tail start to kick up, from the centre, and their angle (radians).
pub(crate) const KICK_START: f32 = 0.245;
const KICK_ANGLE: f32 = 0.35;
/// How long the bend into the kick is.
const KICK_BEND: f32 = 0.05;
/// Rails above the centre of the concave.
const CONCAVE: f32 = 0.009;
/// How far the rounded ends reach in from the tips.
const END_ROUND: f32 = 0.09;
/// Truck centres (the middle of each bolt pattern) and axles, from the deck's centre.
pub(crate) const TRUCK_Z: f32 = 0.2;
pub(crate) const WHEEL_R: f32 = 0.027;
const WHEEL_W: f32 = 0.032;
/// Wheel centres across the board.
const WHEEL_X: f32 = 0.095;

/// The deck top's height in board space at `z` along and `u` (-1..1) across.
pub(crate) fn deck_top(z: f32, u: f32) -> f32 {
    let base = DECK_TOP - DECK_CENTRE;
    let s = z.abs() - KICK_START;
    let tan = KICK_ANGLE.tan();
    let kick = if s <= 0.0 {
        0.0
    } else if s < KICK_BEND {
        tan * s * s / (2.0 * KICK_BEND)
    } else {
        tan * (s - KICK_BEND * 0.5)
    };
    // The concave fades out up the kicks.
    let fade = (1.0 - (s / 0.06).clamp(0.0, 1.0)).max(0.0);
    base + kick + CONCAVE * u * u * fade
}

/// Half the deck's width at `z` along: full between the ends, rounding off into nose and tail.
fn half_width(z: f32) -> f32 {
    let w = DECK_WIDTH * 0.5;
    let from_end = DECK_LEN * 0.5 - z.abs();
    if from_end >= END_ROUND {
        return w;
    }
    // a squarish round (superellipse), as decks are: broad, then turning in at the tip
    let t = ((END_ROUND - from_end) / END_ROUND).clamp(0.0, 1.0);
    w * (1.0 - t.powf(2.4)).max(0.0).powf(1.0 / 2.4)
}

const NZ: usize = 96;
const NX: usize = 14;

fn grid_point(i: usize, j: usize, top: bool) -> Vec3 {
    let z = -DECK_LEN * 0.5 + DECK_LEN * i as f32 / NZ as f32;
    let u = -1.0 + 2.0 * j as f32 / NX as f32;
    let y = deck_top(z, u) - if top { 0.0 } else { DECK_THICK };
    Vec3::new(u * half_width(z), y, z)
}

fn mesh(positions: Vec<Vec3>, indices: Vec<u32>, smooth: bool) -> Mesh {
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions.iter().map(|p| p.to_array()).collect::<Vec<_>>())
        .with_inserted_indices(Indices::U32(indices));
    if smooth {
        m.compute_smooth_normals();
    } else {
        m.duplicate_vertices();
        m.compute_flat_normals();
    }
    m
}

/// The grip surface (top) and the wood (bottom and edges).
fn deck_meshes() -> (Mesh, Mesh) {
    let idx = |i: usize, j: usize| (i * (NX + 1) + j) as u32;
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for i in 0..=NZ {
        for j in 0..=NX {
            top.push(grid_point(i, j, true) + Vec3::Y * 0.0005);
            bottom.push(grid_point(i, j, false));
        }
    }
    let mut ti = Vec::new();
    let mut bi = Vec::new();
    for i in 0..NZ {
        for j in 0..NX {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
            // counter-clockwise from outside: (+z edge) x (+x edge) = +y for the top
            ti.extend([a, b, c, a, c, d]);
            bi.extend([a, c, b, a, d, c]);
        }
    }
    // Edges: the outline at the top and the bottom, joined all round.
    let mut wood = bottom.clone();
    let base = wood.len() as u32;
    let mut ring = Vec::new();
    for i in 0..=NZ {
        ring.push((i, NX));
    }
    for i in (0..=NZ).rev() {
        ring.push((i, 0));
    }
    for &(i, j) in &ring {
        wood.push(grid_point(i, j, true));
        wood.push(grid_point(i, j, false));
    }
    for k in 0..ring.len() as u32 - 1 {
        let (t0, b0, t1, b1) = (base + 2 * k, base + 2 * k + 1, base + 2 * k + 2, base + 2 * k + 3);
        bi.extend([t0, b1, b0, t0, t1, b1]); // facing out
    }
    (mesh(top, ti, true), mesh(wood, bi, false))
}

/// Spawn the board under `board` (an entity with the board root's transform).
pub(crate) fn build(
    commands: &mut Commands,
    board: Entity,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let wood = materials.add(StandardMaterial {
        base_color: Color::srgb(0.84, 0.6, 0.32),
        perceptual_roughness: 0.6,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let mut mat = |c: Color, rough: f32, metal: f32| {
        materials.add(StandardMaterial { base_color: c, perceptual_roughness: rough, metallic: metal, ..default() })
    };
    let grip = mat(Color::srgb(0.07, 0.07, 0.08), 0.95, 0.0);
    let metal = mat(Color::srgb(0.74, 0.75, 0.78), 0.35, 0.9);
    let urethane = mat(Color::srgb(0.95, 0.93, 0.86), 0.55, 0.0);
    let (grip_mesh, wood_mesh) = deck_meshes();
    let mut piece = |mesh: Mesh, material: &Handle<StandardMaterial>, t: Transform| {
        let child = commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone()), t)).id();
        commands.entity(board).add_child(child);
    };
    piece(grip_mesh, &grip, Transform::IDENTITY);
    piece(wood_mesh, &wood, Transform::IDENTITY);
    let under = deck_top(TRUCK_Z, 0.0) - DECK_THICK;
    let axle = WHEEL_R - DECK_CENTRE;
    for end in [1.0f32, -1.0] {
        let z = end * TRUCK_Z;
        // baseplate on the deck, the hanger below it, the axle through, a wheel each side
        piece(Cuboid::new(0.058, 0.008, 0.078).into(), &metal, Transform::from_xyz(0.0, under - 0.004, z));
        piece(
            Cylinder::new(0.008, under - axle).mesh().resolution(12).into(),
            &metal,
            Transform::from_xyz(0.0, (under + axle) * 0.5, z - end * 0.012),
        );
        piece(
            Capsule3d::new(0.013, 0.1).mesh().into(),
            &metal,
            Transform::from_xyz(0.0, axle + 0.01, z).with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
        );
        piece(
            Cylinder::new(0.004, 2.0 * WHEEL_X + WHEEL_W).mesh().resolution(8).into(),
            &metal,
            Transform::from_xyz(0.0, axle, z).with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
        );
        for x in [-1.0f32, 1.0] {
            piece(
                Cylinder::new(WHEEL_R, WHEEL_W).mesh().resolution(24).into(),
                &urethane,
                Transform::from_xyz(x * WHEEL_X, axle, z).with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
            );
        }
    }
}
