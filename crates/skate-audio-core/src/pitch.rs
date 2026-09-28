//! `sub_82B2DAC8` — recompute a voice's pitch ratio and advance its fractional position.
//!
//! Ported from `recomp/src/audio_ports/sub_82B2DAC8.inc`, **STATUS: verified** — 306,361 calls in a
//! played session on `RwAudioCore Dac`.
//!
//! Each block, the voice's playback ratio is `requested / source_rate * scale`, two single-rounded
//! operations. When it moved since the last block it is turned into a 16.16 step — scaled, rounded
//! **half away from zero**, truncated — and clamped at 2^18, in which case the stored ratio becomes a
//! pool constant instead. Then, on every call, the step advances a 16.16 position by the block's frame
//! count, the stream's gain is scaled by the ratio, and the function returns how many whole source
//! frames the block consumes, adjusted by two bytes on the voice and clamped at zero.
//!
//! Three details the tests pin, because each is where a plausible rewrite drifts:
//!
//! - **An unchanged ratio skips the whole recompute, stores included.** The comparison is against the
//!   value stored last time, so the +56/+60/+68 words are not rewritten.
//! - **The tail re-reads the step and the ratio from memory**, not from registers.
//! - **The result is 64-bit and the clamp looks only at the low word.** `whole - bias + credit` can
//!   borrow into the upper half; a set sign bit in the low word clamps the whole result to zero, and
//!   a clear one returns all 64 bits.
//!
//! Nothing in `docs/rw_audio_structs.h` names these offsets.

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp};

/// `lis -32245 ; lfs 16660` — the ratio-to-step scale.
pub const RATIO_SCALE: u32 = (((-32245i32 as u32) & 0xFFFF) << 16) + 16660;
/// `lis -32246 ; lfs -26788` — 0.5, the rounding half.
pub const ROUND_HALF: u32 = (((-32246i32 as u32) & 0xFFFF) << 16).wrapping_sub(26788);
/// `lis -32219 ; lfs 29448` — the ratio stored when the step saturates.
pub const RATIO_CEILING: u32 = (((-32219i32 as u32) & 0xFFFF) << 16) + 29448;
/// `lis r10,4` — the most a step can be: 2^18, four frames per sample in 16.16.
pub const STEP_LIMIT: i32 = 262_144;
const _: () = assert!(RATIO_SCALE == 0x820B_4114 && ROUND_HALF == 0x8209_975C);
const _: () = assert!(RATIO_CEILING == 0x8225_7308 && STEP_LIMIT == 4 << 16);

/// The voice's `f32` scale on the requested rate.
pub const VOICE_SCALE: u32 = 52;
/// The ratio in use — the pool ceiling when the step saturated.
pub const VOICE_RATIO: u32 = 56;
/// The ratio computed last time, which decides whether to recompute.
pub const VOICE_PREVIOUS: u32 = 60;
/// The requested rate.
pub const VOICE_REQUESTED: u32 = 64;
/// The 16.16 step.
pub const VOICE_STEP: u32 = 68;
/// The 16.16 fractional position.
pub const VOICE_FRACTION: u32 = 72;
/// `sth r6,78(r3)` — the block's frame count, stored on every call.
pub const VOICE_FRAMES: u32 = 78;
/// A byte subtracted from the whole-frame count.
pub const VOICE_BIAS: u32 = 80;
/// A byte added back to it.
pub const VOICE_CREDIT: u32 = 81;
/// The stream's gain, scaled by the ratio on every call.
pub const STREAM_GAIN: u32 = 56;

