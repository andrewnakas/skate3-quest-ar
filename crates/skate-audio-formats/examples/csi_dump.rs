//! Print every symbol of every `.csi` project in an archive, with the fields that are not yet named.
//!
//!     cargo run --example csi_dump -- <audiofiles.big> [project_id_hex]

use skate_audio_formats::{banks, eb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: csi_dump <archive> [project_id_hex]")?;
    let only = args
        .next()
        .map(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16))
        .transpose()?;
    let data = std::fs::read(&path)?;
    for e in &eb::Archive::parse(&data)?.entries {
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".csi") {
            continue;
        }
        let Some(bytes) = data.get(e.range()) else {
            continue;
        };
        let csi = banks::Csi::parse(bytes)?;
        if only.is_some_and(|p| p != csi.project_id) {
            continue;
        }
        println!(
            "{name}: project {:#06x}, groups {:?}",
            csi.project_id, csi.group_counts
        );
        for s in &csi.symbols {
            println!(
                "  g{} id {:#06x} a={:08x} b={:08x} ({:>6}) {}",
                s.group, s.id, s.unknown_a, s.unknown_b, s.unknown_b as i32, s.name
            );
        }
    }
    Ok(())
}
