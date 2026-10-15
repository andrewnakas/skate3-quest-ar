//! Quest (Android) process setup. The app's external files directory
//! (`/sdcard/Android/data/<package>/files`) stands in for the desktop
//! package folder.
//!
//! Game data comes from the clean-room pack bundled in the APK
//! (`apk-assets/core.pack`, see [`crate::clean_pack`]), unpacked into
//! `files/clean/` on the first launch of each build. A converted retail
//! installation pushed with `scripts/quest/push-assets.sh` (`files/assets/`)
//! is used instead only when `files/use-retail-data` exists.
use std::io::BufReader;
use std::path::PathBuf;

/// Bump when the bundled pack changes in a way its size would not show.
const BUNDLE_VERSION: u32 = 2;

pub(crate) fn files_dir() -> Result<PathBuf, String> {
    bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app handle not set; start through bevy_main")?
        .external_data_path()
        .ok_or_else(|| "Android external files directory is unavailable".into())
}

/// The asset root: the bundled clean pack (unpacked on first use), or a
/// pushed retail installation when asked for.
pub(crate) fn asset_root() -> Result<PathBuf, String> {
    let files = files_dir()?;
    let retail = files.join("assets");
    if files.join("use-retail-data").exists() && retail.join("private/game.json").is_file() {
        eprintln!("SKATE_ANDROID using pushed retail data in {}", retail.display());
        return Ok(retail);
    }
    let app = bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app handle not set; start through bevy_main")?;
    let open = || {
        app.asset_manager()
            .open(c"core.pack")
            .ok_or("The APK has no core.pack (build with scripts/quest/fetch-clean-pack.sh first)")
    };
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
