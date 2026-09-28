//! Decode one Skate 3 audio stream to PCM, against a real archive.
//!
//! Synthetic tests have never caught a container bug in this work; real data has caught every
//! one. This program is the real-data check for the decode path: it walks an archive entry's
//! block chain, decodes each context through the external decoder, and writes both the
//! per-context PCM (so it can be hashed against an independently produced reference) and the
//! interleaved stream as a WAV that can simply be listened to.
//!
//!   cargo run -p skate-data --example decode_audio -- ARCHIVE OUTDIR [--entry N]
//!                                                     [--blocks N] [--channels N] [--rate N]
//!
//! `--channels` and `--rate` override the header, for archives whose entry table is not walked
//! here. Without `--blocks` the whole stream is decoded.

use std::fs;
use std::path::PathBuf;

use skate_audio_formats::{eaac, eb};
use skate_data::audio::{self, ffmpeg::FfmpegDecoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let archive = PathBuf::from(
        args.next()
            .ok_or("usage: decode_audio ARCHIVE OUTDIR ...")?,
    );
    let outdir = PathBuf::from(
        args.next()
            .ok_or("usage: decode_audio ARCHIVE OUTDIR ...")?,
    );
    let (mut at, mut blocks_limit, mut channels, mut rate) = (usize::MAX, 0usize, 0u8, 0u32);
    let mut entry: Option<String> = None;
    let mut end = usize::MAX;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--at" => at = value.parse()?,
            "--entry" => entry = Some(value),
            "--blocks" => blocks_limit = value.parse()?,
            "--channels" => channels = value.parse()?,
            "--rate" => rate = value.parse()?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    if !FfmpegDecoder::available() {
        return Err("no decoder: set SKATE_FFMPEG or put ffmpeg on PATH".into());
    }
    fs::create_dir_all(&outdir)?;
    let data = fs::read(&archive)?;

    // An archive holds many streams, so the entry table is walked unless a raw offset was given.
    // A member is named by hash, so an index is as legitimate an address as a name here.
    if at == usize::MAX {
        let parsed = eb::Archive::parse(&data)
            .map_err(|e| format!("not an EB archive and no --at given: {}", e.message))?;
        let wanted = entry.as_deref().unwrap_or("0");
        let member = match wanted.parse::<usize>() {
            Ok(index) => parsed.entries.get(index).ok_or_else(|| {
                format!(
                    "entry {index} of {}: the archive has {}",
                    archive.display(),
                    parsed.entries.len()
                )
            })?,
            Err(_) => parsed
                .find(wanted)
                .ok_or_else(|| format!("no member named {wanted} in {}", archive.display()))?,
        };
        if member.is_compressed() {
            return Err(format!("member {wanted} is a chunkref block, not stored audio").into());
        }
        at = member.range().start;
        end = member.range().end.min(data.len());
        println!(
            "entry {wanted}: {} bytes at {at:#x}",
            member.uncompressed_size
        );
    }

    // Some members carry an EA Audio Core header; the ambience and grain archives are bare
    // block chains whose channel count and rate live in the metadata table instead, so those
    // have to be supplied. Guessing them from the first block is not possible: the block header
    // carries a size and a sample count and nothing about the format.
    let mut info = match audio::describe(&data, at) {
        Ok(info) => info,
        Err(e) if channels != 0 => {
            println!("no stream header here ({e}); using the channel count and rate given");
            audio::StreamInfo {
                codec: eaac::Codec::Xma,
                sample_rate: if rate != 0 { rate } else { 48_000 },
                channels,
                num_samples: 0,
                contexts: audio::context_widths(channels).len(),
                header_bytes: 0,
            }
        }
        Err(e) => {
            return Err(format!("{e} -- pass --channels to read it as a bare block chain").into());
        }
    };
    if channels != 0 {
        info.channels = channels;
        info.contexts = audio::context_widths(channels).len();
    }
    if rate != 0 {
        info.sample_rate = rate;
    }
    let widths = audio::context_widths(info.channels);
    println!(
        "{} at {at:#x}: {:?} {} Hz, {} channels in {} contexts {:?}, {} samples ({:.2} s)",
        archive.file_name().unwrap_or_default().to_string_lossy(),
        info.codec,
        info.sample_rate,
        info.channels,
        info.contexts,
        widths,
        info.num_samples,
        info.duration_secs()
    );

    // Gather each context's chain in block order. This is what the decoder needs whole: a chunk
    // decoded alone is short by its MDCT overlap, so nothing is decoded until the chain is done.
    let mut chains: Vec<Vec<Vec<u8>>> = vec![Vec::new(); info.contexts];
    let mut declared = 0u64;
    let mut blocks_seen = 0usize;
    // The chain is bounded by the MEMBER, not by the file: walking off its end reads the next
    // member's bytes as a block header and reports a nonsense size from deep in the archive.
    let end = end.min(data.len());
    // A headered stream's chain starts after the header, whose size depends on whether it loops
    // (zero for a bare chain). Reading the header as a block header is the quiet failure: its low
    // 24 bits are the sample rate, so 48 kHz looks like a 48,000-byte block and the walk dies much
    // later, somewhere that looks like a corrupt archive.
    let chain = at + info.header_bytes;
    for block in eaac::blocks(&data[chain..end])? {
        if blocks_limit != 0 && blocks_seen >= blocks_limit {
            break;
        }
        let range = block.data_range();
        let payload = &data[chain + range.start..chain + range.end];
        for (context, chunk) in eaac::split_block(payload, info.contexts)?
            .into_iter()
            .enumerate()
        {
            chains[context].push(chunk.data.to_vec());
        }
        declared += u64::from(block.num_samples);
        blocks_seen += 1;
    }
    println!("{blocks_seen} blocks, {declared} samples declared by their headers");

    let mut per_context = Vec::new();
    for (context, chain) in chains.iter().enumerate() {
        let pcm = audio::ffmpeg::decode_chain(chain, widths[context], info.sample_rate)?;
        let frames = pcm.len() / usize::from(widths[context]);
        let deficit = declared as i64 - frames as i64;
        println!(
            "  context {context}: {} chunks, {}ch, {frames} frames, deficit {deficit}{}",
            chain.len(),
            widths[context],
            if deficit == 0 {
                " (exact)"
            } else {
                "  <-- NOT EXACT"
            }
        );
        let raw: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        fs::write(outdir.join(format!("context{context}.pcm")), &raw)?;
        per_context.push(pcm);
    }

    let interleaved = audio::interleave_contexts(&per_context, &widths);
    let frames = interleaved.len() / usize::from(info.channels);
    let wav = outdir.join("stream.wav");
    fs::write(
        &wav,
        wav_pcm16(&interleaved, info.channels, info.sample_rate),
    )?;
    println!(
        "wrote {} ({frames} frames, {:.2} s) and {} per-context PCM files",
        wav.display(),
        frames as f64 / f64::from(info.sample_rate),
        per_context.len()
    );
    Ok(())
}

/// A plain 16-bit PCM WAV, so the result is playable by anything.
fn wav_pcm16(pcm: &[i16], channels: u8, rate: u32) -> Vec<u8> {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&u16::from(channels).to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    fmt.extend_from_slice(&(u16::from(channels) * 2).to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(&bytes);
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}
