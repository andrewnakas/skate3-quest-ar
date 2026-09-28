//! Bounded ZIP extraction. A session cache never overwrites another package.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Cache {
    session: Option<PathBuf>,
    entries: BTreeMap<PathBuf, (blake3::Hash, PathBuf)>,
    next: u64,
}

fn safe_name(name: &str) -> Result<PathBuf, String> {
    let name = name.strip_suffix('/').unwrap_or(name);
    if name.is_empty()
        || name.len() > 240
        || name.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
    {
        return Err("Invalid ZIP path".into());
    }
    for part in name.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || matches!(
                stem.as_str(),
                "con"
                    | "prn"
                    | "aux"
                    | "nul"
                    | "com1"
                    | "com2"
                    | "com3"
                    | "com4"
                    | "com5"
                    | "com6"
                    | "com7"
                    | "com8"
                    | "com9"
                    | "lpt1"
                    | "lpt2"
                    | "lpt3"
                    | "lpt4"
                    | "lpt5"
                    | "lpt6"
                    | "lpt7"
                    | "lpt8"
                    | "lpt9"
            )
        {
            return Err(
                "ZIP paths must be ordinary relative paths without traversal or device names"
                    .into(),
            );
        }
    }
    Ok(PathBuf::from(name))
}

impl Cache {
    pub fn materialize(&mut self, root: &Path, archive: &Path) -> Result<PathBuf, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(archive)
            .map_err(|e| e.to_string())?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("ZIP exceeds 64 MiB compressed".into());
        }
        let hash = blake3::hash(&bytes);
        if let Some((old, path)) = self.entries.get(archive) {
            if old == &hash {
                return Ok(path.clone());
            }
        }
        // Validate and decompress into bounded memory before publishing anything.
        let mut zip =
            zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
        if zip.len() > 512 {
            return Err("ZIP exceeds 512 entries".into());
        }
        let mut names = BTreeSet::new();
        let mut files = Vec::new();
        let mut total = 0_u64;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
            let path = safe_name(file.name())?;
            if !names.insert(path.to_string_lossy().to_lowercase()) {
                return Err("Duplicate/case-colliding ZIP path".into());
            }
            let kind = file.unix_mode().unwrap_or(0) & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return Err("ZIP links and special files are unsupported".into());
            }
            if file.is_dir() {
                continue;
            }
            if file.size() > LIMIT - total {
                return Err("ZIP exceeds 64 MiB expanded".into());
            }
            let mut contents = Vec::new();
            file.by_ref()
                .take(LIMIT - total + 1)
                .read_to_end(&mut contents)
                .map_err(|e| e.to_string())?;
            total += contents.len() as u64;
            if total > LIMIT {
                return Err("ZIP exceeds 64 MiB expanded".into());
            }
            files.push((path, contents));
        }
        if !files.iter().any(|(p, _)| p == Path::new("mod.json")) {
            return Err(
                "ZIP must contain mod.json at its root, not inside a wrapper folder".into(),
            );
        }
        if self.session.is_none() {
            let root = root.canonicalize().map_err(|e| e.to_string())?;
            let cache = root.join(".cache");
            if cache.exists()
                && std::fs::symlink_metadata(&cache)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
            {
                return Err("ZIP cache must not be a link".into());
            }
            std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
            if !cache
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&root)
            {
                return Err("ZIP cache escapes mods folder".into());
            }
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let session = cache.join(format!("{}-{nonce}", std::process::id()));
            std::fs::create_dir(&session).map_err(|e| e.to_string())?;
            self.session = Some(session);
        }
        self.next += 1;
        let destination = self.session.as_ref().unwrap().join(self.next.to_string());
        std::fs::create_dir(&destination).map_err(|e| e.to_string())?;
        for (path, contents) in files {
            let target = destination.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .and_then(|mut f| f.write_all(&contents))
                .map_err(|e| e.to_string())?;
        }
        self.entries
            .insert(archive.to_owned(), (hash, destination.clone()));
        Ok(destination)
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        if let Some(path) = &self.session {
            // Only this process's uniquely created session, and never a redirected path.
            if let (Ok(actual), Ok(parent)) =
                (path.canonicalize(), path.parent().unwrap().canonicalize())
            {
                if actual.parent() == Some(parent.as_path())
                    && actual.file_name() == path.file_name()
                {
                    let _ = std::fs::remove_dir_all(path);
                }
            }
        }
    }
}
