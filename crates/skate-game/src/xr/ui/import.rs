//! Importing the player's own Skate 3 data in the headset, for the full
//! retail tier (the faithful port: retail controls, animations, skaters).
//!
//! The headset can't convert a disc. The data is what the desktop setup
//! makes from the player's ISO (`skate3setup`, then
//! `scripts/quest/make-bundle.ps1`): an `assets` folder with
//! `private/game.json`, and beside it `audio-pcm-cache` (decoded sound banks).
//! Put that folder, or a zip of it, in `Download/Skate3` on the headset (over
//! USB, or any file app); the import moves a folder in (no second copy when
//! the storage allows it, else copies) or unzips a zip into the app's files,
//! then writes `use-retail-data` so the next launch runs the retail tier.
//! `scripts/quest/push-assets.sh` does the same over adb.
//!
//! On Quest, reading `Download` needs "All files access" for the app; the
//! import asks for it (see [`crate::set_storage_hooks`]).
use bevy::prelude::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MARKER: &str = "use-retail-data";
const GAME_JSON: &str = "private/game.json";
const PCM_CACHE: &str = "audio-pcm-cache";

/// Where a converted bundle can be found.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
    /// An `assets` folder (its `audio-pcm-cache` sibling, if any, comes too).
    Folder(PathBuf),
    /// A zip with `.../assets/private/game.json` inside.
    Zip(PathBuf),
}

impl Source {
    pub(crate) fn describe(&self) -> String {
        let (kind, path) = match self {
            Source::Folder(p) => ("folder", p),
            Source::Zip(p) => ("zip", p),
        };
        format!("{kind} {}", path.display())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Status {
    #[default]
    Idle,
    /// Last scan's finds (empty: nothing there).
    Scanned(Vec<Source>),
    Working { what: String, done: u64, total: u64 },
    Done(Result<String, String>),
}

/// The import's state, shared with its worker thread.
#[derive(Resource, Clone, Default)]
pub(crate) struct Import {
    status: Arc<Mutex<Status>>,
    /// The app's files directory (where `assets/` and the marker live).
    pub files: PathBuf,
}

impl Import {
    pub(crate) fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set(&self, status: Status) {
        if let Ok(mut s) = self.status.lock() {
            *s = status;
        }
    }

    pub(crate) fn working(&self) -> bool {
        matches!(self.status(), Status::Working { .. })
    }

    /// Imported data is in place.
    pub(crate) fn installed(&self) -> bool {
        self.files.join("assets").join(GAME_JSON).is_file()
    }

    /// The next launch runs the retail tier.
    pub(crate) fn retail_next(&self) -> bool {
        self.files.join(MARKER).exists()
    }

    pub(crate) fn set_retail_next(&self, retail: bool) -> Result<(), String> {
        let marker = self.files.join(MARKER);
        let result = if retail {
            fs::write(&marker, b"")
        } else if marker.exists() {
            fs::remove_file(&marker)
        } else {
            Ok(())
        };
        result.map_err(|e| format!("{}: {e}", marker.display()))
    }

    pub(crate) fn scan(&self) {
        let found = scan_dirs().iter().flat_map(|d| scan(d)).collect();
        self.set(Status::Scanned(found));
    }

    /// Move / unzip `source` into the files directory on a worker thread.
    pub(crate) fn start(&self, source: Source) {
        if self.working() {
            return;
        }
        self.set(Status::Working { what: "Starting".into(), done: 0, total: 0 });
        let this = self.clone();
        std::thread::Builder::new()
            .name("skate3-import".into())
            .spawn(move || {
                let result = this.run(&source);
                match &result {
                    Ok(msg) => info!("IMPORT done: {msg}"),
                    Err(e) => warn!("IMPORT failed: {e}"),
                }
                this.set(Status::Done(result));
            })
            .map_err(|e| self.set(Status::Done(Err(e.to_string()))))
            .ok();
    }

    fn run(&self, source: &Source) -> Result<String, String> {
        info!("IMPORT from {}", source.describe());
        let assets = self.files.join("assets");
        let cache = self.files.join("Skate3RustEngine").join(PCM_CACHE);
        let progress = |what: &str, done: u64, total: u64| {
            self.set(Status::Working { what: what.to_string(), done, total });
        };
        match source {
            Source::Folder(from) => {
                replace_dir(from, &assets, &progress).map_err(|e| format!("assets: {e}"))?;
                if let Some(pcm) = from.parent().map(|p| p.join(PCM_CACHE)).filter(|p| p.is_dir()) {
                    replace_dir(&pcm, &cache, &progress).map_err(|e| format!("sound banks: {e}"))?;
                }
            }
            Source::Zip(zip) => {
                unzip(zip, &assets, &cache, &progress)?;
            }
        }
        if !assets.join(GAME_JSON).is_file() {
            return Err(format!("no {GAME_JSON} after the import"));
        }
        self.set_retail_next(true)?;
        Ok("Skate 3 data imported. Restart the app to play it.".into())
    }
}

pub(crate) struct ImportPlugin;

impl Plugin for ImportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, |files: Res<crate::xr::XrFiles>, mut commands: Commands| {
            commands.insert_resource(Import { files: files.0.clone(), ..default() });
        });
    }
}

/// Where to look: `Download/Skate3` (and `Skate3`) on the headset, or
/// `SKATE3_IMPORT_DIR`.
fn scan_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("SKATE3_IMPORT_DIR") {
        return vec![dir.into()];
    }
    if cfg!(target_os = "android") {
        vec!["/sdcard/Download/Skate3".into(), "/sdcard/Skate3".into()]
    } else {
        std::env::var_os("HOME").map(|h| vec![Path::new(&h).join("Downloads/Skate3")]).unwrap_or_default()
    }
}

