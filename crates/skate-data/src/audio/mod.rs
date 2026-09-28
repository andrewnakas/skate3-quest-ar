//! Skate 3 audio: opening the retail containers and describing what is inside them.
//!
//! This is the engine-side half of the reverse-engineering in the sk8Audio repository. The
//! parsing lives in `skate-audio-formats` and the ported mixer graph in `skate-audio-core`;
//! this module is the seam between those and the game.
//!
//! **Where the boundary is, and why.** Containers parse here, in safe Rust, and a stream's
//! shape -- codec, rate, channels, block chain, chunk splits -- is fully described. What is
//! *not* here is XMA decoding: the retail audio is Xbox 360 XMA2, which the reference decoder
//! handles offline through FFmpeg (`tools/xma_decode.c` in sk8Audio). So this module hands out
//! descriptions and raw chunks, and a decoder is supplied by the caller through [`Decoder`].
//! That keeps the format work usable now and leaves the decoder swappable, which matters
//! because the one property being preserved across this whole effort is sample-exactness.
//!
//! Everything here operates on bytes the caller already owns. No file IO, so a mod or a test
//! can feed it a buffer from anywhere.

use skate_audio_formats::{eaac, eb, mus};

/// One stream's description, taken from an EA Audio Core header.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub codec: eaac::Codec,
    pub sample_rate: u32,
    pub channels: u8,
    pub num_samples: u32,
    /// Decoder contexts the stream needs: XMA pairs channels, so this is not the channel count.
    pub contexts: usize,
    /// Bytes of header before an inline block chain: 8, or 12 when the stream loops, because a
    /// looping header carries its loop start as a third word (16 for a looping streamed sound,
    /// which adds the loop's byte offset). Zero when the header does not sit in front of its chain:
    /// a `.mus` SNR table, a `.sth` sub-sound, or a bare chain described from metadata.
    pub header_bytes: usize,
}

impl StreamInfo {
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        f64::from(self.num_samples) / f64::from(self.sample_rate)
    }
}

/// A decoder for one codec, supplied by the host.
///
/// Deliberately a trait rather than an implementation. The offline reference decoder keeps a
/// *persistent* decoder instance across a stream's blocks; restarting it per block loses the
/// 64-sample MDCT overlap and the result stops being sample-exact. Any implementation of this
/// trait has to preserve that, which is why the contract says so here rather than in a comment
/// somewhere downstream.
pub trait Decoder {
    /// Feed one chunk of one context. Returns interleaved PCM for the samples it produced.
    fn decode_chunk(&mut self, context: usize, chunk: &[u8]) -> Result<Vec<i16>, DecodeError>;

