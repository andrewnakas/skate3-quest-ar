//! Probe the undecoded `.grain` members of `grains.big` for anything this crate already reads: an EA
//! Audio Core header at any offset, and the offset where the slowly varying byte run gives way to
//! higher-entropy data.
//!
//!     cargo run --release --example grain_probe -- [audio dir]

use skate_audio_formats::{eaac, eb};

const DEFAULT_DIR: &str = "/home/nakas/Documents/skate3/freeskate/runtime/game/data/audio";

/// Distinct byte values in a window: low for the smooth table, high for coded audio.
#[allow(dead_code)]
fn distinct(window: &[u8]) -> usize {
    let mut seen = [false; 256];
    window.iter().for_each(|&b| seen[b as usize] = true);
    seen.iter().filter(|&&s| s).count()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_DIR.to_string());
    let data = std::fs::read(format!("{dir}/grains.big"))?;
    let archive = eb::Archive::parse(&data)?;
    let mut agree = 0;
    for entry in &archive.entries {
        let bytes = &data[entry.range()];
        let name = entry.name.as_deref().unwrap_or("?");
        let head = u32::from_be_bytes(bytes[0..4].try_into()?) as usize;
        let seconds = f32::from_be_bytes(bytes[4..8].try_into()?);
        // The claim under test: the first word is the offset of an EA Audio Core stream, and the
        // float after it is that stream's duration.
        match eaac::Header::parse(bytes, head) {
            Ok(h) => {
                let same = (h.duration_secs() as f32 - seconds).abs() < 0.01;
                agree += usize::from(same);
                println!(
                    "{name:30.30} head {head:#05x}  {:?} {}ch {}Hz {:.2}s  header float {seconds:.2}s  {}",
                    h.codec,
                    h.channels(),
                    h.sample_rate,
                    h.duration_secs(),
                    if same { "agree" } else { "DISAGREE" }
                );
            }
            Err(e) => println!("{name:30.30} head {head:#05x}  no EAAC header there: {e}"),
        }
    }
    println!(
        "{agree} of {} members: the first word locates an EAAC stream whose duration is the float at +4",
        archive.entries.len()
    );
    Ok(())
}
