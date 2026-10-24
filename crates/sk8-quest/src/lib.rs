//! SK8 AR, the store build of the Quest app: the clean-room game only. The Android glue is
//! skate-quest's (the GitHub build), shared file for file; this app just has no imported-disc tier
//! (skate-game is built without `retail`) and asks for no storage access.

#[cfg(target_os = "android")]
#[path = "../../skate-quest/src/activity_context.rs"]
mod activity_context;
#[cfg(target_os = "android")]
#[path = "../../skate-quest/src/logcat.rs"]
mod logcat;
#[cfg(target_os = "android")]
#[path = "../../skate-quest/src/permissions.rs"]
#[allow(dead_code)] // (the "All files access" calls are the GitHub build's importer's)
mod permissions;

#[bevy::prelude::bevy_main]
pub fn main() {
    #[cfg(target_os = "android")]
    {
        logcat::capture_stdio();
        // Must run before skate_game::run() builds the OpenXR instance.
        activity_context::point_ndk_context_at_activity();
        permissions::request_scene_permissions();
    }
    #[cfg(target_os = "android")]
    let exit = {
        let app = bevy::android::ANDROID_APP
            .get()
            .expect("bevy_main stores the AndroidApp before calling main")
            .clone();
        // Keep the Activity lifecycle moving while the game loads.
        skate_game::run_with(Some(&mut || {
            winit::platform::android::pump_lifecycle(&app, Some(std::time::Duration::from_millis(16)))
        }))
    };
    #[cfg(not(target_os = "android"))]
    let exit = skate_game::run();
    eprintln!("skate_game::run exited: {exit:?}");
}
