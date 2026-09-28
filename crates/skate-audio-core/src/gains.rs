//! The gain plumbing: the two bodies that drive the DSP kernels over a channel descriptor.
//!
//! Both take a descriptor — `+4` channel 0's float array, `+14` a `u16` count of singles between one
//! channel and the next — walk its channels, and hand each one to a kernel that
//! [`crate::dsp`] already has. They are the layer between [`crate::spatial`], which decides *what*
//! gain a speaker gets, and the kernels, which apply it to samples.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play | kernel |
//! |---|---|---|---|---|---|---|
//! | [`apply_gain_matrix`] | `sub_82B29AF0` | verified | 143 | 245,762 | 404,782 | [`crate::dsp::scale`] |
//! | [`ramp_channels`] | `sub_82B23B50` | verified | 119 | 292,165 | 435,555 | [`crate::dsp::gain_ramp`] |
//! | [`ramp_gain_matrix`] | `sub_82B298E0` | verified | 319 | 142,508 | 180,524 | [`crate::dsp::gain_ramp`], both kernels |
//!
//! All three are replayed against recorded gameplay — 795 + 185 + 404 vectors, 0 disagreements.
//! `sub_82B298E0`'s delta frame is its own stack, so a call with no source rows reads bytes no
//! recording holds; the replay counts those unreplayable rather than seeding them (there were none).
//!
//! `sub_82B298E0` joined the first two later, and is [`apply_gain_matrix`] with a ramp: the same matrix,
//! the same two passes, the same carried destination count, but every gain moves from the value the
//! caller saved before this block to the one the matrix holds now, over the kernels' 64 samples. It is
//! the one body here that **does** reproduce its own frame, because its deltas live there and are read
//! back by `lfsx`; see [`ramp_gain_matrix`].
//!
//! Both `.inc` headers lead with `// STATUS: verified` and `docs/ports.md` agrees. They are here
//! rather than in `dsp/` because neither is a kernel: they contain no sample arithmetic at all, only
//! loop bounds, gain-cursor arithmetic and channel addresses.
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green. The C++
//! bodies were compared call-for-call against the original under the shadow harness on real inputs
//! at zero divergence; the Rust has no recorded vectors of its own.
//!
//! ## The three arithmetic rules these two exist to get right
//!
//! Neither function does anything interesting with floats — one `lfsu` per gain, one `fsubs` and one
//! `fmuls` for a ramp step. What they do is **compute addresses**, and `CLAUDE.md` names all three
//! traps:
//!
//! 1. `mullw` is a **64-bit** product of two sign-extended words, so `stride * channel` can exceed
//!    32 bits before anything truncates it;
//! 2. `rlwinm rX,rX,2,0,29` is a 32-bit shift left by two of the product's **low word** — the
//!    rotate's wrapped bits are masked off;
//! 3. `add` is 64-bit on two zero-extended words, so a channel address can **carry into bit 32**,
//!    and only the callee's own address truncation drops it. Keeping the chain in 64 bits and
//!    casting at the call is the rule that surfaced on the 120th call of another function.
//!
//! [`channel_address`] is that chain, written once and used by both. `crate::mix`'s `row_offset` is
//! the same `mullw`/`rlwinm` pair with the sum taken in 32 bits, because its caller adds in 32 bits;
//! the two are deliberately not shared.
//!
//! ## Reloads reproduced rather than hoisted
//!
//! Every loop bound and every descriptor field is re-read from guest memory on each pass:
//! `sub_82B29AF0` reloads the destination count after **every** kernel call and both descriptors'
//! base and stride on every iteration; `sub_82B23B50` re-reads the applied gain, both strides and
//! both buffers per channel. Both C++ `Windows()` builders decline a call whose output covers any of
//! those words, so a caller whose buffers overlap its own control block is input the harness has
//! never compared. The reloads are kept because the originals have them; what that establishes is
//! that they survived the transcription, not that the guest agrees on such a layout.
//!
//! One reload is load-bearing rather than defensive, and it is worth reading twice.
//! `sub_82B29AF0`'s destination count lives in **one** variable across both passes: the second pass
//! tests whatever the *last* reload of the first pass left there, not a fresh read. If the count was
//! zero at entry the first pass never runs, the variable is still the entry value, and the inner loop
//! stays skipped for every source. Modelling it as two variables would be wrong.
//!
//! ## What no test here can catch
//!
//! Four of these reproductions are invisible to every test in the crate. Each was measured — the
//! change made, all 275 tests re-run, all still green — not argued:
//!
//! - **The `int32_t` casts in [`channel_offset`]'s `mullw`.** Zero-extending both operands instead
//!   gives the identical answer *always*, not merely on these inputs: only the product's low word
//!   survives the `rlwinm`, and a signed and an unsigned multiply agree on every low word. So the
//!   64-bitness of `mullw` cannot be observed through this function at all. It is written signed
//!   because the instruction is, and the rule it protects lives one line down, in the **add**, which
//!   `a_channel_address_is_a_64_bit_sum_of_a_32_bit_offset` does pin.
//! - **[`ramp_channels`] re-reads the applied gain on every iteration.** Nothing inside the loop
//!   writes that word, so hoisting the load is invisible. What is *not* invisible is moving the
//!   tail's latch into the loop, which `every_channel_ramps_from_the_same_starting_gain` catches —
//!   that is the mistranscription this reload actually guards against.
//! - **[`apply_gain_matrix`] carries one destination count across both passes.** The C++ note is
//!   emphatic that "modelling that as two variables would be wrong", and it is right about the
//!   transcription — but replacing pass 2's test with a fresh read leaves the suite green, and the
//!   reason is stronger than "these inputs do not reach it": pass 1's last act before exiting is to
//!   reload that word, and nothing writes it between then and pass 2's test, so the carried value
//!   and a fresh read **cannot** differ on any input. Kept as the original has it.
//! - **[`apply_gain_matrix`] re-reads both descriptor fields on every iteration.** Hoisting them out
//!   of the inner loop is invisible, for the same reason the C++ `Windows()` declines a call whose
//!   output covers them: no comparable input makes them change mid-call.
//!
//! ## What is not reproduced
//!
//! `sub_82B29AF0`'s `stwu r1,-160(r1)`. The C++ **does** reproduce it, because both kernels spill
//! `f1` and their callee-saved registers below the live `r1` and read that scratch back. The Rust
//! kernels keep those values in registers instead — `crate::dsp::scale`'s own note says so — so
//! nothing in this chain writes the guest stack and there is nothing to make room for. Same position
//! `crate::dsp::biquad` is in with its own frame.

use crate::dsp::gain_ramp::{self, AccumulateClobbers, GainRampClobbers};
use crate::dsp::scale;
use crate::vmx::Fpscr;
use crate::{Guest, Result, fp};

// ------------------------------------------------------------------------- the shared descriptor

/// `lwz r10,4(r30)` — channel 0's float array. The same offset [`crate::mix::ROW_BASE`] names on the
/// mixer-side object, which is the same structure seen from another caller.
pub const BUFFER_BASE: u32 = 4;
/// `lhz r11,14(r30)` — `u16` singles between one channel and the next. Not bytes: it is scaled by
/// four. Matches [`crate::mix::ROW_STRIDE`].
pub const CHANNEL_STRIDE: u32 = 14;
/// 16 — the span both C++ `Windows()` builders alias-check, so the whole descriptor.
pub const DESCRIPTOR_BYTES: u32 = 16;

const _: () = assert!(
    BUFFER_BASE == crate::mix::ROW_BASE,
    "the same descriptor mix.rs walks"
);
const _: () = assert!(CHANNEL_STRIDE == crate::mix::ROW_STRIDE);

/// `rlwinm rX,rX,2,0,29` applied to `mullw stride,index`: the byte offset of one channel.
///
/// Two rules in three lines. The product is 64-bit on **sign-extended** words; only its low word is
/// then shifted left by two, and the mask is what makes that a shift rather than a rotate.
pub fn channel_offset(stride: u32, index: u32) -> u32 {
    let elements = (stride as i32 as i64).wrapping_mul(index as i32 as i64); // mullw
    ((elements as u64 as u32) << 2) & 0xFFFF_FFFC // rlwinm rX,rX,2,0,29
}

/// `add rD,rOffset,rBuffer` on two zero-extended words: **64-bit**, so the sum can carry into bit
/// 32.
///
/// Returned as a `u64` on purpose. Every caller here passes it to a kernel whose `r3` is a full
/// 64-bit register, and the truncation to a guest address happens inside that kernel. A chain
/// truncated to 32 bits leaves memory byte-identical and the register wrong, which is exactly the
/// failure `CLAUDE.md` records surfacing on the 120th call of one function.
pub fn channel_address(stride: u32, index: u32, buffer: u32) -> u64 {
    u64::from(channel_offset(stride, index)) + u64::from(buffer)
}

// ================================================================ sub_82B29AF0: the gain matrix

/// `f32[source][dest]` — row `s` at `+444 + 32*s`, column `d` at `+4*d`.
///
/// `addi r29,r3,440` with `lfsu`'s `+4` pre-increment is what puts `gain[0][0]` at `+444`, and
/// [`GAIN_CURSOR_BASE`] is the 440 the port carries so the pre-increment is not folded away.
pub const GAIN_MATRIX: u32 = 444;
/// The cursor the `lfsu` chain starts from: `GAIN_MATRIX - 4`.
pub const GAIN_CURSOR_BASE: u32 = GAIN_MATRIX - 4;
/// `addi r24,r24,32` — eight floats per source row, so at most eight destinations are addressable.
pub const GAIN_ROW_STRIDE: u32 = 32;
/// `u32` source channel count. Only a value above 1 reaches the accumulating pass.
pub const SOURCE_COUNT: u32 = 748;
/// `u32` destination channel count. **Reloaded after every kernel call**, and carried across both
/// passes in one variable — see the module note.
pub const DEST_COUNT: u32 = 752;
/// `li r6,256` at both call sites. Not a parameter: the kernels are called with a literal count.
pub const SAMPLES: u32 = 256;

const _: () = assert!(GAIN_CURSOR_BASE == 440, "addi r29,r3,440");
const _: () = assert!(
    GAIN_MATRIX + GAIN_ROW_STRIDE == 476,
    "addi r24,r28,476 for row 1"
);

/// `sub_82B29AF0` — apply the channel gain matrix.
///
/// `mixer` is `r3`, `dst_desc` is `r4` (the **destination** descriptor) and `src_desc` is `r5` (the
/// **source** one). No result (`kReturnNone`). The direction is not a guess: the kernels take
/// `r3 = dst`, and this function builds their `r3` out of `r4`'s fields.
///
/// Two passes, and the split is the whole point:
///
/// - pass 1 walks destinations `d` and calls [`crate::dsp::scale::scale`] — `dst[k] = src[k]·gain` —
///   with source channel 0, so it **overwrites**;
/// - pass 2 walks sources `s = 1..` and destinations `d` again and calls
///   [`crate::dsp::scale::scale_accumulate`] — `dst[k] += src[k]·gain`.
///
/// Overwrite-then-accumulate is why source 0 gets a different kernel, and why a matrix mix needs no
/// separate clear pass.
///
/// **Writes** exactly the destination channels, 1024 bytes each: both kernels write `[r3, r3 + 4·256)`
/// and nothing else. Pass 2 revisits the same destinations, so one sweep names the whole write set.
/// **Reads** the two counts, the gain matrix, both descriptors, the source channels, and — because
/// the accumulating kernel reads its accumulator — the destination channels too.
///
/// The gains come off one `lfsu` cursor per pass, never an indexed load: pass 1 walks `+444` upward
/// through row 0, and pass 2 restarts the cursor at `row - 4` for each source. A port that indexed
/// the matrix instead would agree on every well-formed mixer and differ the moment a count changed
/// mid-call, which is precisely what the reload makes possible.
pub fn apply_gain_matrix(g: &mut Guest, mixer: u32, dst_desc: u32, src_desc: u32) -> Result<()> {
    let mut fpscr = Fpscr::capture();

    // r11 — one variable across both passes. See the module note; this is not two reads.
    let mut dest_count = g.u32(mixer + DEST_COUNT)?; // lwz r11,752(r3)
    let mut source = u64::from(g.u32(src_desc + BUFFER_BASE)?); // lwz r27,4(r5)

    // cmplwi cr6,r11,0 ; beq cr6 — pass 1: source channel 0 over every destination.
    if dest_count != 0 {
        let mut gain = mixer.wrapping_add(GAIN_CURSOR_BASE); // addi r29,r3,440
        let mut dest: u32 = 0; // li r31,0
        loop {
            // loc_82B29B20. Both descriptor fields are re-read every iteration.
            let stride = u32::from(g.u16(dst_desc + CHANNEL_STRIDE)?); // lhz r11,14(r30)
            let buffer = g.u32(dst_desc + BUFFER_BASE)?; // lwz r10,4(r30)
            fpscr.disable_flush_mode_unconditional(); // emitted at the lfsu
            gain = gain.wrapping_add(4); // lfsu f1,4(r29) updates the address first
            let k = fp::load_single(g, gain)?;
            let destination = channel_address(stride, dest, buffer);
            // bl 0x82b3bed8 — dst[i] = src[i]·k over 256 floats. The guest also hands the kernel
            // whatever the previous call left in r5; both kernels write r5 before reading it, so
            // that chain is invisible here and the Rust signature has no slot for it.
            scale::scale(g, destination as u32, source as u32, SAMPLES, k)?;
            dest_count = g.u32(mixer + DEST_COUNT)?; // lwz r11,752(r28) — after the call
            dest = dest.wrapping_add(1); // addi r31,r31,1
            if dest >= dest_count {
                break; // cmplw cr6,r31,r11 ; blt cr6
            }
        }
    }

    // loc_82B29B54.
    let mut source_count = g.u32(mixer + SOURCE_COUNT)?; // lwz r10,748(r28)
    // cmplwi cr6,r10,1 ; ble cr6 — one source channel means pass 1 was all of it.
    if source_count > 1 {
        let mut row = mixer.wrapping_add(GAIN_MATRIX + GAIN_ROW_STRIDE); // addi r24,r28,476
        let mut src: u32 = 1; // li r26,1
        loop {
            // loc_82B29B68.
            let stride = u32::from(g.u16(src_desc + CHANNEL_STRIDE)?); // lhz r10,14(r25)
            let buffer = g.u32(src_desc + BUFFER_BASE)?; // lwz r9,4(r25)
            source = channel_address(stride, src, buffer); // add r27,r10,r9
            // cmplwi cr6,r11,0 ; beq cr6 — the *carried* count, not a fresh read.
            if dest_count != 0 {
                let mut gain = row.wrapping_sub(4); // addi r29,r24,-4
                let mut dest: u32 = 0; // li r31,0
                loop {
                    // loc_82B29B8C.
                    let dst_stride = u32::from(g.u16(dst_desc + CHANNEL_STRIDE)?); // lhz r11,14(r30)
                    let dst_buffer = g.u32(dst_desc + BUFFER_BASE)?; // lwz r10,4(r30)
                    fpscr.disable_flush_mode_unconditional();
                    gain = gain.wrapping_add(4); // lfsu f1,4(r29)
                    let k = fp::load_single(g, gain)?;
                    let destination = channel_address(dst_stride, dest, dst_buffer);
                    // bl 0x82b44b20 — dst[i] += src[i]·k over 256 floats.
                    scale::scale_accumulate(g, destination as u32, source as u32, SAMPLES, k)?;
                    dest_count = g.u32(mixer + DEST_COUNT)?; // lwz r11,752(r28)
                    dest = dest.wrapping_add(1);
                    if dest >= dest_count {
                        break;
                    }
                }
            }
            // loc_82B29BC0.
            source_count = g.u32(mixer + SOURCE_COUNT)?; // lwz r10,748(r28)
            src = src.wrapping_add(1); // addi r26,r26,1
            row = row.wrapping_add(GAIN_ROW_STRIDE); // addi r24,r24,32
            if src >= source_count {
                break; // cmplw cr6,r26,r10 ; blt cr6
            }
        }
    }
    Ok(())
}

