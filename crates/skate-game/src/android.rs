//! Quest (Android) process setup. The app's external files directory
//! (`/sdcard/Android/data/<package>/files`) stands in for the desktop
//! package folder: `assets/` is the converted installation and
//! `Skate3RustEngine/audio-pcm-cache/` the pre-decoded sound banks, both
//! pushed with `scripts/quest/push-assets.sh`.
use std::path::PathBuf;

pub(crate) fn files_dir() -> Result<PathBuf, String> {
    bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app handle not set; start through bevy_main")?
        .external_data_path()
        .ok_or_else(|| "Android external files directory is unavailable".into())
}

/// Must run before any thread reads the environment.
pub(crate) fn prepare() -> Result<(), String> {
    let files = files_dir()?;
    eprintln!("SKATE_ANDROID files={}", files.display());
    // Every engine cache path is `$LOCALAPPDATA/Skate3RustEngine/...`.
    // SAFETY: called first thing in `run`, before the engine starts threads.
    unsafe {
        std::env::set_var("LOCALAPPDATA", &files);
    }
    Ok(())
}
