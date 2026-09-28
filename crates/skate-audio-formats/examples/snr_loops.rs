//! Survey `.snr` records: how many are extended, and whether the extended ones are the looping ones.
//!
//! A looping header carries its loop start as a third word, so an extended record's first four
//! trailing bytes may be that word rather than unknown data. This checks the whole distribution.
//!
//!     cargo run --example snr_loops -- <resident.big>...

use skate_audio_formats::{eaac, eb};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut table: BTreeMap<(usize, bool, u8), usize> = BTreeMap::new();
    let mut shown = 0;
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path)?;
        for e in &eb::Archive::parse(&data)?.entries {
            let Some(name) = e.name.as_deref() else {
                continue;
            };
            if !name.ends_with(".snr") {
                continue;
            }
            let Some(bytes) = data.get(e.range()) else {
                continue;
            };
            let h = match eaac::Header::parse(bytes, 0) {
                Ok(h) => h,
                Err(err) => {
                    println!("  {name}: {err}");
                    continue;
                }
            };
            *table
                .entry((bytes.len(), h.looping, h.stream_type))
                .or_default() += 1;
            if bytes.len() > 8 && shown < 12 {
                shown += 1;
                let tail: Vec<String> = bytes[8..]
                    .chunks(4)
                    .map(|c| format!("{:02x?}", c))
                    .collect();
                println!(
                    "  {name:40.40} len={} loop={} type={} samples={} loop_start={:?} tail={}",
                    bytes.len(),
                    h.looping,
                    h.stream_type,
                    h.num_samples,
                    h.loop_start,
                    tail.join(" ")
                );
            }
        }
    }
    println!("(record bytes, looping, stream_type) -> count");
    for (k, v) in table {
        println!("  {k:?} -> {v}");
    }
    Ok(())
}