// ============================================================== sub_82B23B50: the per-channel ramp

/// `lbz r27,42(r30)` — `u8` channel count, the loop trip count. The same offset
/// [`crate::mix::CHANNELS`] names.
pub const CHANNEL_COUNT: u32 = 42;
/// `lfs f0,52(r3)` — the target gain, latched into [`APPLIED_GAIN`] on the way out.
pub const TARGET_GAIN: u32 = 52;
/// `lfs f13,56(r30)` — the gain actually applied to the previous block. **Re-read every
/// iteration** and stored twice per call on the restart path.
pub const APPLIED_GAIN: u32 = 56;
/// `lwz r29,28(r4)` — the descriptor read from this block. Swapped with [`DEST_DESC`] on exit.
pub const SOURCE_DESC: u32 = 28;
/// `lwz r28,32(r4)` — the descriptor written this block.
pub const DEST_DESC: u32 = 32;

const _: () = assert!(
    CHANNEL_COUNT == crate::mix::CHANNELS,
    "the same count mix.rs reloads"
);
const _: () = assert!(SOURCE_DESC == crate::mix::PAIR_BACK && DEST_DESC == crate::mix::PAIR_FRONT);

// The per-sample step scale, a rodata single. `((lis_imm & 0xFFFF) << 16) + offsets`, computed:
// lis r11,-32208 -> 0x82300000 ; addi r10,r11,-31232 -> 0x822F8600 ; lfs f0,480(r10).
const LIS_82300000: u32 = ((-32208i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82300000 == 0x8230_0000, "lis -32208");

/// `lfs f0,480(r10)` — **measured 0.015625, i.e. 1/64**, which is what makes this a ramp over 64
/// samples: `crate::dsp::gain_ramp`'s own `RAMP_SPAN` cell measures 64.0 and its ramp covers the
/// first 64 of the block's 256 singles. Read live through the [`Guest`] map anyway.
pub const STEP_SCALE: u32 = LIS_82300000.wrapping_add(-31232i32 as u32) + 480;
const _: () = assert!(STEP_SCALE == 0x822F_87E0);
const _: () = assert!(
    STEP_SCALE == crate::spatial::POOL + 480,
    "the same pool, 480 in"
);

/// What [`ramp_channels`] leaves behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RampResult {
    /// `li r3,1` — the guest's own result, and the mask is `kReturnR3`, so it **is** compared. It is
    /// unconditionally 1: the `li` sits in the tail that every path reaches.
    pub r3: u64,
    /// The `v28`-`v31` clobber of the **last** `sub_82B3C098` call, or `None` when the channel count
    /// was zero and no call was made.
    ///
    /// This function does not save those registers either — it uses `__savegprlr_26`, not
    /// `__savevmx_*` — so the callee's clobber reaches its own caller, and the harness compares
    /// `v28`-`v31` on every call regardless of the result mask. Dropping it would be a divergence on
    /// 292,165 calls a session.
    pub vector_clobbers: Option<GainRampClobbers>,
}

/// `sub_82B23B50` — ramp every channel of a double-buffered block, then swap the pair.
///
/// `object` is `r3` (the gain object), `pair` is `r4` (the descriptor pair) and `restart` is `r5`, of
/// which **only the low byte is read** (`clrlwi r11,r5,24`). A non-zero flag means "no ramp this
/// block": the applied gain is latched to the target up front, so the step below comes out zero.
/// The result is `r3 = 1` and the mask is `kReturnR3`.
///
/// **Writes** the applied gain at `+56` (twice on the restart path, once otherwise), the two swapped
/// descriptor pointers at `pair + 28`/`+32`, and one 1024-byte block per channel through
/// [`crate::dsp::gain_ramp::gain_ramp_copy`]. **Reads** the channel count, both gains, both
/// descriptors, [`STEP_SCALE`], one 1024-byte source block per channel, and — through the kernel,
/// which the C++ `Windows()` does not declare — the thirteen rodata cells
/// `crate::dsp::gain_ramp` names.
///
/// `step = (target − applied) · 1/64`, as `fsubs` then `fmuls`, **never fused**; it goes into `f2`
/// once and is reused for every channel, while `f1` is re-read from `+56` on each iteration. The
/// kernel clobbers `f0` and `f8`-`f13` but never `f1`/`f2`, so the original's reuse is sound. That is
/// the original's own assumption and it is reproduced rather than hardened.
///
/// One consequence of the latch worth stating: because `applied` is re-read inside the loop and only
/// written *after* it, every channel ramps from the same starting gain. A reader expecting the ramp
/// to accumulate across channels would be wrong.
pub fn ramp_channels(g: &mut Guest, object: u32, pair: u32, restart: u64) -> Result<RampResult> {
    let mut fpscr = Fpscr::capture();

    let restart = (restart & 0xFF) as u32; // clrlwi r11,r5,24
    let source = g.u32(pair + SOURCE_DESC)?; // lwz r29,28(r4)
    let dest = g.u32(pair + DEST_DESC)?; // lwz r28,32(r4)

    // cmplwi cr6,r11,0 ; beq cr6
    if restart != 0 {
        fpscr.disable_flush_mode_unconditional();
        let target = fp::load_single(g, object + TARGET_GAIN)?; // lfs f0,52(r3)
        fp::store_single(g, object + APPLIED_GAIN, target)?; // stfs f0,56(r3)
    }

    // loc_82B23B80: the ramp step, computed once for the whole loop.
    fpscr.disable_flush_mode_unconditional();
    let target = fp::load_single(g, object + TARGET_GAIN)?; // lfs f0,52(r30)
    let applied = fp::load_single(g, object + APPLIED_GAIN)?; // lfs f13,56(r30)
    let channels = u32::from(g.u8(object + CHANNEL_COUNT)?); // lbz r27,42(r30)
    let span = fp::sub_single(target, applied); // fsubs f12,f0,f13
    let scale = fp::load_single(g, STEP_SCALE)?; // lfs f0,480(r10)
    let step = fp::mul_single(span, scale); // fmuls f2,f12,f0 — f2, set once

    // cmplwi cr6,r27,0 ; beq cr6 — the count is pre-tested, so the do-while is exactly this loop.
    let mut vector_clobbers = None;
    for channel in 0..channels {
        // loc_82B23BAC.
        let source_stride = u32::from(g.u16(source + CHANNEL_STRIDE)?); // lhz r11,14(r29)
        fpscr.disable_flush_mode_unconditional();
        let gain = fp::load_single(g, object + APPLIED_GAIN)?; // lfs f1,56(r30) — re-read
        let dest_stride = u32::from(g.u16(dest + CHANNEL_STRIDE)?); // lhz r9,14(r28)
        let source_buffer = g.u32(source + BUFFER_BASE)?; // lwz r10,4(r29)
        let dest_buffer = g.u32(dest + BUFFER_BASE)?; // lwz r8,4(r28)
        // mullw ; rlwinm ; add — 64-bit end to end, truncated only where the callee truncates.
        let source_run = channel_address(source_stride, channel, source_buffer);
        let dest_run = channel_address(dest_stride, channel, dest_buffer);
        // bl 0x82b3c098 — one fixed 1024-byte block, gain-ramped.
        vector_clobbers = Some(gain_ramp::gain_ramp_copy(
            g,
            dest_run as u32,
            source_run as u32,
            gain,
            step,
        )?);
    }

    // loc_82B23BE8: swap the pair, then latch the gain. Both words are read before either store.
    let swapped_source = g.u32(pair + DEST_DESC)?; // lwz r11,32(r26)
    let r3: u64 = 1; // li r3,1 — between the two loads, as lifted
    let swapped_dest = g.u32(pair + SOURCE_DESC)?; // lwz r10,28(r26)
    g.set_u32(pair + SOURCE_DESC, swapped_source)?; // stw r11,28(r26)
    g.set_u32(pair + DEST_DESC, swapped_dest)?; // stw r10,32(r26)
    fpscr.disable_flush_mode_unconditional();
    let target = fp::load_single(g, object + TARGET_GAIN)?; // lfs f0,52(r30)
    fp::store_single(g, object + APPLIED_GAIN, target)?; // stfs f0,56(r30)

    Ok(RampResult {
        r3,
        vector_clobbers,
    })
}

// ============================================================ sub_82B298E0: the ramped matrix

/// `stwu r1,-432(r1)` — [`ramp_gain_matrix`]'s own frame, and **reproduced**, because the deltas live
/// in it and are read back out of it by `lfsx`.
pub const RAMP_FRAME_BYTES: u32 = 432;
/// `addi r8,r1,80` — the per-destination deltas, `f32[source][8]`, 32 bytes per source row.
pub const RAMP_DELTAS: u32 = 80;
/// `(432 - 80) / 32` — the rows of deltas the frame can hold. The matrix is 8×8, so a source count past
/// 8 is out of the matrix long before it is out of the frame.
pub const RAMP_FRAME_ROWS: u32 = (RAMP_FRAME_BYTES - RAMP_DELTAS) / GAIN_ROW_STRIDE;
const _: () = assert!(RAMP_FRAME_ROWS == 11 && RAMP_FRAME_ROWS >= 8);

/// `lis r31,-32208 ; addi r31,r31,-31232 ; lfs f0,480(r31)` — measured 0.015625, **1/64**: the delta
/// is the whole gain change spread over the kernels' 64-sample ramp. Computed from the immediates and
/// asserted to be the very cell [`STEP_SCALE`] names for `sub_82B23B50`, reached there from different
/// lifted lines.
pub const MATRIX_RAMP_STEP: u32 = LIS_82300000.wrapping_add(-31232i32 as u32) + 480;
const _: () = assert!(MATRIX_RAMP_STEP == 0x822F_87E0 && MATRIX_RAMP_STEP == STEP_SCALE);

/// The vector registers [`ramp_gain_matrix`] leaves clobbered: whatever its **last** kernel call left.
///
/// `sub_82B298E0` saves none of `v14`-`v31` (`__savegprlr_22`, no `__savevmx_*`), so each callee's
/// clobber reaches its caller, and the harness compares that range on every call. Pass 2 always runs
/// after pass 1, so if any accumulate ran, the last call was one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastKernelClobbers {
    /// No kernel ran — a zero destination count. `v14`-`v31` are the caller's own.
    None,
    /// The last call was `sub_82B3C098`: `v28`-`v31` are its; `v14`-`v27` are the caller's.
    Copy(GainRampClobbers),
    /// The last call was `sub_82B44D18`: every one of `v14`-`v31` is its.
    Accumulate(AccumulateClobbers),
}