/// Recompute the pitch ratio if it moved, advance the position, and return the whole frames consumed
/// (`sub_82B2DAC8`). `voice` is `r3`, `stream` `r4`, `frames` `r6`.
pub fn advance_pitch(g: &mut Guest, voice: u32, stream: u32, frames: u32) -> Result<u64> {
    let format = g.u32(stream.wrapping_add(40))?; // lwz r11,40(r4)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let requested = fp::load_single(g, voice.wrapping_add(VOICE_REQUESTED))?;
    let scale = fp::load_single(g, voice.wrapping_add(VOICE_SCALE))?;
    let previous = fp::load_single(g, voice.wrapping_add(VOICE_PREVIOUS))?;
    let source_rate = fp::load_single(g, format.wrapping_add(12))?;
    let ratio = fp::mul_single(fp::div_single(requested, source_rate), scale); // fdivs ; fmuls

    if previous != ratio {
        let mut scaled = fp::mul_single(ratio, fp::load_single(g, RATIO_SCALE)?);
        let zero = fp::load_single(g, crate::leaves::ZERO_CELL)?;
        let half = fp::load_single(g, ROUND_HALF)?;
        // blt on the compare with zero: round half away from zero, then truncate.
        scaled = if scaled < zero {
            fp::sub_single(scaled, half)
        } else {
            fp::add_single(scaled, half)
        };
        let mut step = fp::fctiwz_low_word(scaled);
        if step as i32 > STEP_LIMIT {
            step = STEP_LIMIT as u32;
            let ceiling = fp::load_single(g, RATIO_CEILING)?;
            fp::store_single(g, voice.wrapping_add(VOICE_RATIO), ceiling)?; // stfs f0,56(r3)
        } else {
            fp::store_single(g, voice.wrapping_add(VOICE_RATIO), ratio)?; // stfs f12,56(r3)
        }
        fp::store_single(g, voice.wrapping_add(VOICE_PREVIOUS), ratio)?; // stfs f12,60(r3)
        g.set_u32(voice.wrapping_add(VOICE_STEP), step)?; // stw r11,68(r3)
    }

    // The tail runs on both paths and re-reads the step and the ratio.
    let stored_step = g.u32(voice.wrapping_add(VOICE_STEP))?;
    let stored_ratio = fp::load_single(g, voice.wrapping_add(VOICE_RATIO))?;
    let fraction = g.u32(voice.wrapping_add(VOICE_FRACTION))?;
    // mullw -- the full 64-bit product of the sign-extended words.
    let advanced = (i64::from(stored_step as i32) * i64::from(frames as i32)) as u64;
    g.set_u16(voice.wrapping_add(VOICE_FRAMES), frames as u16)?; // sth r6,78(r3)
    let bias = u64::from(g.u8(voice.wrapping_add(VOICE_BIAS))?);
    let gain = fp::load_single(g, stream.wrapping_add(STREAM_GAIN))?;
    let credit = u64::from(g.u8(voice.wrapping_add(VOICE_CREDIT))?);
    fp::store_single(
        g,
        stream.wrapping_add(STREAM_GAIN),
        fp::mul_single(stored_ratio, gain),
    )?;
    let position = advanced.wrapping_add(u64::from(fraction));
    let whole = u64::from((position as u32) >> 16); // rlwinm r5,r6,16,16,31
    let remaining = whole.wrapping_sub(bias).wrapping_add(credit);
    let sign = u64::from((remaining as u32) >> 31);
    let mask = sign.wrapping_sub(1); // all ones unless the low word is negative
    Ok(mask & remaining)
}

// ---------------------------------------------------------------- sub_82B2DBA8: one block resampled

