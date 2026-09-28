//! Find which bank holds a sample the running game opened, from what the open probe logs:
//! the descriptor's slot index and the EAAC header's second word.
//!
//!     cargo run --release --example find_samples -- AUDIOFILES.BIG INDEX:WORD [INDEX:WORD ...]
//!
//! INDEX and WORD are hex, as the probe prints them (INDEX is the descriptor byte minus one).
//! An INDEX of `*` matches the word at any slot, for banks whose descriptor index is not the slot.

use skate_audio_formats::{banks, eb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: find_samples ARCHIVE INDEX:WORD...")?;
    // An index of None is `*`: any slot.
    let wanted: Vec<(Option<usize>, u32)> = args
        .map(|a| {
            let (i, w) = a.split_once(':').ok_or("INDEX:WORD")?;
            let index = if i == "*" {
                None
            } else {
                Some(usize::from_str_radix(i, 16)?)
            };
            Ok((index, u32::from_str_radix(w, 16)?))
        })
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    for entry in &archive.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".abk") || entry.is_compressed() {
            continue;
        }
        let bytes = &data[entry.range()];
        let Ok(abk) = banks::Abk::parse(bytes) else {
            continue;
        };
        let mut hits = Vec::new();
        for &(index, word) in &wanted {
            let slots: Vec<usize> = match index {
                Some(i) => vec![i],
                None => (0..abk.samples.len()).collect(),
            };
            for slot in slots {
                let Some(range) = abk.sample_range(slot) else {
                    continue;
                };
                if range.len() < 8 {
                    continue;
                }
                let second =
                    u32::from_be_bytes(bytes[range.start + 4..range.start + 8].try_into()?);
                if second == word {
                    match index {
                        Some(i) => hits.push(format!("{i:x}:{word:x}")),
                        None => hits.push(format!("*:{word:x}@{slot:x}")),
                    }
                }
            }
        }
        if !hits.is_empty() {
            let exports: Vec<&str> = abk.exports.iter().map(|e| e.name.as_str()).collect();
            println!(
                "{name}: {} of {} match [{}], exports {:?}",
                hits.len(),
                wanted.len(),
                hits.join(" "),
                exports
            );
        }
    }
    Ok(())
}
