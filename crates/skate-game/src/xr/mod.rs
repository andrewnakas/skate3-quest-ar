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

mod actions;
mod planes;
mod real_world;
pub(crate) mod rig;
mod spawn;
pub(crate) mod touch;

pub(crate) use crate::config::XrOptions;

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
    // Without this the runtime never receives the Activity (bevy_mod_openxr
    // doesn't enable it), and the session stays IDLE forever on Quest.
    #[cfg(target_os = "android")]
    {
        exts.raw_mut().khr_android_create_instance = true;
    }
    add_xr_plugins(base).set(OxrInitPlugin {
        app_info: AppInfo {
            name: "Skate 3 AR".into(),
            ..AppInfo::BEVY
        },
        exts,
        ..default()
    })
}

pub(crate) struct XrPlugin(pub XrOptions);

impl Plugin for XrPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_mod_openxr::features::fb_passthrough::OxrFbPassthroughPlugin)
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
                rig::RigPlugin(self.0),
                real_world::RealWorldPlugin,
                spawn::SpawnPlugin,
            ))
            // The world plugin sets an opaque clear colour at build time;
            // passthrough needs alpha 0 wherever nothing was drawn.
            .add_systems(PostStartup, |mut clear: ResMut<ClearColor>| {
                clear.0 = Color::NONE;
            })
            .add_systems(PostUpdate, headset_only_rendering);
    }
}

/// Quest's GPU only has budget for the two eye views. The flat-screen
/// gameplay camera (re-activated every frame by `camera::present`) and the
/// window HUD cameras render where nobody can see them, and shadow cascades
/// per eye were most of the remaining cost (13 fps vs 72 on device).
fn headset_only_rendering(
    mut cameras: Query<&mut Camera, Without<bevy_mod_xr::camera::XrCamera>>,
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
