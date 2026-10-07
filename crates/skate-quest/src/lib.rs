//! Android entry point for the Quest build. The game itself is
//! `skate_game::run()`; on Android it always starts in XR passthrough mode
//! and reads assets from the app's external files directory
//! (`/sdcard/Android/data/com.andrewnakas.skate3ar/files/assets`).

#[cfg(target_os = "android")]
mod activity_context;
#[cfg(target_os = "android")]
mod logcat;
#[cfg(target_os = "android")]
mod permissions;

#[bevy::prelude::bevy_main]
pub fn main() {
    #[cfg(target_os = "android")]
    {
        logcat::capture_stdio();
        // Must run before skate_game::run() builds the OpenXR instance.
        activity_context::point_ndk_context_at_activity();
        // Scene permissions must be requested before plane detection is
        // attempted; the consent dialog, if any, happens in parallel with
        // engine startup.
        permissions::request_scene_permissions();
    }
    let exit = skate_game::run();
    eprintln!("skate_game::run exited: {exit:?}");
}
