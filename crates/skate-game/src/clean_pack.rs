//! The skate3-cleanroom asset pack (`core.pack`, published by
//! <https://github.com/andrewnakas/skate3-cleanroom>): skater model, animations,
//! state graphs, tuning, HUD and the clean sounds, laid out like a converted
//! installation (`assets/private/...`). Every texture, mesh and sound in it is
//! regenerated; animations, graphs and tuning are kept as facts (see that
//! repo's README). The Quest APK carries it so players don't import game files.
//!
//! Format SKPK1: the magic `SKPK1` padded to 8 bytes, a u32 file count, then
//! per file a u32 path length, the UTF-8 path relative to the pack root, a u64
//! size and the bytes. All integers little-endian.
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const MAGIC: &[u8; 5] = b"SKPK1";

/// Unpack every file of an SKPK1 stream under `dest`. Returns the file count.
pub(crate) fn unpack(mut pack: impl Read, dest: &Path) -> Result<usize, String> {
    let mut header = [0u8; 12];
    pack.read_exact(&mut header).map_err(|e| format!("pack header: {e}"))?;
    if &header[..5] != MAGIC {
        return Err("not an SKPK1 pack".into());
    }
    let count = u32::from_le_bytes(header[8..12].try_into().expect("4 bytes"));
    for index in 0..count {
        let len = read_u32(&mut pack)? as usize;
        if len == 0 || len > 4096 {
            return Err(format!("pack entry {index}: bad path length {len}"));
        }
        let mut name = vec![0u8; len];
        pack.read_exact(&mut name).map_err(|e| format!("pack entry {index}: {e}"))?;
        let name = String::from_utf8(name).map_err(|_| format!("pack entry {index}: path is not UTF-8"))?;
        let size = read_u64(&mut pack)?;
        let path = dest.join(safe_relative(&name)?);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut file = io::BufWriter::new(
            std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        );
        let copied = io::copy(&mut (&mut pack).take(size), &mut file)
            .map_err(|e| format!("{name}: {e}"))?;
        if copied != size {
            return Err(format!("{name}: pack ends {} bytes early", size - copied));
        }
        file.flush().map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(count as usize)
}

fn read_u32(r: &mut impl Read) -> Result<u32, String> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|e| format!("pack: {e}"))?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> Result<u64, String> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).map_err(|e| format!("pack: {e}"))?;
    Ok(u64::from_le_bytes(b))
}

/// A pack path as a relative path that cannot leave the destination.
fn safe_relative(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if path.components().all(|c| matches!(c, Component::Normal(_))) {
        Ok(path.to_path_buf())
    } else {
        Err(format!("pack path {name:?} is not a plain relative path"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = b"SKPK1\0\0\0".to_vec();
        out.extend((files.len() as u32).to_le_bytes());
        for (name, bytes) in files {
            out.extend((name.len() as u32).to_le_bytes());
            out.extend(name.as_bytes());
            out.extend((bytes.len() as u64).to_le_bytes());
            out.extend(*bytes);
        }
        out
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clean-pack-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn unpacks_nested_files() {
        let dest = scratch("ok");
        let data = pack(&[("assets/private/game.json", b"{}"), ("maps/a.skate", b"xyz")]);
        assert_eq!(unpack(data.as_slice(), &dest).unwrap(), 2);
        assert_eq!(std::fs::read(dest.join("assets/private/game.json")).unwrap(), b"{}");
        assert_eq!(std::fs::read(dest.join("maps/a.skate")).unwrap(), b"xyz");
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn rejects_escaping_paths_and_truncation() {
        let dest = scratch("bad");
        assert!(unpack(pack(&[("../evil", b"x")]).as_slice(), &dest).is_err());
        assert!(unpack(pack(&[("/abs", b"x")]).as_slice(), &dest).is_err());
        let mut short = pack(&[("a.bin", b"12345")]);
        short.truncate(short.len() - 2);
        assert!(unpack(short.as_slice(), &dest).is_err());
        assert!(unpack(&b"NOPE0000\0\0\0\0"[..], &dest).is_err());
        let _ = std::fs::remove_dir_all(&dest);
    }
}
