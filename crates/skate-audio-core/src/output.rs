//! The output stage: route the planes, interleave them, ramp once if asked, and clamp to ±1.
//!
//! Three verified functions, ported from their `recomp/src/audio_ports/sub_*.inc`:
//!
//! | function | guest | lifted lines | calls/boot | calls/play | status |
//! |---|---|---|---|---|---|
//! | [`output_pass`] | `sub_82B21F58` | 25 | 15,953 | 18,300 | verified |
//! | [`mix_and_clamp`] | `sub_82B21D98` | 260 | 31,906 | 36,600 | verified |
//! | [`ramp_block`] | `sub_82B20E18` | 143 | 3 | 3 | **thin** — one call per profile |
//!
//! Replayed against **800 recorded calls, 0 disagreements** — 399 for the wrapper, 400 for the pass,
//! and 1 for the ramp, whose flag is rarely set; the ramp's evidence is the unit tests plus that one call.
//!
//! [`mix_and_clamp`] is almost entirely composition. It builds eight source plane pointers and up to
//! eight destination plane pointers in its own frame, hands both arrays to
//! [`crate::routing::scatter_mix`] with a route range picked by the output channel count, interleaves
//! the destination planes into a 6,144-byte block with [`crate::interleave::interleave_six`], runs
//! [`ramp_block`] over it if a global flag byte asks, clears that flag, and finally clamps every word
//! of the block into `[-1, 1]` in place. So its own risk is the pointer arithmetic, the block address,
//! and the clamp — which is where the tests aim.
//!
//! ## The clamp only stores what it changes
//!
//! Unlike [`crate::dsp::clip`], which rewrites every sample, this clamp loads each word and stores
//! **only when it is out of range**. An in-range word is never rewritten, so its exact bits survive —
//! a NaN included, since both unordered compares fail and neither store runs.
//!
//! ## One vestigial branch, not reproduced
//!
//! The original loads the object's `+52` and a pool single and compares them, and both arms of the
//! branch compute the same route-table address — only the register allocation differs, and the
//! condition register it sets is never read. So the comparison has no effect, and its two loads are
//! not performed here. That is a deliberate omission rather than an oversight: one of those two cells
//! is not in the recorded read set, and loading it would make every replay of this function fail on a
//! read whose result nothing uses.
//!
//! ## The ramp covers half the block
//!
//! [`ramp_block`] is 128 frames of *channel-count* floats — 768 words at six channels — while the
//! block is 256 frames of 24 bytes. So when the flag is set, the first half of the block is ramped
//! and the second is not. Reproduced as the original does it; what it is *for* is not established.

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp, interleave, routing};

