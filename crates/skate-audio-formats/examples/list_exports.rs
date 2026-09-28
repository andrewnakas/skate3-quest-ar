//! Print every `.abk` member of an archive with the objects and variables it exports, one line per
//! bank, so a sound object can be traced to the bank that plays it.
//!
//!     cargo run --release --example list_exports -- AUDIOFILES.BIG

use skate_audio_formats::{banks, eb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: list_exports ARCHIVE")?;
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    for entry in &archive.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".abk") || entry.is_compressed() {
            continue;
        }
        let Ok(abk) = banks::Abk::parse(&data[entry.range()]) else {
            println!("{name}: does not parse");
            continue;
        };
        let exports: Vec<&str> = abk.exports.iter().map(|e| e.name.as_str()).collect();
        println!("{name} ({} samples): {}", abk.present(), exports.join(" "));
    }
    Ok(())
}