/// `lwz r8,8(r31)` — the system whose `+0` is the arena with the bump pointer at `+32`.
pub const BLOCK_SYSTEM: u32 = 8;
/// `lbz r9,42(r31)` — channels, reloaded at every loop test.
pub const BLOCK_CHANNELS: u32 = 42;
/// `lfs f13,64(r3)` — the stream rate this voice was last set up for.
pub const BLOCK_RATE_CACHE: u32 = 64;
/// `lhz r9,76(r31)` — the per-channel tail rows' offset in the voice, 24 bytes a channel.
pub const BLOCK_TAIL_OFFSET: u32 = 76;
/// `lwz r27,28(r19)` / `lwz r26,32(r19)` — the source and destination descriptors.
pub const BLOCK_PAIR_BACK: u32 = 28;
/// The destination descriptor.
pub const BLOCK_PAIR_FRONT: u32 = 32;
/// `lwz r11,40(r4)` — the format, whose `+12` is its rate.
pub const BLOCK_FORMAT: u32 = 40;
/// `lwz r10,48(r19)` — input frames in; output frames out.
pub const BLOCK_INPUT_FRAMES: u32 = 48;
/// `lfs f0,52(r4)` — the stream rate the cache is compared against.
pub const BLOCK_STREAM_RATE: u32 = 52;
/// `lwz r17,32(r7)` — the arena's bump pointer.
pub const ARENA_TOP: u32 = 32;
/// `stwu r1,-224(r1)`. Real: the resampler's cursor and phase slots are `r1 + 80` and `r1 + 84`.
pub const BLOCK_FRAME: u32 = 224;

fn scale4(value: u64) -> u64 {
    u64::from((value as u32) << 2)
}

/// `mullw ; rlwinm ; add` — one channel's float array.
fn block_channel(stride: u64, index: u64, base: u64) -> u64 {
    let elements = i64::from(stride as u32 as i32) * i64::from(index as u32 as i32);
    scale4(elements as u64).wrapping_add(base)
}