/// `sub_82B298E0` — apply the channel gain matrix **with a ramp**: each gain moves from the value the
/// caller saved to the value the matrix holds now, over the kernels' 64-sample ramp.
///
/// `mixer` is `r3` (the same object [`apply_gain_matrix`] takes), `dst_desc` is `r4`, `src_desc` is
/// `r5`, `saved` is `r6` — the caller's copy of the matrix as it stood before this block, same
/// layout, which `sub_82B29BE0` hands in as its own `r1 + 96` — and `sp` is the entry `r1`. No
/// result: the mask is `kReturnNone`. What it returns is the vector clobber ([`LastKernelClobbers`]).
///
/// It is [`apply_gain_matrix`] with a ramp, and the same shape:
///
/// 1. if `+748` is **signed**-positive, one row of eight deltas per source row,
///    `delta[s][c] = (live[s][c] − saved[s][c]) · 1/64`, stored into this function's own frame;
/// 2. source channel 0 into every destination through [`gain_ramp::gain_ramp_copy`], which
///    **overwrites**, starting at `saved[0][d]` and ramping by `delta[0][d]`;
/// 3. sources `1..` into every destination through [`gain_ramp::gain_ramp_accumulate`], from
///    `saved[s][d]` by `delta[s][d]`.
///
/// **Writes**: the back chain at `sp − 432`; the deltas at `sp − 352 + 32·s + 4·c` for every source row
/// the delta loop visits and **all eight** columns, whatever the destination count; and, per
/// destination channel, `[dst & !127, (dst & !0xF) + 1024)` through the copy (its `dcbzl` rounds the
/// start down) and `[dst & !0xF, +1024)` through the accumulate. **Reads**: both counts, the live
/// matrix rows the delta loop visits, the saved matrix, both descriptors, [`MATRIX_RAMP_STEP`], the
/// source channels, the destination channels (the accumulate reads its accumulator), the thirteen
/// cells the kernels read, and the deltas back out of the frame.
///
/// Reproduced rather than fixed, each as the lifted body has it:
///
/// - **The delta loop's test is signed and the pass bounds are unsigned.** `cmpwi r10,0 ; ble` skips
///   the deltas for any count with bit 31 set, while `cmplwi r10,1 ; ble` then sends that same count
///   into pass 2.
/// - **With no delta rows, pass 1 still runs and reads the frame as it found it.** A source count of
///   zero and a non-zero destination count skip step 1 and run step 2, whose `lfsx` loads deltas this
///   call never wrote. The original reads whatever the stack held; so does this, because the deltas are
///   guest memory here and not locals.
/// - **Past eight destinations the cursors walk into the next row** — the saved-gain cursor into the
///   next saved row, the delta cursor into the next delta row or past the written ones. The matrix has
///   eight columns and nothing bounds the destination count against it.
/// - **The deltas' store addresses are `row + (r1 + 80 + 4c − r6)`**, built by six `subf` on 64-bit
///   registers and truncated only at the store. That equals `sp − 352 + 32·s + 4·c` modulo 2^32 for
///   every `r6`, and it is written the lifted way below; the store order (columns 0, 2, 3, 5, 4, 6, 7,
///   1) and the differencing order (2, 3, 0, 5, 4, 6, 7, 1) are kept too.
/// - **The source base is read once, before pass 1**, and not per destination; pass 2 re-reads the
///   source descriptor for every source row. The destination count is reloaded after every kernel
///   call and carried across both passes in one variable, exactly as in [`apply_gain_matrix`].
///
/// Not reproduced: `__savegprlr_22`'s spill of `r22`-`r31` and the link register into
/// `[sp − 88, sp)`, and the kernels' red-zone scratch below `sp − 432` — none of it is read back, and
/// this crate has no register file. The kernels here keep their splats in registers.
pub fn ramp_gain_matrix(
    g: &mut Guest,
    mixer: u32,
    dst_desc: u32,
    src_desc: u32,
    saved: u32,
    sp: u32,
) -> Result<LastKernelClobbers> {
    // stwu r1,-432(r1): the back chain, then r1 moves.
    let frame = sp.wrapping_sub(RAMP_FRAME_BYTES);
    g.set_u32(frame, sp)?;
    let deltas = frame.wrapping_add(RAMP_DELTAS); // r1 + 80
    let mut fpscr = Fpscr::capture();

    let source_first = g.u32(mixer.wrapping_add(SOURCE_COUNT))?; // lwz r10,748(r3)

    // cmpwi cr6,r10,0 ; ble cr6,0x82b29a00 — a SIGNED test.
    if (source_first as i32) > 0 {
        // The six store offsets, `subf rN,r25,rN`: each a frame slot minus the saved matrix's base, so
        // that adding the saved-row cursor lands on the frame. Wrapping, as the stores truncate.
        let r8 = deltas.wrapping_sub(saved); // addi r8,r1,80 ; subf r8,r25,r8
        let r7 = deltas.wrapping_add(4).wrapping_sub(saved); // addi r7,r1,84
        let r6 = deltas.wrapping_add(8).wrapping_sub(saved); // addi r6,r1,88
        let r5 = deltas.wrapping_add(12).wrapping_sub(saved); // addi r5,r1,92
        let r4 = deltas.wrapping_add(16).wrapping_sub(saved); // addi r4,r1,96
        let r3 = deltas.wrapping_add(20).wrapping_sub(saved); // addi r3,r1,100
        fpscr.disable_flush_mode_unconditional(); // emitted at lfs f0,480(r31)
        let step = fp::load_single(g, MATRIX_RAMP_STEP)?; // lfs f0,480(r31)
        let mut r9 = deltas.wrapping_add(4).wrapping_sub(GAIN_ROW_STRIDE); // addi r9,r11,-32, r11 = r1+84
        let mut row = saved.wrapping_add(8); // addi r11,r25,8
        let mut live = mixer.wrapping_add(GAIN_CURSOR_BASE); // addi r10,r27,440
        let mut trips = source_first; // mtctr r10
        loop {
            // loc_82B29958. All sixteen loads precede all eight stores.
            fpscr.disable_flush_mode_unconditional(); // emitted at the first lfs
            let live2 = fp::load_single(g, live.wrapping_add(12))?; // lfs f11,12(r10)
            let saved2 = fp::load_single(g, row)?; // lfs f5,0(r11)
            let live3 = fp::load_single(g, live.wrapping_add(16))?; // lfs f10,16(r10)
            let d2 = fp::sub_single(live2, saved2); // fsubs f2,f11,f5
            let saved3 = fp::load_single(g, row.wrapping_add(4))?; // lfs f3,4(r11)
            let saved0 = fp::load_single(g, row.wrapping_sub(8))?; // lfs f6,-8(r11)
            let d3 = fp::sub_single(live3, saved3); // fsubs f11,f10,f3
            let live0 = fp::load_single(g, live.wrapping_add(4))?; // lfs f13,4(r10)
            let live5 = fp::load_single(g, live.wrapping_add(24))?; // lfs f8,24(r10)
            let d0 = fp::sub_single(live0, saved0); // fsubs f4,f13,f6
            let saved5 = fp::load_single(g, row.wrapping_add(12))?; // lfs f10,12(r11)
            let live1 = fp::load_single(g, live.wrapping_add(8))?; // lfs f12,8(r10)
            let d5 = fp::sub_single(live5, saved5); // fsubs f5,f8,f10
            let live6 = fp::load_single(g, live.wrapping_add(28))?; // lfs f7,28(r10)
            let saved4 = fp::load_single(g, row.wrapping_add(8))?; // lfs f1,8(r11)
            let live4 = fp::load_single(g, live.wrapping_add(20))?; // lfs f9,20(r10)
            let saved6 = fp::load_single(g, row.wrapping_add(16))?; // lfs f6,16(r11)
            let d4 = fp::sub_single(live4, saved4); // fsubs f9,f9,f1
            let saved7 = fp::load_single(g, row.wrapping_add(20))?; // lfs f3,20(r11)
            let d6 = fp::sub_single(live6, saved6); // fsubs f1,f7,f6
            live = live.wrapping_add(GAIN_ROW_STRIDE); // lfsu f13,32(r10) updates the address first
            let live7 = fp::load_single(g, live)?;
            let saved1 = fp::load_single(g, row.wrapping_sub(4))?; // lfs f10,-4(r11)
            let d7 = fp::sub_single(live7, saved7); // fsubs f8,f13,f3
            let d1 = fp::sub_single(live1, saved1); // fsubs f7,f12,f10

            fp::store_single(g, r9.wrapping_add(28), fp::mul_single(d0, step))?; // fmuls f6,f4,f0 ; stfs f6,28(r9)
            fp::store_single(g, row.wrapping_add(r8), fp::mul_single(d2, step))?; // fmuls f4,f2,f0 ; stfsx f4,r11,r8
            fp::store_single(g, row.wrapping_add(r7), fp::mul_single(d3, step))?; // fmuls f3,f11,f0 ; stfsx f3,r11,r7
            fp::store_single(g, row.wrapping_add(r5), fp::mul_single(d5, step))?; // fmuls f13,f5,f0 ; stfsx f13,r11,r5
            fp::store_single(g, row.wrapping_add(r6), fp::mul_single(d4, step))?; // fmuls f2,f9,f0 ; stfsx f2,r11,r6
            fp::store_single(g, row.wrapping_add(r4), fp::mul_single(d6, step))?; // fmuls f12,f1,f0 ; stfsx f12,r11,r4
            fp::store_single(g, row.wrapping_add(r3), fp::mul_single(d7, step))?; // fmuls f11,f8,f0 ; stfsx f11,r11,r3
            r9 = r9.wrapping_add(GAIN_ROW_STRIDE); // fmuls f10,f7,f0 ; stfsu f10,32(r9)
            fp::store_single(g, r9, fp::mul_single(d1, step))?;
            row = row.wrapping_add(GAIN_ROW_STRIDE); // addi r11,r11,32

            trips = trips.wrapping_sub(1); // bdnz 0x82b29958
            if trips == 0 {
                break;
            }
        }
    }

    // loc_82B29A00. One destination count across both passes, as in apply_gain_matrix.
    let mut dest_count = g.u32(mixer.wrapping_add(DEST_COUNT))?; // lwz r11,752(r27)
    let source = g.u32(src_desc.wrapping_add(BUFFER_BASE))?; // lwz r28,4(r22) — read once, here
    // addi r11,r1,80 ; subf r29,r25,r11 — the offset from a gain slot to its delta slot.
    let delta_offset = deltas.wrapping_sub(saved);
    let mut last = LastKernelClobbers::None;

    // cmplwi cr6,r11,0 ; beq cr6,0x82b29a58 — pass 1: source channel 0 over every destination.
    if dest_count != 0 {
        let mut gain = saved; // mr r30,r25
        let mut dest: u32 = 0; // li r31,0
        loop {
            // loc_82B29A20. Both descriptor fields re-read every iteration.
            let stride = u32::from(g.u16(dst_desc.wrapping_add(CHANNEL_STRIDE))?); // lhz r10,14(r26)
            let buffer = g.u32(dst_desc.wrapping_add(BUFFER_BASE))?; // lwz r11,4(r26)
            fpscr.disable_flush_mode_unconditional(); // emitted at the lfsx
            let slope = fp::load_single(g, gain.wrapping_add(delta_offset))?; // lfsx f2,r30,r29
            let start = fp::load_single(g, gain)?; // lfs f1,0(r30)
            let destination = channel_address(stride, dest, buffer); // mullw ; rlwinm ; add r3
            // bl 0x82b3c098 — dst = src * gain(k), gain ramping from f1 by f2. r4 is the entry source.
            last = LastKernelClobbers::Copy(gain_ramp::gain_ramp_copy(
                g,
                destination as u32,
                source,
                start,
                slope,
            )?);
            dest_count = g.u32(mixer.wrapping_add(DEST_COUNT))?; // lwz r11,752(r27) — after the call
            dest = dest.wrapping_add(1); // addi r31,r31,1
            gain = gain.wrapping_add(4); // addi r30,r30,4
            if dest >= dest_count {
                break; // cmplw cr6,r31,r11 ; blt cr6
            }
        }
    }

    // loc_82B29A58.
    let mut source_count = g.u32(mixer.wrapping_add(SOURCE_COUNT))?; // lwz r10,748(r27)
    // cmplwi cr6,r10,1 ; ble cr6,0x82b29ae4 — UNSIGNED, unlike the delta loop's test.
    if source_count > 1 {
        let mut row = saved.wrapping_add(GAIN_ROW_STRIDE); // addi r23,r25,32
        let mut src: u32 = 1; // li r24,1
        loop {
            // loc_82B29A6C. The source descriptor is re-read for every source row.
            let stride = u32::from(g.u16(src_desc.wrapping_add(CHANNEL_STRIDE))?); // lhz r9,14(r22)
            let buffer = g.u32(src_desc.wrapping_add(BUFFER_BASE))?; // lwz r10,4(r22)
            let channel = channel_address(stride, src, buffer); // mullw r8,r9,r24 ; rlwinm ; add r28
            // cmplwi cr6,r11,0 ; beq cr6 — the carried count, not a fresh read.
            if dest_count != 0 {
                let mut gain = row; // mr r30,r23
                let mut dest: u32 = 0; // li r31,0
                loop {
                    // loc_82B29A98.
                    let dst_stride = u32::from(g.u16(dst_desc.wrapping_add(CHANNEL_STRIDE))?); // lhz r10,14(r26)
                    let dst_buffer = g.u32(dst_desc.wrapping_add(BUFFER_BASE))?; // lwz r11,4(r26)
                    fpscr.disable_flush_mode_unconditional(); // emitted at the lfsx
                    let slope = fp::load_single(g, gain.wrapping_add(delta_offset))?; // lfsx f2,r30,r29
                    let start = fp::load_single(g, gain)?; // lfs f1,0(r30)
                    let destination = channel_address(dst_stride, dest, dst_buffer);
                    // bl 0x82b44d18 — dst += src * gain(k).
                    last = LastKernelClobbers::Accumulate(gain_ramp::gain_ramp_accumulate(
                        g,
                        destination as u32,
                        channel as u32,
                        start,
                        slope,
                    )?);
                    dest_count = g.u32(mixer.wrapping_add(DEST_COUNT))?; // lwz r11,752(r27)
                    dest = dest.wrapping_add(1); // addi r31,r31,1
                    gain = gain.wrapping_add(4); // addi r30,r30,4
                    if dest >= dest_count {
                        break; // cmplw cr6,r31,r11 ; blt cr6
                    }
                }
            }
            // loc_82B29AD0.
            source_count = g.u32(mixer.wrapping_add(SOURCE_COUNT))?; // lwz r10,748(r27)
            src = src.wrapping_add(1); // addi r24,r24,1
            row = row.wrapping_add(GAIN_ROW_STRIDE); // addi r23,r23,32
            if src >= source_count {
                break; // cmplw cr6,r24,r10 ; blt cr6
            }
        }
    }

    // loc_82B29AE4: addi r1,r1,432 ; b __restgprlr_22.
    Ok(last)
}

