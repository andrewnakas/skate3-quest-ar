//! An XMA2 decoder backed by the `ffmpeg` binary.
//!
//! This is the one part of the audio path that reaches outside the process, and it exists
//! because XMA2 is a hardware codec with no Rust decoder: the Xbox 360 decodes it in silicon,
//! and `libavcodec` is the only free implementation. The parent module stays IO-free; the
//! process lives here.
//!
//! ## Why one process per context, fed every chunk at once
//!
//! Skate 3 stores each decoder context as a chain of separately framed chunks, one per block.
//! Decoding a chunk on its own loses the MDCT overlap it needed from the chunk before, which
//! costs exactly 64 samples per chunk and audibly ticks at every block boundary. So the whole
//! chain has to reach one decoder instance, which is why `decode_chunk` buffers and
//! `finish_context` is where the audio appears.
//!
//! The chain is handed over as one XMA2 RIFF. Each EA chunk is padded independently to its next
//! 2048-byte packet boundary. Empty packets cannot be inserted merely to make unequal chunks a
//! common size: short resident-loop intros and 22.05 kHz samples prove that changes codec state.
//!
//! Three properties of this were measured on real ambience data (5 channels, 8 blocks, three
//! contexts of 2 + 2 + 1) rather than assumed:
//!
//! * zero-fill is required for the partial final XMA packet; `0xff` corrupts valid 22.05 kHz
//!   samples even though it happens to decode many 48 kHz members;
//! * the sample count equals the sum of the block headers' declared counts **exactly**, on
//!   every context, with no decoder errors -- the per-chunk deficit is gone;
//! * the first chunk's audio is byte-identical to decoding that chunk alone, which is the one
//!   chunk needing no prior state and therefore the one place the two paths must agree.
//!
//! An unpadded concatenation, or one declaring a flat 2048 when chunks are larger, desyncs
//! after about 23 KB. The padding to a 2048 multiple is the whole trick.
//!
//! `SamplesEncoded` in the container header was measured not to affect the output at all (0,
//! the true count, four times it, and 0xFFFFFF all decode identically), so this builds the
//! header without needing the block headers' counts plumbed through.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::{DecodeError, Decoder};

/// Resident EA-XMA loops with an intro contain two separately encoded segments. The loop
/// boundary is an EA block boundary, and its own decoder priming must not borrow the intro's
/// MDCT history. Keep streaming blocks together inside each segment; trim only to the authored
/// frame counts. This follows EAAC's segmented-loop layout, not whole-file repetition.
pub fn decode_resident(data: &[u8]) -> Result<Vec<i16>, super::Error> {
    use skate_audio_formats::eaac;
    let header = eaac::Header::parse(data, 0)?;
    let blocks = eaac::blocks(&data[header.size()..])?;
    let widths = super::context_widths(header.channels());
    let mut chains = vec![Vec::new(); widths.len()];
    let mut result = Vec::new();
    let mut total = 0u32;
    let mut segment_frames = 0usize;
    let split = header.loop_start.filter(|s| *s > 0);
    for (index, block) in blocks.iter().enumerate() {
        let range = block.data_range();
        let payload = &data[header.size() + range.start..header.size() + range.end];
        for (context, chunk) in eaac::split_block(payload, widths.len())?.iter().enumerate() {
            chains[context].push(chunk.data.to_vec());
        }
        total += block.num_samples;
        segment_frames += block.num_samples as usize;
        if split == Some(total) || index + 1 == blocks.len() {
            let mut decoded = Vec::new();
            for (chain, &width) in chains.iter_mut().zip(&widths) {
                let mut pcm =
                    decode_chain(chain, width, header.sample_rate).map_err(super::Error::Decode)?;
                let expected = segment_frames * usize::from(width);
                if pcm.len() < expected {
                    return Err(super::Error::Format(format!(
                        "EA-XMA segment declares {segment_frames} frames but decoded {}",
                        pcm.len() / usize::from(width)
                    )));
                }
                pcm.truncate(expected);
                decoded.push(pcm);
                chain.clear();
            }
            result.extend(super::interleave_contexts(&decoded, &widths));
            segment_frames = 0;
        }
    }
    Ok(result)
}

/// XMA2 packets are 2048 bytes, and a chunk must be padded to a whole number of them.
const PACKET: usize = 2048;