/// Resample one block of a stream (`sub_82B2DBA8`). Returns 1.
///
/// `voice` is `r3`, `stream` `r4`, `sp` `r1`. A stream rate that no longer matches the voice's cache
/// only re-caches it and resets the stream's rate from the format. Otherwise a scratch run is taken
/// off the arena's bump pointer; the output frame count is `((usable + 1) << 16 - phase - 1) / step`
/// (8,192 for a zero step), capped at `+78`; each channel's previous tail and new input are copied
/// into the scratch run, resampled by [`crate::dsp::resample::resample`] with this frame's cursor and
/// phase slots, and what was not consumed becomes the channel's tail — four singles at a time, then a
/// chunked copy. The channel count is reloaded every iteration. The pair is swapped, the frame count
/// and the rate republished, and the arena pointer restored through a re-read system pointer.
pub fn resample_block(g: &mut Guest, voice: u32, stream: u32, sp: u32) -> Result<u64> {
    let frame = sp.wrapping_sub(BLOCK_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-224(r1)
    let index_slot = frame.wrapping_add(80);
    let phase_slot = frame.wrapping_add(84);
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let rate = fp::load_single(g, stream.wrapping_add(BLOCK_STREAM_RATE))?; // lfs f0,52(r4)
    let cached = fp::load_single(g, voice.wrapping_add(BLOCK_RATE_CACHE))?; // lfs f13,64(r3)
    if rate != cached {
        fp::store_single(g, voice.wrapping_add(BLOCK_RATE_CACHE), rate)?; // stfs f0,64(r3)
        let format = g.u32(stream.wrapping_add(BLOCK_FORMAT))?; // lwz r11,40(r4)
        let format_rate = fp::load_single(g, format.wrapping_add(12))?; // after the store above
        fp::store_single(g, stream.wrapping_add(BLOCK_STREAM_RATE), format_rate)?;
        return Ok(1);
    }
    let system = g.u32(voice.wrapping_add(BLOCK_SYSTEM))?;
    let input_frames = u64::from(g.u32(stream.wrapping_add(BLOCK_INPUT_FRAMES))?);
    let carried_in = u64::from(g.u8(voice.wrapping_add(VOICE_BIAS))?); // lbz r11,80(r31)
    let available = carried_in.wrapping_add(input_frames); // add r22,r11,r10
    let arena = g.u32(system)?; // lwz r7,0(r8)
    let block = u64::from((scale4(input_frames).wrapping_add(151) as u32) & 0xFFFF_FF80);
    let arena_top = g.u32(arena.wrapping_add(ARENA_TOP))?; // lwz r17,32(r7)
    let scratch = u64::from(arena_top).wrapping_sub(block); // subf r29,r5,r17
    g.set_u32(arena.wrapping_add(ARENA_TOP), scratch as u32)?; // stw r29,32(r7)
    let tail_offset = u64::from(g.u16(voice.wrapping_add(BLOCK_TAIL_OFFSET))?);
    let bias = u64::from(g.u8(voice.wrapping_add(VOICE_CREDIT))?); // lbz r4,81(r31)
    let usable = available.wrapping_sub(bias);
    let src_desc = g.u32(stream.wrapping_add(BLOCK_PAIR_BACK))?;
    let needed = usable.wrapping_add(1); // addic. r11,r3,1
    let dst_desc = g.u32(stream.wrapping_add(BLOCK_PAIR_FRONT))?;
    let tail_base = tail_offset.wrapping_add(u64::from(voice)); // add r18,r9,r31
    let mut frames = 0u32;
    if needed as u32 as i32 > 0 {
        let step = g.u32(voice.wrapping_add(VOICE_STEP))?;
        if step == 0 {
            frames = 8192; // li r25,8192
        } else {
            let phase = u64::from(g.u32(voice.wrapping_add(VOICE_FRACTION))?);
            let scaled = u64::from((needed as u32) << 16);
            frames = (scaled.wrapping_sub(phase).wrapping_sub(1) as u32) / step; // divwu r25,r6,r9
        }
    }
    let cap = u32::from(g.u16(voice.wrapping_add(VOICE_FRAMES))?); // lhz r11,78(r31)
    frames = frames.min(cap);

    let mut channels = u32::from(g.u8(voice.wrapping_add(BLOCK_CHANNELS))?);
    let mut phase_word = 0u32;
    let mut leftover = 0u64;
    if channels != 0 {
        let run_bytes = scale4(input_frames);
        let (mut tail_index, mut tail_row, mut channel) = (0u64, tail_base, 0u32);
        loop {
            let carried = u64::from(g.u8(voice.wrapping_add(VOICE_BIAS))?); // reloaded
            if carried != 0 {
                let bytes = u64::from((carried as u32 & 0xFF).rotate_left(2));
                crate::mem::memcpy_chunked(g, scratch as u32, tail_row as u32, bytes)?; // bl 0x82f52fb8
            }
            let src_stride = u64::from(g.u16(src_desc.wrapping_add(14))?);
            let dst_stride = u64::from(g.u16(dst_desc.wrapping_add(14))?);
            let src_base = u64::from(g.u32(src_desc.wrapping_add(4))?);
            let dst_base = u64::from(g.u32(dst_desc.wrapping_add(4))?);
            let source = block_channel(src_stride, u64::from(channel), src_base);
            let body = scale4(carried).wrapping_add(scratch);
            let destination = block_channel(dst_stride, u64::from(channel), dst_base);
            crate::mem::memcpy(g, body as u32, source as u32, run_bytes)?; // bl 0x82edf460
            let phase = g.u32(voice.wrapping_add(VOICE_FRACTION))?; // lwz r11,72(r31)
            g.set_u32(index_slot, 0)?; // stw r20,80(r1)
            let step = g.u32(voice.wrapping_add(VOICE_STEP))?;
            g.set_u32(phase_slot, phase << 16)?; // rlwinm r10,r11,16,0,15 ; stw r10,84(r1)
            crate::dsp::resample::resample(
                g,
                frames,
                scratch as u32,
                destination as u32,
                index_slot,
                phase_slot,
                u64::from(step),
            )?; // bl 0x82b43fb8
            let consumed = u64::from(g.u32(index_slot)?);
            let mut copied = 0u64;
            leftover = available.wrapping_sub(consumed); // subf r30,r10,r22
            if leftover as u32 as i32 >= 4 {
                let groups = u64::from(((leftover.wrapping_sub(4) as u32) >> 2) & 0x3FFF_FFFF) + 1;
                let mut from = scale4(consumed).wrapping_add(scratch).wrapping_sub(4) as u32;
                let mut to = tail_row.wrapping_sub(4) as u32;
                copied = scale4(groups);
                for _ in 0..groups as u32 {
                    fpscr.disable_flush_mode_unconditional();
                    let a = fp::load_single(g, from.wrapping_add(4))?;
                    let b = fp::load_single(g, from.wrapping_add(8))?;
                    let c = fp::load_single(g, from.wrapping_add(12))?;
                    from = from.wrapping_add(16); // lfsu f0,16(r8)
                    let d = fp::load_single(g, from)?;
                    fp::store_single(g, to.wrapping_add(4), a)?;
                    fp::store_single(g, to.wrapping_add(8), b)?;
                    fp::store_single(g, to.wrapping_add(12), c)?;
                    to = to.wrapping_add(16); // stfsu f0,16(r7)
                    fp::store_single(g, to, d)?;
                }
            }
            if (copied as u32) < (leftover as u32) {
                let out_at = scale4(tail_index.wrapping_add(copied)).wrapping_add(tail_base);
                let in_at = scale4(copied.wrapping_add(consumed)).wrapping_add(scratch);
                let bytes = scale4(leftover.wrapping_sub(copied));
                crate::mem::memcpy_chunked(g, out_at as u32, in_at as u32, bytes)?; // bl 0x82f52fb8
            }
            channels = u32::from(g.u8(voice.wrapping_add(BLOCK_CHANNELS))?); // reloaded
            channel += 1;
            tail_row = tail_row.wrapping_add(24);
            tail_index = tail_index.wrapping_add(6);
            if channel >= channels {
                break;
            }
        }
        phase_word = g.u32(phase_slot)?; // lwz r11,84(r1)
    }
    g.set_u8(voice.wrapping_add(VOICE_BIAS), leftover as u8)?; // stb r30,80(r31)
    g.set_u32(voice.wrapping_add(VOICE_FRACTION), phase_word >> 16)?; // rlwinm r9,r11,16,16,31
    let format = g.u32(stream.wrapping_add(BLOCK_FORMAT))?;
    let back = g.u32(stream.wrapping_add(BLOCK_PAIR_BACK))?;
    let front = g.u32(stream.wrapping_add(BLOCK_PAIR_FRONT))?;
    g.set_u32(stream.wrapping_add(BLOCK_PAIR_FRONT), back)?;
    g.set_u32(stream.wrapping_add(BLOCK_PAIR_BACK), front)?;
    g.set_u32(stream.wrapping_add(BLOCK_INPUT_FRAMES), frames)?; // stw r25,48(r19)
    fpscr.disable_flush_mode_unconditional();
    let format_rate = fp::load_single(g, format.wrapping_add(12))?;
    fp::store_single(g, stream.wrapping_add(BLOCK_STREAM_RATE), format_rate)?;
    let system_now = g.u32(voice.wrapping_add(BLOCK_SYSTEM))?; // re-read
    let arena_now = g.u32(system_now)?;
    g.set_u32(arena_now.wrapping_add(ARENA_TOP), arena_top)?; // stw r17,32(r4)
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const VOICE: u32 = BASE + 0x100;
    const STREAM: u32 = BASE + 0x200;
    const FORMAT: u32 = BASE + 0x300;
    const POISON: u32 = 0xDEAD_BEEF;

    fn guest(requested: f32, rate: f32, scale: f32, previous: f32) -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        for (addr, v) in [
            (RATIO_SCALE, 65536.0f32),
            (ROUND_HALF, 0.5),
            (RATIO_CEILING, 4.0),
            (crate::leaves::ZERO_CELL, 0.0),
        ] {
            g.put(addr, v.to_bits().to_be_bytes().to_vec());
        }
        g.set_u32(STREAM + 40, FORMAT).unwrap();
        g.set_u32(FORMAT + 12, rate.to_bits()).unwrap();
        g.set_u32(STREAM + STREAM_GAIN, 1.0f32.to_bits()).unwrap();
        g.set_u32(VOICE + VOICE_REQUESTED, requested.to_bits())
            .unwrap();
        g.set_u32(VOICE + VOICE_SCALE, scale.to_bits()).unwrap();
        g.set_u32(VOICE + VOICE_PREVIOUS, previous.to_bits())
            .unwrap();
        g.set_u32(VOICE + VOICE_RATIO, POISON).unwrap();
        g.set_u32(VOICE + VOICE_STEP, POISON).unwrap();
        g
    }

    #[test]
    fn a_new_ratio_becomes_a_rounded_step_and_advances_the_position() {
        // 24000 / 48000 * 1.0 = 0.5, so the step is 0.5 * 65536 + 0.5 truncated = 32768.
        let mut g = guest(24_000.0, 48_000.0, 1.0, -1.0);
        let consumed = advance_pitch(&mut g, VOICE, STREAM, 256).unwrap();
        assert_eq!(g.u32(VOICE + VOICE_STEP).unwrap(), 32768);
        assert_eq!(g.f32(VOICE + VOICE_RATIO).unwrap(), 0.5);
        assert_eq!(g.f32(VOICE + VOICE_PREVIOUS).unwrap(), 0.5);
        assert_eq!(g.u16(VOICE + VOICE_FRAMES).unwrap(), 256);
        assert_eq!(
            consumed, 128,
            "256 frames at half speed consume 128 source frames"
        );
        assert_eq!(
            g.f32(STREAM + STREAM_GAIN).unwrap(),
            0.5,
            "the gain is scaled by the ratio"
        );
    }

    #[test]
    fn an_unchanged_ratio_skips_the_recompute_and_its_stores() {
        let mut g = guest(24_000.0, 48_000.0, 1.0, 0.5);
        g.set_u32(VOICE + VOICE_RATIO, 0.25f32.to_bits()).unwrap(); // what the tail will read
        g.set_u32(VOICE + VOICE_STEP, 16384).unwrap();
        let consumed = advance_pitch(&mut g, VOICE, STREAM, 256).unwrap();
        assert_eq!(g.u32(VOICE + VOICE_STEP).unwrap(), 16384, "not rewritten");
        assert_eq!(g.f32(VOICE + VOICE_RATIO).unwrap(), 0.25, "not rewritten");
        assert_eq!(
            consumed, 64,
            "the tail uses the stored step, re-read from memory"
        );
        assert_eq!(
            g.f32(STREAM + STREAM_GAIN).unwrap(),
            0.25,
            "and the stored ratio"
        );
    }

    #[test]
    fn the_step_rounds_half_away_from_zero() {
        // A negative scale makes a negative ratio: -0.5 * 65536 = -32768 exactly, minus the half is
        // -32768.5, truncated toward zero is -32768. Take a ratio whose scaled value is x.5 to see it:
        // ratio 1/131072 scales to exactly 0.5, and half away from zero makes that 1, not 0.
        let mut g = guest(1.0, 131_072.0, 1.0, -1.0);
        advance_pitch(&mut g, VOICE, STREAM, 1).unwrap();
        assert_eq!(g.u32(VOICE + VOICE_STEP).unwrap(), 1, "0.5 rounds up to 1");
        let mut g = guest(1.0, 131_072.0, -1.0, 1.0);
        advance_pitch(&mut g, VOICE, STREAM, 1).unwrap();
        assert_eq!(
            g.u32(VOICE + VOICE_STEP).unwrap() as i32,
            -1,
            "-0.5 rounds down to -1"
        );
    }

    #[test]
    fn a_saturated_step_stores_the_ceiling_ratio_instead() {
        // A ratio of 8 scales to 2^19, past the 2^18 limit.
        let mut g = guest(8.0, 1.0, 1.0, -1.0);
        advance_pitch(&mut g, VOICE, STREAM, 1).unwrap();
        assert_eq!(g.u32(VOICE + VOICE_STEP).unwrap(), STEP_LIMIT as u32);
        assert_eq!(
            g.f32(VOICE + VOICE_RATIO).unwrap(),
            4.0,
            "the pool ceiling, not 8"
        );
        assert_eq!(
            g.f32(VOICE + VOICE_PREVIOUS).unwrap(),
            8.0,
            "while +60 keeps the real ratio"
        );
    }

    #[test]
    fn the_result_clamps_at_zero_when_the_bias_outweighs_the_frames() {
        let mut g = guest(24_000.0, 48_000.0, 1.0, -1.0);
        g.set_u8(VOICE + VOICE_BIAS, 200).unwrap();
        g.set_u8(VOICE + VOICE_CREDIT, 3).unwrap();
        assert_eq!(
            advance_pitch(&mut g, VOICE, STREAM, 256).unwrap(),
            0,
            "128 - 200 + 3 clamps"
        );
        let mut g = guest(24_000.0, 48_000.0, 1.0, -1.0);
        g.set_u8(VOICE + VOICE_BIAS, 20).unwrap();
        g.set_u8(VOICE + VOICE_CREDIT, 3).unwrap();
        assert_eq!(
            advance_pitch(&mut g, VOICE, STREAM, 256).unwrap(),
            111,
            "128 - 20 + 3"
        );
    }

    #[test]
    fn the_pool_addresses_come_from_the_lis_immediates() {
        assert_eq!(RATIO_SCALE, 0x820B_0000 + 16660);
        assert_eq!(ROUND_HALF, 0x820A_0000 - 26788);
        assert_eq!(
            RATIO_CEILING,
            crate::dsp::gain_ramp::STEP_SCALE,
            "the same cell, reached twice"
        );
    }
}

