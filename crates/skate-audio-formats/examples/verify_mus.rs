//! Walk every segment of a real `.mus` and check each against the file's own SNR table.
//!
//! The SNR table states each segment's sample count independently of the block headers,
//! so agreement between the two is a real check rather than a self-consistency one.
//!
//! The offsets and the segment count come out of the file header now; earlier they were
//! passed on the command line, and the value passed for "segments" was 8 -- which is the
//! constant at header `0x38`, not a count. The real totals are three to four orders of
//! magnitude larger, so almost the whole file went unchecked.
//!
//!     cargo run --release --example verify_mus -- <file.mus>

use skate_audio_formats::mus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: verify_mus <file.mus>")?;
    let data = std::fs::read(&path)?;
    let header = mus::Header::parse(&data)?;

    println!("{path}");
    println!("  segments           {}", header.segment_count);
    println!("  SNR table at       {:#x}", header.snr_table_offset);
    println!("  first block at     {:#x}", header.first_block_offset);
    println!(
        "  0x28 / 0x38        {:#010x} / {}",
        header.unknown_28, header.unknown_38
    );

    let segments = mus::segments(&data)?;
    let blocks: usize = segments.iter().map(|s| s.blocks.len()).sum();
    let samples: u64 = segments.iter().map(mus::Segment::num_samples).sum();
    let end = segments.last().map_or(0, mus::Segment::end);

    for (i, seg) in segments.iter().take(3).enumerate() {
        println!(
            "  segment {i}: {:3} blocks  {:7} samples",
            seg.blocks.len(),
            seg.num_samples()
        );
    }
    println!("  ---");
    println!(
        "  walked             {} segments, {blocks} blocks, {samples} samples",
        segments.len()
    );
    println!("  every segment agreed with the SNR table (mus::segments errors otherwise)");
    println!(
        "  last block ends at {end:#x}; file is {:#x} ({} bytes unused)",
        data.len(),
        data.len() - end
    );
    Ok(())
}