/// The decoder program. A source checkout can keep a private Windows build in `.local/ffmpeg`;
/// installed builds use an explicit `SKATE_FFMPEG` path or the system `PATH` instead.
fn program() -> PathBuf {
    if let Some(program) = std::env::var_os("SKATE_FFMPEG") {
        return program.into();
    }
    let private = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/ffmpeg");
    if let Ok(entries) = std::fs::read_dir(private) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("bin").join("ffmpeg.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

/// Build an XMA2 RIFF container around one context's padded chunk chain.
///
/// `channels` is the context's own channel count (one or two), not the stream's.
pub fn riff_xma2(payload: &[u8], channels: u8, rate: u32, block_align: u32) -> Vec<u8> {
    let mask: u32 = if channels == 1 { 0x4 } else { 0x3 };
    let blocks = ((payload.len() as u32).div_ceil(block_align.max(1))).max(1) as u16;
    // XMA2WAVEFORMATEX, 34 bytes. Only NumStreams, ChannelMask, BytesPerBlock and BlockCount
    // matter to the decoder; the sample and loop fields were measured to be inert.
    let mut ext = Vec::with_capacity(34);
    ext.extend_from_slice(&1u16.to_le_bytes()); // NumStreams
    ext.extend_from_slice(&mask.to_le_bytes()); // ChannelMask
    ext.extend_from_slice(&0u32.to_le_bytes()); // SamplesEncoded (inert, measured)
    ext.extend_from_slice(&block_align.to_le_bytes()); // BytesPerBlock
    ext.extend_from_slice(&0u32.to_le_bytes()); // PlayBegin
    ext.extend_from_slice(&0u32.to_le_bytes()); // PlayLength
    ext.extend_from_slice(&0u32.to_le_bytes()); // LoopBegin
    ext.extend_from_slice(&0u32.to_le_bytes()); // LoopLength
    ext.push(0); // LoopCount
    ext.push(4); // EncoderVersion
    ext.extend_from_slice(&blocks.to_le_bytes()); // BlockCount

    let mut fmt = Vec::with_capacity(18 + ext.len());
    fmt.extend_from_slice(&0x0166u16.to_le_bytes()); // WAVE_FORMAT_XMA2
    fmt.extend_from_slice(&u16::from(channels).to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    fmt.extend_from_slice(&(block_align as u16).to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    fmt.extend_from_slice(&(ext.len() as u16).to_le_bytes());
    fmt.extend_from_slice(&ext);

    let pad = payload.len() % 2;
    let mut body = Vec::with_capacity(12 + 8 + fmt.len() + 8 + payload.len() + pad);
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    body.extend_from_slice(payload);
    body.extend(std::iter::repeat_n(0u8, pad));

    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Pad each chunk independently to a whole number of XMA2 packets.
///
/// Returns the payload and the alignment used. An empty chain yields an empty payload rather
/// than a one-packet block of silence.
pub fn pad_chain(chunks: &[Vec<u8>]) -> (Vec<u8>, u32) {
    let longest = chunks.iter().map(Vec::len).max().unwrap_or(0);
    if longest == 0 {
        return (Vec::new(), PACKET as u32);
    }
    // EA removes padding only from each block's final XMA packet. Adding full empty packets
    // between unequal chunks changes decoder state (resident loops can have 13-byte intros).
    // See vgmstream src/meta/ea_eaac_streamfile.h, EA-XMA logical packet reconstruction. The
    // missing bytes are zero in the retail packet buffer; 0xff breaks sense_of_speed sample 10.
    let align = PACKET;
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(chunk);
        out.extend(std::iter::repeat_n(
            0u8,
            (align - chunk.len() % align) % align,
        ));
    }
    (out, align as u32)
}

/// Decode one padded chain through the external decoder, returning interleaved 16-bit PCM.
pub fn decode_chain(chunks: &[Vec<u8>], channels: u8, rate: u32) -> Result<Vec<i16>, DecodeError> {
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let (payload, align) = pad_chain(chunks);
    let container = riff_xma2(&payload, channels, rate, align);
    let mut child = Command::new(program())
        .args([
            "-v",
            "error",
            "-i",
            "pipe:0",
            "-af",
            "asetpts=N/SR/TB",
            "-f",
            "s16le",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DecodeError {
            message: format!("cannot run {}: {e}", program().display()),
        })?;
    // The container can exceed a pipe buffer, so the write has to happen while the child is
    // draining stdout. Handing stdin to a thread is the simple way to avoid the deadlock.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let writer = std::thread::spawn(move || stdin.write_all(&container));
    let out = child.wait_with_output().map_err(|e| DecodeError {
        message: format!("{}: {e}", program().display()),
    })?;
    let write_result = writer.join();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(DecodeError {
            message: format!("{} failed: {}", program().display(), err.trim()),
        });
    }
    if let Ok(Err(e)) = write_result {
        // A broken pipe with a successful exit means the decoder stopped early on its own.
        return Err(DecodeError {
            message: format!("feeding {}: {e}", program().display()),
        });
    }
    if out.stdout.len() % 2 != 0 {
        return Err(DecodeError {
            message: "decoder returned a half sample".into(),
        });
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        // A chunk split that balances but is wrong still decodes, with complaints. Treat any
        // decoder diagnostic as a failure: silently keeping damaged audio is what this whole
        // path exists to avoid.
        return Err(DecodeError {
            message: format!("decoder reported: {}", stderr.trim()),
        });
    }
    Ok(out
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect())
}

/// A [`Decoder`] that buffers each context's chunks and decodes the whole chain at once.
pub struct FfmpegDecoder {
    rate: u32,
    widths: Vec<u8>,
    chains: Vec<Vec<Vec<u8>>>,
}

impl FfmpegDecoder {
    /// `widths` is the per-context channel count, from [`super::context_widths`].
    pub fn new(rate: u32, widths: Vec<u8>) -> Self {
        let chains = vec![Vec::new(); widths.len()];
        Self {
            rate,
            widths,
            chains,
        }
    }

    /// Build one for a described stream.
    pub fn for_stream(info: &super::StreamInfo) -> Self {
        Self::new(info.sample_rate, super::context_widths(info.channels))
    }

    /// Is the decoder program runnable? Callers that can fall back should ask first.
    pub fn available() -> bool {
        Command::new(program())
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

impl Decoder for FfmpegDecoder {
    fn decode_chunk(&mut self, context: usize, chunk: &[u8]) -> Result<Vec<i16>, DecodeError> {
        let contexts = self.chains.len();
        let chain = self.chains.get_mut(context).ok_or_else(|| DecodeError {
            message: format!("context {context} is outside the {contexts} this stream has"),
        })?;
        chain.push(chunk.to_vec());
        // Nothing yet: the chain is only decodable once complete, which is the point.
        Ok(Vec::new())
    }

    fn finish_context(&mut self, context: usize) -> Result<Vec<i16>, DecodeError> {
        let width = *self.widths.get(context).ok_or_else(|| DecodeError {
            message: format!("context {context} has no channel width"),
        })?;
        let chain = std::mem::take(&mut self.chains[context]);
        decode_chain(&chain, width, self.rate)
    }
}
