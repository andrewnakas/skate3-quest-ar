//! Quest (Android) process setup. The app's external files directory
//! (`/sdcard/Android/data/<package>/files`) stands in for the desktop
//! package folder.
//!
//! Game data is the player's own converted installation, pushed with
//! `scripts/quest/push-assets.sh` into `files/assets/`. Public releases carry
//! nothing else.
//!
//! Development builds can bundle an asset pack in the APK
//! (`apk-assets/core.pack`, see [`crate::clean_pack`]); it is unpacked into
//! `files/clean/` on the first launch of each build and used unless
//! `files/use-retail-data` exists. That path is for the fully clean-room
//! pack; the current skate3-cleanroom pack still keeps retail animations,
//! graphs, tuning and colliders, so it must not ship in a public APK.
#[cfg(feature = "retail")]
use std::io::BufReader;
use std::path::PathBuf;

/// Bump when the bundled pack changes in a way its size would not show.
#[cfg(feature = "retail")]
const BUNDLE_VERSION: u32 = 2;

pub(crate) fn files_dir() -> Result<PathBuf, String> {
    bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app handle not set; start through bevy_main")?
        .external_data_path()
        .ok_or_else(|| "Android external files directory is unavailable".into())
}

/// The asset root: a pack bundled in the APK (unpacked on first use) unless
/// `files/use-retail-data` asks otherwise; else the pushed installation.
#[cfg(feature = "retail")]
pub(crate) fn asset_root() -> Result<PathBuf, String> {
    let files = files_dir()?;
    let pushed = files.join("assets");
    let app = bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app handle not set; start through bevy_main")?;
    let open = || app.asset_manager().open(c"core.pack");
    if open().is_none() || files.join("use-retail-data").exists() {
        if !pushed.join("private/game.json").is_file() {
            eprintln!(
                "SKATE_ANDROID no game data: convert your own Skate 3 disc with the desktop engine and push it with scripts/quest/push-assets.sh (expected {})",
                pushed.display()
            );
        }
        return Ok(pushed);
    }
    let open = || open().ok_or("core.pack vanished from the APK");
    let dest = files.join("clean");
    let marker = dest.join(".bundle");
    let id = format!("{BUNDLE_VERSION} {}", open()?.length());
    if std::fs::read_to_string(&marker).is_ok_and(|m| m == id) {
        return Ok(dest.join("assets"));
    }
    let started = std::time::Instant::now();
    let staging = files.join("clean.unpacking");
    let _ = std::fs::remove_dir_all(&staging);
    // Only the asset tree: the pack's settings/ (the web build's default
    // map, University, which isn't in it), maps/ and mods/ belong to the web
    // build. Without a default map the Quest starts on the AR course.
    let count = crate::clean_pack::unpack(
        BufReader::with_capacity(1 << 20, open()?),
        &staging,
        |path| path.starts_with("assets/"),
    )?;
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&staging, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    std::fs::write(&marker, &id).map_err(|e| format!("{}: {e}", marker.display()))?;
    eprintln!(
        "SKATE_ANDROID unpacked the clean pack: {count} files in {:.1} s",
        started.elapsed().as_secs_f32()
    );
    Ok(dest.join("assets"))
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
