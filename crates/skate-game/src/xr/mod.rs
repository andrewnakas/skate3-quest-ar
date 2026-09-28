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
pub(crate) mod rig;
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
            .add_plugins((actions::TouchActionsPlugin, rig::RigPlugin(self.0)))
            // The world plugin sets an opaque clear colour at build time;
            // passthrough needs alpha 0 wherever nothing was drawn.
            .add_systems(PostStartup, |mut clear: ResMut<ClearColor>| {
                clear.0 = Color::NONE;
            });
    }
}
