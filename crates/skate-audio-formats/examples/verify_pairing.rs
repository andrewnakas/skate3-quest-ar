//! Cross-check a `.snr` header against its paired `.sns` block chain.
//!
//! The header claims a total sample count; the block chain carries per-block counts.
//! If the parsers are right, the blocks must sum to the header. This is the strongest
//! available correctness check short of decoding audio.
//!
//! A looping streamed header also names the byte offset of the loop's block. That offset must be a
//! block boundary of the `.sns` chain, and the block starting there must hold the loop start sample.
//!
//!     cargo run --example verify_pairing -- <resident.big> <payload.big>

use skate_audio_formats::{eaac, eb};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let resident = std::fs::read(args.next().ok_or("need <resident.big>")?)?;
    let payload = std::fs::read(args.next().ok_or("need <payload.big>")?)?;

    // Report header parse failures rather than dropping them: a silently missing header
    // shows up later as "unpaired", which looks like a data problem and is not.
    let mut headers: HashMap<String, eaac::SnrRecord> = HashMap::new();
    for e in &eb::Archive::parse(&resident)?.entries {
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        let Some(stem) = eaac::stem(name) else {
            continue;
        };
        let Some(bytes) = resident.get(e.range()) else {
            println!("  {name:38.38} range outside archive");
            continue;
        };
        match eaac::SnrRecord::parse(bytes) {
            Ok(rec) => {
                headers.insert(stem.to_owned(), rec);
            }
            Err(err) => println!("  {name:38.38} header parse failed: {err}"),
        }
    }

    let mut ok = 0;
    let mut mismatched = 0;
    let mut unpaired = 0;
    let mut loops_ok = 0;
    let mut loops_bad = 0;
    for entry in &eb::Archive::parse(&payload)?.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".sns") {
            continue;
        }
        let Some(stem) = eaac::stem(name) else {
            continue;
        };
        let Some(rec) = headers.get(stem) else {
            println!("  {name:38.38} NO HEADER");
            unpaired += 1;
            continue;
        };
        let Some(bytes) = payload.get(entry.range()) else {
            continue;
        };
        let blocks = eaac::blocks(bytes)?;
        let summed: u64 = blocks.iter().map(|b| u64::from(b.num_samples)).sum();
        let claimed = u64::from(rec.header.num_samples);
        if let (Some(start), Some(offset)) = (rec.header.loop_start, rec.header.loop_offset) {
            // Samples before the block at `offset`, if some block starts there.
            let mut before = 0u64;
            let mut found = None;
            for b in &blocks {
                if b.offset == offset as usize {
                    found = Some((before, before + u64::from(b.num_samples)));
                    break;
                }
                before += u64::from(b.num_samples);
            }
            match found {
                Some((first, end)) if (first..end).contains(&u64::from(start)) => loops_ok += 1,
                Some((first, end)) => {
                    loops_bad += 1;
                    println!(
                        "  {name:38.38} loop start {start} outside its block's samples {first}..{end}"
                    );
                }
                None => {
                    loops_bad += 1;
                    println!("  {name:38.38} loop offset {offset:#x} is not a block boundary");
                }
            }
        }
        if summed == claimed {
            ok += 1;
        } else {
            mismatched += 1;
            let delta = summed as i64 - claimed as i64;
            println!(
                "  {name:38.38} blocks={:<5} summed={summed} claimed={claimed} delta={delta:+}",
                blocks.len()
            );
        }
    }
    println!("\n  exact match: {ok}   mismatched: {mismatched}   unpaired: {unpaired}");
    println!("  loop offsets on a block holding the loop start: {loops_ok}   wrong: {loops_bad}");
    Ok(())
}
