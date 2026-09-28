//! Walk a real block chain and split every payload with the length field.
//!
//! The unit tests use synthetic payloads, which only prove the code is self-consistent.
//! This runs the same code over the user's own archives, where a wrong length encoding
//! shows up immediately as a payload that does not account exactly.
//!
//!     cargo run --example verify_blocks -- <archive.big> <entry> <channels>

use skate_audio_formats::{eaac, eb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: verify_blocks <archive> <entry> <channels>")?;
    let entry: usize = args.next().ok_or("need entry index")?.parse()?;
    let channels: u8 = args.next().ok_or("need channel count")?.parse()?;

    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    let member = archive.entries.get(entry).ok_or("entry out of range")?;
    let payload_all = data
        .get(member.range())
        .ok_or("entry range outside archive")?;

    let contexts = eaac::context_count(channels);
    println!(
        "{}  entry {entry} ({})",
        path,
        member.name.as_deref().unwrap_or("?")
    );
    println!("  {channels} channels -> {contexts} context(s)");

    let mut at = 0usize;
    let (mut blocks, mut samples, mut bad) = (0u32, 0u64, 0u32);
    let mut per_ctx = vec![0usize; contexts];
    while at + eaac::Block::HEADER_SIZE <= payload_all.len() {
        let block = match eaac::Block::parse(payload_all, at) {
            Ok(b) => b,
            Err(_) => break,
        };
        let payload = &payload_all[block.data_range()];
        match eaac::split_block(payload, contexts) {
            Ok(chunks) => {
                for (i, c) in chunks.iter().enumerate() {
                    per_ctx[i] += c.data.len();
                }
            }
            Err(e) => {
                if bad < 3 {
                    println!("  block {blocks}: {e}");
                }
                bad += 1;
            }
        }
        blocks += 1;
        samples += u64::from(block.num_samples);
        at += block.size as usize;
    }

    println!("  blocks: {blocks}   failed splits: {bad}");
    println!("  declared samples: {samples}");
    println!("  bytes per context: {per_ctx:?}");
    if bad == 0 {
        println!("  every payload accounted for exactly");
    }
    Ok(())
}
