use skate_mods::Manager;
use std::{
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "skate-zip-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn zip(&self, name: &str, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(self.0.join(name)).unwrap());
        for (name, contents) in files {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
    }
    fn manager(&self) -> Manager {
        Manager::new(self.0.clone(), self.0.join(".settings"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let actual = self.0.canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(actual.parent() == Some(temp.as_path()));
        std::fs::remove_dir_all(actual).unwrap();
    }
}
const MANIFEST: &[u8] = br#"{"id":"zip.example","api":1,"name":"ZIP Example","version":"1.0.0","author":"Test","description":"Test","entry":"main.lua"}"#;

#[test]
fn zip_assets_reload_and_removal_follow_the_archive() {
    let f = Fixture::new();
    let code = b"return {on_load=function() sdk.log(sdk.read_text('assets/message.txt')) end}";
    f.zip(
        "example.zip",
        &[
            ("mod.json", MANIFEST),
            ("main.lua", code),
            ("assets/message.txt", b"first"),
        ],
    );
    let mut m = f.manager();
    m.scan(true);
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    m.enable("zip.example", true).unwrap();
    assert!(m.packages["zip.example"].running());
    let old = m.packages["zip.example"].root.clone();
    assert_eq!(
        std::fs::read(old.join("assets/message.txt")).unwrap(),
        b"first"
    );
    m.scan(true);
    assert_eq!(m.packages["zip.example"].root, old);
    f.zip(
        "example.zip",
        &[
            ("mod.json", MANIFEST),
            ("main.lua", code),
            ("assets/message.txt", b"second"),
        ],
    );
    m.scan(true);
    assert_ne!(m.packages["zip.example"].root, old);
    assert!(m.packages["zip.example"].running());
    std::fs::remove_file(f.0.join("example.zip")).unwrap();
    m.scan(true);
    assert!(m.packages.is_empty());
    assert!(m.retired.contains(&"zip.example".to_owned()));
}

#[test]
fn unsafe_paths_wrapper_folders_and_case_collisions_are_rejected() {
    for name in [
        "../escape.txt",
        "/absolute.txt",
        "C:/escape.txt",
        "a\\escape.txt",
        "nul.txt",
        "a./x",
        "a/../x",
    ] {
        let f = Fixture::new();
        f.zip("bad.zip", &[("mod.json", MANIFEST), (name, b"bad")]);
        let mut m = f.manager();
        m.scan(true);
        assert!(m.packages.is_empty(), "{name}");
        assert!(!m.diagnostics.is_empty());
    }
    for files in [
        vec![("wrapper/mod.json", MANIFEST)],
        vec![("mod.json", MANIFEST), ("MOD.JSON", MANIFEST)],
    ] {
        let f = Fixture::new();
        f.zip("bad.zip", &files);
        let mut m = f.manager();
        m.scan(true);
        assert!(m.packages.is_empty());
        assert!(!m.diagnostics.is_empty());
    }
}

#[test]
fn expansion_limits_and_duplicate_ids_are_rejected() {
    let f = Fixture::new();
    let huge = vec![0_u8; 64 * 1024 * 1024 + 1];
    f.zip("huge.zip", &[("mod.json", MANIFEST), ("huge.bin", &huge)]);
    let mut m = f.manager();
    m.scan(true);
    assert!(m.packages.is_empty());
    assert!(m.diagnostics.iter().any(|s| s.contains("64 MiB")));
    std::fs::remove_file(f.0.join("huge.zip")).unwrap();
    for name in ["one.zip", "two.zip"] {
        f.zip(name, &[("mod.json", MANIFEST), ("main.lua", b"return {}")]);
    }
    m.scan(true);
    assert!(m.packages.is_empty());
    assert!(m.diagnostics.iter().any(|s| s.contains("Duplicate mod ID")));
}

#[test]
fn links_excess_entries_and_invalid_replacements_do_not_run() {
    let f = Fixture::new();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(f.0.join("bad.zip")).unwrap());
    zip.add_symlink(
        "escape",
        "../outside",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    let mut m = f.manager();
    m.scan(true);
    assert!(m.packages.is_empty());
    assert!(m.diagnostics.iter().any(|s| s.contains("links")));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(f.0.join("bad.zip")).unwrap());
    for i in 0..513 {
        zip.start_file(
            format!("file-{i}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    }
    zip.finish().unwrap();
    m.scan(true);
    assert!(m.diagnostics.iter().any(|s| s.contains("512")));
    f.zip(
        "bad.zip",
        &[("mod.json", MANIFEST), ("main.lua", b"return {}")],
    );
    m.scan(true);
    m.enable("zip.example", true).unwrap();
    assert!(m.packages["zip.example"].running());
    std::fs::write(f.0.join("bad.zip"), b"not a ZIP").unwrap();
    m.scan(true);
    assert!(m.packages.is_empty());
    assert!(!m.diagnostics.is_empty());
}
