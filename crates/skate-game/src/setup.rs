//! Each portable copy owns its installation; explicit --assets is for development.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn installed(base: &Path) -> Result<Option<(PathBuf, serde_json::Value)>, String> {
    let bytes = match std::fs::read(base.join("installation.json")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let marker: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if marker["version"].as_u64() != Some(1) {
        return Err("Unsupported installation version".into());
    }
    let relative = Path::new(
        marker["directory"]
            .as_str()
            .ok_or("Invalid installation path")?,
    );
    let parts: Vec<_> = relative.components().collect();
    if parts.len() != 2
        || parts[0].as_os_str() != "installations"
        || !parts[1].as_os_str().to_str().is_some_and(|s| {
            s.len() == 32
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err("Invalid installation path".into());
    }
    let assets = base.join(relative).join("assets");
    if !assets.is_dir() {
        return Ok(None);
    }
    if !assets
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(base.canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("Installation escapes its package data directory".into());
    }
    Ok(Some((assets, marker)))
}

pub(crate) fn asset_root() -> Result<PathBuf, String> {
    if std::env::args_os().any(|arg| arg == "--assets") {
        return Ok(PathBuf::from("assets"));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = exe.parent().ok_or("No executable directory")?;
    let base = root.join("data");
    let mut expected_customiser = None;
    let expected = match std::fs::read(root.join("release.json")) {
        Ok(bytes) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| e.to_string())?
                .trim_start_matches('\u{feff}');
            let release: serde_json::Value =
                serde_json::from_str(text).map_err(|e| e.to_string())?;
            expected_customiser = release["character_customiser"].as_str().map(str::to_owned);
            release.get("asset_pipelines").cloned()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let existing = installed(&base)?;
    if let Some((assets, marker)) = &existing {
        if expected
            .as_ref()
            .is_none_or(|versions| marker.get("pipelines") == Some(versions))
            && assets.join("private/game.json").is_file()
            && marker.get("outputs").is_none_or(|groups| {
                groups.as_object().is_some_and(|groups| {
                    groups
                        .values()
                        .all(|files| receipt_present(assets.parent().unwrap(), files))
                })
            })
            && customiser_current(assets, expected_customiser.as_deref())
        {
            return Ok(assets.clone());
        }
    }
    let setup = root.join("support/skate3setup.exe");
    if !setup.is_file() {
        return Err("This copy has not been set up. Use the complete Windows package, or --assets DIRECTORY for development.".into());
    }
    let mut command = Command::new(setup);
    command.arg("--base").arg(&base).arg("--game-exe").arg(&exe);
    if existing.is_some() {
        command.arg("--refresh");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command
        .status()
        .map_err(|e| format!("Could not start setup: {e}"))?;
    if !status.success() {
        return Err("Setup was cancelled or did not complete".into());
    }
    let (assets, marker) =
        installed(&base)?.ok_or("Setup did not publish a complete installation")?;
    if expected
        .as_ref()
        .is_some_and(|versions| marker.get("pipelines") != Some(versions))
    {
        return Err("Setup helper does not match this release's asset extractors. Unpack the complete package.".into());
    }
    if !customiser_current(&assets, expected_customiser.as_deref()) {
        return Err("Character customiser preparation did not complete for this release.".into());
    }
    Ok(assets)
}

fn customiser_current(assets: &Path, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| {
        let degraded =
            std::fs::read(assets.join("private/customisation/customiser-availability.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if let Some(v) = degraded.filter(|v| {
            v["version"].as_u64() == Some(1) && v["fingerprint"].as_str() == Some(expected)
        }) {
            if v["status"] == "unavailable" {
                return true;
            }
            if v["status"] == "retained" {
                let directory = crate::customiser_parts::asset_directory(assets);
                return ["catalog", "library", "menu", "lighting", "roster"]
                    .iter()
                    .all(|stage| {
                        std::fs::read(directory.join(format!("{stage}-complete.json")))
                            .ok()
                            .and_then(|bytes| {
                                serde_json::from_slice::<serde_json::Value>(&bytes).ok()
                            })
                            .is_some_and(|v| receipt_present(&directory, &v["files"]))
                    });
            }
        }
        std::fs::read(assets.join("private/customisation/current.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| {
                v["version"].as_u64() == Some(1)
                    && v["fingerprint"].as_str() == Some(expected)
                    && v["set"]
                        .as_str()
                        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            })
            && [
                "library-v3.json",
                "extra-menu.json",
                "native-lighting.json",
                "native-roster/complete.json",
            ]
            .iter()
            .all(|name| {
                crate::customiser_parts::asset_directory(assets)
                    .join(name)
                    .is_file()
            })
            && ["catalog", "library", "menu", "lighting", "roster"]
                .iter()
                .all(|stage| {
                    let directory = crate::customiser_parts::asset_directory(assets);
                    std::fs::read(directory.join(format!("{stage}-complete.json")))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .is_some_and(|v| receipt_present(&directory, &v["files"]))
                })
    })
}

// Cheap launch-time completeness check. Setup verifies SHA-256 before reuse;
// hashing every map on every game launch would read gigabytes unnecessarily.
fn receipt_present(root: &Path, files: &serde_json::Value) -> bool {
    files.as_object().is_some_and(|files| {
        !files.is_empty()
            && files.iter().all(|(name, entry)| {
                let relative = Path::new(name);
                !relative.is_absolute()
                    && relative
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))
                    && std::fs::metadata(root.join(relative))
                        .ok()
                        .is_some_and(|m| m.is_file() && Some(m.len()) == entry["size"].as_u64())
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledged_optional_failure_is_versioned_and_retained_data_is_checked() {
        let root = std::env::temp_dir().join(format!(
            "sk8-availability-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = root.join("private/customisation");
        std::fs::create_dir_all(&base).unwrap();
        let availability = base.join("customiser-availability.json");
        std::fs::write(
            &availability,
            br#"{"version":1,"fingerprint":"new","status":"unavailable"}"#,
        )
        .unwrap();
        assert!(customiser_current(&root, Some("new")));
        assert!(!customiser_current(&root, Some("future")));
        assert_eq!(
            crate::customiser_parts::asset_directory(&root),
            base.join("unavailable")
        );
        std::fs::write(
            &availability,
            br#"{"version":1,"fingerprint":"new","status":"retained"}"#,
        )
        .unwrap();
        assert!(!customiser_current(&root, Some("new")));
        std::fs::write(
            base.join("current.json"),
            br#"{"set":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .unwrap();
        let directory = base.join("sets/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("payload"), b"ok").unwrap();
        for stage in ["catalog", "library", "menu", "lighting", "roster"] {
            std::fs::write(
                directory.join(format!("{stage}-complete.json")),
                br#"{"files":{"payload":{"size":2}}}"#,
            )
            .unwrap();
        }
        assert!(customiser_current(&root, Some("new")));
        std::fs::remove_file(directory.join("payload")).unwrap();
        assert!(!customiser_current(&root, Some("new")));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn receipts_reject_missing_files_empty_lists_and_traversal() {
        let root = std::env::temp_dir();
        assert!(!receipt_present(&root, &serde_json::json!({})));
        assert!(!receipt_present(
            &root,
            &serde_json::json!({"../missing": {"size": 0}})
        ));
        assert!(!receipt_present(
            &root,
            &serde_json::json!({"nonexistent-skate-setup-test": {"size": 0}})
        ));
    }
}