/// Bundles in `dir`: an `assets` folder with `private/game.json` at most two
/// levels down, and zips there.
pub(crate) fn scan(dir: &Path) -> Vec<Source> {
    let mut found = Vec::new();
    let visit = |d: &Path, depth: u32, found: &mut Vec<Source>| -> Vec<PathBuf> {
        if d.join(GAME_JSON).is_file() {
            found.push(Source::Folder(d.to_path_buf()));
            return Vec::new();
        }
        let mut next = Vec::new();
        for entry in fs::read_dir(d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 2 {
                next.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
                found.push(Source::Zip(path));
            }
        }
        next
    };
    let mut level = vec![dir.to_path_buf()];
    for depth in 0..=2 {
        let mut next = Vec::new();
        for d in &level {
            next.extend(visit(d, depth, &mut found));
        }
        level = next;
    }
    found.sort_by_key(|s| matches!(s, Source::Zip(_)));
    found
}

/// Put `from` at `to` (replacing it): a rename when the storage allows,
/// else a copy.
fn replace_dir(from: &Path, to: &Path, progress: &dyn Fn(&str, u64, u64)) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if to.exists() {
        fs::remove_dir_all(to)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let files = list_files(from)?;
    let total: u64 = files.iter().map(|(_, len)| len).sum();
    let mut done = 0;
    for (path, len) in files {
        let rel = path.strip_prefix(from).map_err(io::Error::other)?;
        let out = to.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&path, &out)?;
        done += len;
        progress(&format!("Copying {}", rel.display()), done, total);
    }
    Ok(())
}

fn list_files(dir: &Path) -> io::Result<Vec<(PathBuf, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                out.push((entry.path(), meta.len()));
            }
        }
    }
    Ok(out)
}

/// Where a zip's `assets` folder starts: the prefix of its
/// `assets/private/game.json` entry, e.g. `quest-data/assets/`.
pub(crate) fn assets_prefix<'a>(names: impl Iterator<Item = &'a str>) -> Option<String> {
    names
        .map(|n| n.replace('\\', "/"))
        .filter(|n| n.ends_with(GAME_JSON) && n.len() > GAME_JSON.len())
        .min_by_key(|n| n.len())
        .map(|n| n[..n.len() - GAME_JSON.len()].to_string())
}

fn unzip(zip: &Path, assets: &Path, cache: &Path, progress: &dyn Fn(&str, u64, u64)) -> Result<String, String> {
    let file = fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file)).map_err(|e| format!("{}: {e}", zip.display()))?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let prefix = assets_prefix(names.iter().map(String::as_str))
        .ok_or_else(|| format!("{} has no assets/{GAME_JSON}", zip.display()))?;
    // `audio-pcm-cache` sits beside `assets`.
    let parent = prefix.trim_end_matches('/').rsplit_once('/').map_or(String::new(), |(p, _)| format!("{p}/"));
    let pcm_prefix = format!("{parent}{PCM_CACHE}/");
    for dir in [assets, cache] {
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    let total: u64 = (0..archive.len())
        .filter_map(|i| archive.by_index_raw(i).ok().map(|f| f.size()))
        .sum();
    let mut done = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let size = entry.size();
        let (root, rel) = if let Some(rel) = name.strip_prefix(&prefix) {
            (assets, rel.to_string())
        } else if let Some(rel) = name.strip_prefix(&pcm_prefix) {
            (cache, rel.to_string())
        } else {
            continue;
        };
        // No absolute paths or `..` out of the target.
        let Some(safe) = entry.enclosed_name().map(|_| PathBuf::from(&rel)) else { continue };
        if entry.is_dir() || rel.is_empty() {
            continue;
        }
        let out = root.join(safe);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut file = fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        io::copy(&mut entry, &mut file).map_err(|e| format!("{}: {e}", out.display()))?;
        done += size;
        progress(&format!("Unzipping {rel}"), done, total);
    }
    Ok(format!("unzipped {}", zip.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_assets_prefix_in_a_zip() {
        let names = ["quest-data/", "quest-data/assets/private/game.json", "quest-data/audio-pcm-cache/a.v3.pcm"];
        assert_eq!(assets_prefix(names.into_iter()), Some("quest-data/assets/".into()));
        assert_eq!(assets_prefix(["assets/private/game.json"].into_iter()), Some("assets/".into()));
        assert_eq!(assets_prefix(["private/game.json", "other.txt"].into_iter()), None);
    }

    #[test]
    fn scans_folders_and_zips() {
        let dir = std::env::temp_dir().join(format!("skate3-import-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("quest-data/assets/private")).unwrap();
        fs::write(dir.join("quest-data/assets/private/game.json"), b"{}").unwrap();
        fs::write(dir.join("bundle.zip"), b"").unwrap();
        let found = scan(&dir);
        assert_eq!(
            found,
            vec![Source::Folder(dir.join("quest-data/assets")), Source::Zip(dir.join("bundle.zip"))]
        );
        // Moving it in.
        let files = dir.join("files");
        let import = Import { files: files.clone(), ..default() };
        import.run(&found[0]).unwrap();
        assert!(import.installed() && import.retail_next());
        import.set_retail_next(false).unwrap();
        assert!(!import.retail_next());
        let _ = fs::remove_dir_all(&dir);
    }
}