// ================================================== sub_82B29BE0: republishing the spatial mix

/// `lfs f31,52(r3)` — the first of the ten live placement parameters, 8 bytes apart.
pub const LIVE_FIRST: u32 = 52;
/// The stride between live parameters.
pub const LIVE_STRIDE: u32 = 8;
/// Ten live parameters, and ten cached copies.
pub const LIVE_COUNT: u32 = 10;
/// `addi r3,r31,128` — the configuration [`crate::spatial::fill_mix_matrix`] takes.
pub const MIX_CONFIG: u32 = 128;
/// `addi r30,r3,316` — eight panner entries.
pub const MIX_ENTRIES: u32 = 316;
/// `addi r10,r31,444` — the 8x8 gain matrix.
pub const MIX_MATRIX: u32 = 444;
/// `stfs f30,700(r31)` — the seventh live parameter, stored on the recompute path.
pub const MIX_COMPARED: u32 = 700;
/// The ten cached parameters, 4 bytes apart.
pub const MIX_CACHED: u32 = 704;
/// `lfs f4,744(r31)` — the matrix fill's gain.
pub const MIX_TAIL: u32 = 744;
/// `lwz r28,28(r4)` — the descriptor this call mixes from.
pub const REPUBLISH_PAIR_BACK: u32 = 28;
/// `lwz r27,32(r4)` — the descriptor it mixes into.
pub const REPUBLISH_PAIR_FRONT: u32 = 32;
/// `stwu r1,-496(r1)`. Real: the saved matrix at `r1+96` is handed to [`ramp_gain_matrix`].
pub const REPUBLISH_FRAME_BYTES: u32 = 496;
/// `addi r6,r1,96` — the saved matrix, 32 bytes a source row.
pub const REPUBLISH_SAVED: u32 = 96;
/// The deepest stack the call reaches: its own frame and [`ramp_gain_matrix`]'s below it.
pub const REPUBLISH_STACK_DEPTH: u32 = REPUBLISH_FRAME_BYTES + RAMP_FRAME_BYTES;

/// Recompute the panner entries, then the matrix from them. The source count is reloaded for the
/// matrix, after the layout's stores.
fn recompute_placement<T: crate::mathlib::Trig>(
    g: &mut Guest,
    trig: &mut T,
    mixer: u32,
    count: u32,
    live: &[f64; LIVE_COUNT as usize],
    frame: u32,
) -> Result<()> {
    let layout = crate::spatial::PannerLayout {
        angle: live[0],
        distance: live[1],
        radius: live[2],
        turn: live[3],
        spreads: [live[7], live[8], live[9]],
    };
    let entries = mixer.wrapping_add(MIX_ENTRIES);
    crate::spatial::lay_out_panners(g, trig, entries, count as i32, layout, frame)?; // bl 0x82b45c50
    let tail = fp::load_single(g, mixer.wrapping_add(MIX_TAIL))?; // lfs f4,744(r31)
    let sources = g.u32(mixer.wrapping_add(SOURCE_COUNT))? as i32; // lwz r5,748(r31)
    let gains = crate::spatial::MatrixGains {
        weight: live[5],
        focus: live[4],
        fill: live[6],
        gain: tail,
    };
    let config = mixer.wrapping_add(MIX_CONFIG);
    let matrix = mixer.wrapping_add(MIX_MATRIX);
    crate::spatial::fill_mix_matrix(g, trig, config, entries, sources, matrix, gains) // bl 0x82b460a0
}

/// Republish a source's spatial mix (`sub_82B29BE0`). Returns 1.
///
/// `mixer` is `r3`, `pair` the two-descriptor pair in `r4`, `flag` `r5` (only its low byte is
/// tested) and `sp` `r1`. The ten live placement parameters are compared with their cached copies,
/// stopping at the first that differs; a NaN always differs.
///
/// - **Nothing moved:** the panners and the matrix are recomputed only if the flag asks, then the
///   hard mix [`apply_gain_matrix`] runs. Nothing is cached.
/// - **Something moved:** the seventh parameter is stored at +700, the matrix is saved into the
///   frame row by row (every row's eight loads before its eight stores, for as many rows as the
///   source count says, eight or not), the panners and matrix are recomputed, and the flag picks
///   the hard mix or [`ramp_gain_matrix`] out of the saved matrix. Then the ten parameters are cached.
///
/// Either way the pair is swapped, both words **reloaded** first.
pub fn republish_mix<T: crate::mathlib::Trig>(
    g: &mut Guest,
    trig: &mut T,
    mixer: u32,
    pair: u32,
    flag: u32,
    sp: u32,
) -> Result<u64> {
    let frame = sp.wrapping_sub(REPUBLISH_FRAME_BYTES);
    g.set_u32(frame, sp)?; // stwu r1,-496(r1)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let mut live = [0.0f64; LIVE_COUNT as usize];
    for (i, value) in live.iter_mut().enumerate() {
        *value = fp::load_single(g, mixer.wrapping_add(LIVE_FIRST + LIVE_STRIDE * i as u32))?;
    }
    let back = g.u32(pair.wrapping_add(REPUBLISH_PAIR_BACK))?; // lwz r28,28(r4)
    let front = g.u32(pair.wrapping_add(REPUBLISH_PAIR_FRONT))?; // lwz r27,32(r4)
    let mut unchanged = true;
    for (i, value) in live.iter().enumerate() {
        if *value != fp::load_single(g, mixer.wrapping_add(MIX_CACHED + 4 * i as u32))? {
            unchanged = false; // fcmpu ; bne -- the first difference ends the compare
            break;
        }
    }

    if unchanged {
        if flag & 0xFF != 0 {
            let count = g.u32(mixer.wrapping_add(SOURCE_COUNT))?; // lwz r4,748(r3)
            recompute_placement(g, trig, mixer, count, &live, frame)?;
        }
        apply_gain_matrix(g, mixer, front, back)?; // loc_82B29D04: bl 0x82b29af0
    } else {
        // loc_82B29D18
        let count = g.u32(mixer.wrapping_add(SOURCE_COUNT))?; // lwz r4,748(r31)
        fp::store_single(g, mixer.wrapping_add(MIX_COMPARED), live[6])?; // stfs f30,700(r31)
        if count as i32 > 0 {
            let mut src = mixer.wrapping_add(MIX_MATRIX).wrapping_sub(4); // addi r11,r31,440
            let mut dst = frame.wrapping_add(REPUBLISH_SAVED).wrapping_sub(4); // addi r10,r1,92
            for _ in 0..count {
                let mut row = [0.0f64; 8];
                for (k, word) in row.iter_mut().take(7).enumerate() {
                    *word = fp::load_single(g, src.wrapping_add(4 + 4 * k as u32))?; // lfs 4..28(r11)
                }
                src = src.wrapping_add(32); // lfsu f0,32(r11)
                row[7] = fp::load_single(g, src)?;
                for (k, word) in row.iter().take(7).enumerate() {
                    fp::store_single(g, dst.wrapping_add(4 + 4 * k as u32), *word)?; // stfs 4..28(r10)
                }
                dst = dst.wrapping_add(32); // stfsu f0,32(r10)
                fp::store_single(g, dst, row[7])?;
            }
        }
        recompute_placement(g, trig, mixer, count, &live, frame)?;
        if flag & 0xFF != 0 {
            apply_gain_matrix(g, mixer, front, back)?; // bl 0x82b29af0
        } else {
            let saved = frame.wrapping_add(REPUBLISH_SAVED); // addi r6,r1,96
            ramp_gain_matrix(g, mixer, front, back, saved, frame)?; // bl 0x82b298e0
        }
        fpscr.disable_flush_mode_unconditional();
        for (i, value) in live.iter().enumerate() {
            fp::store_single(g, mixer.wrapping_add(MIX_CACHED + 4 * i as u32), *value)?; // stfs 704..740
        }
    }

    // loc_82B29E18: both words reloaded, then swapped.
    let swap_front = g.u32(pair.wrapping_add(REPUBLISH_PAIR_FRONT))?; // lwz r11,32(r26)
    let swap_back = g.u32(pair.wrapping_add(REPUBLISH_PAIR_BACK))?; // lwz r10,28(r26)
    g.set_u32(pair.wrapping_add(REPUBLISH_PAIR_BACK), swap_front)?; // stw r11,28(r26)
    g.set_u32(pair.wrapping_add(REPUBLISH_PAIR_FRONT), swap_back)?; // stw r10,32(r26)
    Ok(1) // li r3,1
}

// ====================================================== sub_82B238A8: advancing a voice's gain ramp

/// `lbz r11,41(r31)` — channels to scale, reloaded after every channel.
pub const VOICE_RAMP_CHANNELS: u32 = 0x29;
/// `stw r10,48(r31)` — stamped with `0x7FF7FFF1` on the way out.
pub const VOICE_RAMP_LAST_WORD: u32 = 0x30;
/// `stfs f0,52(r31)` — the block's last envelope sample.
pub const VOICE_RAMP_LAST_GAIN: u32 = 0x34;
/// `lfd f10,56(r3)` — latched into [`VOICE_RAMP_TIME`] on a start request.
pub const VOICE_RAMP_START_TIME: u32 = 0x38;
/// `lfs f0,64(r3)` — the requested length, in seconds.
pub const VOICE_RAMP_REQUEST_SPAN: u32 = 0x40;
/// `lfs f9,68(r3)` — the requested end value.
pub const VOICE_RAMP_REQUEST_EXTRA: u32 = 0x44;
/// `lwz r10,72(r3)` — the requested curve, narrowed to [`VOICE_RAMP_CURVE`].
pub const VOICE_RAMP_REQUEST_CURVE: u32 = 0x48;
/// `stfd f10,80(r3)` — the ramp's start time.
pub const VOICE_RAMP_TIME: u32 = 0x50;
/// `stfs f0,88(r3)` — a copy of the requested span.
pub const VOICE_RAMP_SPAN_COPY: u32 = 0x58;
/// `stw r9,92(r3)` — the ramp length in samples, at least 1.
pub const VOICE_RAMP_SAMPLES: u32 = 0x5C;
/// `stw r11,96(r31)` — samples produced so far; starts at or below zero.
pub const VOICE_RAMP_CURSOR: u32 = 0x60;
/// `stfs f11,100(r3)` — the gain the ramp starts from.
pub const VOICE_RAMP_FROM: u32 = 0x64;
/// `stfs f9,104(r3)` — the gain it ends at.
pub const VOICE_RAMP_EXTRA: u32 = 0x68;
/// `lfs f11,108(r3)` — the gain now.
pub const VOICE_RAMP_CURRENT_GAIN: u32 = 0x6C;
/// `lbz r10,112(r3)` — 1 when a start request is waiting.
pub const VOICE_RAMP_START_PENDING: u32 = 0x70;
/// `lbz r11,113(r31)` — 0 steady, 1 armed, 2 running.
pub const VOICE_RAMP_STATE: u32 = 0x71;
/// `lbz r11,114(r31)` — 0 linear, 1 square root, anything else sine.
pub const VOICE_RAMP_CURVE: u32 = 0x72;
/// The mix descriptor's `+16`: the time base subtracted from the ramp's start time.
pub const MIX_DESC_TIME_BASE: u32 = 0x10;
/// `+28`: the channel-buffer descriptor.
pub const MIX_DESC_CHANNELS: u32 = 0x1C;
/// `+32`: the mixer, whose `+4` is the envelope buffer.
pub const MIX_DESC_MIXER: u32 = 0x20;
/// `+40`: the clock, whose `+12` is samples per second.
pub const MIX_DESC_CLOCK: u32 = 0x28;
/// `lfd f0,256(r10)` — the double the ramp time is compared against.
pub const VOICE_RAMP_TIME_SENTINEL: u32 = crate::leaves::SENTINEL_DOUBLE;
/// `lfs f0,-22460(r11)` — the gain a steady voice needs no envelope for.
pub const VOICE_RAMP_GAIN_SENTINEL: u32 = crate::routing::UNITY_GAIN;

