//! Parse a real EB archive and report its contents.
//!
//! Not a unit test: it needs the user's own copy of the game, so it lives here rather
//! than in the committed test suite.
//!
//!     cargo run --example inspect_archive -- /path/to/ambience.big

use skate_audio_formats::{eaac, eb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect_archive <file.big>")?;
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;

    println!("{path}");
    println!("  entries      {}", archive.entries.len());
    println!("  offset shift {}", archive.offset_shift);
    println!(
        "  total size   {} (actual {})",
        archive.total_size,
        data.len()
    );
    match archive.validate() {
        Ok(()) => println!("  bounds       all members inside the archive"),
        Err(e) => println!("  bounds       VIOLATION: {e}"),
    }
    if archive.total_size as usize != data.len() {
        println!("  WARNING: header size disagrees with the file on disk");
    }

    let compressed = archive.entries.iter().filter(|e| e.is_compressed()).count();
    println!("  compressed   {compressed} of {}", archive.entries.len());

    let named = archive.entries.iter().filter(|e| e.name.is_some()).count();
    println!("  named        {named}");

    let show_all = std::env::args().nth(2).as_deref() == Some("--all");
    for entry in archive.entries.iter().take(if show_all { usize::MAX } else { 6 }) {
        let name = entry.name.as_deref().unwrap_or("<unnamed>");
        print!("    {name:44.44} {:>10} bytes", entry.uncompressed_size);
        // Anything with an EAAC header should decode as codec 3 at a sane rate.
        if let Some(bytes) = data.get(entry.range()) {
            match eaac::Header::parse(bytes, 0) {
                Ok(h) => print!(
                    "  {:?} {}ch {}Hz {:.2}s",
                    h.codec,
                    h.channels(),
                    h.sample_rate,
                    h.duration_secs()
                ),
                Err(_) => print!("  (no EAAC header)"),
            }
        }
        println!();
    }
    Ok(())
}