#[cfg(test)]
mod resample_block_tests {
    use super::*;
    use crate::Segment;

    const BASE: u32 = 0x4000_0000;
    const VOICE: u32 = BASE;
    const SYSTEM: u32 = BASE + 0x200;
    const ARENA: u32 = BASE + 0x240;
    const STREAM: u32 = BASE + 0x300;
    const SRC: u32 = BASE + 0x400;
    const DST: u32 = BASE + 0x440;
    const FORMAT: u32 = BASE + 0x480;
    const INPUT: u32 = BASE + 0x1000;
    const OUTPUT: u32 = BASE + 0x2000;
    const TOP: u32 = BASE + 0x8000;
    const SP: u32 = BASE + 0xF000;

    /// One channel of eight input samples 1..8 at unit step and zero phase.
    fn guest(bias: u8) -> Guest {
        let mut g = Guest::from_segments(vec![
            Segment {
                base: BASE,
                bytes: vec![0u8; 0x10000],
            },
            Segment {
                base: 0x822F_8000,
                bytes: vec![0u8; 0x1000],
            },
        ]);
        g.set_u32(
            crate::dsp::resample::FRACTION_SCALE_CELL,
            (1.0f32 / 65536.0).to_bits(),
        )
        .unwrap();
        g.set_u32(VOICE + BLOCK_SYSTEM, SYSTEM).unwrap();
        g.set_u32(SYSTEM, ARENA).unwrap();
        g.set_u32(ARENA + ARENA_TOP, TOP).unwrap();
        g.set_u8(VOICE + BLOCK_CHANNELS, 1).unwrap();
        g.set_u32(VOICE + BLOCK_RATE_CACHE, 48_000.0f32.to_bits())
            .unwrap();
        g.set_u32(VOICE + VOICE_STEP, 0x1_0000).unwrap();
        g.set_u16(VOICE + BLOCK_TAIL_OFFSET, 0x80).unwrap();
        g.set_u16(VOICE + VOICE_FRAMES, 256).unwrap();
        g.set_u8(VOICE + VOICE_CREDIT, bias).unwrap();
        g.set_u32(STREAM + BLOCK_PAIR_BACK, SRC).unwrap();
        g.set_u32(STREAM + BLOCK_PAIR_FRONT, DST).unwrap();
        g.set_u32(STREAM + BLOCK_FORMAT, FORMAT).unwrap();
        g.set_u32(FORMAT + 12, 44_100.0f32.to_bits()).unwrap();
        g.set_u32(STREAM + BLOCK_INPUT_FRAMES, 8).unwrap();
        g.set_u32(STREAM + BLOCK_STREAM_RATE, 48_000.0f32.to_bits())
            .unwrap();
        g.set_u32(SRC + 4, INPUT).unwrap();
        g.set_u16(SRC + 14, 256).unwrap();
        g.set_u32(DST + 4, OUTPUT).unwrap();
        g.set_u16(DST + 14, 256).unwrap();
        for i in 0..8u32 {
            g.set_u32(INPUT + 4 * i, ((i + 1) as f32).to_bits())
                .unwrap();
        }
        g
    }