const LIS_83060000: u32 = ((-31994i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_83060000 == 0x8306_0000, "lis -31994");

/// `lis -31994 ; lbz 28765` — the number of output planes, a global byte.
pub const CHANNEL_COUNT_BYTE: u32 = LIS_83060000 + 28765;
/// `lis -31994 ; lbz/stb 28759` — "ramp the next block once", cleared after the ramp runs.
pub const RAMP_FLAG_BYTE: u32 = LIS_83060000 + 28759;
/// `lis -32241 ; addi -10548` — two-byte route ranges, one per output count, into [`OP_TABLE`].
pub const PAIR_TABLE: u32 = (((-32241i32 as u32) & 0xFFFF) << 16).wrapping_sub(10548);
/// `lis -32241 ; addi -10532` — the route bytes the scatter-mixer decodes.
pub const OP_TABLE: u32 = (((-32241i32 as u32) & 0xFFFF) << 16).wrapping_sub(10532);
/// `lis -32206 ; lfs -22460` — measured 1.0: the clamp's upper bound, and the ramp's step.
pub const CLAMP_HIGH: u32 = (((-32206i32 as u32) & 0xFFFF) << 16).wrapping_sub(22460);
/// `lis -32233 ; lfs -8480` — measured −1.0: the clamp's lower bound.
pub const CLAMP_LOW: u32 = (((-32233i32 as u32) & 0xFFFF) << 16).wrapping_sub(8480);
/// `lis -32234 ; lfs 23056` — the image's zero, where the ramp's frame index starts.
pub const RAMP_ZERO: u32 = (((-32234i32 as u32) & 0xFFFF) << 16).wrapping_add(23056);
/// `lis -32253 ; lfs 212` — the ramp's per-frame scale.
pub const RAMP_SCALE: u32 = (((-32253i32 as u32) & 0xFFFF) << 16).wrapping_add(212);
/// `lis -32206 ; lfs -22460` — the ramp's frame step, which is the same cell as [`CLAMP_HIGH`].
pub const RAMP_STEP: u32 = CLAMP_HIGH;

const _: () = assert!(CHANNEL_COUNT_BYTE == 0x8306_705D && RAMP_FLAG_BYTE == 0x8306_7057);
const _: () = assert!(PAIR_TABLE == 0x820E_D6CC && OP_TABLE == 0x820E_D6DC);
const _: () = assert!(CLAMP_HIGH == 0x8231_A844 && CLAMP_LOW == 0x8216_DEE0);
const _: () = assert!(RAMP_ZERO == 0x8216_5A10 && RAMP_SCALE == 0x8203_00D4);
const _: () = assert!(
    CLAMP_HIGH == routing::UNITY_GAIN,
    "the clamp bound is the mixer's 1.0"
);

/// `lwz r10,44(r3)` — the object's pointer to its pair of plane descriptors.
pub const DESC_HOLDER: u32 = 44;
/// `lwz r8,28(r10)` — the eight source planes' descriptor.
pub const SRC_DESC: u32 = 28;
/// `lwz r30,32(r10)` — the output planes' descriptor.
pub const DST_DESC: u32 = 32;
/// `lwz r10,84(r31)` — the base of the output blocks.
pub const BLOCK_BASE: u32 = 84;
/// `lwz r11,92(r31)` — which output block this pass fills.
pub const BLOCK_INDEX: u32 = 92;

/// `stwu r1,-176(r1)` — the mixing pass's frame, which holds the two pointer arrays.
pub const MIX_FRAME_BYTES: u32 = 176;
/// `addi r4,r1,80` — eight source plane pointers.
pub const SRC_ARRAY: u32 = 80;
/// `addi r3,r1,112` — up to eight destination plane pointers.
pub const DST_ARRAY: u32 = 112;
/// `stwu r1,-96(r1)` — the wrapper's frame.
pub const PASS_FRAME_BYTES: u32 = 96;
/// `li r6,256` — frames per plane, the count the scatter-mixer is given.
pub const MIX_FRAMES: u32 = 256;
/// `addi r11,r31,6144` — 256 frames of 24 bytes.
pub const BLOCK_BYTES: u32 = 6144;
/// The clamp's constant trip count, `((6144 - 1) >> 2) + 1`.
pub const CLAMP_TRIPS: u32 = ((BLOCK_BYTES - 1) >> 2) + 1;
/// Frames the ramp covers: `rotlwi r10,r11,9` is `channels * 512` bytes.
pub const RAMP_FRAMES: u32 = 128;

/// Apply a rising per-frame gain across 128 frames of an interleaved block (`sub_82B20E18`).
///
/// `r3` is the block, taken at full width because an empty block returns without touching it. The
/// channel count is the global byte, read once. Each frame's gain is `frame_index * scale`, single
/// rounded, and the index starts at the image's zero and steps by the image's 1.0 — so frame *k* is
/// scaled by `k * scale`. The first four-at-a-time loop and the remainder loop compute that gain
/// **separately and identically**; the tests use a channel count that reaches both.
///
/// Returns the block pointer the loop leaves in `r3`: the end of the 128th frame.
pub fn ramp_block(g: &mut Guest, r3: u64) -> Result<u64> {
    let start = r3 as u32;
    let channels = u32::from(g.u8(CHANNEL_COUNT_BYTE)?); // lbz r11,28765(r11)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let mut frame_index = fp::load_single(g, RAMP_ZERO)?; // lfs f13,23056(r10)
    // rotlwi r10,r11,9 ; add r7,r10,r3
    let end = channels.rotate_left(9).wrapping_add(start);
    if start >= end {
        return Ok(r3); // bgelr -- an empty block writes nothing, and r3 is untouched
    }
    let stride = channels.wrapping_mul(4) & 0xFFFF_FFFC; // rlwinm r6,r11,2,0,29
    let step = fp::load_single(g, RAMP_STEP)?; // lfs f11,-22460(r11)
    let scale = fp::load_single(g, RAMP_SCALE)?; // lfs f12,212(r10)

    let mut block = start;
    loop {
        let mut done = 0u32;
        if channels as i32 >= 4 {
            let gain = fp::mul_single(frame_index, scale); // fmuls f0,f13,f12
            let trips = ((channels - 4) >> 2) + 1;
            done = trips * 4;
            let mut at = block;
            for _ in 0..trips {
                // Four loads, then four stores; the last is `stfsu`, which also advances by 16.
                let s0 = fp::load_single(g, at)?;
                let s1 = fp::load_single(g, at + 4)?;
                let s2 = fp::load_single(g, at + 8)?;
                let s3 = fp::load_single(g, at + 12)?;
                fp::store_single(g, at, fp::mul_single(s0, gain))?;
                fp::store_single(g, at + 4, fp::mul_single(s1, gain))?;
                fp::store_single(g, at + 8, fp::mul_single(s2, gain))?;
                fp::store_single(g, at + 12, fp::mul_single(s3, gain))?;
                at = at.wrapping_add(16);
            }
        }
        if (done as i32) < channels as i32 {
            let gain = fp::mul_single(frame_index, scale); // the same fmuls, again
            let mut at = block.wrapping_add(done * 4);
            for _ in done..channels {
                let s = fp::load_single(g, at)?;
                fp::store_single(g, at, fp::mul_single(s, gain))?;
                at = at.wrapping_add(4);
            }
        }
        block = block.wrapping_add(stride); // add r3,r6,r3
        frame_index = fp::add_single(frame_index, step); // fadds f13,f13,f11
        if block >= end {
            break; // cmplw ; blt
        }
    }
    Ok(u64::from(block))
}

/// The 5.1 mix-and-clamp pass (`sub_82B21D98`). `object` is `r3` and `sp` the guest `r1` on entry.
///
/// Its frame is reproduced: the two pointer arrays are real guest memory that the scatter-mixer
/// reads back, so they have to be written where the original writes them.
pub fn mix_and_clamp(g: &mut Guest, object: u32, sp: u32) -> Result<()> {
    let frame = sp.wrapping_sub(MIX_FRAME_BYTES); // stwu r1,-176(r1)
    g.set_u32(frame, sp)?;

    let holder = g.u32(object + DESC_HOLDER)?; // lwz r10,44(r3)
    let src_desc = g.u32(holder + SRC_DESC)?; // lwz r8,28(r10)
    let outputs = u32::from(g.u8(CHANNEL_COUNT_BYTE)?); // lbz r5,28765(r9)
    let dst_desc = g.u32(holder + DST_DESC)?; // lwz r30,32(r10)
    let src_frames = u64::from(g.u16(src_desc + interleave::PLANE_FRAMES)?); // lhz r11,14(r8)
    let src_base = u64::from(g.u32(src_desc + interleave::PLANE_BASE)?); // lwz r10,4(r8)

    // Eight source plane pointers, spelled out one at a time in the original: 4k * frames + base,
    // 64-bit adds on zero-extended words, each stored as its low word.
    for k in 0..8u32 {
        let pointer = u64::from(4 * k) * src_frames + src_base;
        g.set_u32(frame + SRC_ARRAY + 4 * k, pointer as u32)?;
    }

    // cmplwi cr6,r5,0 ; beq -- a zero count leaves the destination array alone, and is also what
    // stops `mtctr` running 2^32 times.
    if outputs != 0 {
        let stride =
            u64::from(u32::from(g.u16(dst_desc + interleave::PLANE_FRAMES)?).rotate_left(2));
        let mut pointer = u64::from(g.u32(dst_desc + interleave::PLANE_BASE)?); // lwz r11,4(r30)
        for i in 0..outputs {
            g.set_u32(frame + DST_ARRAY + 4 * i, pointer as u32)?; // stwu r11,4(r10)
            pointer += stride; // add r11,r9,r11 -- a 64-bit running sum, truncated at each store
        }
    }

    // The vestigial compare of +52 against a pool single sits here; see the module note.

    // rlwinm r5,1,0,30 ; add ; addi r7,r11,-2 -- the route range for this output count.
    let pair = PAIR_TABLE.wrapping_add(2 * outputs).wrapping_sub(2);
    routing::scatter_mix(
        g,
        frame + DST_ARRAY,
        frame + SRC_ARRAY,
        outputs,
        MIX_FRAMES,
        pair,
        OP_TABLE,
    )?;

    let block_index = g.u32(object + BLOCK_INDEX)?; // lwz r11,92(r31)
    let block_base = u64::from(g.u32(object + BLOCK_BASE)?); // lwz r10,84(r31)
    // rlwinm r9,r11,1,0,30 ; add -- 3 * index, whose carry out of bit 31 the next rlwinm drops.
    let triple = block_index.wrapping_add((block_index << 1) & 0xFFFF_FFFE);
    let block_offset = (triple << 11) & 0xFFFF_F800; // rlwinm r11,r11,11,0,20
    // add r31,r11,r10 -- 64-bit, and passed on whole in r3.
    let block = u64::from(block_offset) + block_base;

    interleave::interleave_six(g, block, dst_desc)?; // bl 0x82b46b30

    // lbz r10,28759(r5) -- the ramp flag, read after the interleave.
    if g.u8(RAMP_FLAG_BYTE)? != 0 {
        ramp_block(g, block)?; // bl 0x82b20e18
        g.set_u8(RAMP_FLAG_BYTE, 0)?; // li r11,0 ; stb r11,28759(r5)
    }

    // addi r11,r31,6144 ; cmplw ; bge -- an unsigned 32-bit compare, so a block whose end wraps
    // 2^32 skips the clamp entirely.
    if (block as u32) < ((block + u64::from(BLOCK_BYTES)) as u32) {
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        let high = fp::load_single(g, CLAMP_HIGH)?; // lfs f12,-22460(r9)
        let low = fp::load_single(g, CLAMP_LOW)?; // lfs f13,-8480(r11)
        let mut word = block as u32;
        for _ in 0..CLAMP_TRIPS {
            let value = fp::load_single(g, word)?; // lfs f0,0(r10)
            if value < low {
                fp::store_single(g, word, low)?; // stfs f13,0(r10)
            } else if value > high {
                fp::store_single(g, word, high)?; // stfs f12,0(r10)
            }
            // An in-range word -- or a NaN, which fails both compares -- is not rewritten.
            word = word.wrapping_add(4);
        }
    }
    Ok(())
}

/// The frame wrapper around [`mix_and_clamp`] (`sub_82B21F58`): open 96 bytes, run it, return 1.
///
/// The frame is reproduced because the pass it calls opens its own below it, and that pass's pointer
/// arrays would otherwise land 96 bytes higher, on top of the caller's locals.
pub fn output_pass(g: &mut Guest, object: u32, sp: u32) -> Result<u64> {
    let frame = sp.wrapping_sub(PASS_FRAME_BYTES); // stwu r1,-96(r1)
    g.set_u32(frame, sp)?;
    mix_and_clamp(g, object, frame)?; // bl 0x82b21d98
    Ok(1) // li r3,1 -- on every path
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE + 0x40;
    const HOLDER: u32 = BASE + 0x100;
    const SRC_DESC_AT: u32 = BASE + 0x140;
    const DST_DESC_AT: u32 = BASE + 0x180;
    const SRC_PLANES: u32 = BASE + 0x1000;
    const DST_PLANES: u32 = BASE + 0x3000;
    const BLOCKS: u32 = BASE + 0x6000;
    /// Block index 1 puts the block 6,144 bytes into the output area.
    const BLOCK: u32 = BLOCKS + 6144;
    const SP: u32 = BASE + 0xF000;
    const SCALE: f32 = 1.0 / 128.0;

    fn source(plane: u32, frame: u32) -> f32 {
        (plane as f32 - 2.5) * 0.6 + frame as f32 * 0.001 // planes 0 and 5 leave [-1, 1]
    }

    fn slot_of(plane: u32) -> u32 {
        interleave::LANES
            .iter()
            .find(|(p, _)| *p == plane)
            .unwrap()
            .1
    }

    /// Six outputs, each routed from the source plane of the same index at gain 1.0.
    fn guest(ramp: bool) -> Guest {
        let mut g = Guest::single(BASE, 0x1_0000);
        g.put(0x8306_7000, vec![0u8; 0x100]);
        g.put(routing::GAIN_TABLE, vec![0u8; 0x80]); // gains, then the pair and op tables
        g.put(CLAMP_HIGH, 1.0f32.to_bits().to_be_bytes().to_vec());
        g.put(CLAMP_LOW, (-1.0f32).to_bits().to_be_bytes().to_vec());
        g.put(RAMP_ZERO, 0.0f32.to_bits().to_be_bytes().to_vec());
        g.put(RAMP_SCALE, SCALE.to_bits().to_be_bytes().to_vec());

        g.set_u8(CHANNEL_COUNT_BYTE, 6).unwrap();
        g.set_u8(RAMP_FLAG_BYTE, u8::from(ramp)).unwrap();
        g.set_u32(routing::GAIN_TABLE, 1.0f32.to_bits()).unwrap();
        // Six outputs: the range is at PAIR_TABLE + 2*6 - 2, and names op bytes 0..=5.
        g.set_u8(PAIR_TABLE + 10, 0).unwrap();
        g.set_u8(PAIR_TABLE + 11, 5).unwrap();
        for i in 0..6u8 {
            g.set_u8(OP_TABLE + u32::from(i), (i << 3) | i).unwrap(); // gain 0, source i, dest i
        }

        g.set_u32(OBJECT + DESC_HOLDER, HOLDER).unwrap();
        g.set_u32(HOLDER + SRC_DESC, SRC_DESC_AT).unwrap();
        g.set_u32(HOLDER + DST_DESC, DST_DESC_AT).unwrap();
        g.set_u32(SRC_DESC_AT + interleave::PLANE_BASE, SRC_PLANES)
            .unwrap();
        g.set_u16(SRC_DESC_AT + interleave::PLANE_FRAMES, 256)
            .unwrap();
        g.set_u32(DST_DESC_AT + interleave::PLANE_BASE, DST_PLANES)
            .unwrap();
        g.set_u16(DST_DESC_AT + interleave::PLANE_FRAMES, 256)
            .unwrap();
        g.set_u32(OBJECT + BLOCK_BASE, BLOCKS).unwrap();
        g.set_u32(OBJECT + BLOCK_INDEX, 1).unwrap();
        for p in 0..8u32 {
            for f in 0..256u32 {
                g.set_u32(SRC_PLANES + p * 1024 + f * 4, source(p, f).to_bits())
                    .unwrap();
            }
        }
        g
    }

    fn clamp(v: f32) -> f32 {
        v.clamp(-1.0, 1.0)
    }

    #[test]
    fn it_routes_interleaves_and_clamps_into_the_indexed_block() {
        let mut g = guest(false);
        mix_and_clamp(&mut g, OBJECT, SP).unwrap();
        for f in [0u32, 1, 100, 255] {
            for p in 0..6u32 {
                let at = BLOCK + f * interleave::FRAME_BYTES + slot_of(p);
                assert_eq!(
                    g.f32(at).unwrap(),
                    clamp(source(p, f)),
                    "frame {f}, plane {p}"
                );
            }
        }
        // The destination planes carry the unclamped routed values; the clamp is on the block only.
        assert_eq!(
            g.f32(DST_PLANES).unwrap(),
            source(0, 0),
            "plane 0 before the clamp"
        );
        assert!(
            source(0, 0) < -1.0,
            "and it was out of range, so the clamp had work to do"
        );
    }

    #[test]
    fn the_block_index_selects_a_6144_byte_block() {
        // Index 1 is 6,144 bytes in; the block before it is untouched.
        let mut g = guest(false);
        g.set_u32(BLOCKS, 0xDEAD_BEEF).unwrap();
        mix_and_clamp(&mut g, OBJECT, SP).unwrap();
        assert_eq!(g.u32(BLOCKS).unwrap(), 0xDEAD_BEEF, "block 0 untouched");
        assert_eq!(BLOCK - BLOCKS, 3 << 11, "rlwinm ...,11: 3 * index << 11");
    }

    #[test]
    fn a_set_ramp_flag_ramps_the_first_128_frames_and_is_cleared() {
        let mut g = guest(true);
        mix_and_clamp(&mut g, OBJECT, SP).unwrap();
        assert_eq!(g.u8(RAMP_FLAG_BYTE).unwrap(), 0, "the flag is cleared");
        for f in [0u32, 1, 64, 127, 128, 200] {
            for p in [1u32, 3] {
                let routed = source(p, f);
                let want = if f < RAMP_FRAMES {
                    // gain = f * scale, both single-rounded; exact here because scale is 2^-7
                    clamp(routed * (f as f32 * SCALE))
                } else {
                    clamp(routed) // the second half of the block is not ramped
                };
                let at = BLOCK + f * interleave::FRAME_BYTES + slot_of(p);
                assert_eq!(g.f32(at).unwrap(), want, "frame {f}, plane {p}");
            }
        }
    }

    #[test]
    fn the_clamp_leaves_an_in_range_word_and_a_nan_alone() {
        let mut g = guest(false);
        g.set_u32(SRC_PLANES + 1024 + 4 * 7, 0x7FC0_1234).unwrap(); // plane 1, frame 7: a NaN
        mix_and_clamp(&mut g, OBJECT, SP).unwrap();
        let at = BLOCK + 7 * interleave::FRAME_BYTES + slot_of(1);
        assert!(
            g.f32(at).unwrap().is_nan(),
            "both compares fail, so no store"
        );
    }

    #[test]
    fn the_wrapper_opens_its_frame_below_and_returns_one() {
        let mut g = guest(false);
        assert_eq!(output_pass(&mut g, OBJECT, SP).unwrap(), 1);
        let pass_frame = SP - PASS_FRAME_BYTES;
        let mix_frame = pass_frame - MIX_FRAME_BYTES;
        assert_eq!(g.u32(pass_frame).unwrap(), SP, "the wrapper's back chain");
        assert_eq!(
            g.u32(mix_frame).unwrap(),
            pass_frame,
            "the pass's, 96 bytes lower"
        );
        assert_eq!(
            g.u32(mix_frame + SRC_ARRAY).unwrap(),
            SRC_PLANES,
            "arrays in the lower frame"
        );
        assert_eq!(g.u32(mix_frame + DST_ARRAY + 4).unwrap(), DST_PLANES + 1024);
    }

    #[test]
    fn more_than_eight_outputs_is_refused() {
        let mut g = guest(false);
        g.set_u8(CHANNEL_COUNT_BYTE, 9).unwrap();
        assert!(
            mix_and_clamp(&mut g, OBJECT, SP).is_err(),
            "the scatter-mixer refuses nine"
        );
    }

    // ---------------------------------------------------------------------------- the ramp

    fn ramp_guest(channels: u8, fill: f32) -> Guest {
        let mut g = guest(false);
        g.set_u8(CHANNEL_COUNT_BYTE, channels).unwrap();
        for i in 0..(RAMP_FRAMES * u32::from(channels) + 8) {
            g.set_u32(BLOCK + i * 4, fill.to_bits()).unwrap();
        }
        g
    }

    #[test]
    fn the_ramp_scales_frame_k_by_k_times_the_scale_on_both_loops() {
        // Five channels reach the four-wide loop and the remainder loop, which compute the gain
        // separately and must agree.
        let mut g = ramp_guest(5, 2.0);
        let end = ramp_block(&mut g, u64::from(BLOCK)).unwrap();
        assert_eq!(
            end,
            u64::from(BLOCK + RAMP_FRAMES * 5 * 4),
            "r3 is the end of frame 127"
        );
        for k in [0u32, 1, 2, 63, 127] {
            for c in 0..5u32 {
                let want = 2.0 * (k as f32 * SCALE);
                assert_eq!(
                    g.f32(BLOCK + (k * 5 + c) * 4).unwrap(),
                    want,
                    "frame {k}, channel {c}"
                );
            }
        }
        // And the word after the 128th frame is not touched.
        assert_eq!(g.f32(BLOCK + RAMP_FRAMES * 5 * 4).unwrap(), 2.0);
    }

    #[test]
    fn an_empty_ramp_returns_r3_untouched() {
        let mut g = ramp_guest(0, 2.0);
        let r3 = 0xFFFF_FFFF_0000_0000 | u64::from(BLOCK);
        assert_eq!(
            ramp_block(&mut g, r3).unwrap(),
            r3,
            "bgelr before r3 is written"
        );
        assert_eq!(g.f32(BLOCK).unwrap(), 2.0);
    }

    #[test]
    fn the_addresses_come_from_the_lis_immediates() {
        assert_eq!(CHANNEL_COUNT_BYTE, 0x8306_0000 + 28765);
        assert_eq!(RAMP_FLAG_BYTE, 0x8306_0000 + 28759);
        assert_eq!(PAIR_TABLE, 0x820F_0000 - 10548);
        assert_eq!(
            OP_TABLE - PAIR_TABLE,
            16,
            "eight two-byte ranges, then the route bytes"
        );
        assert_eq!(
            CLAMP_LOW,
            crate::dsp::clip::FLOOR_SCALE,
            "the clipper's -1.0, reached again"
        );
        assert_eq!(CLAMP_TRIPS, 1536);
    }
}
