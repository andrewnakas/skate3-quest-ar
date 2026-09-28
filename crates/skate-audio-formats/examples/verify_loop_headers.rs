//! Test one reading against every bank sample and every grain: a stream whose header sets the loop
//! bit carries a third word, the loop start, so its block chain begins at +12 rather than +8.
//!
//! Each stream is walked from the offset the reading predicts. The walk has to land exactly on the
//! stream's end, every block's chunk has to account for its payload, and the loop start has to fall
//! inside the stream -- all things a wrong offset fails.
//!
//!     cargo run --release --example verify_loop_headers -- [audio dir]

use skate_audio_formats::{banks, eaac, eb};

const DEFAULT_DIR: &str = "/home/nakas/Documents/skate3/freeskate/runtime/game/data/audio";

#[derive(Default)]
struct Tally {
    looping: usize,
    plain: usize,
    ok: usize,
    padded: usize,
    failures: Vec<String>,
}

fn check(tally: &mut Tally, label: &str, stream: &[u8], last: bool) {
    let header = match eaac::Header::parse(stream, 0) {
        Ok(h) => h,
        Err(e) => return tally.failures.push(format!("{label}: header {e}")),
    };
    // The parser now reads the loop start itself and rejects one outside the stream.
    if header.looping {
        tally.looping += 1;
    } else {
        tally.plain += 1;
    }
    let chain = header.size();
    let payload = &stream[chain..];
    let blocks = match eaac::blocks(payload) {
        Ok(b) => b,
        Err(e) => {
            return tally
                .failures
                .push(format!("{label}: chain at +{chain}: {e}"));
        }
    };
    let covered: usize = blocks.iter().map(|b| b.size as usize).sum();
    let samples: u64 = blocks.iter().map(|b| u64::from(b.num_samples)).sum();
    // The last sample in a bank runs to the end of its section, which is padded to four bytes, so
    // up to three trailing bytes are allowed there -- and only there.
    let pad = payload.len() - covered;
    if pad > 0 && !(last && pad < 4) {
        return tally.failures.push(format!(
            "{label}: blocks cover {covered} of {} bytes",
            payload.len()
        ));
    }
    tally.padded += usize::from(pad > 0);
    if samples != u64::from(header.num_samples) {
        return tally.failures.push(format!(
            "{label}: blocks carry {samples} samples, header says {}",
            header.num_samples
        ));
    }
    for block in &blocks {
        if let Err(e) = eaac::split_block(
            &payload[block.data_range()],
            eaac::context_count(header.channels()),
        ) {
            return tally
                .failures
                .push(format!("{label}: block at +{}: {e}", block.offset));
        }
    }
    tally.ok += 1;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_DIR.to_string());
    let mut tally = Tally::default();
    let data = std::fs::read(format!("{dir}/audiofiles.big"))?;
    let archive = eb::Archive::parse(&data)?;
    for entry in &archive.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".abk") {
            continue;
        }
        let bytes = &data[entry.range()];
        let Ok(abk) = banks::Abk::parse(bytes) else {
            continue;
        };
        let present = abk.present();
        for i in 0..present {
            if let Some(range) = abk.sample_range(i) {
                check(
                    &mut tally,
                    &format!("{name}#{i}"),
                    &bytes[range],
                    i + 1 == present,
                );
            }
        }
    }
    let bank_total = tally.ok + tally.failures.len();
    println!(
        "bank samples: {bank_total} checked, {} looping, {} plain, {} consistent ({} with a section pad)",
        tally.looping, tally.plain, tally.ok, tally.padded
    );
    let grains = std::fs::read(format!("{dir}/grains.big"))?;
    let grain_archive = eb::Archive::parse(&grains)?;
    let before = (tally.ok, tally.looping);
    for entry in &grain_archive.entries {
        let bytes = &grains[entry.range()];
        let head = u32::from_be_bytes(bytes[0..4].try_into()?) as usize;
        check(
            &mut tally,
            entry.name.as_deref().unwrap_or("?"),
            &bytes[head..],
            true,
        );
    }
    println!(
        "grains: {} consistent, {} of them looping",
        tally.ok - before.0,
        tally.looping - before.1
    );
    println!("failures: {}", tally.failures.len());
    for f in tally.failures.iter().take(12) {
        println!("  {f}");
    }
    Ok(())
}