/// Advance a voice's gain ramp for one block and scale all its channels by it (`sub_82B238A8`).
/// Returns 1.
///
/// `state` is `r3` and `desc` the mix descriptor in `r4`. A pending start request latches the ramp's
/// parameters and arms it, its length `fctiwz(span * rate)` floored at one sample. An armed ramp is
/// placed from the clock — a cursor of `-(rate * (time - base))`, or dropped if it already finished —
/// and runs. A running ramp has [`crate::dsp::ramps`]'s curve writer fill the block's 256 envelope
/// samples and advances the cursor a block; otherwise a gain other than the sentinel fills the envelope
/// flat, reloading the gain before every store. Then every channel is multiplied by the envelope, 16
/// samples to a vector with the envelope the first operand, and the last envelope sample becomes the
/// gain now.
pub fn advance_gain_ramp<T: crate::mathlib::Trig>(
    g: &mut Guest,
    trig: &mut T,
    state: u32,
    desc: u32,
) -> Result<u64> {
    use crate::fp::{fctiwz_low_word, load_single, mul_single, store_single};
    use core::arch::x86_64::_mm_mul_ps;
    let at = |offset: u32| state.wrapping_add(offset);
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let clock = g.u32(desc.wrapping_add(MIX_DESC_CLOCK))?; // lwz r11,40(r4)
    let rate = load_single(g, clock.wrapping_add(12))?; // lfs f12,12(r11) -- unconditional

    if g.u8(at(VOICE_RAMP_START_PENDING))? == 1 {
        let span = load_single(g, at(VOICE_RAMP_REQUEST_SPAN))?;
        let curve = g.u32(at(VOICE_RAMP_REQUEST_CURVE))?;
        let samples_f = mul_single(span, rate); // fmuls f13,f0,f12
        let gain = load_single(g, at(VOICE_RAMP_CURRENT_GAIN))?;
        let start_time = g.u64(at(VOICE_RAMP_START_TIME))?; // lfd f10,56(r3)
        let extra = load_single(g, at(VOICE_RAMP_REQUEST_EXTRA))?;
        store_single(g, at(VOICE_RAMP_FROM), gain)?;
        g.set_u64(at(VOICE_RAMP_TIME), start_time)?; // stfd f10,80(r3)
        g.set_u8(at(VOICE_RAMP_CURVE), curve as u8)?;
        store_single(g, at(VOICE_RAMP_SPAN_COPY), span)?;
        store_single(g, at(VOICE_RAMP_EXTRA), extra)?;
        let samples = fctiwz_low_word(samples_f);
        g.set_u32(at(VOICE_RAMP_SAMPLES), samples)?;
        if samples as i32 <= 0 {
            g.set_u32(at(VOICE_RAMP_SAMPLES), 1)?;
        }
        g.set_u8(at(VOICE_RAMP_STATE), 1)?;
        g.set_u8(at(VOICE_RAMP_START_PENDING), 0)?;
    }

    if g.u8(at(VOICE_RAMP_STATE))? == 1 {
        fpscr.disable_flush_mode_unconditional();
        let when = f64::from_bits(g.u64(at(VOICE_RAMP_TIME))?); // lfd f13,80(r31)
        let sentinel = f64::from_bits(g.u64(VOICE_RAMP_TIME_SENTINEL)?); // lfd f0,256(r10)
        let mut position = sentinel; // f0 survives when the two are equal
        if when != sentinel {
            let base_time = f64::from_bits(g.u64(desc.wrapping_add(MIX_DESC_TIME_BASE))?);
            position = when - base_time; // fsub f0,f13,f0 -- a double, not rounded
        }
        fpscr.disable_flush_mode_unconditional();
        let raw = fctiwz_low_word(rate * position); // fmul f0,f12,f0 ; fctiwz
        if (raw as i32) < 256 {
            let samples = g.u32(at(VOICE_RAMP_SAMPLES))?;
            let cursor = raw.wrapping_neg(); // neg r11,r11
            g.set_u32(at(VOICE_RAMP_CURSOR), cursor)?;
            if cursor as i32 > samples.wrapping_sub(1) as i32 {
                g.set_u8(at(VOICE_RAMP_STATE), 0)?; // already finished
            } else {
                if cursor as i32 > 0 {
                    g.set_u32(at(VOICE_RAMP_CURSOR), 0)?;
                }
                g.set_u8(at(VOICE_RAMP_STATE), 2)?;
            }
        }
    }

    let mixer = g.u32(desc.wrapping_add(MIX_DESC_MIXER))?; // lwz r10,32(r4)
    let ramp_state = g.u8(at(VOICE_RAMP_STATE))?;
    let channels = g.u32(desc.wrapping_add(MIX_DESC_CHANNELS))?; // lwz r30,28(r4)
    let envelope = g.u32(mixer.wrapping_add(4))?; // lwz r28,4(r10)
    if ramp_state != 0 && ramp_state != 1 {
        let curve = g.u8(at(VOICE_RAMP_CURVE))?;
        fpscr.disable_flush_mode_unconditional();
        let end = load_single(g, at(VOICE_RAMP_EXTRA))?; // lfs f2,104(r31)
        let start = load_single(g, at(VOICE_RAMP_FROM))?; // lfs f1,100(r31)
        let samples = g.u32(at(VOICE_RAMP_SAMPLES))? as i32;
        let cursor = g.u32(at(VOICE_RAMP_CURSOR))? as i32;
        match curve {
            0 => crate::dsp::ramps::linear_ramp(g, envelope, cursor, samples, start, end)?,
            1 => crate::dsp::ramps::sqrt_ramp(g, envelope, cursor, samples, start, end)?,
            _ => crate::dsp::ramps::sine_ramp(g, trig, envelope, cursor, samples, start, end)?,
        };
        let advanced = g.u32(at(VOICE_RAMP_CURSOR))?.wrapping_add(256); // reloaded
        let samples_now = g.u32(at(VOICE_RAMP_SAMPLES))?;
        g.set_u32(at(VOICE_RAMP_CURSOR), advanced)?;
        if advanced as i32 >= samples_now as i32 {
            g.set_u8(at(VOICE_RAMP_STATE), 0)?;
        }
    } else {
        fpscr.disable_flush_mode_unconditional();
        let gain = load_single(g, at(VOICE_RAMP_CURRENT_GAIN))?;
        let sentinel = load_single(g, VOICE_RAMP_GAIN_SENTINEL)?;
        if gain == sentinel {
            return Ok(1); // beq -- straight out, not one store
        }
        let mut cursor = envelope.wrapping_sub(4); // addi r11,r28,-4
        for _ in 0..32 {
            fpscr.disable_flush_mode_unconditional();
            for slot in (4..=32).step_by(4) {
                let held = load_single(g, at(VOICE_RAMP_CURRENT_GAIN))?; // reloaded every store
                store_single(g, cursor.wrapping_add(slot), held)?;
            }
            cursor = cursor.wrapping_add(32);
        }
    }

    let mut count = g.u8(at(VOICE_RAMP_CHANNELS))?; // lbz r11,41(r31)
    if count != 0 {
        if !crate::vmx::supported() {
            return Err(crate::vmx::unsupported());
        }
        let row = envelope.wrapping_add(48); // addi r29,r28,48
        let mut channel = 0u32;
        loop {
            let stride = u32::from(g.u16(channels.wrapping_add(14))?); // reloaded per channel
            let first = g.u32(channels.wrapping_add(4))?;
            let index = (stride as i32).wrapping_mul(channel as i32) as u32; // mullw
            let start = first.wrapping_add(index << 2);
            let mut dst = start.wrapping_add(32);
            let bias = envelope.wrapping_sub(start); // subf r9,r9,r28
            let mut src = row;
            // SAFETY: vmx support was checked above; guest accesses are bounds-checked.
            unsafe {
                for _ in 0..16 {
                    let chan0 = crate::vmx::lvx128_ps(g, dst.wrapping_sub(32))?;
                    let env0 = crate::vmx::lvx128_ps(g, src.wrapping_sub(48))?;
                    fpscr.enable_flush_mode_unconditional();
                    crate::vmx::stvx128_ps(g, dst.wrapping_sub(32), _mm_mul_ps(env0, chan0))?;
                    let chan1 = crate::vmx::lvx128_ps(g, dst.wrapping_sub(16))?;
                    let env1 = crate::vmx::lvx128_ps(g, src.wrapping_sub(32))?;
                    crate::vmx::stvx128_ps(g, dst.wrapping_sub(16), _mm_mul_ps(env1, chan1))?;
                    let env2 = crate::vmx::lvx128_ps(g, bias.wrapping_add(dst))?;
                    let chan2 = crate::vmx::lvx128_ps(g, dst)?;
                    crate::vmx::stvx128_ps(g, dst, _mm_mul_ps(env2, chan2))?;
                    let env3 = crate::vmx::lvx128_ps(g, src)?;
                    src = src.wrapping_add(64);
                    let chan3 = crate::vmx::lvx128_ps(g, dst.wrapping_add(16))?;
                    crate::vmx::stvx128_ps(g, dst.wrapping_add(16), _mm_mul_ps(env3, chan3))?;
                    dst = dst.wrapping_add(64);
                }
            }
            count = g.u8(at(VOICE_RAMP_CHANNELS))?; // reloaded each pass
            channel += 1;
            if channel >= u32::from(count) {
                break;
            }
        }
    }

    fpscr.disable_flush_mode_unconditional();
    let last = load_single(g, envelope.wrapping_add(1020))?; // lfs f0,1020(r28)
    store_single(g, at(VOICE_RAMP_CURRENT_GAIN), last)?;
    store_single(g, at(VOICE_RAMP_LAST_GAIN), last)?;
    g.set_u32(at(VOICE_RAMP_LAST_WORD), 0x7FF7_FFF1)?; // lis r11,32759 ; ori
    Ok(1) // li r3,1
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXER: u32 = 0x4000_0000;
    const DST_DESC: u32 = MIXER + 0x0400;
    const SRC_DESC: u32 = MIXER + 0x0410;
    const DST_BUF: u32 = MIXER + 0x1000;
    const SRC_BUF: u32 = MIXER + 0x9000;

    const RAMP_OBJ: u32 = MIXER + 0x0500;
    const PAIR: u32 = MIXER + 0x0600;
    const DESC_A: u32 = MIXER + 0x0700;
    const DESC_B: u32 = MIXER + 0x0710;

    /// A guest map with the work area plus every rodata cell the two kernels read: this module's own
    /// [`STEP_SCALE`] and the thirteen cells `crate::dsp::gain_ramp` names, all with the values read
    /// out of the validated image dump.
    ///
    /// The thirteen are listed here because **`sub_82B23B50`'s C++ `Windows()` does not declare any
    /// of them as reads** — it declares only its own `0x822F87E0`. A recorded vector for it is
    /// therefore missing the callee's constants, and replaying it would fail on the first cell rather
    /// than on a divergence.
    fn guest() -> Guest {
        use crate::dsp::gain_ramp as gr;
        let mut g = Guest::from_segments(vec![
            crate::Segment {
                base: MIXER,
                bytes: vec![0u8; 0x0002_0000],
            },
            crate::Segment {
                base: 0x8206_0000,
                bytes: vec![0u8; 0x4000],
            },
            crate::Segment {
                base: 0x8225_7000,
                bytes: vec![0u8; 0x1000],
            },
            crate::Segment {
                base: 0x820E_D000,
                bytes: vec![0u8; 0x1000],
            },
            crate::Segment {
                base: 0x8231_BA00,
                bytes: vec![0u8; 0x100],
            },
            crate::Segment {
                base: 0x822F_8600,
                bytes: vec![0u8; 0x1000],
            },
        ]);
        g.set_u32(STEP_SCALE, 0.015625f32.to_bits()).unwrap(); // measured 1/64
        g.set_u32(gr::STEP_SCALE, 4.0f32.to_bits()).unwrap();
        g.set_u32(gr::RAMP_SPAN, 64.0f32.to_bits()).unwrap();
        g.set_u32(gr::LANE2_SCALE, 2.0f32.to_bits()).unwrap();
        g.set_u32(gr::LANE3_SCALE, 3.0f32.to_bits()).unwrap();
        for group in 1..8u32 {
            splat(&mut g, gr::SCALE[group as usize], group as f32);
        }
        splat(&mut g, gr::SCALE_STEP, 8.0);
        g
    }

    fn splat(g: &mut Guest, base: u32, v: f32) {
        for i in 0..4 {
            g.set_u32(base + 4 * i, v.to_bits()).unwrap();
        }
    }

    fn fill(g: &mut Guest, base: u32, n: u32, v: f32) {
        for i in 0..n {
            g.set_u32(base + 4 * i, v.to_bits()).unwrap();
        }
    }

    fn at(g: &Guest, ea: u32) -> f32 {
        g.f32(ea).unwrap()
    }

    /// A descriptor: `+4` the buffer, `+14` the `u16` stride in singles.
    fn descriptor(g: &mut Guest, desc: u32, buffer: u32, stride: u16) {
        g.set_u32(desc + BUFFER_BASE, buffer).unwrap();
        g.set_u16(desc + CHANNEL_STRIDE, stride).unwrap();
    }

    fn gain(g: &mut Guest, source: u32, dest: u32, value: f32) {
        g.set_u32(
            MIXER + GAIN_MATRIX + GAIN_ROW_STRIDE * source + 4 * dest,
            value.to_bits(),
        )
        .unwrap();
    }

    // =========================================================== the address arithmetic

    #[test]
    fn a_channel_address_is_a_64_bit_sum_of_a_32_bit_offset() {
        // The offset is the low word of `stride*index`, scaled by four and masked: a multiple of 4,
        // always inside 32 bits.
        assert_eq!(channel_offset(256, 0), 0);
        assert_eq!(channel_offset(256, 1), 1024);
        assert_eq!(channel_offset(256, 3), 3072);
        // The `rlwinm` keeps only the low word, so a product that overflows 32 bits wraps rather
        // than carrying: stride 0x8000 by index 0x8000 is 2^30 words, i.e. 2^32 bytes, i.e. zero.
        assert_eq!(channel_offset(0x8000, 0x8000), 0);
        // And it is a shift, not a rotate. 0x8000 * 0x10001 has low word 0x80008000, whose top two
        // bits the mask discards: a rotate would bring them back in as 0x00020002.
        assert_eq!(channel_offset(0x8000, 0x1_0001), 0x0002_0000);

        // The sum is where the 64 bits are load-bearing. `CLAUDE.md`: a chain truncated to 32 bits
        // leaves memory byte-identical and the register wrong.
        let wide = channel_address(256, 1, 0xFFFF_FF00);
        assert_eq!(wide, 0x1_0000_0300, "the carry into bit 32 survives");
        assert_eq!(
            wide as u32, 0x0000_0300,
            "and the truncation is the callee's, not ours"
        );
        assert!(
            wide > u64::from(u32::MAX),
            "a u32 chain could not represent this at all"
        );
    }

    // =========================================================== sub_82B29AF0

    #[test]
    fn pass_one_overwrites_and_pass_two_accumulates() {
        // Two sources into two destinations. dst[d] must end up at src0*gain[0][d] + src1*gain[1][d],
        // and the pre-filled junk in dst proves the first pass *overwrote* rather than accumulated.
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 2).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 2).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, DST_BUF, 768, 7.0); // 256 words past the two channels, to catch an overrun
        fill(&mut g, SRC_BUF, 256, 1.0);
        fill(&mut g, SRC_BUF + 1024, 256, 2.0);
        gain(&mut g, 0, 0, 0.5);
        gain(&mut g, 0, 1, 0.25);
        gain(&mut g, 1, 0, 1.0);
        gain(&mut g, 1, 1, 2.0);

        apply_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC).unwrap();

        // 1*0.5 + 2*1.0 and 1*0.25 + 2*2.0.
        for i in 0..256u32 {
            assert_eq!(at(&g, DST_BUF + 4 * i), 2.5, "dest 0 sample {i}");
            assert_eq!(at(&g, DST_BUF + 1024 + 4 * i), 4.25, "dest 1 sample {i}");
        }
        assert_eq!(at(&g, DST_BUF + 2048), 7.0, "nothing past the two channels");
    }

    #[test]
    fn a_single_source_channel_never_reaches_the_accumulating_pass() {
        // `cmplwi r10,1 ; ble` — a count of exactly 1 skips pass 2 entirely, so row 1 of the matrix
        // must never be read. Poison it with a value that would be obvious in the output.
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 1).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 1).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, SRC_BUF, 256, 1.0);
        fill(&mut g, SRC_BUF + 1024, 256, 1000.0);
        gain(&mut g, 0, 0, 0.5);
        gain(&mut g, 1, 0, 1000.0);
        apply_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC).unwrap();
        assert_eq!(at(&g, DST_BUF), 0.5, "only source 0 and only gain[0][0]");
    }

    #[test]
    fn a_zero_destination_count_writes_nothing_at_all() {
        // Both passes are gated on it: pass 1 by `cmplwi r11,0 ; beq`, pass 2's inner loop by the
        // same test on the carried variable. A port that ran the inner loop unconditionally, or that
        // tested the source count first, would write here.
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 3).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 0).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, DST_BUF, 1024, 7.0);
        fill(&mut g, SRC_BUF, 1024, 1.0);
        // Every gain the two passes could reach is loud, so a single stray kernel call is visible.
        // With only gain[0][0] set the control "run pass 2's inner loop unconditionally" wrote an
        // accumulate of zero and this test passed; that is what a poisoned matrix is for.
        for s in 0..3u32 {
            for d in 0..2u32 {
                gain(&mut g, s, d, 5.0);
            }
        }
        apply_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC).unwrap();
        for i in 0..1024u32 {
            assert_eq!(at(&g, DST_BUF + 4 * i), 7.0, "word {i}");
        }
    }

    #[test]
    fn the_destination_count_is_reloaded_after_every_kernel_call() {
        // Point the destination buffer at the count word itself. The first kernel call writes zeros
        // over it, the reload reads 0, and the loop stops after one channel — where an entry-once
        // read of 4 would have written three more channels.
        //
        // The buffer is deliberately 4-byte rather than 128-byte aligned, so the kernel takes its
        // scalar path and writes exactly [dst, dst+1024): a vector-path `dcbzl` would round the
        // start down to a 128-byte line and also clear the source count, which is not what this test
        // is about. The C++ `Windows()` declines this layout, so nothing here establishes what the
        // guest does with it — only that the reload survived translation.
        let mut g = guest();
        let dst_buf = MIXER + DEST_COUNT;
        assert_ne!(dst_buf & 0x7F, 0, "must take the kernel's scalar path");
        g.set_u32(MIXER + SOURCE_COUNT, 1).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 4).unwrap();
        descriptor(&mut g, DST_DESC, dst_buf, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, SRC_BUF, 256, 0.0);
        fill(&mut g, dst_buf + 1024, 1024, 7.0);
        gain(&mut g, 0, 0, 0.0);
        gain(&mut g, 0, 1, 5.0);
        gain(&mut g, 0, 2, 5.0);
        gain(&mut g, 0, 3, 5.0);
        apply_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC).unwrap();
        assert_eq!(
            g.u32(MIXER + DEST_COUNT).unwrap(),
            0,
            "the first channel cleared the count"
        );
        for i in 0..1024u32 {
            assert_eq!(
                at(&g, dst_buf + 1024 + 4 * i),
                7.0,
                "channel 1 word {i} was written"
            );
        }
    }

    #[test]
    fn the_gain_cursor_walks_one_row_per_source_and_restarts_at_its_head() {
        // `addi r29,r3,440` with `lfsu`'s pre-increment puts gain[0][0] at +444, and pass 2 restarts
        // the cursor at `row - 4` for every source with `addi r24,r24,32` between them. Distinct
        // gains in every cell make a cursor that failed to restart, or that used the wrong stride,
        // produce the wrong sum.
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 3).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 2).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        for s in 0..3u32 {
            fill(&mut g, SRC_BUF + 1024 * s, 256, 1.0);
            for d in 0..2u32 {
                gain(&mut g, s, d, (1 + s * 2 + d) as f32); // 1,2 / 3,4 / 5,6
            }
        }
        apply_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC).unwrap();
        assert_eq!(at(&g, DST_BUF), 1.0 + 3.0 + 5.0, "column 0");
        assert_eq!(at(&g, DST_BUF + 1024), 2.0 + 4.0 + 6.0, "column 1");
        // And the row stride really is 32 bytes: the eight words of row 0 end where row 1 begins.
        assert_eq!(MIXER + GAIN_MATRIX + GAIN_ROW_STRIDE, MIXER + 476);
    }

    // =========================================================== sub_82B23B50

    /// The gain object and the descriptor pair, with `channels` channels of 256 singles each.
    fn ramp_layout(g: &mut Guest, channels: u8, target: f32, applied: f32) {
        g.set_u8(RAMP_OBJ + CHANNEL_COUNT, channels).unwrap();
        g.set_u32(RAMP_OBJ + TARGET_GAIN, target.to_bits()).unwrap();
        g.set_u32(RAMP_OBJ + APPLIED_GAIN, applied.to_bits())
            .unwrap();
        g.set_u32(PAIR + SOURCE_DESC, DESC_A).unwrap();
        g.set_u32(PAIR + DEST_DESC, DESC_B).unwrap();
        descriptor(g, DESC_A, MIXER + 0x1_0000, 256);
        descriptor(g, DESC_B, MIXER + 0x1_8000, 256);
    }

    #[test]
    fn it_ramps_every_channel_then_swaps_the_pair_and_latches_the_gain() {
        let mut g = guest();
        ramp_layout(&mut g, 2, 1.0, 0.0);
        let src = MIXER + 0x1_0000;
        let dst = MIXER + 0x1_8000;
        fill(&mut g, src, 512, 1.0);
        fill(&mut g, dst + 2048, 256, 7.0);

        let out = ramp_channels(&mut g, RAMP_OBJ, PAIR, 0).unwrap();
        assert_eq!(out.r3, 1, "li r3,1 on every path");
        assert!(
            out.vector_clobbers.is_some(),
            "two calls were made, so v28-v31 moved"
        );

        // step = (1.0 - 0.0)/64, so the kernel's gain is k/64 for the first 64 samples and then 1.0.
        for channel in 0..2u32 {
            let base = dst + 1024 * channel; // row 0 of the destination, then a channel of 256
            assert_eq!(
                at(&g, base),
                0.0,
                "channel {channel} sample 0 takes the applied gain"
            );
            assert_eq!(at(&g, base + 4), 1.0 / 64.0, "sample 1");
            assert_eq!(at(&g, base + 4 * 64), 1.0, "sample 64 holds at the target");
            assert_eq!(at(&g, base + 4 * 255), 1.0, "and so does the last");
        }
        assert_eq!(at(&g, dst + 2048), 7.0, "nothing past the two channels");

        // The pair is swapped and the gain latched.
        assert_eq!(g.u32(PAIR + SOURCE_DESC).unwrap(), DESC_B);
        assert_eq!(g.u32(PAIR + DEST_DESC).unwrap(), DESC_A);
        assert_eq!(at(&g, RAMP_OBJ + APPLIED_GAIN), 1.0, "applied := target");
    }

    #[test]
    fn every_channel_ramps_from_the_same_starting_gain() {
        // The applied gain is re-read inside the loop and only written after it, so channel 1 starts
        // where channel 0 did. A port that latched the gain before the loop, or inside it, would
        // give the second channel a flat copy.
        let mut g = guest();
        ramp_layout(&mut g, 2, 1.0, 0.0);
        let src = MIXER + 0x1_0000;
        let dst = MIXER + 0x1_8000;
        fill(&mut g, src, 512, 1.0);
        ramp_channels(&mut g, RAMP_OBJ, PAIR, 0).unwrap();
        assert_eq!(
            at(&g, dst),
            at(&g, dst + 1024),
            "both channels start at 0.0"
        );
        assert_eq!(at(&g, dst + 4), at(&g, dst + 1024 + 4));
    }

    #[test]
    fn the_restart_flag_latches_the_gain_first_so_the_step_comes_out_zero() {
        // Non-zero means "no ramp this block": applied := target before the step is computed, so the
        // span is zero and every sample takes the target gain.
        let mut g = guest();
        ramp_layout(&mut g, 1, 1.0, 0.0);
        let src = MIXER + 0x1_0000;
        let dst = MIXER + 0x1_8000;
        fill(&mut g, src, 256, 1.0);
        ramp_channels(&mut g, RAMP_OBJ, PAIR, 1).unwrap();
        for i in [0u32, 1, 32, 64, 255] {
            assert_eq!(at(&g, dst + 4 * i), 1.0, "sample {i} is flat at the target");
        }
    }

    #[test]
    fn only_the_low_byte_of_the_flag_is_read() {
        // `clrlwi r11,r5,24`. 0x100 has no low byte set, so it must behave exactly like 0 — and the
        // ramp has to be visible, which is why sample 0 is checked rather than the latch (the latch
        // happens on both paths).
        let mut g = guest();
        ramp_layout(&mut g, 1, 1.0, 0.0);
        let src = MIXER + 0x1_0000;
        let dst = MIXER + 0x1_8000;
        fill(&mut g, src, 256, 1.0);
        ramp_channels(&mut g, RAMP_OBJ, PAIR, 0x100).unwrap();
        assert_eq!(at(&g, dst), 0.0, "0x100 is a zero flag, so the ramp runs");
        assert_eq!(at(&g, dst + 4), 1.0 / 64.0);

        // And 0x101 is not.
        let mut h = guest();
        ramp_layout(&mut h, 1, 1.0, 0.0);
        fill(&mut h, src, 256, 1.0);
        ramp_channels(&mut h, RAMP_OBJ, PAIR, 0x101).unwrap();
        assert_eq!(at(&h, dst), 1.0, "0x101 does have a low byte");
    }

    #[test]
    fn a_zero_channel_count_still_swaps_the_pair_and_latches_the_gain() {
        // The tail is unconditional, and the clobber report has to say that no kernel call was made
        // rather than inventing a vector register state.
        let mut g = guest();
        ramp_layout(&mut g, 0, 0.75, 0.0);
        let dst = MIXER + 0x1_8000;
        fill(&mut g, dst, 256, 7.0);
        let out = ramp_channels(&mut g, RAMP_OBJ, PAIR, 0).unwrap();
        assert_eq!(out.r3, 1);
        assert_eq!(out.vector_clobbers, None, "no call, so nothing to report");
        assert_eq!(at(&g, dst), 7.0, "no channel was copied");
        assert_eq!(g.u32(PAIR + SOURCE_DESC).unwrap(), DESC_B);
        assert_eq!(at(&g, RAMP_OBJ + APPLIED_GAIN), 0.75);
    }

    #[test]
    fn the_step_scale_cell_is_read_live_and_is_one_sixty_fourth() {
        // Measured 0.015625, which is what makes the ramp span 64 samples — the same 64 the kernel's
        // own `RAMP_SPAN` cell holds. Patch the cell and the step follows it, because the body loads
        // it rather than folding it in.
        let mut g = guest();
        assert_eq!(at(&g, STEP_SCALE), 0.015625);
        ramp_layout(&mut g, 1, 1.0, 0.0);
        let src = MIXER + 0x1_0000;
        let dst = MIXER + 0x1_8000;
        fill(&mut g, src, 256, 1.0);
        g.set_u32(STEP_SCALE, 0.03125f32.to_bits()).unwrap(); // 1/32
        ramp_channels(&mut g, RAMP_OBJ, PAIR, 0).unwrap();
        assert_eq!(
            at(&g, dst + 4),
            1.0 / 32.0,
            "the step doubled with the cell"
        );
    }

    // =========================================================== sub_82B298E0

    const SAVED: u32 = MIXER + 0x0800;
    const SP: u32 = MIXER + 0x1_F000;
    const FRAME: u32 = SP - RAMP_FRAME_BYTES;

    fn saved_gain(g: &mut Guest, saved: u32, source: u32, dest: u32, value: f32) {
        g.set_u32(saved + GAIN_ROW_STRIDE * source + 4 * dest, value.to_bits())
            .unwrap();
    }

    fn delta_slot(source: u32, column: u32) -> u32 {
        FRAME + RAMP_DELTAS + GAIN_ROW_STRIDE * source + 4 * column
    }

    /// Every value below is dyadic with few significant bits, so every gain, every product and every
    /// sum the two kernels form is exact in single precision. That keeps these tests neutral on how a
    /// `vmaddfp` rounds, which is `crate::vmx`'s question and not this module's.
    #[test]
    fn the_ramp_starts_at_the_saved_gains_and_arrives_at_the_live_ones() {
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 2).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 2).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, DST_BUF, 768, 7.0); // a third channel's worth, to catch an overrun
        fill(&mut g, SRC_BUF, 256, 1.0);
        fill(&mut g, SRC_BUF + 1024, 256, 2.0);
        let saved = [[0.0f32, 0.5], [1.0, 0.25]];
        let live = [[1.0f32, 0.25], [1.0, 0.75]];
        for s in 0..2 {
            for d in 0..2 {
                saved_gain(&mut g, SAVED, s, d, saved[s as usize][d as usize]);
                gain(&mut g, s, d, live[s as usize][d as usize]);
            }
        }

        ramp_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap();

        let src = [1.0f64, 2.0];
        for d in 0..2usize {
            for k in 0..256usize {
                let t = k.min(64) as f64 / 64.0;
                let expected: f64 = (0..2)
                    .map(|s| src[s] * (saved[s][d] as f64 + t * (live[s][d] - saved[s][d]) as f64))
                    .sum();
                assert_eq!(
                    at(&g, DST_BUF + 1024 * d as u32 + 4 * k as u32),
                    expected as f32,
                    "dest {d} sample {k}"
                );
            }
        }
        assert_eq!(at(&g, DST_BUF + 2048), 7.0, "nothing past the two channels");
    }

    #[test]
    fn the_deltas_live_in_its_own_frame_behind_a_back_chain() {
        // Eight columns per visited row are written whatever the destination count, and nothing else in
        // the frame is touched.
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 2).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 1).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        for s in 0..2 {
            for c in 0..8 {
                saved_gain(&mut g, SAVED, s, c, 0.25 * c as f32);
                gain(&mut g, s, c, 0.25 * c as f32 + (1 + s + c) as f32);
            }
        }
        g.fill(FRAME, 0xA5, RAMP_FRAME_BYTES).unwrap();

        ramp_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap();

        assert_eq!(
            g.u32(FRAME).unwrap(),
            SP,
            "stwu r1,-432(r1) stores the entry r1"
        );
        for s in 0..2 {
            for c in 0..8 {
                let expected = (1 + s + c) as f32 / 64.0;
                assert_eq!(at(&g, delta_slot(s, c)), expected, "delta[{s}][{c}]");
            }
        }
        let written = |ea: u32| {
            (FRAME..FRAME + 4).contains(&ea) || (delta_slot(0, 0)..delta_slot(2, 0)).contains(&ea)
        };
        for ea in FRAME..SP {
            if !written(ea) {
                assert_eq!(
                    g.u8(ea).unwrap(),
                    0xA5,
                    "frame byte {:#x} was written",
                    ea - FRAME
                );
            }
        }
    }

    #[test]
    fn with_no_source_rows_pass_one_reads_whatever_the_frame_already_held() {
        // `cmpwi r10,0 ; ble` skips the delta loop for a source count of zero, but pass 1 is gated on
        // the destination count alone, so its lfsx reads a delta this call never wrote. Seed that slot
        // as a previous call might have left it: the output ramps by it.
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 0).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 1).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, SRC_BUF, 256, 1.0);
        saved_gain(&mut g, SAVED, 0, 0, 0.0);
        gain(&mut g, 0, 0, 5.0); // never read: no delta row is computed
        g.set_u32(delta_slot(0, 0), (1.0f32 / 64.0).to_bits())
            .unwrap();

        let last = ramp_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap();

        assert_eq!(at(&g, DST_BUF), 0.0);
        assert_eq!(
            at(&g, DST_BUF + 4),
            1.0 / 64.0,
            "ramped by the stale frame slot"
        );
        assert_eq!(at(&g, DST_BUF + 4 * 64), 1.0);
        assert_eq!(
            at(&g, delta_slot(0, 0)),
            1.0 / 64.0,
            "and the slot was not rewritten"
        );
        assert!(
            matches!(last, LastKernelClobbers::Copy(_)),
            "pass 1 ran; pass 2 did not"
        );
    }

    #[test]
    fn the_ramped_destination_count_is_reloaded_after_every_kernel_call() {
        // Destination channel 0 is laid over the mixer's own counts, so the first kernel call writes
        // zeros across both and the reload ends pass 1 after one channel; the source count's reload
        // then skips pass 2. An entry-once read of 4 would have written channels 1 to 3.
        let mut g = guest();
        let dst_desc = MIXER + 0x1800;
        let src_desc = MIXER + 0x1810;
        let saved = MIXER + 0x1900;
        let channel0 = MIXER + 256;
        assert!(channel0 <= MIXER + SOURCE_COUNT && MIXER + DEST_COUNT + 4 <= channel0 + 1024);
        g.set_u32(MIXER + SOURCE_COUNT, 1).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 4).unwrap();
        descriptor(&mut g, dst_desc, channel0, 256);
        descriptor(&mut g, src_desc, SRC_BUF, 256);
        fill(&mut g, SRC_BUF, 256, 0.0); // +0.0 * a non-negative gain is +0.0: the counts become 0
        fill(&mut g, channel0 + 1024, 3 * 256, 7.0);
        for d in 0..4 {
            saved_gain(&mut g, saved, 0, d, 0.5);
            gain(&mut g, 0, d, 0.5);
        }
        ramp_gain_matrix(&mut g, MIXER, dst_desc, src_desc, saved, SP).unwrap();
        assert_eq!(
            g.u32(MIXER + DEST_COUNT).unwrap(),
            0,
            "channel 0 cleared the count"
        );
        for i in 0..3 * 256u32 {
            assert_eq!(
                at(&g, channel0 + 1024 + 4 * i),
                7.0,
                "channels 1-3 word {i} was written"
            );
        }
    }

    #[test]
    fn pass_one_reads_the_source_base_once_before_its_loop() {
        // `lwz r28,4(r22)` sits before loc_82B29A20, so every destination of pass 1 mixes from the
        // base read at that moment. Here channel 0's output lands on the source descriptor and zeroes
        // its base: a port that re-read it per destination would mix channel 1 from address zero.
        // The C++ Windows() refuses this layout, so this pins the transcription, not the guest.
        let mut g = guest();
        let dst_desc = MIXER + 0x1800;
        let channel0 = MIXER + 0x2000;
        let src_desc = channel0 + 64;
        g.set_u32(MIXER + SOURCE_COUNT, 1).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 2).unwrap();
        descriptor(&mut g, dst_desc, channel0, 256);
        descriptor(&mut g, src_desc, SRC_BUF, 256);
        fill(&mut g, SRC_BUF, 256, 1.0);
        saved_gain(&mut g, SAVED, 0, 0, 0.0);
        saved_gain(&mut g, SAVED, 0, 1, 0.5);
        gain(&mut g, 0, 0, 0.0);
        gain(&mut g, 0, 1, 0.5);
        ramp_gain_matrix(&mut g, MIXER, dst_desc, src_desc, SAVED, SP).unwrap();
        assert_eq!(
            g.u32(src_desc + BUFFER_BASE).unwrap(),
            0,
            "channel 0 zeroed the base"
        );
        for i in 0..256u32 {
            assert_eq!(at(&g, channel0 + 1024 + 4 * i), 0.5, "channel 1 word {i}");
        }
    }

    #[test]
    fn a_single_source_row_never_reaches_the_accumulate() {
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 1).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 1).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        fill(&mut g, SRC_BUF, 256, 1.0);
        fill(&mut g, SRC_BUF + 1024, 256, 1000.0);
        saved_gain(&mut g, SAVED, 0, 0, 0.5);
        gain(&mut g, 0, 0, 0.5);
        saved_gain(&mut g, SAVED, 1, 0, 1000.0);
        gain(&mut g, 1, 0, 1000.0);
        let last = ramp_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap();
        for i in 0..256u32 {
            assert_eq!(
                at(&g, DST_BUF + 4 * i),
                0.5,
                "word {i}: row 0 and source 0 only"
            );
        }
        assert!(matches!(last, LastKernelClobbers::Copy(_)));
    }

    #[test]
    fn the_returned_clobbers_are_the_last_kernels() {
        let setup = |sources: u32, dests: u32| {
            let mut g = guest();
            g.set_u32(MIXER + SOURCE_COUNT, sources).unwrap();
            g.set_u32(MIXER + DEST_COUNT, dests).unwrap();
            descriptor(&mut g, DST_DESC, DST_BUF, 256);
            descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
            g
        };
        let mut none = setup(2, 0);
        fill(&mut none, DST_BUF, 256, 7.0);
        assert_eq!(
            ramp_gain_matrix(&mut none, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap(),
            LastKernelClobbers::None,
            "no destination, no kernel"
        );
        assert_eq!(at(&none, DST_BUF), 7.0, "and no channel written");

        let mut copy = setup(1, 1);
        match ramp_gain_matrix(&mut copy, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap() {
            LastKernelClobbers::Copy(c) => assert_eq!(c.v28, [8.0f32.to_bits(); 4]),
            other => panic!("expected the copy's clobber, got {other:?}"),
        }

        let mut accumulate = setup(2, 1);
        match ramp_gain_matrix(&mut accumulate, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap() {
            LastKernelClobbers::Accumulate(c) => {
                assert_eq!(c.v(14), [8.0f32.to_bits(); 4]);
                assert_eq!(c.v(21), [1.0f32.to_bits(); 4]);
            }
            other => panic!("expected the accumulate's clobber, got {other:?}"),
        }
    }

    #[test]
    fn ramp_gain_matrix_restores_the_entry_flush_mode() {
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 2).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 1).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        let before = crate::vmx::get_mxcsr();
        ramp_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC, SAVED, SP).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }

    #[test]
    fn both_bodies_restore_the_entry_flush_mode() {
        let mut g = guest();
        g.set_u32(MIXER + SOURCE_COUNT, 2).unwrap();
        g.set_u32(MIXER + DEST_COUNT, 1).unwrap();
        descriptor(&mut g, DST_DESC, DST_BUF, 256);
        descriptor(&mut g, SRC_DESC, SRC_BUF, 256);
        ramp_layout(&mut g, 1, 1.0, 0.0);
        let before = crate::vmx::get_mxcsr();
        apply_gain_matrix(&mut g, MIXER, DST_DESC, SRC_DESC).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "apply_gain_matrix");
        ramp_channels(&mut g, RAMP_OBJ, PAIR, 0).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "ramp_channels");
    }

    // ------------------------------------------------------------------ sub_82B29BE0

    const RP_PAIR: u32 = 0x4003_0000;
    const RP_DESC_BACK: u32 = 0x4003_0100;
    const RP_DESC_FRONT: u32 = 0x4003_0200;
    const RP_STACK_TOP: u32 = 0x4005_0000;
    const RP_LIVE: [f32; 10] = [0.1, 0.5, 0.25, 0.2, 0.5, 2.0, 0.75, 0.1, 0.3, 0.4];

    /// Set a word whether or not a segment already covers it.
    fn word(g: &mut Guest, at: u32, w: u32) {
        if g.set_u32(at, w).is_err() {
            g.put(at, w.to_be_bytes().to_vec());
        }
    }

    /// Two sources, no destinations (so both mixes touch no channel), a stereo configuration for the
    /// matrix, the panner layout's constants and pools, a stack, and the pair.
    fn republish_guest(cached_equal: bool) -> Guest {
        let mut g = guest();
        g.put(RP_PAIR, vec![0u8; 0x300]);
        g.put(RP_STACK_TOP - 0x1000, vec![0u8; 0x1000]);
        for (at, v) in [
            (crate::spatial::ONE_SINGLE, 1.0f32),
            (crate::spatial::SNAP_SINGLE, 0.999),
            (crate::spatial::ZERO_SINGLE, 0.0),
            (crate::spatial::HALF_SINGLE, 0.5),
            (crate::spatial::LAYOUT_ANGLE_SCALE, 1.0),
            (crate::spatial::LAYOUT_SPREAD_SCALE, 1.0),
            (crate::spatial::LAYOUT_MIRROR_MARKER, 7.0),
            (crate::spatial::LAYOUT_MIRROR_BIAS, 0.5),
        ] {
            word(&mut g, at, v.to_bits());
        }
        crate::mathlib::tests::with_atan_pool(&mut g);
        for (i, v) in RP_LIVE.iter().enumerate() {
            word(
                &mut g,
                MIXER + LIVE_FIRST + LIVE_STRIDE * i as u32,
                v.to_bits(),
            );
            let cached = if cached_equal { *v } else { v + 1.0 };
            word(&mut g, MIXER + MIX_CACHED + 4 * i as u32, cached.to_bits());
        }
        word(&mut g, MIXER + MIX_COMPARED, 0xDEAD_BEEF);
        word(&mut g, MIXER + MIX_TAIL, 0.5f32.to_bits());
        word(&mut g, MIXER + SOURCE_COUNT, 2);
        word(&mut g, MIXER + DEST_COUNT, 0);
        word(
            &mut g,
            MIXER + MIX_CONFIG + crate::spatial::MATRIX_DEST_COUNT,
            2,
        );
        for k in 0..64u32 {
            word(
                &mut g,
                MIXER + MIX_MATRIX + 4 * k,
                (k as f32 * 0.01).to_bits(),
            );
        }
        word(&mut g, RP_PAIR + REPUBLISH_PAIR_BACK, RP_DESC_BACK);
        word(&mut g, RP_PAIR + REPUBLISH_PAIR_FRONT, RP_DESC_FRONT);
        g
    }

    fn scripted() -> crate::mathlib::tests::Scripted {
        crate::mathlib::tests::Scripted {
            sine: 0.25,
            cosine: 0.5,
            asked: vec![],
        }
    }

    fn mixer_words(g: &Guest) -> Vec<u32> {
        (MIX_ENTRIES / 4..(MIX_TAIL + 12) / 4)
            .map(|k| g.u32(MIXER + 4 * k).unwrap())
            .collect()
    }

    fn swapped(g: &Guest) {
        assert_eq!(g.u32(RP_PAIR + REPUBLISH_PAIR_BACK).unwrap(), RP_DESC_FRONT);
        assert_eq!(g.u32(RP_PAIR + REPUBLISH_PAIR_FRONT).unwrap(), RP_DESC_BACK);
    }

    #[test]
    fn nothing_moved_and_no_flag_only_mixes_and_swaps() {
        let mut g = republish_guest(true);
        let before = mixer_words(&g);
        assert_eq!(
            republish_mix(&mut g, &mut scripted(), MIXER, RP_PAIR, 0x100, RP_STACK_TOP).unwrap(),
            1
        );
        assert_eq!(
            mixer_words(&g),
            before,
            "no recompute, no cache, no +700: the flag's low byte is 0"
        );
        swapped(&g);
    }

    #[test]
    fn nothing_moved_with_the_flag_recomputes_without_caching() {
        let mut g = republish_guest(true);
        let mut h = g.clone();
        republish_mix(&mut g, &mut scripted(), MIXER, RP_PAIR, 1, RP_STACK_TOP).unwrap();
        let live: [f64; 10] = RP_LIVE.map(f64::from);
        recompute_placement(
            &mut h,
            &mut scripted(),
            MIXER,
            2,
            &live,
            RP_STACK_TOP - REPUBLISH_FRAME_BYTES,
        )
        .unwrap();
        assert_eq!(mixer_words(&g), mixer_words(&h));
        assert_eq!(
            g.u32(MIXER + MIX_COMPARED).unwrap(),
            0xDEAD_BEEF,
            "+700 only on the moved path"
        );
    }

    #[test]
    fn a_moved_parameter_saves_the_matrix_recomputes_ramps_and_caches() {
        let mut g = republish_guest(false);
        let mut h = g.clone();
        let old: Vec<u32> = (0..16)
            .map(|k| g.u32(MIXER + MIX_MATRIX + 4 * k).unwrap())
            .collect();
        republish_mix(&mut g, &mut scripted(), MIXER, RP_PAIR, 0, RP_STACK_TOP).unwrap();
        let frame = RP_STACK_TOP - REPUBLISH_FRAME_BYTES;
        let saved: Vec<u32> = (0..16)
            .map(|k| g.u32(frame + REPUBLISH_SAVED + 4 * k).unwrap())
            .collect();
        assert_eq!(
            saved, old,
            "two source rows saved before the recompute overwrote them"
        );
        let live: [f64; 10] = RP_LIVE.map(f64::from);
        for k in 0..16u32 {
            h.set_u32(frame + REPUBLISH_SAVED + 4 * k, old[k as usize])
                .unwrap();
        }
        recompute_placement(&mut h, &mut scripted(), MIXER, 2, &live, frame).unwrap();
        ramp_gain_matrix(
            &mut h,
            MIXER,
            RP_DESC_FRONT,
            RP_DESC_BACK,
            frame + REPUBLISH_SAVED,
            frame,
        )
        .unwrap();
        fp::store_single(&mut h, MIXER + MIX_COMPARED, 0.75).unwrap();
        for (i, v) in RP_LIVE.iter().enumerate() {
            h.set_u32(MIXER + MIX_CACHED + 4 * i as u32, v.to_bits())
                .unwrap();
        }
        assert_eq!(mixer_words(&g), mixer_words(&h));
        swapped(&g);
    }
}

