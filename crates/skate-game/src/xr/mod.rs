//! OpenXR passthrough mode (`--xr`): the game world is shown as a miniature
//! in the real room, skated with Quest Touch controllers.
//!
//! Physics, animation and audio are untouched and still run in real-world
//! metres. The miniature is purely a view transform: the XR tracking root is
//! scaled by `k` (see [`rig`]), so the player's head and hands live in a room
//! that is `k` times larger than the physical one.
use bevy::app::PluginGroupBuilder;
use bevy::prelude::*;
use bevy_mod_openxr::{
    add_xr_plugins, exts::OxrExtensions, init::OxrInitPlugin, resources::OxrSessionConfig,
    types::AppInfo,
};

pub(crate) mod actions;
pub(crate) mod build;
pub(crate) mod link;
mod mesh_planes;
pub(crate) mod panel;
mod plane_view;
mod planes;
mod real_world;
pub(crate) mod rig;
mod spatial_audio;
pub(crate) mod spawn;
pub(crate) mod touch;

pub(crate) use crate::config::XrOptions;

/// Directory for the XR layer's saved state (`ar-layout.json`): next to the
/// assets folder for the imported-disc tier, the app's files directory for
/// the clean tier.
#[derive(Resource, Clone)]
pub(crate) struct XrFiles(pub std::path::PathBuf);

/// Wrap the game's `DefaultPlugins` so the OpenXR runtime owns rendering.
/// `add_xr_plugins` replaces the `RenderPlugin` with one built on the
/// runtime's Vulkan device; if no runtime/headset is present it falls back
/// to a normal window.
pub(crate) fn plugins(base: PluginGroupBuilder) -> PluginGroupBuilder {
    let mut exts = OxrExtensions::default();
    exts.enable_fb_passthrough();
    exts.enable_hand_tracking();
    // Space Setup surfaces (Meta Scene API), used to ground the placement
    // anchor; see `planes` and `rig`.
    let raw = exts.raw_mut();
    raw.fb_spatial_entity = true;
    raw.fb_spatial_entity_query = true;
    raw.fb_spatial_entity_storage = true;
    raw.fb_scene = true;
    // The room's triangle mesh, for objects Scene has no plane or box for.
    raw.meta_spatial_entity_mesh = true;
    // In-app "rescan the room" (runs Space Setup).
    raw.fb_scene_capture = true;
    // Probe only: Oculus 206 didn't offer it. bevy_mod_openxr drops any
    // extension the runtime lacks; `log_scene_extensions` reports the result.
    raw.ext_plane_detection = true;
    // Without this the runtime never receives the Activity (bevy_mod_openxr
    // doesn't enable it), and the session stays IDLE forever on Quest.
    #[cfg(target_os = "android")]
    {
        exts.raw_mut().khr_android_create_instance = true;
    }
    add_xr_plugins(base)
        .set(OxrInitPlugin {
            app_info: AppInfo {
                name: "Skate 3 AR".into(),
                ..AppInfo::BEVY
            },
            exts,
            ..default()
        })
        // Every bevy_mod_openxr example runs without it. With it, the OpenXR
        // frame loop is split across two threads; that is a suspect in the
        // Adreno GPU hangs.
        .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
}

/// The XR layer. `retail` adds the adapter for the faithful port; the clean
/// tier adds its own (`crate::clean`).
pub(crate) struct XrPlugin {
    pub options: XrOptions,
    pub files: std::path::PathBuf,
    pub retail: bool,
}

