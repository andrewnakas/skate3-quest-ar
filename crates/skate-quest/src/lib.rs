//! Android entry point for the Quest build. The game itself is
//! `skate_game::run()`; on Android it always starts in XR passthrough mode
//! and reads assets from the app's external files directory
//! (`/sdcard/Android/data/com.andrewnakas.skate3ar/files/assets`).

#[cfg(target_os = "android")]
mod logcat;

#[bevy::prelude::bevy_main]
pub fn main() {
    #[cfg(target_os = "android")]
    logcat::capture_stdio();
    let exit = skate_game::run();
    eprintln!("skate_game::run exited: {exit:?}");
}
