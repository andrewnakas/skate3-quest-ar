//! Cross-check `.sth` sub-sound records against their `.dat` payloads.
//!
//! Speech packs several sub-sounds into one payload, with a `.sth` member giving each
//! one's offset and header. If the model is right, the block chain starting at every
//! sub-sound's offset must sum to that sub-sound's claimed sample count.
//!
//!     cargo run --example verify_speech -- <speech.big>

use skate_audio_formats::{eaac, eb};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: verify_speech <speech.big>")?;
    let data = std::fs::read(&path)?;
    let top = eb::Archive::parse(&data)?;

    // The `.sth` members live in a nested archive alongside the `.dat` payloads.
    let sth_entry = top
        .entries
        .iter()
        .find(|e| e.name.as_deref().is_some_and(|n| n.ends_with("sth.big")))
        .ok_or("no nested *sth.big member")?;
    let sth_base = sth_entry.offset as usize;
    let nested = eb::Archive::parse(&data[sth_base..])?;

    let mut subs: HashMap<String, Vec<eaac::SubSound>> = HashMap::new();
    for e in &nested.entries {
        let Some(stem) = e.name.as_deref().and_then(eaac::stem) else {
            continue;
        };
        let range = sth_base + e.range().start..sth_base + e.range().end;
        let Some(bytes) = data.get(range) else {
            continue;
        };
        match eaac::sub_sounds(bytes) {
            Ok(v) => {
                subs.insert(stem.to_owned(), v);
            }
            Err(err) => println!("  {stem:34.34} sth parse failed: {err}"),
        }
    }
    println!("  parsed {} .sth members", subs.len());

    let (mut ok, mut bad, mut missing, mut total_subs) = (0, 0, 0, 0);
    for entry in &top.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".dat") {
            continue;
        }
        let Some(stem) = eaac::stem(name) else {
            continue;
        };
        let Some(list) = subs.get(stem) else {
            missing += 1;
            continue;
        };
        let Some(payload) = data.get(entry.range()) else {
            continue;
        };
        for (i, sub) in list.iter().enumerate() {
            total_subs += 1;
            let Some(tail) = payload.get(sub.data_offset as usize..) else {
                bad += 1;
                continue;
            };
            // Walk only as far as this sub-sound's own sample budget. `tail` is the
            // whole remaining payload, so block sizes validate against real bounds.
            let mut summed = 0u64;
            let mut at = 0usize;
            let mut fault = false;
            while summed < u64::from(sub.header.num_samples) && at + 8 <= tail.len() {
                match eaac::Block::parse(tail, at) {
                    Ok(b) => {
                        summed += u64::from(b.num_samples);
                        at += b.size as usize;
                    }
                    Err(_) => {
                        fault = true;
                        break;
                    }
                }
            }
            if !fault && summed == u64::from(sub.header.num_samples) {
                ok += 1;
            } else {
                bad += 1;
                if bad <= 3 {
                    println!(
                        "  {stem:30.30}#{i} summed={summed} claimed={}",
                        sub.header.num_samples
                    );
                }
            }
        }
    }
    println!("\n  sub-sounds: {total_subs}   exact: {ok}   mismatched: {bad}   no .sth: {missing}");
    Ok(())
}
