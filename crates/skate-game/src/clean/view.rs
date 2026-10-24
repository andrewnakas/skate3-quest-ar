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
            // a high sun, shadows on: the shadow under the rider shows how high the board goes
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(2.0, 12.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // a soft fill from the open sky the other way: the shadows light, the body still shaped
    commands.spawn((
        DirectionalLight { illuminance: 2600.0, shadows_enabled: false, ..default() },
        Transform::from_xyz(-3.0, 8.0, -5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 320.0, // (the sun does more of the shaping)
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
const PARK: [(PieceKind, f32, [f32; 3], f32); 8] = [
    (PieceKind::Kicker, 1.0, [-4.0, 0.0, 12.0], 0.0),
    (PieceKind::Rail, 1.2, [3.0, 0.0, 14.0], 0.0),
    (PieceKind::FunBox, 1.0, [0.0, 0.0, 26.0], 0.0),
    (PieceKind::Ledge, 1.0, [-7.0, 0.0, 22.0], 0.0),
    (PieceKind::QuarterPipe, 1.2, [0.0, 0.0, 40.0], 0.0),
    (PieceKind::Bank, 1.0, [8.0, 0.0, 30.0], -0.6),
    (PieceKind::Stairs, 1.0, [-12.0, 0.0, 34.0], 0.0),
    // along the run-in, to ride
    (PieceKind::Wall, 2.0, [16.0, 0.0, 14.0], std::f32::consts::FRAC_PI_2),
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
    // a pale, hazy sky
    commands.insert_resource(ClearColor(Color::srgb(0.79, 0.9, 0.99)));
    commands.spawn((
        ChaseCamera,
        Camera3d::default(),
        // as wide as the port's: its camera's 75 degrees is the vertical field of view (three.js)
        Projection::Perspective(PerspectiveProjection { fov: 75f32.to_radians(), ..default() }),
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
            base_color: Color::srgb(0.4, 0.41, 0.44), // cool grey concrete
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
                base_color: desktop_color(kind),
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

/// Which kind of shot the camera is taking (our own set, after the way the port picks one by what the
/// skater is doing: riding, crouched for a pop, a manual, a low or a big air, back into a ramp, a
/// rail, a plant, off the board, a bail).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum Shot {
    #[default]
    Chase,
    Crouch,
    Manual,
    /// Riding up a steep transition: held low, looking up the wall (so it doesn't climb with the rider).
    Wall,
    SmallAir,
    BigAir,
    Reentry,
    Grind,
    GrindSlow,
    Plant,
    OnFoot,
    Bail,
}

/// (distance back, height, how far ahead it looks, look height, sideways toward the heel side,
/// share of the rider's rise it follows) for a shot, metres.
fn framing(shot: Shot) -> (f32, f32, f32, f32, f32, f32) {
    match shot {
        Shot::Chase => (2.65, 2.0, 1.0, 1.0, 0.0, 1.0),
        Shot::Crouch => (2.65, 1.9, 1.0, 0.95, 0.0, 1.0),
        Shot::Manual => (2.65, 1.9, 1.0, 0.9, 0.0, 1.0),
        // a small air: framed as on the ground, the camera barely rising with the pop
        Shot::Wall => (3.9, 1.3, 0.0, 1.0, 0.0, 0.3),
        Shot::SmallAir => (2.65, 2.0, 1.0, 1.0, 0.0, 0.3),
        // a big air: up over the rider, close, looking down on it (as the port's)
        Shot::BigAir => (1.135, 3.1, 0.3, 0.55, 0.0, 1.0),
        Shot::Reentry => (4.2, 1.1, 0.0, 1.0, 0.0, 0.2),
        Shot::Grind => (2.65, 1.9, 1.0, 0.95, 0.5, 1.0),
        Shot::GrindSlow => (2.45, 1.9, 0.6, 0.9, 0.4, 1.0),
        // a plant: low, out to the side
        Shot::Plant => (0.4, 0.9, 0.0, 0.95, 2.3, 0.5),
        Shot::OnFoot => (2.72, 2.4, 0.5, 1.045, 0.0, 1.0),
        Shot::Bail => (2.72, 2.5, 0.0, 0.3, 0.0, 1.0),
    }
}

#[derive(Default)]
pub(crate) struct CameraState {
    heading: Option<Vec3>,
    shot: Shot,
    /// Ground height the rider took off from (the air shots frame against it).
    base_y: f32,
    /// Where it sat and looked last frame (smoothed), and a landing's shake.
    at: Option<Vec3>,
    look: Vec3,
    shake: f32,
    shake_t: f32,
    /// A shot held still (plant, re-entry, bail): where it stays.
    hold: Option<Vec3>,
}

/// On ground steeper than this (the up vector's height) the camera stops climbing with the rider.
const STEEP_UP: f32 = 0.85;

fn pick_shot(s: &skate_lite_sys::State, previous: Shot, base_y: f32) -> Shot {
    use skate_lite_sys::Mode;
    let v = Vec3::from_array(s.velocity);
    let flat = v.with_y(0.0).length();
    let up = Quat::from_array(s.rider_rot) * Vec3::Y;
    match s.mode {
        Mode::Ride | Mode::Slide | Mode::Manual if up.y < STEEP_UP => Shot::Wall,
        Mode::Bail => Shot::Bail,
        Mode::Walk => Shot::OnFoot,
        Mode::Grind if s.grind_kind == 15 || s.grind_kind == 16 => Shot::Plant,
        Mode::Grind if s.speed < 2.0 => Shot::GrindSlow,
        Mode::Grind => Shot::Grind,
        Mode::Manual => Shot::Manual,
        Mode::Air if s.on_foot => Shot::OnFoot,
        Mode::Air => {
            // going up or coming down nearly straight (out of a ramp and back in): held below
            if v.y.abs() > 2.5 * flat.max(0.5) {
                return Shot::Reentry;
            }
            // how high the air will go above where it left: a low one keeps the ground framing
            let peak = s.position[1] + (v.y.max(0.0)).powi(2) / (2.0 * 12.0) - base_y;
            if previous == Shot::BigAir || peak > 1.4 {
                Shot::BigAir
            } else if previous == Shot::Reentry {
                Shot::Reentry
            } else {
                Shot::SmallAir
            }
        }
        _ if s.crouch > 0.5 => Shot::Crouch,
        _ => Shot::Chase,
    }
}

/// The desktop camera: a shot picked by what the skater is doing, eased between, pulled in where a
/// wall is in the way, with a little shake on a hard landing.
pub(crate) fn chase(
    time: Res<Time<Real>>,
    sim: Res<CleanSkate>,
    mut camera: Query<&mut Transform, With<ChaseCamera>>,
    mut st: Local<CameraState>,
) {
    use skate_lite_sys::{EventKind, Mode};
    let Ok(mut cam) = camera.single_mut() else { return };
    let dt = time.delta_secs().min(0.1);
    let s = &sim.current;
    let pos = Vec3::from_array(s.position);
    let vel = Vec3::from_array(s.velocity).with_y(0.0);
    let facing = Quat::from_array(s.rider_rot) * Vec3::Z;
    for e in &sim.events {
        match e.kind {
            EventKind::Respawn => *st = CameraState::default(),
            // the harder it comes down, the more the camera jolts
            EventKind::Land if e.value == 1 || Vec3::from_array(s.velocity).length() > 7.0 => {
                st.shake = if e.value == 1 { 0.09 } else { 0.05 };
                st.shake_t = 0.0;
            }
            _ => {}
        }
    }
    let flat_ground = (Quat::from_array(s.rider_rot) * Vec3::Y).y >= STEEP_UP;
    if matches!(s.mode, Mode::Walk | Mode::Grind) || (matches!(s.mode, Mode::Ride | Mode::Manual | Mode::Slide) && flat_ground) {
        st.base_y = pos.y;
    }
    let shot = pick_shot(s, st.shot, st.base_y);
    if shot != st.shot {
        st.hold = None;
    }
    st.shot = shot;
    // the line it follows: the way it's going, eased (slower in a turn, so the shot swings wide)
    let want = if vel.length() > 0.8 { vel.normalize() } else { facing.with_y(0.0).normalize_or(Vec3::Z) };
    let h0 = *st.heading.get_or_insert(want);
    let turn_rate = if matches!(shot, Shot::Grind | Shot::GrindSlow) { 2.0 } else { 3.0 };
    let heading = h0.lerp(want, 1.0 - (-turn_rate * dt).exp()).normalize_or(want);
    if !matches!(shot, Shot::Bail | Shot::Reentry | Shot::Plant | Shot::Wall) {
        st.heading = Some(heading);
    }
    let heading = st.heading.unwrap_or(heading);
    let (back, height, ahead, look_h, side, rise) = framing(shot);
    // the heel side (away from the chest) for rails: the rail and the feet in view
    let chest = (super::skater::body_rot(s) * Vec3::NEG_X).with_y(0.0).normalize_or(Vec3::X);
    let lateral = -chest * side;
    let y = st.base_y + (pos.y - st.base_y) * rise;
    let mut target = Vec3::new(pos.x, y, pos.z) - heading * back + lateral + Vec3::Y * height;
    // held shots: where the camera was as it started, watching from there
    if matches!(shot, Shot::Bail | Shot::Reentry | Shot::Plant) {
        let at = st.at.unwrap_or(target);
        let hold = *st.hold.get_or_insert(at.lerp(target, 0.5));
        target = hold;
    }
    let look_target = pos + heading * ahead + Vec3::Y * look_h;
    // eased: quick to follow riding, slower into the held / air shots
    let stiff = match shot {
        Shot::Bail | Shot::Plant | Shot::Reentry | Shot::Wall => 2.5,
        Shot::BigAir => 4.0,
        _ => 6.0,
    };
    let at = st.at.get_or_insert(target);
    *at = at.lerp(target, 1.0 - (-stiff * dt).exp());
    let mut at = *at;
    st.look = if st.look == Vec3::ZERO { look_target } else { st.look.lerp(look_target, 1.0 - (-9.0 * dt).exp()) };
    // a wall between the rider and the camera: in front of it instead
    if let Some(grid) = &sim.camera_grid {
        let from = pos + Vec3::Y * 1.0;
        if let Some((f, ..)) = grid.cast(from, at - from) {
            at = from + (at - from) * (f - 0.05).max(0.1);
        }
    }
    // landing shake: a quick damped bob
    if st.shake > 0.0 {
        st.shake_t += dt;
        let k = (-st.shake_t / 0.12).exp();
        at.y -= st.shake * k * (st.shake_t * 40.0).cos();
        if k < 0.01 {
            st.shake = 0.0;
        }
    }
    cam.translation = at;
    cam.look_at(st.look, Vec3::Y);
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
    // (only with SK8_VIEWS=3: otherwise the chase camera keeps the whole window)
    if !three_views() {
        return;
    }
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
    // The chest faces the body's -X (switch turns the engine's frame round, not the body).
    let chest = (super::skater::body_rot(s) * Vec3::NEG_X).with_y(0.0).normalize_or(Vec3::X);
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

/// The desktop park's colours: peach transitions, light cool-grey boxes and kickers, coral ledges and
/// rails (the headset's park builder keeps its own brighter set).
fn desktop_color(kind: PieceKind) -> Color {
    match kind {
        PieceKind::QuarterPipe | PieceKind::HalfPipe | PieceKind::Spine | PieceKind::Bank => Color::srgb(0.92, 0.66, 0.55),
        PieceKind::Kicker | PieceKind::FunBox | PieceKind::Platform | PieceKind::Stairs | PieceKind::Wall => {
            Color::srgb(0.76, 0.77, 0.8)
        }
        PieceKind::Ledge | PieceKind::Rail => Color::srgb(0.9, 0.57, 0.48),
    }
}
