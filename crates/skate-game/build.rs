use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    // Cargo must refresh provenance after a commit as well as after source edits.
    for name in ["HEAD", "index", "refs"] {
        if let Ok(output) = Command::new("git")
            .args(["rev-parse", "--git-path", name])
            .output()
        {
            if output.status.success() {
                println!(
                    "cargo:rerun-if-changed={}",
                    String::from_utf8_lossy(&output.stdout).trim()
                );
            }
        }
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../skate-core/src");
    println!("cargo:rerun-if-changed=../skate-data/src");
    println!("cargo:rerun-if-changed=../skate-net/src");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    for path in [
        "../../Cargo.toml",
        "Cargo.toml",
        "../skate-core/Cargo.toml",
        "../skate-data/Cargo.toml",
        "../skate-net/Cargo.toml",
        "../../vendor/bevy_pbr",
        "../../vendor/bevy_core_pipeline",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let revision = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "revision-unavailable".into());
    println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");
    println!("cargo:rustc-env=SKATE_RELEASE_REVISION={revision}");
    println!(
        "cargo:rustc-env=SKATE_RELEASE_BUILD={}",
        env::var("GITHUB_RUN_NUMBER").unwrap_or_else(|_| "0".into())
    );
    let dirty = git(&["status", "--porcelain", "--untracked-files=normal"]).map(|s| !s.is_empty());
    let compiler = Command::new(env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    println!(
        "cargo:rustc-env=SKATE_BUILD_ID={} revision={} dirty={dirty:?} build_unix_ns={stamp} target={} profile={} dynamic={} compiler={compiler}",
        env::var("CARGO_PKG_VERSION").unwrap(),
        revision,
        env::var("TARGET").unwrap(),
        env::var("PROFILE").unwrap(),
        env::var_os("CARGO_FEATURE_DEV_DYNAMIC").is_some()
    );
    println!("cargo:rerun-if-changed=../../docs/images/skating-crab.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let sdk = PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK location"))
        .join("Windows Kits/10/bin");
    let rc = env::var_os("RC")
        .map(PathBuf::from)
        .or_else(|| {
            let mut paths: Vec<_> = fs::read_dir(sdk)
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("x64/rc.exe"))
                .filter(|path| path.is_file())
                .collect();
            paths.sort();
            paths.pop()
        })
        .expect("Install the Windows SDK resource compiler or set RC to rc.exe");
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../docs/images/skating-crab.ico")
        .canonicalize()
        .unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let script = output.join("skate3rust.rc");
    let resource = output.join("skate3rust.res");
    fs::write(
        &script,
        format!(
            "1 ICON \"{}\"\n",
            icon.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    assert!(
        Command::new(rc)
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(script)
            .status()
            .expect("Run Windows resource compiler")
            .success(),
        "Icon compilation failed"
    );
    println!("cargo:rustc-link-arg-bin=skate3rust={}", resource.display());
}
