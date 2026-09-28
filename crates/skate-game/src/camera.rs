//! Normal gameplay camera rendering endpoint. Simulation owns the cadence of
//! CameraRuntime::advance after its complete physical/animation publication.
mod collision;
mod graph;
mod graph_conditions;
mod graph_subject;
mod publication;
mod runtime;
mod settings;
mod shake_data;
mod shot_data;
mod stock_names;
mod subject;
mod trajectory;
mod world_query;
use crate::{app::FrameSet, config::Config};
use bevy::prelude::*;
pub(crate) use graph_subject::CameraGraphEnvironment;
pub(crate) use publication::{
    CameraAirOutput, CameraAnimationOutput, CameraEventsOutput, CameraGrindOutput,
    CameraOffboardOutput, CameraPreferences, CameraPublicationInputs, CameraStateOutput,
    snapshot as publish_camera_subject,
};
pub(crate) use runtime::CameraRuntime;

#[derive(Component)]
pub(crate) struct GameplayCamera;

pub(crate) struct CameraPlugin;
impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(
            Update,
            present
                .after(FrameSet::Animation)
                .before(FrameSet::Verification),
        );
    }
}
fn spawn(
    mut commands: Commands,
    config: Res<Config>,
    retail: Res<crate::retail_render::RetailScene>,
) {
    let runtime = CameraRuntime::load(&config.asset_root)
        .unwrap_or_else(|error| panic!("Cannot initialize normal gameplay camera: {error}"));
    commands.insert_resource(runtime);
    let mut camera = commands.spawn((
        GameplayCamera,
        Camera3d::default(),
        Camera {
            is_active: false,
            ..default()
        },
        Transform::default(),
    ));
    if retail.0 {
        camera.insert((
            bevy::render::view::Hdr,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            crate::retail_render::RetailTone::default(),
        ));
    }
}

#[derive(Default)]
pub(crate) struct VehicleCameraBlend {
    active: bool,
    previous: Option<Transform>,
    from: Option<Transform>,
    elapsed: f32,
}

pub(crate) fn present(
    vehicles: Res<crate::modding::vehicles::Vehicles>,
    mut runtime: ResMut<CameraRuntime>,
    windows: Query<&Window>,
    history: Res<crate::presentation::Presentation>,
    time: Res<Time<Fixed>>,
    replay: Res<crate::replay::Replay>,
    virtual_time: Res<Time<Virtual>>,
    mut vehicle_blend: Local<VehicleCameraBlend>,
    customiser: Option<Res<crate::customiser::Customiser>>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<GameplayCamera>>,
) {
    if let Ok(window) = windows.single() {
        // A minimised window reports zero height, and 0/0 is NaN. That NaN
        // reaches the field of view, the frame fails the finite check in
        // `runtime`, and the game exits with "Normal gameplay camera produced
        // a non-finite frame" -- so minimising the window killed the running
        // game. Keep the last good ratio instead: there is nothing to draw at
        // zero area anyway, and it is restored the moment the window is.
        let (width, height) = (window.width(), window.height());
        if width > 0. && height > 0. {
            runtime.set_aspect_ratio(width / height);
        }
    }
    let Some((previous, current, alpha)) = history.view(&replay, time.overstep_fraction()) else {
        return;
    };
    for (mut camera, mut transform, mut projection) in &mut cameras {
        *transform = crate::presentation::blend(previous.camera, current.camera, alpha);
        if replay.active {
            if let Some(free) = replay.free_camera {
                *transform = free;
            }
        }
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = previous.fov + (current.fov - previous.fov) * alpha;
        }
        if let Some(customiser) = customiser.as_ref().filter(|c| c.open) {
            let (height, distance, yaw) = customiser.preview_camera();
            let center = current.root.translation + Vec3::Y * height;
            let offset =
                current.root.rotation * Quat::from_rotation_y(yaw) * Vec3::new(0.0, 0.25, distance);
            let eye = center + offset;
            let right = offset.normalize().cross(Vec3::Y).normalize();
            *transform = Transform::from_translation(eye)
                .looking_at(center + right * (distance * 0.265625), Vec3::Y);
            if let Projection::Perspective(p) = &mut *projection {
                p.fov = 50_f32.to_radians();
            }
        }
        let vehicle = vehicles.camera();
        let active = vehicle.is_some();
        if active != vehicle_blend.active {
            vehicle_blend.from = vehicle_blend.previous;
            vehicle_blend.elapsed = 0.;
            vehicle_blend.active = active;
        }
        if let Some(pose) = vehicle {
            *transform = pose;
        }
        vehicle_blend.elapsed += virtual_time.delta_secs();
        if let Some(from) = vehicle_blend.from {
            if vehicle_blend.elapsed < 0.5 && from.translation.distance(transform.translation) < 50.
            {
                let t = (vehicle_blend.elapsed / 0.5).clamp(0., 1.);
                *transform = crate::presentation::blend(from, *transform, t * t * (3. - 2. * t));
            } else {
                vehicle_blend.from = None;
            }
        }
        // Vehicle motion is already interpolated with the rider/chassis. A separate
        // follow filter introduces relative motion and makes fixed ticks visible.
        vehicle_blend.previous = Some(*transform);
        camera.is_active = true;
    }
}

/// Keep the presentation camera/target/settings while replacing world-specific
/// postprocessing. A procedural scene must not inherit native HDR/tone state.
pub(crate) fn set_world_environment(world: &mut World, retail: bool) {
    let cameras: Vec<_> = world
        .query_filtered::<Entity, With<GameplayCamera>>()
        .iter(world)
        .collect();
    for id in cameras {
        let mut camera = world.entity_mut(id);
        if retail {
            camera.insert((
                bevy::render::view::Hdr,
                bevy::core_pipeline::tonemapping::Tonemapping::None,
                crate::retail_render::RetailTone::default(),
            ));
        } else {
            camera.remove::<(bevy::render::view::Hdr, crate::retail_render::RetailTone)>();
            camera.insert(bevy::core_pipeline::tonemapping::Tonemapping::default());
        }
        camera.get_mut::<Camera>().unwrap().is_active = false;
        *camera.get_mut::<Transform>().unwrap() = Transform::default();
    }
}

#[cfg(test)]
mod environment_tests {
    use super::*;
    #[test]
    fn native_procedural_switch_clears_tone_hdr_and_retains_camera_settings() {
        let mut world = World::new();
        let id = world
            .spawn((
                GameplayCamera,
                Camera3d::default(),
                Msaa::Sample4,
                bevy::camera::RenderTarget::default(),
            ))
            .id();
        for retail in [true, false, true, false] {
            set_world_environment(&mut world, retail);
            let entity = world.entity(id);
            assert_eq!(entity.contains::<bevy::render::view::Hdr>(), retail);
            assert_eq!(
                entity.contains::<crate::retail_render::RetailTone>(),
                retail
            );
            assert_eq!(*entity.get::<Msaa>().unwrap(), Msaa::Sample4);
            assert!(!entity.get::<Camera>().unwrap().is_active);
            assert_eq!(
                *entity
                    .get::<bevy::core_pipeline::tonemapping::Tonemapping>()
                    .unwrap(),
                if retail {
                    bevy::core_pipeline::tonemapping::Tonemapping::None
                } else {
                    bevy::core_pipeline::tonemapping::Tonemapping::default()
                }
            );
        }
    }
}