#[cfg(test)]
mod gain_ramp_advance_tests {
    use super::*;
    use crate::mathlib::tests::Scripted;

    const BASE: u32 = 0x4000_0000;
    const STATE: u32 = BASE;
    const DESC: u32 = BASE + 0x200;
    const CLOCK: u32 = BASE + 0x300;
    const MIXER: u32 = BASE + 0x400;
    const CHANNELS: u32 = BASE + 0x500;
    const ENVELOPE: u32 = BASE + 0x1000;
    const CHANNEL0: u32 = BASE + 0x2000;

    /// One channel of 2.0s at 256 samples a second; the gain sentinel 1.0 and the time sentinel -1.
    fn guest(gain: f32) -> Guest {
        let mut g = Guest::single(BASE, 0x4000);
        g.put(
            crate::routing::UNITY_GAIN,
            1.0f32.to_bits().to_be_bytes().to_vec(),
        );
        g.put(
            VOICE_RAMP_TIME_SENTINEL,
            (-1.0f64).to_bits().to_be_bytes().to_vec(),
        );
        g.put(
            crate::leaves::ZERO_CELL,
            0.0f32.to_bits().to_be_bytes().to_vec(),
        );
        g.put(
            crate::dsp::ramps::RAMP_INDEX_STRIDE,
            [4.0f32; 4]
                .iter()
                .flat_map(|v| v.to_bits().to_be_bytes())
                .collect(),
        );
        g.set_u32(DESC + MIX_DESC_CLOCK, CLOCK).unwrap();
        g.set_u32(CLOCK + 12, 256.0f32.to_bits()).unwrap();
        g.set_u32(DESC + MIX_DESC_MIXER, MIXER).unwrap();
        g.set_u32(MIXER + 4, ENVELOPE).unwrap();
        g.set_u32(DESC + MIX_DESC_CHANNELS, CHANNELS).unwrap();
        g.set_u32(CHANNELS + 4, CHANNEL0).unwrap();
        g.set_u16(CHANNELS + 14, 256).unwrap();
        g.set_u8(STATE + VOICE_RAMP_CHANNELS, 1).unwrap();
        g.set_u32(STATE + VOICE_RAMP_CURRENT_GAIN, gain.to_bits())
            .unwrap();
        for i in 0..256u32 {
            g.set_u32(CHANNEL0 + 4 * i, 2.0f32.to_bits()).unwrap();
            g.set_u32(ENVELOPE + 4 * i, 0x7777_7777).unwrap();
        }
        g
    }