    #[test]
    fn a_moved_rate_only_recaches_and_resets_the_stream_rate() {
        let mut g = guest(0);
        g.set_u32(STREAM + BLOCK_STREAM_RATE, 32_000.0f32.to_bits())
            .unwrap();
        assert_eq!(resample_block(&mut g, VOICE, STREAM, SP).unwrap(), 1);
        assert_eq!(g.f32(VOICE + BLOCK_RATE_CACHE).unwrap(), 32_000.0);
        assert_eq!(
            g.f32(STREAM + BLOCK_STREAM_RATE).unwrap(),
            44_100.0,
            "from the format"
        );
        assert_eq!(g.u32(STREAM + BLOCK_PAIR_BACK).unwrap(), SRC, "no swap");
    }

    #[test]
    fn a_block_resamples_through_the_scratch_run_and_swaps_the_pair() {
        let mut g = guest(0);
        let mut h = g.clone();
        assert_eq!(resample_block(&mut g, VOICE, STREAM, SP).unwrap(), 1);
        // Eight frames at unit step consume all eight inputs: no tail is left.
        assert_eq!(g.u32(STREAM + BLOCK_INPUT_FRAMES).unwrap(), 8);
        assert_eq!(g.u8(VOICE + VOICE_BIAS).unwrap(), 0);
        assert_eq!(g.u32(STREAM + BLOCK_PAIR_BACK).unwrap(), DST);
        assert_eq!(g.u32(STREAM + BLOCK_PAIR_FRONT).unwrap(), SRC);
        assert_eq!(
            g.u32(ARENA + ARENA_TOP).unwrap(),
            TOP,
            "the arena pointer is restored"
        );
        assert_eq!(g.f32(STREAM + BLOCK_STREAM_RATE).unwrap(), 44_100.0);
        // The destination is what the resampler makes of the input copied to the scratch run.
        let scratch = TOP - 128;
        for i in 0..8u32 {
            h.set_u32(scratch + 4 * i, ((i + 1) as f32).to_bits())
                .unwrap();
        }
        crate::dsp::resample::resample(&mut h, 8, scratch, OUTPUT, SP - 144, SP - 140, 0x1_0000)
            .unwrap();
        for i in 0..8u32 {
            assert_eq!(
                g.u32(OUTPUT + 4 * i).unwrap(),
                h.u32(OUTPUT + 4 * i).unwrap(),
                "output {i}"
            );
        }
    }

    #[test]
    fn what_the_resampler_does_not_consume_becomes_the_tail() {
        // A bias of four holds four samples back: four frames, four consumed, 5..8 kept.
        let mut g = guest(4);
        resample_block(&mut g, VOICE, STREAM, SP).unwrap();
        assert_eq!(g.u32(STREAM + BLOCK_INPUT_FRAMES).unwrap(), 4);
        assert_eq!(g.u8(VOICE + VOICE_BIAS).unwrap(), 4);
        let tail: Vec<f32> = (0..4)
            .map(|i| g.f32(VOICE + 0x80 + 4 * i).unwrap())
            .collect();
        assert_eq!(tail, [5.0, 6.0, 7.0, 8.0]);
    }
}
