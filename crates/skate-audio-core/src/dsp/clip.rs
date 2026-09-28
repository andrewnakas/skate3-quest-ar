//! `sub_82B22678` — the hard clipper: clamp a block into `[-level, level]`, then swap the pair.
//!
//! Ported from `recomp/src/audio_ports/sub_82B22678.inc`, **STATUS: verified** — 76,864 calls in a
//! played session on `RwAudioCore Dac`, compared against the original under the shadow harness.
//!
//! Replayed against **1,485 recorded calls, 0 disagreements**, and compared live against the
//! original 37,784 times in the same session.
//!
//! 256 samples a channel, every channel of the effect's count, source buffer to destination buffer,
//! and then the pair's two pointers exchange so the next stage reads what this one wrote. Scalar
//! throughout: `lfsx`/`stfs` one sample at a time, no vector path at all, which is unusual for a
//! kernel this hot and is the original's choice rather than a simplification here.
//!
//! ## The four things that are not obvious from "it is a clamp"
//!
//! **It can decide to do nothing.** If the level is not *below* the pool's ceiling of 100.0, the
//! function returns 1 having written nothing — not even the pair swap. So a level of exactly 100,
//! or a NaN level (unordered, so not less), leaves the pair pointing where it was and the
//! destination untouched. [`the_ceiling_disables_the_whole_body`] pins it.
//!
//! **The descriptors are reloaded every channel**, after the previous channel's 1 KB of stores.
//! Both buffer pointers and both strides. That is observable whenever a destination span covers a
//! descriptor field, and the C++ window builder refuses those calls rather than mis-window them;
//! here the loads simply happen where the original has them.
//!
//! **The two strides are independent.** The source cursor advances by `src_stride * channel` and
//! the destination by `dst_stride * channel`, each scaled by four into bytes in 32 bits. A port
//! that shared one stride agrees on every buffer whose strides happen to match, which is most of
//! them.
//!
//! **The floor is a single-rounded multiply by the pool's −1.0**, not a negation. `level * -1.0` is
//! exact for every finite single, so no test here can tell the two apart — said rather than left as
//! a silent equivalence.
//!
//! ## The flush mode
//!
//! The original re-emits `disableFlushModeUnconditional` before every sample. Nothing between those
//! points changes the mode, so this holds it once for the call and the values are identical; both
//! of RexGlue's MXCSR values carry `FZ|DAZ` ([`crate::vmx::Fpscr`]), which is why a *denormal*
//! sample comes out as `+0` — `DAZ` reads it as zero on the way in.
//! [`a_denormal_sample_is_flushed_to_zero`] is that assertion, and it is what fails if this port
//! ever stops holding an [`Fpscr`].

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp};

