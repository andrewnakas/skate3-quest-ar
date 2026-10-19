//! Light for both modes; on the desktop (`--clean` without `--xr`) also a
//! chase camera and a small test park, so the clean tier can be skated and
//! checked without a headset.
use super::collide::{Grid, SURFACE_GROUND, SURFACE_METAL, SURFACE_WOOD};
use super::sim::CleanSkate;
use bevy::prelude::*;

// The placeable pieces' geometry, shared with build mode.
#[path = "../xr/build/pieces.rs"]
#[allow(dead_code)]
mod pieces;
use pieces::{Piece, PieceGeometry, PieceKind};

pub(crate) fn lights(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            // On for the 3-view check: the shadow is what shows how high the board and rider go.
            shadows_enabled: three_views(),
            ..default()
        },
        Transform::from_xyz(4.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 450.0,
        ..default()
    });
}

#[derive(Component)]
pub(crate) struct ChaseCamera;

/// `SK8_VIEWS=3`: two more cameras beside the chase one, split three ways, for checking animation
/// from fixed angles (0 = beside the line of travel on the chest side, 1 = ahead on the line, which
/// sees the rider's profile). The same layout as the skatecore web page's `?views=3`.
#[derive(Component)]
pub(crate) struct ViewCamera(u8);

pub(crate) fn three_views() -> bool {
    std::env::var("SK8_VIEWS").is_ok_and(|v| v == "3")
}

/// (kind, size, position, turn) of the desktop test park, metres.
const PARK: [(PieceKind, f32, [f32; 3], f32); 7] = [
    (PieceKind::Kicker, 1.0, [-4.0, 0.0, 12.0], 0.0),
    (PieceKind::Rail, 1.2, [3.0, 0.0, 14.0], 0.0),
    (PieceKind::FunBox, 1.0, [0.0, 0.0, 26.0], 0.0),
    (PieceKind::Ledge, 1.0, [-7.0, 0.0, 22.0], 0.0),
    (PieceKind::QuarterPipe, 1.2, [0.0, 0.0, 40.0], 0.0),
    (PieceKind::Bank, 1.0, [8.0, 0.0, 30.0], -0.6),
    (PieceKind::Stairs, 1.0, [-12.0, 0.0, 34.0], 0.0),
];
const FLOOR_HALF: f32 = 120.0;

fn piece_mesh(geometry: &PieceGeometry) -> Mesh {
    let positions: Vec<[f32; 3]> = geometry
        .all_quads()
        .flat_map(|q| [0, 2, 1, 0, 3, 2].map(|i| q[i].to_array()))
        .collect();
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.compute_flat_normals();
    mesh
}

pub(crate) fn desktop_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut sim: ResMut<CleanSkate>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.55, 0.72, 0.9)));
    commands.spawn((
        ChaseCamera,
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.5, -5.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
    if three_views() {
        for (i, order) in [(0u8, 1isize), (1, 2)] {
            commands.spawn((
                ViewCamera(i),
                Camera3d::default(),
                Camera { order, ..default() },
                Projection::Perspective(PerspectiveProjection { fov: 38f32.to_radians(), ..default() }),
                Transform::default(),
            ));
        }
    }
    let mut grid = Grid::default();
    let f = FLOOR_HALF;
    grid.add_quad(
        [
            Vec3::new(-f, 0.0, -f),
            Vec3::new(-f, 0.0, f),
            Vec3::new(f, 0.0, f),
            Vec3::new(f, 0.0, -f),
        ],
        SURFACE_GROUND,
    );
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(2.0 * f, 2.0 * f))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.62, 0.62, 0.6),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::default(),
    ));
    let mut rails = Vec::new();
    for (kind, size, at, turn) in PARK {
        let piece = Piece { kind, size };
        let geometry = piece.geometry();
        let t = Transform::from_translation(Vec3::from_array(at)).with_rotation(Quat::from_rotation_y(turn));
        for q in &geometry.quads {
            grid.add_quad(q.map(|p| t.transform_point(p)), SURFACE_WOOD);
        }
        for q in &geometry.rail_quads {
            grid.add_quad(q.map(|p| t.transform_point(p)), SURFACE_METAL);
        }
        rails.extend(geometry.rails.iter().map(|r| r.iter().map(|&p| t.transform_point(p)).collect::<Vec<_>>()));
        commands.spawn((
            Mesh3d(meshes.add(piece_mesh(&geometry))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: kind.color(),
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
            t,
        ));
    }
    sim.set_collision(grid, &rails);
    sim.place(Vec3::ZERO, 0.0);
    info!("SK8 desktop park: {} triangles, {} rails", sim.collision_triangles, rails.len());
}