    /// Flush whatever the decoder is still holding for one context at end of stream.
    ///
    /// Per context, not per stream: a multichannel stream has one decoder instance per context
    /// and each holds its own MDCT tail, so a single flush would drop every context but one.
    fn finish_context(&mut self, context: usize) -> Result<Vec<i16>, DecodeError> {
        let _ = context;
        Ok(Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub message: String,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DecodeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The container itself did not parse.
    Format(String),
    /// No decoder was supplied for a codec that needs one.
    NoDecoder(&'static str),
    Decode(DecodeError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(m) => write!(f, "container: {m}"),
            Self::NoDecoder(c) => write!(f, "no decoder supplied for {c}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<skate_audio_formats::Error> for Error {
    fn from(e: skate_audio_formats::Error) -> Self {
        Self::Format(format!("at {:#x}: {}", e.offset, e.message))
    }
}

/// Describe the stream beginning at `at` without decoding any of it.
///
/// Skip [`StreamInfo::header_bytes`] to reach the block chain, never a fixed 8. Getting it wrong is
/// quiet rather than loud. Read from +0, the header's first word carries the sample rate in its low
/// 24 bits, so a 48 kHz stream announces a block of exactly 48,000 bytes and the walk fails deep in
/// the audio. Read from +8 on a looping stream, the loop start is taken for a block header, usually
/// a block of size 0: 399 of the 5,168 bank samples in `audiofiles.big` loop, the grind sounds among
/// them.
pub fn describe(data: &[u8], at: usize) -> Result<StreamInfo, Error> {
    let header = eaac::Header::parse(data, at)?;
    let channels = header.channels();
    Ok(StreamInfo {
        codec: header.codec,
        sample_rate: header.sample_rate,
        channels,
        num_samples: header.num_samples,
        contexts: eaac::context_count(channels),
        header_bytes: header.size(),
    })
}

/// What kind of container a file turned out to be.
///
/// The three asset classes are packed differently and need different entry points. Treating an
/// archive of ambience streams as the general case gets speech and music wrong -- speech packs
/// several sub-sounds behind one `.sth` table, and music is a segment chain with its own header.
/// That mistake was made here first and caught by running all three classes, which is the rule
/// this project keeps relearning: do not generalise from the head of a distribution.
#[derive(Clone, Debug, PartialEq)]
pub enum Container {
    /// One stream per archive entry: ambience, wheels, grains, post.
    Streams(Vec<(String, StreamInfo)>),
    /// A `.sth` table naming sub-sounds inside a sibling `.dat` payload: speech.
    SubSounds(Vec<(String, Vec<StreamInfo>)>),
    /// An interactive-music segment chain.
    Music {
        segments: usize,
        num_samples: u64,
        streams: Vec<StreamInfo>,
    },
    /// `.sns` payloads whose headers live in a **separate** archive.
    ///
    /// Ambience is split across two files: `ambienceresident.big` carries the `.snr` headers and
    /// `ambience.big` the matching block chains. A payload archive alone cannot say what its
    /// streams are, and saying so is the useful answer -- reading it as a bare stream instead
    /// produces "implausible sample rate", which looks like a parser bug and is not.
    Payloads {
        names: Vec<String>,
        pair_with: Option<String>,
    },
    /// Sound banks and cue tables: `.abk`, `.bnk`, `.csi`, `.grain`, `.ems`.
    ///
    /// Deliberately undecoded. `docs/PLAN.md` lists these as non-goals: ambience, speech and
    /// music cover the overwhelming majority of in-game audio, and no missing sound has been
    /// traced to a bank. Reported by kind so that a future need is visible rather than silent.
    Banks { kinds: Vec<(String, usize)> },
}

/// Identify and describe any of the three audio containers.
pub fn describe_any(data: &[u8]) -> Result<Container, Error> {
    // Music announces itself with its own 0x40-byte header, so try it first: a `.mus` would
    // otherwise be read as a bare stream and produce a nonsense sample rate.
    if let Ok(header) = mus::Header::parse(data) {
        let segments = mus::segments(data)?;
        let mut streams = Vec::new();
        // One SNR record per segment, so sampling the first few is enough to report the
        // stream shape without walking thousands of identical records.
        for i in 0..(header.segment_count as usize).min(4) {
            if let Ok(h) = header.snr(data, i) {
                let channels = h.channels();
                streams.push(StreamInfo {
                    codec: h.codec,
                    sample_rate: h.sample_rate,
                    channels,
                    num_samples: h.num_samples,
                    contexts: eaac::context_count(channels),
                    header_bytes: 0,
                });
            }
        }
        return Ok(Container::Music {
            segments: segments.len(),
            num_samples: segments.iter().map(|s| s.num_samples()).sum(),
            streams,
        });
    }
    if let Ok(archive) = eb::Archive::parse(data) {
        let subs = sub_sound_entries(data, &archive);
        if !subs.is_empty() {
            return Ok(Container::SubSounds(subs));
        }
        let streams = describe_archive(data)?;
        if !streams.is_empty() {
            return Ok(Container::Streams(streams));
        }
        // No headers of its own. Either it is the payload half of a pair, or it holds banks.
        let mut payloads = Vec::new();
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for entry in &archive.entries {
            let Some(name) = entry.name.as_deref() else {
                continue;
            };
            match name.rsplit('.').next() {
                Some("sns") | Some("dat") => payloads.push(name.to_string()),
                Some(ext @ ("abk" | "bnk" | "csi" | "grain" | "ems")) => {
                    *kinds.entry(ext.to_string()).or_default() += 1;
                }
                _ => {}
            }
        }
        if !payloads.is_empty() {
            return Ok(Container::Payloads {
                names: payloads,
                pair_with: None,
            });
        }
        if !kinds.is_empty() {
            return Ok(Container::Banks {
                kinds: kinds.into_iter().collect(),
            });
        }
    }
    // A bare stream, last: it is the weakest test, since almost any bytes parse as a header.
    Ok(Container::Streams(vec![(
        String::new(),
        describe(data, 0)?,
    )]))
}

/// The sub-sound tables of a speech archive, each expanded into its sub-sounds.
///
/// Speech is packed two levels deep, which is why a flat scan finds nothing. The outer `.big`
/// holds one `.dat` payload per line of dialogue plus **two** nested archives, `*hdr.big` and
/// `*sth.big`. The sub-sound tables are in the `sth` one; matching on `hdr` instead finds a
/// nested archive that parses fine and yields zero usable tables, which reads exactly like
/// "speech has no audio". That cost a debugging pass here.
///
/// The addressing is the other trap. A nested member's range is relative to the nested
/// archive's own base **in the outer file**, so the base is added back and the bytes are taken
/// from `data`, not from a slice cut to the nested entry's length. This mirrors
/// `skate-audio-formats/examples/verify_speech.rs`, which is verified against retail archives:
/// 405 tables, 2,822 sub-sounds, every one exact.
fn sub_sound_entries(data: &[u8], archive: &eb::Archive) -> Vec<(String, Vec<StreamInfo>)> {
    let mut out = Vec::new();
    let Some(sth) = archive
        .entries
        .iter()
        .find(|e| e.name.as_deref().is_some_and(|n| n.ends_with("sth.big")))
    else {
        return out;
    };
    let base = sth.offset as usize;
    let Some(tail) = data.get(base..) else {
        return out;
    };
    let Ok(nested) = eb::Archive::parse(tail) else {
        return out;
    };
    for member in &nested.entries {
        let Some(stem) = member.name.as_deref().and_then(eaac::stem) else {
            continue;
        };
        let range = base + member.range().start..base + member.range().end;
        let Some(bytes) = data.get(range) else {
            continue;
        };
        let Ok(subs) = eaac::sub_sounds(bytes) else {
            continue;
        };
        if subs.is_empty() {
            continue;
        }
        let infos = subs
            .iter()
            .map(|sub| {
                let channels = sub.header.channels();
                StreamInfo {
                    codec: sub.header.codec,
                    sample_rate: sub.header.sample_rate,
                    channels,
                    num_samples: sub.header.num_samples,
                    contexts: eaac::context_count(channels),
                    header_bytes: 0,
                }
            })
            .collect();
        out.push((stem.to_owned(), infos));
    }
    out
}

/// Every stream an EB archive holds, by entry name.
///
/// An entry that does not parse as a stream is skipped rather than failing the archive: these
/// are retail files, and a `.big` mixes stream data with other resources.
pub fn describe_archive(data: &[u8]) -> Result<Vec<(String, StreamInfo)>, Error> {
    let archive = eb::Archive::parse(data)?;
    let mut found = Vec::new();
    for entry in &archive.entries {
        // A nameless entry is still a stream; the hash is what the game looks it up by, so it
        // is reported rather than dropped.
        let name = entry
            .name
            .clone()
            .unwrap_or_else(|| format!("{:08X}", entry.name_hash));
        if entry.is_compressed() {
            continue;
        }
        let range = entry.range();
        if range.end > data.len() {
            continue;
        }
        if let Ok(info) = describe(data, range.start) {
            found.push((name, info));
        }
    }
    Ok(found)
}

/// Decode one stream to interleaved PCM through a caller-supplied decoder.
///
/// The chunk order matters and is the container's, not ours: each block splits into one chunk
/// per decoder context, and the contexts are fed in step. Returns whatever the decoder
/// produced, concatenated in that order.
/// The per-context channel widths of a stream: XMA pairs channels, and an odd channel count
/// leaves the last context mono. Five channels are 2 + 2 + 1, matching the three hardware
/// contexts `sub_82B4FC00` sets up and the three chunks each block splits into.
pub fn context_widths(channels: u8) -> Vec<u8> {
    let mut w = vec![2u8; usize::from(channels / 2)];
    if channels % 2 == 1 {
        w.push(1);
    }
    w
}

/// Decode one stream to interleaved 16-bit PCM.
///
/// The assembly is the part worth reading. Each context is an **independent** XMA sub-stream
/// carrying one or two channels, and its chunks arrive one per block. So a context's chunks are
/// accumulated in order into that context's own PCM, and only at the end are the contexts
/// interleaved into the stream's channel order. Concatenating a block's contexts as they are
/// read would put the rear channels' first block where the front channels' second block belongs;
/// on a stereo stream, where there is one context, the two are indistinguishable, which is
/// exactly why this needs saying rather than testing on stereo alone.
///
/// Contexts are truncated to the shortest, because a stream whose contexts disagree on length
/// has no sample-aligned interpretation and padding one would invent audio.
pub fn decode_stream(data: &[u8], at: usize, decoder: &mut dyn Decoder) -> Result<Vec<i16>, Error> {
    let info = describe(data, at)?;
    let widths = context_widths(info.channels);
    let mut per_context: Vec<Vec<i16>> = vec![Vec::new(); info.contexts];
    // The chain follows the header this call just parsed.
    let chain = at + info.header_bytes;
    for block in eaac::blocks(&data[chain..])? {
        let range = block.data_range();
        let payload = &data[chain + range.start..chain + range.end];
        for (context, chunk) in eaac::split_block(payload, info.contexts)?
            .into_iter()
            .enumerate()
        {
            let out = decoder
                .decode_chunk(context, chunk.data)
                .map_err(Error::Decode)?;
            per_context[context].extend_from_slice(&out);
        }
    }
    for (context, pcm) in per_context.iter_mut().enumerate() {
        let tail = decoder.finish_context(context).map_err(Error::Decode)?;
        pcm.extend_from_slice(&tail);
    }
    Ok(interleave_contexts(&per_context, &widths))
}

/// Interleave per-context PCM into one stream. Each context's samples are already interleaved
/// across its own one or two channels, so a frame of the result is every context's frame in
/// context order.
pub fn interleave_contexts(per_context: &[Vec<i16>], widths: &[u8]) -> Vec<i16> {
    if per_context.is_empty() || widths.len() != per_context.len() {
        return Vec::new();
    }
    let frames = per_context
        .iter()
        .zip(widths)
        .map(|(pcm, &w)| {
            if w == 0 {
                0
            } else {
                pcm.len() / usize::from(w)
            }
        })
        .min()
        .unwrap_or(0);
    let total: usize = widths.iter().map(|&w| usize::from(w)).sum();
    let mut out = Vec::with_capacity(frames * total);
    for frame in 0..frames {
        for (pcm, &w) in per_context.iter().zip(widths) {
            let w = usize::from(w);
            out.extend_from_slice(&pcm[frame * w..frame * w + w]);
        }
    }
    out
}

pub mod ambience;
pub mod catalog;
pub mod ffmpeg;
pub mod grains;
pub mod splice;

#[cfg(test)]
mod tests;
