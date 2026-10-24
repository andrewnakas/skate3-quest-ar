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
        // The in-app Skate 3 import reads Download/Skate3.
        skate_game::set_storage_hooks(skate_game::StorageHooks {
            has_access: permissions::has_all_files_access,
            request_access: permissions::request_all_files_access,
        });
    }
    #[cfg(target_os = "android")]
    let exit = {
        let app = bevy::android::ANDROID_APP
            .get()
            .expect("bevy_main stores the AndroidApp before calling main")
            .clone();
        // Keep the Activity lifecycle moving while the game loads; see
        // `winit::platform::android::pump_lifecycle`.
        skate_game::run_with(Some(&mut || {
            winit::platform::android::pump_lifecycle(
                &app,
                Some(std::time::Duration::from_millis(16)),
            )
        }))
    };
    #[cfg(not(target_os = "android"))]
    let exit = skate_game::run();
    eprintln!("skate_game::run exited: {exit:?}");
}
