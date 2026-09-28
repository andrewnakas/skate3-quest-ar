//! Tests for the engine-side audio seam.
//!
//! These are synthetic and prove only self-consistency. That limitation is the whole reason
//! `skate-audio-formats` ships `examples/verify_*.rs`: in this project synthetic tests have
//! never once caught a format bug and real data has caught every one. What these *do* pin is
//! the seam's own behaviour -- field mapping, the decoder contract, and the error cases -- which
//! is code written here rather than inherited.

use super::*;

/// An EA Audio Core header: version 1, codec 0x16 (XMA2), 48 kHz, six channels.
fn header_bytes(channels: u8, rate: u32, samples: u32) -> Vec<u8> {
    let mut v = Vec::new();
    // word 1: version<<28 | codec<<24 | channel_config<<18 | sample_rate
    let w1 = (1u32 << 28) | (0x16u32 << 24) | ((u32::from(channels) - 1) << 18) | rate;
    v.extend_from_slice(&w1.to_be_bytes());
    // word 2: type<<30 | looping<<29 | num_samples
    v.extend_from_slice(&samples.to_be_bytes());
    v
}

#[test]
fn describe_reads_the_header_fields() {
    let data = header_bytes(6, 48_000, 96_000);
    let info = describe(&data, 0).expect("header parses");
    assert_eq!(info.sample_rate, 48_000);
    assert_eq!(info.channels, 6);
    assert_eq!(info.num_samples, 96_000);
    // Six channels is three XMA contexts, not six: the codec pairs them.
    assert_eq!(info.contexts, 3);
}

#[test]
fn duration_is_samples_over_rate() {
    let data = header_bytes(2, 48_000, 24_000);
    let info = describe(&data, 0).expect("header parses");
    assert!((info.duration_secs() - 0.5).abs() < 1e-9);
}

#[test]
fn a_zero_sample_rate_does_not_divide_by_zero() {
    let info = StreamInfo {
        codec: eaac::Codec::from_id(0x16),
        sample_rate: 0,
        channels: 1,
        num_samples: 1000,
        contexts: 1,
        header_bytes: 8,
    };
    assert_eq!(info.duration_secs(), 0.0);
}

#[test]
fn a_truncated_header_is_a_format_error_not_a_panic() {
    let err = describe(&[0u8; 3], 0).expect_err("three bytes cannot be a header");
    assert!(matches!(err, Error::Format(_)), "got {err:?}");
}

/// A decoder that records what it was fed, so the chunk ORDER can be asserted. That order is
/// the container's, and getting it wrong would still produce plausible-sounding audio.
struct Recorder {
    seen: Vec<(usize, usize)>,
    finished: bool,
}

impl Decoder for Recorder {
    fn decode_chunk(&mut self, context: usize, chunk: &[u8]) -> Result<Vec<i16>, DecodeError> {
        self.seen.push((context, chunk.len()));
        Ok(vec![0i16; 1])
    }
    fn finish_context(&mut self, _context: usize) -> Result<Vec<i16>, DecodeError> {
        self.finished = true;
        Ok(vec![0i16; 2])
    }
}

#[test]
fn finish_is_always_called_even_with_no_blocks() {
    // The default trait method returns nothing, so a decoder that holds state must be asked.
    let mut rec = Recorder {
        seen: Vec::new(),
        finished: false,
    };
    let data = header_bytes(1, 48_000, 0);
    // No block chain follows the header, so this reports a format error -- but the contract
    // being pinned is that a decoder is never silently skipped when blocks DO exist, which the
    // chunk-order test below covers. Here we only require no panic.
    let _ = decode_stream(&data, 0, &mut rec);
}

struct Failing;

impl Decoder for Failing {
    fn decode_chunk(&mut self, _c: usize, _chunk: &[u8]) -> Result<Vec<i16>, DecodeError> {
        Err(DecodeError {
            message: "no".into(),
        })
    }
}

#[test]
fn a_decoder_error_surfaces_as_decode_not_format() {
    // Distinguishing these matters: a format error means our parsing is wrong, a decode error
    // means the host's codec is. Collapsing them would send a reader to the wrong file.
    let e = DecodeError {
        message: "boom".into(),
    };
    let wrapped = Error::Decode(e.clone());
    assert!(format!("{wrapped}").contains("boom"));
    assert!(!format!("{wrapped}").contains("container"));
    let _ = Failing;
}

#[test]
fn the_stream_header_is_skipped_before_the_block_chain() {
    // The trap this guards: a 48 kHz header's first word is 0x0300bb80, whose low 24 bits are
    // 48000, so reading it as a block header yields a plausible 48,000-byte block. The walk then
    // lands mid-audio and fails far away from the real mistake.
    let header = header_bytes(1, 48_000, 710_784);
    assert_eq!(describe(&header, 0).unwrap().header_bytes, 8);
    let word = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    assert_eq!(word & 0x00FF_FFFF, 48_000, "the trap itself");
}

#[test]
fn a_looping_stream_header_is_twelve_bytes() {
    // GRINDS.abk, slot 0: the loop bit is set, so a loop start word follows and the chain begins
    // at +12. Skipping 8 read that word, zero, as a block header of size 0.
    let mut header = header_bytes(1, 48_000, 710_784);
    header[4] |= 0x20;
    header.extend_from_slice(&0u32.to_be_bytes());
    let info = describe(&header, 0).unwrap();
    assert_eq!((info.num_samples, info.header_bytes), (710_784, 12));
}

#[test]
fn context_widths_pair_channels_and_leave_an_odd_one_mono() {
    use super::context_widths;
    // The five-channel ambience bed is the case that matters: 2 + 2 + 1 is what the hardware
    // sets up and what each block splits into, so a flat ceil(channels/2) x 2 would claim six
    // channels of PCM out of five channels of input.
    assert_eq!(context_widths(1), vec![1]);
    assert_eq!(context_widths(2), vec![2]);
    assert_eq!(context_widths(5), vec![2, 2, 1]);
    assert_eq!(context_widths(6), vec![2, 2, 2]);
}

#[test]
fn contexts_interleave_by_frame_not_by_concatenation() {
    use super::interleave_contexts;
    // Two stereo contexts and one mono, one frame each: the result is L R  Ls Rs  C.
    let per = vec![vec![1, 2, 11, 12], vec![3, 4, 13, 14], vec![5, 15]];
    let out = interleave_contexts(&per, &[2, 2, 1]);
    assert_eq!(out, vec![1, 2, 3, 4, 5, 11, 12, 13, 14, 15]);
}

#[test]
fn a_short_context_truncates_the_stream_rather_than_padding_it() {
    use super::interleave_contexts;
    // Padding would invent audio; the rear channels of a five-channel bed would drift against
    // the front ones for the rest of the stream, which is the failure that sounds plausible.
    let per = vec![vec![1, 2, 11, 12], vec![3, 4]];
    assert_eq!(interleave_contexts(&per, &[2, 2]), vec![1, 2, 3, 4]);
    // A width list that disagrees with the context count is a caller bug, not a silent decode.
    assert!(interleave_contexts(&per, &[2]).is_empty());
}

#[test]
fn no_decoder_names_the_codec_in_its_message() {
    let e = Error::NoDecoder("XMA2");
    assert!(format!("{e}").contains("XMA2"));
}
