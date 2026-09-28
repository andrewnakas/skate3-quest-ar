//! Resolve a named sound through the audio banks and see whether its samples are playable.
//!
//!   cargo run -p skate-data --example bank_sound -- audiofiles.big [NAME] [OUTDIR]
//!
//! Skate 3 names every audio object with a string and hashes it; the hash is the join key
//! between code, world data and the banks (`docs/audio-banks.md` in the research repo). A bank
//! carries a sample bank of its own, so if those samples are ordinary EA Audio Core streams then
//! naming a sound is enough to play it, which is what the engine needs before it can make a sound
//! when the board lands.
//!
//! This program answers that question against real data rather than assuming it.

use skate_audio_formats::{banks, eb};
use skate_data::audio;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: bank_sound ARCHIVE [NAME] [OUTDIR]")?;
    let wanted = args.next();
    let outdir = args.next();
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;

    let mut banks_seen = 0usize;
    let mut samples_total = 0usize;
    let mut parsed_as_stream = 0usize;
    let mut first_failure: Option<String> = None;
    let mut chosen: Option<(String, banks::Abk, usize)> = None;

    for entry in &archive.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".abk") {
            continue;
        }
        let Some(bytes) = data.get(entry.range()) else {
            continue;
        };
        let Ok(abk) = banks::Abk::parse(bytes) else {
            continue;
        };
        banks_seen += 1;
        // Only the used prefix of the slot table. The rest hold 0xFFFFFFFF, and counting those
        // as samples is what made 89,431 slots look like 89,431 sounds of which 94% failed.
        for i in 0..abk.present() {
            let Some(range) = abk.sample_range(i) else {
                continue;
            };
            samples_total += 1;
            // A sample is a stream only if its header parses. Anything else is reported, not
            // assumed to be audio.
            match audio::describe(bytes, range.start) {
                Ok(_) => {
                    parsed_as_stream += 1;
                    if chosen.is_none()
                        && wanted.as_deref().is_none_or(|w| {
                            name.to_ascii_lowercase().contains(&w.to_ascii_lowercase())
                        })
                    {
                        chosen = Some((name.to_string(), abk.clone(), i));
                    }
                }
                Err(e) => {
                    if first_failure.is_none() {
                        first_failure = Some(format!("{name} sample {i}: {e}"));
                    }
                }
            }
        }
    }

    println!("{banks_seen} banks, {samples_total} samples in used slots");
    println!("{parsed_as_stream} parsed as EA Audio Core streams");
    if let Some(f) = &first_failure {
        println!("first sample that did not: {f}");
    }

    let Some((name, abk, index)) = chosen else {
        println!("nothing to decode");
        return Ok(());
    };
    let entry = archive
        .entries
        .iter()
        .find(|e| e.name.as_deref() == Some(name.as_str()))
        .ok_or("member vanished")?;
    let bytes = &data[entry.range()];
    let range = abk.sample_range(index).ok_or("sample vanished")?;
    let info = audio::describe(bytes, range.start)?;
    println!(
        "\n{name} sample {index}: {:?} {} Hz, {} channels, {} samples ({:.2} s)",
        info.codec,
        info.sample_rate,
        info.channels,
        info.num_samples,
        info.duration_secs()
    );

    let Some(outdir) = outdir else { return Ok(()) };
    std::fs::create_dir_all(&outdir)?;
    let widths = audio::context_widths(info.channels);
    let chain_start = range.start + info.header_bytes;
    let mut chains: Vec<Vec<Vec<u8>>> = vec![Vec::new(); info.contexts];
    for block in skate_audio_formats::eaac::blocks(&bytes[chain_start..range.end])? {
        let r = block.data_range();
        let payload = &bytes[chain_start + r.start..chain_start + r.end];
        for (c, chunk) in skate_audio_formats::eaac::split_block(payload, info.contexts)?
            .into_iter()
            .enumerate()
        {
            chains[c].push(chunk.data.to_vec());
        }
    }
    let mut per_context = Vec::new();
    for (c, chain) in chains.iter().enumerate() {
        per_context.push(audio::ffmpeg::decode_chain(
            chain,
            widths[c],
            info.sample_rate,
        )?);
    }
    let pcm = audio::interleave_contexts(&per_context, &widths);
    let frames = pcm.len() / usize::from(info.channels).max(1);
    println!(
        "decoded {frames} frames ({:.2} s)",
        frames as f64 / f64::from(info.sample_rate)
    );
    Ok(())
}