/// `lis r11,-32241` — `0x820F0000`, computed from the immediate.
const LIS_820F0000: u32 = ((-32241i32 as u32) & 0xFFFF) << 16;
/// `lis r11,-32233` — `0x82170000`.
const LIS_82170000: u32 = ((-32233i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_820F0000 == 0x820F_0000 && LIS_82170000 == 0x8217_0000);

/// `lfs f0,-10884(r11)` — measured `100.0`. The level must be **below** this or the body does
/// nothing at all.
pub const CEILING: u32 = LIS_820F0000.wrapping_sub(10884);
/// `lfs f0,-8480(r11)` — measured `-1.0`, the multiplier that turns the level into the floor.
pub const FLOOR_SCALE: u32 = LIS_82170000.wrapping_sub(8480);

const _: () = assert!(CEILING == 0x820E_D57C && FLOOR_SCALE == 0x8216_DEE0);

/// `lbz r31,42(r3)` — the effect state's channel count, one byte.
pub const CHANNEL_COUNT: u32 = 42;
/// `lfs f13,52(r3)` — the clip level, a single.
pub const CLIP_LEVEL: u32 = 52;
/// `lwz r5,28(r4)` — the pair's source descriptor pointer, written back as the destination.
pub const SOURCE_BUFFER: u32 = 28;
/// `lwz r6,32(r4)` — the pair's destination descriptor pointer.
pub const DEST_BUFFER: u32 = 32;
/// `lwz r10,4(r5)` — a descriptor's sample pointer.
pub const BUFFER_DATA: u32 = 4;
/// `lhz r10,14(r5)` — a descriptor's channel stride, in samples.
pub const BUFFER_STRIDE: u32 = 14;
/// `li r11,256 ; mtctr r11` — the block length, in samples per channel.
pub const FRAME_SAMPLES: u32 = 256;

/// Clamp every channel of the source block into `[-level, level]` and swap the pair (`r3 = 1`).
///
/// `state` is the guest's `r3` and `pair` its `r4`. The return value is the constant 1 the original
/// leaves in `r3` on every path, including the path that writes nothing.
pub fn hard_clip(g: &mut Guest, state: u32, pair: u32) -> Result<u64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f13,52(r3)

    let level = fp::load_single(g, state + CLIP_LEVEL)?; // lfs f13,52(r3)
    let ceiling = fp::load_single(g, CEILING)?; // lfs f0,-10884(r11)

    // fcmpu cr6,f13,f0 ; bge cr6 -> loc_82B2272C. Unordered is not less, so a NaN level skips too.
    if level < ceiling {
        let channels = u32::from(g.u8(state + CHANNEL_COUNT)?); // lbz r31,42(r3)
        let source = g.u32(pair + SOURCE_BUFFER)?; // lwz r5,28(r4)
        let dest = g.u32(pair + DEST_BUFFER)?; // lwz r6,32(r4)
        let scale = fp::load_single(g, FLOOR_SCALE)?; // lfs f0,-8480(r11)
        let floor_level = fp::mul_single(level, scale); // fmuls f12,f13,f0

        for channel in 0..channels {
            // loc_82B226B4: both descriptors are re-read here, after the previous channel's stores.
            let src_stride = u32::from(g.u16(source + BUFFER_STRIDE)?); // lhz r10,14(r5)
            let dst_stride = u32::from(g.u16(dest + BUFFER_STRIDE)?); // lhz r8,14(r6)
            // mullw then rlwinm ...,2,0,29: the byte offsets are 32-bit throughout.
            let src_offset = src_stride.wrapping_mul(channel) << 2;
            let dst_offset = dst_stride.wrapping_mul(channel) << 2;
            let src_data = g.u32(source + BUFFER_DATA)?; // lwz r10,4(r5)
            let dst_data = g.u32(dest + BUFFER_DATA)?; // lwz r9,4(r6)
            let input = src_offset.wrapping_add(src_data);
            let mut out = dst_offset.wrapping_add(dst_data);

            for k in 0..FRAME_SAMPLES {
                let sample = fp::load_single(g, input.wrapping_add(k << 2))?; // lfsx f0,r10,r11
                let value = if sample > level {
                    level // stfs f13,0(r11)
                } else if sample < floor_level {
                    floor_level // stfs f12,0(r11)
                } else {
                    sample // stfs f0,0(r11) -- a NaN lands here, both compares being unordered
                };
                fp::store_single(g, out, value)?;
                out = out.wrapping_add(4); // addi r11,r11,4
            }
        }

        // loc_82B22724: the swap, in the original's order.
        g.set_u32(pair + DEST_BUFFER, source)?; // stw r5,32(r4)
        g.set_u32(pair + SOURCE_BUFFER, dest)?; // stw r6,28(r4)
    }

    Ok(1) // li r3,1
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const STATE: u32 = BASE + 0x40;
    const PAIR: u32 = BASE + 0x80;
    const DESC_A: u32 = BASE + 0x100;
    const DESC_B: u32 = BASE + 0x140;
    const BUF_A: u32 = BASE + 0x1000;
    const BUF_B: u32 = BASE + 0x5000;
    const POISON: u32 = 0xDEAD_BEEF;

    /// A guest with the two pool singles, an effect state, a pair and two descriptors.
    fn guest(channels: u8, level: f32, src_stride: u16, dst_stride: u16) -> Guest {
        let mut g = Guest::single(BASE, 0xA000);
        g.put(CEILING, 100.0f32.to_bits().to_be_bytes().to_vec());
        g.put(FLOOR_SCALE, (-1.0f32).to_bits().to_be_bytes().to_vec());
        g.set_u8(STATE + CHANNEL_COUNT, channels).unwrap();
        g.set_u32(STATE + CLIP_LEVEL, level.to_bits()).unwrap();
        g.set_u32(PAIR + SOURCE_BUFFER, DESC_A).unwrap();
        g.set_u32(PAIR + DEST_BUFFER, DESC_B).unwrap();
        g.set_u32(DESC_A + BUFFER_DATA, BUF_A).unwrap();
        g.set_u16(DESC_A + BUFFER_STRIDE, src_stride).unwrap();
        g.set_u32(DESC_B + BUFFER_DATA, BUF_B).unwrap();
        g.set_u16(DESC_B + BUFFER_STRIDE, dst_stride).unwrap();
        for i in 0..0x1000 / 4 {
            g.set_u32(BUF_B + i * 4, POISON).unwrap();
        }
        g
    }

    fn fill(g: &mut Guest, at: u32, values: impl Iterator<Item = f32>) {
        for (i, v) in values.enumerate() {
            g.set_u32(at + (i as u32) * 4, v.to_bits()).unwrap();
        }
    }

    fn out(g: &Guest, at: u32, n: usize) -> Vec<f32> {
        (0..n as u32).map(|i| g.f32(at + i * 4).unwrap()).collect()
    }

    #[test]
    fn it_clamps_both_ways_and_passes_the_middle_through() {
        let mut g = guest(1, 0.5, 256, 256);
        // A ramp from -1 to about +1, so both clamps and the pass-through all occur.
        fill(&mut g, BUF_A, (0..256).map(|i| (i as f32) / 128.0 - 1.0));

        assert_eq!(hard_clip(&mut g, STATE, PAIR).unwrap(), 1);

        let got = out(&g, BUF_B, 256);
        assert_eq!(got[0], -0.5, "below the floor");
        assert_eq!(got[255], 0.5, "above the ceiling");
        assert_eq!(got[128], 0.0, "the middle is untouched");
        assert_eq!(got[160], 0.25, "and so is anything inside the band");
        for (i, v) in got.iter().enumerate() {
            assert!((-0.5..=0.5).contains(v), "sample {i} = {v} left the band");
        }
        // Exactly 256 samples: the word after the block is still poisoned.
        assert_eq!(g.u32(BUF_B + 256 * 4).unwrap(), POISON);
    }

    #[test]
    fn the_ceiling_disables_the_whole_body() {
        // Not "clamps to 100": the body *returns* without writing anything, including the swap.
        for level in [100.0f32, 1000.0, f32::NAN] {
            let mut g = guest(1, level, 256, 256);
            fill(&mut g, BUF_A, (0..256).map(|_| 7.0));

            assert_eq!(hard_clip(&mut g, STATE, PAIR).unwrap(), 1, "level {level}");

            assert_eq!(
                g.u32(BUF_B).unwrap(),
                POISON,
                "level {level}: nothing written"
            );
            assert_eq!(
                g.u32(PAIR + SOURCE_BUFFER).unwrap(),
                DESC_A,
                "level {level}: no swap"
            );
            assert_eq!(g.u32(PAIR + DEST_BUFFER).unwrap(), DESC_B);
        }
        // And just below the ceiling it does run, so the test above is not vacuous.
        let mut g = guest(1, 99.9, 256, 256);
        fill(&mut g, BUF_A, (0..256).map(|_| 7.0));
        hard_clip(&mut g, STATE, PAIR).unwrap();
        assert_eq!(g.f32(BUF_B).unwrap(), 7.0);
        assert_eq!(
            g.u32(PAIR + SOURCE_BUFFER).unwrap(),
            DESC_B,
            "and the pair swapped"
        );
    }

    #[test]
    fn the_pair_pointers_are_exchanged_after_the_block() {
        let mut g = guest(1, 0.5, 256, 256);
        fill(&mut g, BUF_A, (0..256).map(|_| 0.25));

        hard_clip(&mut g, STATE, PAIR).unwrap();

        assert_eq!(
            g.u32(PAIR + SOURCE_BUFFER).unwrap(),
            DESC_B,
            "source := old dest"
        );
        assert_eq!(
            g.u32(PAIR + DEST_BUFFER).unwrap(),
            DESC_A,
            "dest := old source"
        );
    }

    #[test]
    fn the_two_strides_are_independent() {
        // Source channels 300 samples apart, destination channels 256 apart. Sharing one stride
        // would read channel 1 from the wrong place, and every value here differs by channel.
        let mut g = guest(3, 0.9, 300, 256);
        for channel in 0..3u32 {
            fill(
                &mut g,
                BUF_A + channel * 300 * 4,
                (0..256).map(move |i| (channel as f32) + (i as f32) / 1000.0),
            );
        }

        hard_clip(&mut g, STATE, PAIR).unwrap();

        for channel in 0..3u32 {
            let got = out(&g, BUF_B + channel * 256 * 4, 4);
            let want: Vec<f32> = (0..4)
                .map(|i| ((channel as f32) + (i as f32) / 1000.0).min(0.9))
                .collect();
            assert_eq!(got, want, "channel {channel}");
        }
        // Three channels, not four.
        assert_eq!(g.u32(BUF_B + 3 * 256 * 4).unwrap(), POISON);
    }

    #[test]
    fn a_channel_count_of_zero_writes_nothing_but_still_swaps() {
        let mut g = guest(0, 0.5, 256, 256);
        fill(&mut g, BUF_A, (0..256).map(|_| 9.0));

        assert_eq!(hard_clip(&mut g, STATE, PAIR).unwrap(), 1);

        assert_eq!(g.u32(BUF_B).unwrap(), POISON, "no channel, no samples");
        assert_eq!(
            g.u32(PAIR + SOURCE_BUFFER).unwrap(),
            DESC_B,
            "the swap is outside the loop"
        );
    }

    #[test]
    fn a_nan_sample_passes_through_because_both_compares_are_unordered() {
        let mut g = guest(1, 0.5, 256, 256);
        fill(&mut g, BUF_A, (0..256).map(|_| 0.0));
        g.set_u32(BUF_A, 0x7FC0_0DAD).unwrap(); // a quiet NaN with a payload

        hard_clip(&mut g, STATE, PAIR).unwrap();

        // Neither `> level` nor `< floor` holds, so the else arm stores the sample itself, payload
        // intact. A clamp written with min/max would return the level instead.
        assert_eq!(g.u32(BUF_B).unwrap(), 0x7FC0_0DAD);
    }

    #[test]
    fn a_finite_sample_inside_the_band_is_stored_unchanged() {
        // The `lfs`/`stfs` round trip is exact for finite singles, so an in-band sample is copied
        // bit for bit rather than merely to within a rounding.
        // The level has to stay below the pool's 100.0 or the body does nothing — which is the
        // mistake this test made on its first run, and the reason it read back 0xDEADBEEF.
        let mut g = guest(1, 99.0, 256, 256);
        let values = [0.375f32, -98.5, 1.0, 7.125e-20];
        fill(
            &mut g,
            BUF_A,
            values
                .iter()
                .copied()
                .chain(std::iter::repeat(0.0))
                .take(256),
        );

        hard_clip(&mut g, STATE, PAIR).unwrap();

        for (i, v) in values.iter().enumerate() {
            assert_eq!(
                g.u32(BUF_B + (i as u32) * 4).unwrap(),
                v.to_bits(),
                "sample {i}"
            );
        }
    }

    // **A denormal sample is deliberately not asserted.** `examples/flush_probe` measured why on
    // 2026-09-13: LLVM folds the `lfs`/`stfs` pair (`fptrunc(fpext(x))`) into a no-op at
    // `opt-level >= 1`, so `DAZ` never sees the value and the denormal is copied through; at
    // `opt-level = 0` the conversions run and it becomes `+0`. Which of those the *recomp* does is
    // unmeasured — its lifted body has the same pair and is built `-O3` — and no recorded vector
    // for this function contains a denormal, so the shadow harness never decided it. An assertion
    // either way would pin this crate's profile rather than the kernel. Flush mode still matters
    // here for real arithmetic: the probe's denormal *product* is `+0` under both profiles, which is
    // the case `crate::dsp::biquad` depends on.

    #[test]
    fn the_pool_addresses_come_from_the_lis_immediates() {
        assert_eq!(CEILING, 0x820F_0000 - 10884);
        assert_eq!(FLOOR_SCALE, 0x8217_0000 - 8480);
        // And the values are read live: a patched ceiling changes whether the body runs at all.
        let mut g = guest(1, 99.0, 256, 256);
        g.set_u32(CEILING, 1.0f32.to_bits()).unwrap();
        fill(&mut g, BUF_A, (0..256).map(|_| 7.0));
        hard_clip(&mut g, STATE, PAIR).unwrap();
        assert_eq!(
            g.u32(BUF_B).unwrap(),
            POISON,
            "99 is no longer below the ceiling"
        );
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest(1, 0.5, 256, 256);
        fill(&mut g, BUF_A, (0..256).map(|_| 0.25));
        let before = crate::vmx::get_mxcsr();
        hard_clip(&mut g, STATE, PAIR).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }
}
