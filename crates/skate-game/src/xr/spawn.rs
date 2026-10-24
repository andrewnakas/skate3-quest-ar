//! Drop the skater anywhere: hold the left grip and left trigger together and
//! point the left controller at a surface (real or course); a ray and target
//! ring show where. Release to spawn there, facing the pointing direction.
//! The pad is neutral while aiming so the squeeze doesn't also grab/bump.
use super::actions::{Hand, LatestTouch};
use super::rig::Rig;
use super::link::{ArWorld, TeleportSkater};
use crate::input::ExternalPad;
use bevy::prelude::*;
use bevy_mod_openxr::openxr_session_running;
use super::touch::XboxState;

const SQUEEZE: f32 = 0.7;
/// Game-world units (metres at skater scale).
const MAX_RANGE: f32 = 2000.0;
const MISS_LENGTH: f32 = 40.0;
/// Only ground-like faces are spawn targets, not walls.
const MIN_UP: f32 = 0.6;
const SPAWN_LIFT: f32 = 0.05;

#[derive(Resource, Default)]
struct Aim {
    /// Ray origin, end, and whether `end` is a valid spawn point.
    ray: Option<(Vec3, Vec3, bool)>,
    target: Option<(Vec3, Vec3)>,
}

#[derive(Component)]
struct AimRay;
#[derive(Component)]
struct AimMarker;

pub(crate) struct SpawnPlugin;

impl Plugin for SpawnPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Aim>()
            .add_systems(Startup, spawn_visuals)
            .add_systems(
                PreUpdate,
                aim_and_spawn
                    .after(super::actions::TouchRead)
                    .before(crate::input::poll_controllers)
                    .run_if(openxr_session_running),
            )
            .add_systems(Update, show_aim);
    }
}

fn spawn_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = |color: Color| StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    };
    commands.spawn((
        AimRay,
        Mesh3d(meshes.add(Cylinder::new(0.04, 1.0))),
        MeshMaterial3d(materials.add(material(Color::srgb(0.2, 0.9, 1.0)))),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.spawn((
        AimMarker,
        Mesh3d(meshes.add(Annulus::new(0.5, 0.8))),
        MeshMaterial3d(materials.add(StandardMaterial {
            double_sided: true,
            cull_mode: None,
            ..material(Color::srgb(0.2, 1.0, 0.4))
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
}

fn aim_and_spawn(
    touch: Res<LatestTouch>,
    rig: Res<Rig>,
    build: Res<super::build::BuildMode>,
    hands: Query<(&Hand, &Transform)>,
    world: Res<ArWorld>,
    mut teleport: MessageWriter<TeleportSkater>,
    mut pad: ResMut<ExternalPad>,
    mut aim: ResMut<Aim>,
) {
    let touch = &touch.0;
    let held = touch.left_grip >= SQUEEZE && touch.left_trigger >= SQUEEZE && !build.active;
    if !held {
        aim.ray = None;
        if let Some((point, forward)) = aim.target.take() {
            info!("XR spawn at {point:?}");
            teleport.write(TeleportSkater { point, forward });
        }
        return;
    }
    pad.set(XboxState {
        buttons: 0,
        triggers: [0, 0],
        left: [0, 0],
        right: [0, 0],
    });
    let Some(root) = rig.root_transform() else {
        return;
    };
    let Some((_, controller)) = hands.iter().find(|(hand, _)| matches!(hand, Hand::Left)) else {
        return;
    };
    let origin = root.transform_point(controller.translation);
    let direction = (root.rotation * (controller.rotation * Vec3::NEG_Z)).normalize();
    let hit = raycast(&world, origin, direction);
    aim.target = hit.map(|point| (point, direction));
    aim.ray = Some(match hit {
        Some(point) => (origin, point, true),
        None => (origin, origin + direction * MISS_LENGTH, false),
    });
}

fn show_aim(
    aim: Res<Aim>,
    mut ray: Query<(&mut Transform, &mut Visibility), (With<AimRay>, Without<AimMarker>)>,
    mut marker: Query<(&mut Transform, &mut Visibility), (With<AimMarker>, Without<AimRay>)>,
) {
    let (Ok((mut ray_tf, mut ray_vis)), Ok((mut marker_tf, mut marker_vis))) =
        (ray.single_mut(), marker.single_mut())
    else {
        return;
    };
    let Some((start, end, valid)) = aim.ray else {
        *ray_vis = Visibility::Hidden;
        *marker_vis = Visibility::Hidden;
        return;
    };
    let length = start.distance(end).max(1e-3);
    *ray_tf = Transform {
        translation: (start + end) * 0.5,
        rotation: Quat::from_rotation_arc(Vec3::Y, (end - start) / length),
        scale: Vec3::new(1.0, length, 1.0),
    };
    *ray_vis = Visibility::Visible;
    if valid {
        *marker_tf = Transform::from_translation(end + Vec3::Y * 0.03)
            .with_rotation(Quat::from_rotation_arc(Vec3::Z, Vec3::Y));
        *marker_vis = Visibility::Visible;
    } else {
        *marker_vis = Visibility::Hidden;
    }
}

/// Nearest ground-like collision face along the ray.
fn raycast(world: &ArWorld, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    world
        .all_triangles()
        .filter_map(|[a, b, c]| {
            let normal = (b - a).cross(c - a).normalize_or_zero();
            if normal.y.abs() < MIN_UP {
                return None;
            }
            ray_triangle(origin, direction, a, b, c)
        })
        .filter(|&t| t <= MAX_RANGE)
        .min_by(f32::total_cmp)
        .map(|t| origin + direction * t)
}

/// Möller-Trumbore, both faces.
pub(crate) fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// Row-major teleport matrix (axes in rows 0-2, position in row 3); the
/// skater faces row 2.
pub(super) fn spawn_matrix(point: Vec3, aim: Vec3) -> [[f32; 4]; 4] {
    let forward = aim.with_y(0.0).try_normalize().unwrap_or(Vec3::Z);
    let right = Vec3::Y.cross(forward);
    let p = point + Vec3::Y * SPAWN_LIFT;
    [
        [right.x, right.y, right.z, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [forward.x, forward.y, forward.z, 0.0],
        [p.x, p.y, p.z, 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_the_floor_below() {
        let t = ray_triangle(
            Vec3::new(0.2, 2.0, 0.2),
            Vec3::NEG_Y,
            Vec3::ZERO,
            Vec3::X,
            Vec3::Z,
        )
        .unwrap();
        assert!((t - 2.0).abs() < 1e-5);
        assert!(ray_triangle(Vec3::new(2.0, 2.0, 2.0), Vec3::NEG_Y, Vec3::ZERO, Vec3::X, Vec3::Z).is_none());
    }

    #[test]
    fn identity_heading_matches_the_course_spawn() {
        let m = spawn_matrix(Vec3::ZERO, Vec3::new(0.0, -0.5, 1.0));
        assert_eq!(m[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(m[2], [0.0, 0.0, 1.0, 0.0]);
        assert!((m[3][1] - SPAWN_LIFT).abs() < 1e-6);
    }
}