/// Follow behind the skater's travel direction.
pub(crate) fn chase(
    time: Res<Time<Real>>,
    sim: Res<CleanSkate>,
    mut camera: Query<&mut Transform, With<ChaseCamera>>,
    mut heading: Local<Option<Vec3>>,
) {
    let Ok(mut cam) = camera.single_mut() else { return };
    let s = &sim.current;
    let pos = Vec3::from_array(s.position);
    let vel = Vec3::from_array(s.velocity).with_y(0.0);
    let facing = Quat::from_array(s.rider_rot) * Vec3::Z;
    let want = if vel.length() > 0.8 { vel.normalize() } else { facing.with_y(0.0).normalize_or(Vec3::Z) };
    let k = 1.0 - (-3.0 * time.delta_secs()).exp();
    let h = heading.get_or_insert(want);
    *h = h.lerp(want, k).normalize_or(want);
    let target = pos - *h * 4.2 + Vec3::Y * 1.9;
    cam.translation = cam.translation.lerp(target, 1.0 - (-6.0 * time.delta_secs()).exp());
    cam.look_at(pos + Vec3::Y * 0.8, Vec3::Y);
}

/// Split the window three ways and aim the fixed cameras: they follow the line of travel (the body
/// turns to push, so its own axes would swing the views), one beside it on the chest side, one ahead.
#[allow(clippy::type_complexity)]
pub(crate) fn views(
    sim: Res<CleanSkate>,
    windows: Query<&Window>,
    mut chase: Query<&mut Camera, (With<ChaseCamera>, Without<ViewCamera>)>,
    mut fixed: Query<(&ViewCamera, &mut Camera, &mut Transform), Without<ChaseCamera>>,
    mut dir: Local<Option<Vec3>>,
) {
    let Ok(window) = windows.single() else { return };
    let (w, h) = (window.physical_width() / 3, window.physical_height());
    if w == 0 || h == 0 {
        return;
    }
    let viewport = |i: u32| Some(bevy::camera::Viewport {
        physical_position: UVec2::new(i * w, 0),
        physical_size: UVec2::new(w, h),
        ..default()
    });
    if let Ok(mut cam) = chase.single_mut() {
        cam.viewport = viewport(0);
    }
    let s = &sim.current;
    let pos = Vec3::from_array(s.position);
    let rot = Quat::from_array(s.rider_rot);
    let vel = Vec3::from_array(s.velocity).with_y(0.0);
    let d = dir.get_or_insert((rot * Vec3::Z).with_y(0.0).normalize_or(Vec3::Z));
    if vel.length() > 0.3 {
        *d = d.lerp(vel.normalize(), 0.06).normalize_or(*d);
    }
    // The chest faces the rider's -X.
    let chest = (rot * Vec3::NEG_X).with_y(0.0).normalize_or(Vec3::X);
    let mut beside = Vec3::new(-d.z, 0.0, d.x);
    if beside.dot(chest) < 0.0 {
        beside = -beside;
    }
    // SK8_VIEW_DIST / SK8_VIEW_AIM: closer in and lower for checking the feet on the board.
    let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (dist, aim_y) = (env("SK8_VIEW_DIST", 4.6), env("SK8_VIEW_AIM", 0.85));
    let aim = pos + Vec3::Y * aim_y;
    for (v, mut cam, mut tf) in &mut fixed {
        cam.viewport = viewport(1 + v.0 as u32);
        let at = aim + if v.0 == 0 { beside } else { *d } * dist + Vec3::Y * 0.1;
        *tf = Transform::from_translation(at).looking_at(aim, Vec3::Y);
    }
}