    fn trig() -> Scripted {
        Scripted {
            sine: 0.0,
            cosine: 0.0,
            asked: vec![],
        }
    }

    #[test]
    fn a_steady_voice_at_the_sentinel_gain_changes_nothing() {
        let mut g = guest(1.0);
        assert_eq!(
            advance_gain_ramp(&mut g, &mut trig(), STATE, DESC).unwrap(),
            1
        );
        assert_eq!(g.u32(ENVELOPE).unwrap(), 0x7777_7777);
        assert_eq!(g.f32(CHANNEL0).unwrap(), 2.0);
        assert_eq!(g.u32(STATE + VOICE_RAMP_LAST_WORD).unwrap(), 0);
    }

    #[test]
    fn a_steady_gain_fills_the_envelope_and_scales_the_channel() {
        let mut g = guest(0.5);
        advance_gain_ramp(&mut g, &mut trig(), STATE, DESC).unwrap();
        assert!((0..256).all(|i| g.f32(ENVELOPE + 4 * i).unwrap() == 0.5));
        assert!((0..256).all(|i| g.f32(CHANNEL0 + 4 * i).unwrap() == 1.0));
        assert_eq!(g.f32(STATE + VOICE_RAMP_CURRENT_GAIN).unwrap(), 0.5);
        assert_eq!(g.u32(STATE + VOICE_RAMP_LAST_WORD).unwrap(), 0x7FF7_FFF1);
    }

