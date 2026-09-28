//! Dump the head and tail of one bank sample, raw, to see how its block chain is laid out.
//!
//!     cargo run --release --example sample_dump -- AUDIOFILES.BIG BANK.abk [SLOT]

use skate_audio_formats::{banks, eb};

fn hex(bytes: &[u8]) -> String {
    bytes
        .chunks(4)
        .map(|w| w.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: sample_dump ARCHIVE BANK [SLOT]")?;
    let bank = args
        .next()
        .ok_or("usage: sample_dump ARCHIVE BANK [SLOT]")?;
    let slot: usize = args.next().map_or(Ok(0), |s| s.parse())?;
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    let entry = archive.find(&bank).ok_or("no such member")?;
    let bytes = &data[entry.range()];
    let abk = banks::Abk::parse(bytes)?;
    let range = abk.sample_range(slot).ok_or("no such slot")?;
    let sample = &bytes[range.clone()];
    println!(
        "{bank} slot {slot}: bank bytes {range:?} ({} bytes); next slot starts {:?}",
        sample.len(),
        abk.sample_range(slot + 1).map(|r| r.start)
    );
    for row in 0..6 {
        let at = row * 24;
        if at >= sample.len() {
            break;
        }
        println!(
            "  +{at:04x}  {}",
            hex(&sample[at..(at + 24).min(sample.len())])
        );
    }
    let tail = sample.len().saturating_sub(32);
    println!("  tail +{tail:04x}  {}", hex(&sample[tail..]));
    // Walk what look like block headers after an 8-byte stream header: a u32 whose low 24 bits
    // are a size, printing each so the terminator is visible.
    let mut at = 8usize;
    for _ in 0..12 {
        if at + 8 > sample.len() {
            println!("  walk: ran off the end at +{at:#x}");
            break;
        }
        let word = u32::from_be_bytes(sample[at..at + 4].try_into()?);
        let next = u32::from_be_bytes(sample[at + 4..at + 8].try_into()?);
        println!(
            "  walk +{at:#06x}: word {word:#010x} (flag {:#04x}, size {}) then {next:#010x}",
            word >> 24,
            word & 0x00FF_FFFF
        );
        let size = (word & 0x00FF_FFFF) as usize;
        if size == 0 {
            println!("  walk: size 0 here");
            break;
        }
        at += size;
    }
    Ok(())
}