impl Plugin for XrPlugin {
    fn build(&self, app: &mut App) {
        if self.retail {
            app.add_plugins(link::RetailLinkPlugin);
        } else {
            app.add_plugins(panel::PanelPlugin);
        }
        app.insert_resource(XrFiles(self.files.clone()))
            .add_plugins(link::LinkPlugin)
            .add_plugins(bevy_mod_openxr::features::fb_passthrough::OxrFbPassthroughPlugin)
            .insert_resource(OxrSessionConfig {
                blend_mode_preference: vec![
                    openxr::EnvironmentBlendMode::ALPHA_BLEND,
                    openxr::EnvironmentBlendMode::ADDITIVE,
                    openxr::EnvironmentBlendMode::OPAQUE,
                ],
                ..default()
            })
            .insert_resource(crate::input::ExternalPad::default())
            .add_plugins((
                actions::TouchActionsPlugin,
                planes::PlanesPlugin,
                rig::RigPlugin(self.options),
                real_world::RealWorldPlugin,
                spawn::SpawnPlugin,
                build::BuildPlugin,
                plane_view::PlaneViewPlugin,
                spatial_audio::SpatialAudioPlugin,
            ))
            // The world plugin sets an opaque clear colour at build time;
            // passthrough needs alpha 0 wherever nothing was drawn.
            .add_systems(PostStartup, |mut clear: ResMut<ClearColor>| {
                clear.0 = Color::NONE;
            })
            .add_systems(Startup, log_scene_extensions)
            .add_systems(PostUpdate, headset_only_rendering)
            .add_observer(no_window_surface)
            .add_observer(no_eye_occlusion_culling)
            .add_observer(no_eye_indirect_drawing);
    }
}

fn log_scene_extensions(exts: Option<Res<bevy_mod_openxr::exts::OxrEnabledExtensions>>) {
    if let Some(e) = exts {
        info!(
            "XR scene extensions: spatial_entity={} query={} scene={} mesh={} scene_capture={} plane_detection={}",
            e.fb_spatial_entity,
            e.fb_spatial_entity_query,
            e.fb_scene,
            e.meta_spatial_entity_mesh,
            e.fb_scene_capture,
            e.ext_plane_detection
        );
    }
}

/// Nothing is ever drawn to the Android window, but bevy_render still builds
/// a Vulkan swapchain on it for any window with a `RawHandleWrapper`, holds
/// an acquired image forever and presents one undefined frame. That surface
/// shares the NativeActivity window with the OpenXR runtime and is torn
/// down and rebuilt on every pause/resume. Without the wrapper the window is
/// never extracted, so no surface exists. bevy_winit re-inserts the wrapper
/// on each Android resume, which re-triggers this.
fn no_window_surface(add: On<Add, bevy::window::RawHandleWrapper>, mut commands: Commands) {
    commands
        .entity(add.entity)
        .remove::<bevy::window::RawHandleWrapper>();
}

/// Draw the eye views with CPU-built batches instead of GPU preprocessing
/// and indirect draws. On device the Adreno hung at the same command-buffer
/// offset every frame from the first XR frame on, and the indirect path is
/// the one the vendored bevy_pbr patch changes. The component must be there
/// before the camera first renders, hence an observer on spawn.
fn no_eye_indirect_drawing(add: On<Add, bevy_mod_xr::camera::XrCamera>, mut commands: Commands) {
    commands
        .entity(add.entity)
        .insert(bevy::render::view::NoIndirectDrawing);
}

/// The graphics menu puts GPU occlusion culling on every `Camera3d`. The eye
/// cameras skip it: the scene is a few props, and the indirect/preprocess
/// path it enables is a suspect in the Adreno GPU hangs.
fn no_eye_occlusion_culling(
    add: On<Add, bevy::render::experimental::occlusion_culling::OcclusionCulling>,
    eyes: Query<(), With<bevy_mod_xr::camera::XrCamera>>,
    mut commands: Commands,
) {
    if eyes.contains(add.entity) {
        commands.entity(add.entity).remove::<(
            bevy::core_pipeline::prepass::DepthPrepass,
            bevy::render::experimental::occlusion_culling::OcclusionCulling,
        )>();
    }
}

/// Quest's GPU only has budget for the two eye views (and the menu panel's
/// small UI camera, `panel`, while it is open). The flat-screen
/// gameplay camera and the window HUD cameras render where nobody can see
/// them (`camera::present` leaves the gameplay one off in XR), and shadow cascades
/// per eye were most of the remaining cost (13 fps vs 72 on device).
fn headset_only_rendering(
    mut cameras: Query<
        &mut Camera,
        (Without<bevy_mod_xr::camera::XrCamera>, Without<panel::XrPanelCamera>),
    >,
    mut lights: Query<&mut DirectionalLight>,
) {
    for mut camera in &mut cameras {
        if camera.is_active {
            camera.is_active = false;
        }
    }
    for mut light in &mut lights {
        if light.shadows_enabled {
            light.shadows_enabled = false;
        }
    }
}