    #[test]
    fn a_start_request_runs_a_one_block_linear_ramp_to_its_end() {
        let mut g = guest(0.0);
        g.set_u8(STATE + VOICE_RAMP_START_PENDING, 1).unwrap();
        g.set_u32(STATE + VOICE_RAMP_REQUEST_SPAN, 1.0f32.to_bits())
            .unwrap(); // 256 samples
        g.set_u32(STATE + VOICE_RAMP_REQUEST_EXTRA, 256.0f32.to_bits())
            .unwrap();
        g.set_u32(STATE + VOICE_RAMP_REQUEST_CURVE, 0).unwrap();
        g.set_u64(STATE + VOICE_RAMP_START_TIME, 10.0f64.to_bits())
            .unwrap();
        g.set_u64(DESC + MIX_DESC_TIME_BASE, 10.0f64.to_bits())
            .unwrap();
        let mut h = g.clone();
        advance_gain_ramp(&mut g, &mut trig(), STATE, DESC).unwrap();
        crate::dsp::ramps::linear_ramp(&mut h, ENVELOPE, 0, 256, 0.0, 256.0).unwrap();
        for i in 0..256u32 {
            assert_eq!(
                g.u32(ENVELOPE + 4 * i).unwrap(),
                h.u32(ENVELOPE + 4 * i).unwrap(),
                "envelope {i}"
            );
            let scaled = h.f32(ENVELOPE + 4 * i).unwrap() * 2.0;
            assert_eq!(g.f32(CHANNEL0 + 4 * i).unwrap(), scaled, "channel {i}");
        }
        assert_eq!(g.u32(STATE + VOICE_RAMP_SAMPLES).unwrap(), 256);
        assert_eq!(g.u32(STATE + VOICE_RAMP_CURSOR).unwrap(), 256);
        assert_eq!(
            g.u8(STATE + VOICE_RAMP_STATE).unwrap(),
            0,
            "the ramp finished within the block"
        );
        assert_eq!(g.f32(STATE + VOICE_RAMP_CURRENT_GAIN).unwrap(), 256.0);
    }
}
