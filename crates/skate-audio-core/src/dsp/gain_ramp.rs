//! `sub_82B3C098` and `sub_82B44D18` — a gain-ramped copy of a fixed 256-single block, and its
//! accumulating sibling.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`gain_ramp_copy`] | `sub_82B3C098` | verified | 278 | 2,126,778 | 3,023,068 |
//! | [`gain_ramp_accumulate`] | `sub_82B44D18` | verified | 402 | 34,866 | 71,847 |
//!
//! Both are replayed against recorded gameplay: 166 vectors for the copy and 31 for the
//! accumulate, 0 disagreements.
//!
//! Both `.inc` headers lead with `// STATUS: verified` and `docs/ports.md` agrees. They share a file
//! because they share everything but the last step: the same thirteen rodata cells in the same roles,
//! the same two-loop shape, the same literal 1024-byte extent. `sub_82B44D18` differs in three places —
//! every group is a `vmaddfp` **into** the destination instead of a `vmulfp128` over it, there is no
//! `dcbzl`, and it clobbers all eighteen of `v14`-`v31` rather than four. Its only caller in the audio
//! set that matters here is `sub_82B298E0` ([`crate::gains::ramp_gain_matrix`]), which runs this
//! kernel for every source channel after the first.
//!
//! What follows up to the next heading is about `sub_82B3C098`, and was written before its sibling
//! joined it.
//!
//!
//! `STATUS: verified`, zero divergence over 902,638 compared calls in one boot session, at
//! **2,126,778 calls per boot** and 3,023,068 per played session on `RwAudioCore Dac`. A leaf: no
//! callees, no imports, no indirect calls, no timebase.
//!
//! `dst[k] = src[k] * gain(k)` for `k` in `0..256`, where `gain(k) = f1 + k*f2` for the first 64
//! samples and `f1 + 64*f2` for the remaining 192. **The length is not a parameter**: `src + 256`
//! and `src + 1024` are both literal in the lifted body, so every call touches exactly 1024 bytes
//! in and 1024 out.
//!
//! ## What this port had to reason out, and what later measured it
//!
//! `probe/ports/notes/sub_82B3C098.md` records that the C++ inferred five constants from the
//! register plumbing and **did not measure them** — "no image dump was taken for this port" — while
//! being careful to load every one through the guest map so that nothing depended on the guess.
//! Those inferences are now checked. Reading the validated dump (`probe/harness/out/image/`) at the
//! addresses below gives exactly what the note predicted:
//!
//! | cell | inferred | measured |
//! |---|---|---|
//! | [`STEP_SCALE`] | 4.0 | 4.0 |
//! | [`RAMP_SPAN`] | 64.0 | 64.0 |
//! | [`LANE2_SCALE`] | 2.0 | 2.0 |
//! | [`LANE3_SCALE`] | 3.0 | 3.0 |
//! | the eight pool vectors | splats of 1 … 8 | splats of 1 … 8, in the mapped order |
//!
//! So the register-plumbing reading of which pool vector scales which 16-byte group is confirmed,
//! not merely consistent. The body below still reads all thirteen cells live, for the same reasons
//! the C++ does: it is what the original does, and a patched image should reach the port.
//!
//! ## The two vector materialisations
//!
//! The lifted body builds two vectors through its own red zone — four `stfs` then one `lvx128` —
//! and neither is reproduced as a memory write. `stfs` writes big-endian and `lvx128` reverses all
//! sixteen bytes, so **the two reversals cancel**: the single written at `-96(r1)` comes back as
//! host lane 3. That is the fact the whole per-lane gain ordering rests on, and the harness settled
//! it over 900,000 calls rather than it being argued here.
//!
//! ## Rule 4 and the clobbers
//!
//! Sixteen `vmaddfp` and sixteen `vmulfp128` sites, every one with the lifted operand order. See
//! [`crate::dsp`] for why that is necessary and not sufficient.
//!
//! The original clobbers `v28`-`v31` — inside the ABI-preserved range the harness compares on every
//! call — and never saves them: it uses `__savegprlr_23`, not `__savevmx_*`. So four group
//! multipliers are compared whether or not the result mask names anything, and
//! [`GainRampClobbers`] is those four.
//!
//! ## What no test here can catch
//!
//! Four of the seventeen `vmaddfp` sites are multiply-adds by a **power of two** — the group-1,
//! group-2 and group-4 multipliers and the whole-block step of 8 — and a product by a power of two
//! is exact, so at those four sites a fused multiply-add and a separate multiply-then-add are the
//! same function rather than two answers that happen to agree. Rewriting any of them the wrong way
//! leaves every test in this crate passing. That was measured by making the change and watching the
//! suite, not assumed; the same change at group 3 or group 5, where the multiplier is not a power of
//! two, fails `it_matches_the_independent_model_bit_for_bit`. All seventeen go through
//! [`vmx::vmaddfp`], which rounds twice as the recomp does. *Corrected 2026-09-13:* this said
//! "fused", and the measurement above was taken under that reading.

#![allow(unused_unsafe)] // see the note at the top of `crate::vmx`

use crate::fp;
use crate::vmx::{self, Fpscr};
use crate::{Guest, Result};
use core::arch::x86_64::*;

// The rodata singles. Each address is `((lis_imm & 0xFFFF) << 16) + offset`, computed from the
// immediate rather than read off the disassembly: one misread digit in `sub_82B2FE00` cost this
// project its first shadow divergence.
const LIS_82250000: u32 = ((-32219i32 as u32) & 0xFFFF) << 16;
const LIS_820F0000: u32 = ((-32241i32 as u32) & 0xFFFF) << 16;
const LIS_82060000: u32 = ((-32250i32 as u32) & 0xFFFF) << 16;
const LIS_82320000: u32 = ((-32206i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82250000 == 0x8225_0000, "lis -32219");
const _: () = assert!(LIS_820F0000 == 0x820F_0000, "lis -32241");
const _: () = assert!(LIS_82060000 == 0x8206_0000, "lis -32250");
const _: () = assert!(LIS_82320000 == 0x8232_0000, "lis -32206");

/// `lfs f0,29448(r9)` — measured 4.0, the samples-per-16-byte-group the step is scaled by. Shared
/// with `sub_82B2DAC8`.
pub const STEP_SCALE: u32 = LIS_82250000.wrapping_add(29448);
/// `lfs f13,-9896(r8)` — measured 64.0, the length of the ramp in samples.
pub const RAMP_SPAN: u32 = LIS_820F0000.wrapping_add(-9896i32 as u32);
/// `lfs f0,3152(r7)` — measured 2.0. Shared with `sub_82B1D3D0`.
pub const LANE2_SCALE: u32 = LIS_82060000.wrapping_add(3152);
/// `lfs f13,15112(r9)` — measured 3.0.
pub const LANE3_SCALE: u32 = LIS_82060000.wrapping_add(15112);

const _: () = assert!(STEP_SCALE == 0x8225_7308);
const _: () = assert!(RAMP_SPAN == 0x820E_D958);
const _: () = assert!(LANE2_SCALE == 0x8206_0C50);
const _: () = assert!(LANE3_SCALE == 0x8206_3B08);

/// The eight 16-byte group multipliers at `0x8231BA20` … `0x8231BA9F`, named by the 16-byte group
/// of the block each one offsets. `SCALE[0]` is unused — group 0 takes the running gain unscaled,
/// through a `vor128` rather than a multiply-add — and [`SCALE_STEP`] is the whole-block increment.
///
/// The mapping from pool address to group is read off the register plumbing (`addi -17824 -> v4`
/// and so on), and the measured values confirm it: the vector at `-17824` really is a splat of 1.0.
pub const SCALE: [u32; 8] = [
    0,                                           // group 0 is not scaled at all
    LIS_82320000.wrapping_add(-17824i32 as u32), // addi -17824 -> v4, measured 1.0
    LIS_82320000.wrapping_add(-17808i32 as u32), // addi -17808 -> v3, measured 2.0
    LIS_82320000.wrapping_add(-17792i32 as u32), // addi -17792 -> v2, measured 3.0
    LIS_82320000.wrapping_add(-17776i32 as u32), // addi -17776 -> v1, measured 4.0
    LIS_82320000.wrapping_add(-17888i32 as u32), // addi -17888 -> v31, measured 5.0
    LIS_82320000.wrapping_add(-17872i32 as u32), // addi -17872 -> v30, measured 6.0
    LIS_82320000.wrapping_add(-17856i32 as u32), // addi -17856 -> v29, measured 7.0
];
/// `addi -17840 -> v28`, measured 8.0: the increment applied once per 128-byte block.
pub const SCALE_STEP: u32 = LIS_82320000.wrapping_add(-17840i32 as u32);

const _: () = assert!(SCALE[1] == 0x8231_BA60);
const _: () = assert!(SCALE[2] == 0x8231_BA70);
const _: () = assert!(SCALE[3] == 0x8231_BA80);
const _: () = assert!(SCALE[4] == 0x8231_BA90);
const _: () = assert!(SCALE[5] == 0x8231_BA20);
const _: () = assert!(SCALE[6] == 0x8231_BA30);
const _: () = assert!(SCALE[7] == 0x8231_BA40);
const _: () = assert!(SCALE_STEP == 0x8231_BA50);
// All eight are 16-byte aligned, so `lvx128`'s address masking is a no-op on them.
const _: () = assert!(SCALE_STEP % 16 == 0 && SCALE[1] % 16 == 0 && SCALE[5] % 16 == 0);

/// Vectors per 128-byte block.
pub const VECTORS_PER_BLOCK: usize = 8;
/// `addi r10,r10,128` / `addi r11,r11,128`.
pub const BLOCK_BYTES: u32 = 128;
/// `r31 = src + 256` — the first loop's limit, two blocks, 64 singles with the gain still moving.
pub const RAMP_BLOCKS: usize = 2;
/// `r30 = src + 1024` — the second loop's limit, six more blocks at the held gain.
pub const HOLD_BLOCKS: usize = 6;
/// 1024. Not a parameter: both limits are literal in the lifted body.
pub const TOTAL_BYTES: u32 = (RAMP_BLOCKS as u32 + HOLD_BLOCKS as u32) * BLOCK_BYTES;
const _: () = assert!(TOTAL_BYTES == 1024);

/// The four ABI-preserved vector registers `sub_82B3C098` clobbers without saving.
///
/// The function is void and leaves `r3 = 64` as scratch, so the result mask is `kReturnNone` and
/// memory is the whole intended comparison — but `v28`-`v31` are inside `SHADOW_PRESERVED_VRS` and
/// are therefore compared on every single call regardless. Returned here for the same reason the
/// C++ stores them back: dropping the clobber would be a divergence on two million calls a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GainRampClobbers {
    /// `v28` — the whole-block increment, measured 8.0.
    pub v28: [u32; 4],
    /// `v29` — the group-7 multiplier, measured 7.0.
    pub v29: [u32; 4],
    /// `v30` — the group-6 multiplier, measured 6.0.
    pub v30: [u32; 4],
    /// `v31` — the group-5 multiplier, measured 5.0.
    pub v31: [u32; 4],
}

/// `sub_82B3C098`. `dst` is `r3`, `src` is `r4`, `gain` is `f1` (the gain at sample 0) and `step`
/// is `f2` (the increment per sample).
///
/// Writes `[dst & !127, (dst & !0xF) + 1024)` — one span covering both write shapes, the `dcbzl`
/// line clears and the `stvx128`s. For the 128-byte aligned `dst` the mixer passes in practice the
/// two coincide and the span is exactly 1024 bytes. Reads 1024 bytes at `src` plus thirteen rodata
/// cells.
pub fn gain_ramp_copy(
    g: &mut Guest,
    dst: u32,
    src: u32,
    gain: f64,
    step: f64,
) -> Result<GainRampClobbers> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    unsafe { gain_ramp_copy_impl(g, dst, src, gain, step) }
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn gain_ramp_copy_impl(
    g: &mut Guest,
    dst: u32,
    src: u32,
    f1: f64,
    f2: f64,
) -> Result<GainRampClobbers> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f0,29448(r9)

    let step_scale = fp::load_single(g, STEP_SCALE)?; // lfs f0,29448(r9)
    let ramp_span = fp::load_single(g, RAMP_SPAN)?; // lfs f13,-9896(r8)
    let step = fp::mul_single(f2, step_scale); // fmuls f11,f2,f0
    let held = fp::fmadd_single(f2, ramp_span, f1); // fmadds f12,f2,f13,f1
    let gain1 = fp::add_single(f1, f2); // fadds f10,f1,f2
    let lane2_scale = fp::load_single(g, LANE2_SCALE)?; // lfs f0,3152(r7)
    let lane3_scale = fp::load_single(g, LANE3_SCALE)?; // lfs f13,15112(r9)
    let gain2 = fp::fmadd_single(f2, lane2_scale, f1); // fmadds f9,f2,f0,f1
    let gain3 = fp::fmadd_single(f2, lane3_scale, f1); // fmadds f8,f2,f13,f1

    // Four copies of the step at -112(r1), and the four starting lane gains at -96(r1), each read
    // back with one `lvx128`. The two byte reversals cancel, so the single stored lowest is host
    // lane 3 — i.e. the gain of sample 0. Built in registers here; the port never writes the guest
    // stack.
    let step_v = unsafe { _mm_set1_ps(step as f32) };
    let start = unsafe { _mm_set_ps(f1 as f32, gain1 as f32, gain2 as f32, gain3 as f32) };

    // The eight group multipliers, in the order the lifted body loads them.
    let scale_step = unsafe { vmx::lvx128_ps(g, SCALE_STEP)? }; // lvx128 v28,r0,r9
    let scale7 = unsafe { vmx::lvx128_ps(g, SCALE[7])? }; // lvx128 v29,r0,r8
    let scale6 = unsafe { vmx::lvx128_ps(g, SCALE[6])? }; // lvx128 v30,r0,r7
    let scale5 = unsafe { vmx::lvx128_ps(g, SCALE[5])? }; // lvx128 v31,r0,r29
    let scale4 = unsafe { vmx::lvx128_ps(g, SCALE[4])? }; // lvx128 v1,r0,r28
    let scale3 = unsafe { vmx::lvx128_ps(g, SCALE[3])? }; // lvx128 v2,r0,r27
    let scale2 = unsafe { vmx::lvx128_ps(g, SCALE[2])? }; // lvx128 v3,r0,r26
    let scale1 = unsafe { vmx::lvx128_ps(g, SCALE[1])? }; // lvx128 v4,r0,r25

    let mut src_cursor = src; // r10
    let mut dst_cursor = dst; // r11
    let mut gain = start; // v0

    // First loop: the gain still moving. Two blocks of eight vectors, 64 singles.
    for _ in 0..RAMP_BLOCKS {
        // lvx128 v63..v56. Every source vector is loaded before the first store of the block, so a
        // destination overlapping the source still sees the pre-call bytes.
        let mut sample = [unsafe { _mm_setzero_ps() }; VECTORS_PER_BLOCK];
        for (i, s) in sample.iter_mut().enumerate() {
            *s = unsafe { vmx::lvx128_ps(g, src_cursor + 16 * i as u32)? };
        }
        src_cursor = src_cursor.wrapping_add(BLOCK_BYTES); // addi r10,r10,128

        fpscr.enable_flush_mode_unconditional(); // emitted at vmaddfp v12,v28,v13,v0
        let next = unsafe { vmx::vmaddfp(scale_step, step_v, gain) }; // vmaddfp v12,v28,v13,v0
        let mut group_gain = [gain; VECTORS_PER_BLOCK];
        group_gain[1] = unsafe { vmx::vmaddfp(scale1, step_v, gain) }; // vmaddfp v11,v4,v13,v0
        group_gain[0] = gain; // vor128 v55,v0,v0
        group_gain[2] = unsafe { vmx::vmaddfp(scale2, step_v, gain) };
        group_gain[3] = unsafe { vmx::vmaddfp(scale3, step_v, gain) };
        group_gain[4] = unsafe { vmx::vmaddfp(scale4, step_v, gain) };
        group_gain[5] = unsafe { vmx::vmaddfp(scale5, step_v, gain) };
        group_gain[6] = unsafe { vmx::vmaddfp(scale6, step_v, gain) };
        group_gain[7] = unsafe { vmx::vmaddfp(scale7, step_v, gain) }; // vmaddfp v5,v29,v13,v0
        gain = next; // vor v0,v12,v12

        vmx::dcbzl(g, dst_cursor)?; // dcbzl r0,r11

        // vmulfp128 v54..v47: a plain multiply, which must never fuse with the vmaddfp above.
        let mut out = [unsafe { _mm_setzero_ps() }; VECTORS_PER_BLOCK];
        for (i, o) in out.iter_mut().enumerate() {
            *o = unsafe { vmx::vmulfp(group_gain[i], sample[i]) }; // vmulfp128 v54,v55,v63 ...
        }
        for (i, o) in out.iter().enumerate() {
            unsafe { vmx::stvx128_ps(g, dst_cursor + 16 * i as u32, *o)? };
        }
        dst_cursor = dst_cursor.wrapping_add(BLOCK_BYTES); // addi r11,r11,128
    }

    // The ramp is over at sample 64: every remaining single takes the held gain.
    fpscr.disable_flush_mode(); // the *guarded* form, emitted at stfs f12,-96(r1)
    let held_v = unsafe { _mm_set1_ps(held as f32) }; // stfs f12 x4 ; lvx128 v63,r0,r31

    // Second loop: six blocks, 192 singles, constant gain.
    for _ in 0..HOLD_BLOCKS {
        let mut sample = [unsafe { _mm_setzero_ps() }; VECTORS_PER_BLOCK];
        for (i, s) in sample.iter_mut().enumerate() {
            *s = unsafe { vmx::lvx128_ps(g, src_cursor + 16 * i as u32)? }; // lvx128 v62..v55
        }
        src_cursor = src_cursor.wrapping_add(BLOCK_BYTES);

        vmx::dcbzl(g, dst_cursor)?; // dcbzl r0,r11

        fpscr.enable_flush_mode_unconditional(); // emitted at vmulfp128 v46,v63,v62
        let mut out = [unsafe { _mm_setzero_ps() }; VECTORS_PER_BLOCK];
        for (i, o) in out.iter_mut().enumerate() {
            *o = unsafe { vmx::vmulfp(held_v, sample[i]) }; // held gain is the FIRST operand
        }
        for (i, o) in out.iter().enumerate() {
            unsafe { vmx::stvx128_ps(g, dst_cursor + 16 * i as u32, *o)? };
        }
        dst_cursor = dst_cursor.wrapping_add(BLOCK_BYTES);
    }

    Ok(GainRampClobbers {
        v28: unsafe { lanes_ps(scale_step) },
        v29: unsafe { lanes_ps(scale7) },
        v30: unsafe { lanes_ps(scale6) },
        v31: unsafe { lanes_ps(scale5) },
    })
}

// ======================================================= sub_82B44D18: the accumulating sibling

/// The ramp loop's `stvx128` order in [`gain_ramp_accumulate`], by 16-byte group — **not** ascending.
///
/// `stvx128 v25,r0,r11`, `v24,r11,r31`, `v1,r11,r3`, `v2,r11,r5`, then `v11,r11,r8` (group 6), then
/// `v9,r11,r6` and `v10,r11,r7` (groups 4, 5), then `v12,r11,r9`. Reproduced literally, as the C++'s
/// `kRampStoreOrder` is. The hold loop stores in ascending order.
pub const ACCUMULATE_RAMP_STORE_ORDER: [usize; VECTORS_PER_BLOCK] = [0, 1, 2, 3, 6, 4, 5, 7];

/// The eighteen ABI-preserved vector registers `sub_82B44D18` leaves clobbered: **all** of
/// `v14`-`v31`, none of them saved — it uses `__savegprlr_24`, not `__savevmx_*`.
///
/// `vr[n - 14]` is `vn` as four host lanes, the layout the C++ writes with
/// `simde_mm_store_ps(ctx.vN.f32, ...)`. `v14`-`v21` are the eight pool multipliers; `v22`-`v31` are
/// the **second ramp block's** intermediates, which survive because the hold loop writes only
/// `v0`-`v13`:
///
/// | register | holds |
/// |---|---|
/// | `v14` | the whole-block step, [`SCALE_STEP`] |
/// | `v15`, `v16`, `v17` | the group 7, 6, 5 multipliers |
/// | `v18`, `v19`, `v20`, `v21` | the group 4, 3, 2, 1 multipliers |
/// | `v22`, `v23` | the destination vectors that block read at `+48` and `+32` |
/// | `v24`, `v25` | that block's results for groups 1 and 0 |
/// | `v26` | the gain a third ramp block would have started from |
/// | `v27` … `v31` | that block's gains for groups 7, 6, 5, 4, 3 |
///
/// The harness compares `v14`-`v31` on every call whatever the result mask says, so this is part of
/// the function's observable output even though the guest function is void.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccumulateClobbers {
    /// `vr[n - 14]` is `vn`.
    pub vr: [[u32; 4]; 18],
}

impl AccumulateClobbers {
    /// The lowest register [`AccumulateClobbers::vr`] holds.
    pub const FIRST: usize = 14;

    /// `vn`'s four host lanes, for `n` in `14..=31`.
    pub fn v(&self, n: usize) -> [u32; 4] {
        self.vr[n - Self::FIRST]
    }
}

/// `sub_82B44D18` — `dst[k] += src[k] * gain(k)` over 256 singles, the accumulating sibling of
/// [`gain_ramp_copy`]. `dst` is `r3`, `src` is `r4`, `gain` is `f1` (the gain at sample 0) and `step`
/// is `f2` (the increment per sample). `gain(k)` ramps for 64 samples and then holds, exactly as in
/// [`gain_ramp_copy`], and the length is again literal rather than a parameter.
///
/// **Writes** `[dst & !0xF, (dst & !0xF) + 1024)` through 64 `stvx128`, and nothing else. There is
/// **no `dcbzl`**: the destination's old contents are the accumulator, so there is nothing to discard,
/// and an unaligned `dst` leaves the bytes below it in its line alone — the one place this kernel's
/// write set differs from its sibling's. **Reads** that same span, 1024 bytes at `src & !0xF`
/// (`lvx128` masks its address, so an unaligned `src` is read from below itself), and the thirteen
/// rodata cells [`gain_ramp_copy`] reads, in the same roles.
///
/// Returns the clobbered `v14`-`v31` ([`AccumulateClobbers`]). The guest function is void and leaves
/// `r3 = 32` as scratch (`li r3,32`); its mask is `kReturnNone`, so that word is not a result.
///
/// Every vector multiply-add is [`vmx::vmaddfp`] with its operands in the lifted order — `(gain,
/// sample, accumulator)` for the samples and `(multiplier, step, gain)` for the group gains. How that
/// operation rounds is [`crate::vmx`]'s to state, not this function's. The four scalar `fmadds` of the
/// preamble are scalar and go through [`fp::fmadd_single`].
///
/// Reproduced rather than tidied: the ramp loop's store order ([`ACCUMULATE_RAMP_STORE_ORDER`]); all
/// eight source loads of a block before its eight destination loads, and all sixteen before any store,
/// so a destination one vector ahead of its source still sees the pre-call source within a block; and
/// the guarded `disableFlushMode` form at the `stfs f12,-96(r1)` between the two loops, where the other
/// eleven toggles are unconditional.
pub fn gain_ramp_accumulate(
    g: &mut Guest,
    dst: u32,
    src: u32,
    gain: f64,
    step: f64,
) -> Result<AccumulateClobbers> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    unsafe { gain_ramp_accumulate_impl(g, dst, src, gain, step) }
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn gain_ramp_accumulate_impl(
    g: &mut Guest,
    dst: u32,
    src: u32,
    f1: f64,
    f2: f64,
) -> Result<AccumulateClobbers> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f0,29448(r9)

    let step_scale = fp::load_single(g, STEP_SCALE)?; // lfs f0,29448(r9)
    let ramp_span = fp::load_single(g, RAMP_SPAN)?; // lfs f13,-9896(r8)
    let step = fp::mul_single(f2, step_scale); // fmuls f11,f2,f0
    let held = fp::fmadd_single(f2, ramp_span, f1); // fmadds f12,f2,f13,f1
    let gain1 = fp::add_single(f1, f2); // fadds f10,f1,f2
    let lane2_scale = fp::load_single(g, LANE2_SCALE)?; // lfs f0,3152(r7)
    let lane3_scale = fp::load_single(g, LANE3_SCALE)?; // lfs f13,15112(r6)
    let gain2 = fp::fmadd_single(f2, lane2_scale, f1); // fmadds f9,f2,f0,f1
    let gain3 = fp::fmadd_single(f2, lane3_scale, f1); // fmadds f8,f2,f13,f1

    // stfs f11 x4 at -112(r1), read back by lvx128 v13; stfs f1/f10/f9/f8 at -96(r1), read back by
    // lvx128 v0. Built in registers: the stfs byte order and the lvx128 reversal cancel, exactly as in
    // `gain_ramp_copy`, so the single stored lowest (f1, sample 0's gain) is host lane 3.
    let step_v = unsafe { _mm_set1_ps(step as f32) };
    let start = unsafe { _mm_set_ps(f1 as f32, gain1 as f32, gain2 as f32, gain3 as f32) };

    // The eight pool multipliers, in the order the lifted body loads them.
    let scale_step = unsafe { vmx::lvx128_ps(g, SCALE_STEP)? }; // lvx128 v14,r0,r9
    let scale2 = unsafe { vmx::lvx128_ps(g, SCALE[2])? }; // lvx128 v20,r0,r9
    let scale7 = unsafe { vmx::lvx128_ps(g, SCALE[7])? }; // lvx128 v15,r0,r8
    let scale6 = unsafe { vmx::lvx128_ps(g, SCALE[6])? }; // lvx128 v16,r0,r7
    let scale5 = unsafe { vmx::lvx128_ps(g, SCALE[5])? }; // lvx128 v17,r0,r6
    let scale4 = unsafe { vmx::lvx128_ps(g, SCALE[4])? }; // lvx128 v18,r0,r29
    let scale3 = unsafe { vmx::lvx128_ps(g, SCALE[3])? }; // lvx128 v19,r0,r28
    let scale1 = unsafe { vmx::lvx128_ps(g, SCALE[1])? }; // lvx128 v21,r0,r27

    let mut src_cursor = src; // r10
    let mut dst_cursor = dst; // r11
    let mut gain = start; // v0

    // Outside the loop on purpose: the last ramp block's values are part of the result (v22-v31).
    // The loop always runs twice, so none of these is returned unwritten.
    let zero = unsafe { _mm_setzero_ps() };
    let mut accum = [zero; VECTORS_PER_BLOCK];
    let mut out = [zero; VECTORS_PER_BLOCK];
    let mut group_gain = [zero; VECTORS_PER_BLOCK];
    let mut next = zero;

    // First loop, loc_82B44E14: the gain still moving. Two blocks of eight vectors, 64 singles.
    for _ in 0..RAMP_BLOCKS {
        // lvx128 v12, v11, v10, v9, v8, v7, v6, v5: the source, all eight before anything else.
        let mut sample = [zero; VECTORS_PER_BLOCK];
        for (i, s) in sample.iter_mut().enumerate() {
            *s = unsafe { vmx::lvx128_ps(g, src_cursor.wrapping_add(16 * i as u32))? };
        }
        src_cursor = src_cursor.wrapping_add(BLOCK_BYTES); // addi r10,r10,128

        fpscr.enable_flush_mode_unconditional(); // emitted at vmaddfp v2,v21,v13,v0
        group_gain[0] = gain; // group 0 is v0 itself, the fourth operand of vmaddfp v25,v0,v12,v4
        group_gain[1] = unsafe { vmx::vmaddfp(scale1, step_v, gain) }; // vmaddfp v2,v21,v13,v0
        group_gain[2] = unsafe { vmx::vmaddfp(scale2, step_v, gain) }; // vmaddfp v1,v20,v13,v0
        group_gain[3] = unsafe { vmx::vmaddfp(scale3, step_v, gain) }; // vmaddfp v31,v19,v13,v0
        group_gain[6] = unsafe { vmx::vmaddfp(scale6, step_v, gain) }; // vmaddfp v28,v16,v13,v0
        group_gain[4] = unsafe { vmx::vmaddfp(scale4, step_v, gain) }; // vmaddfp v30,v18,v13,v0
        group_gain[5] = unsafe { vmx::vmaddfp(scale5, step_v, gain) }; // vmaddfp v29,v17,v13,v0
        group_gain[7] = unsafe { vmx::vmaddfp(scale7, step_v, gain) }; // vmaddfp v27,v15,v13,v0

        // lvx128 v4, v24, v23, v22, v3, v4, v12, v0: the accumulator. Interleaved with the gains in
        // the lifted body, and every one of them still precedes every store of the block.
        for (i, a) in accum.iter_mut().enumerate() {
            *a = unsafe { vmx::lvx128_ps(g, dst_cursor.wrapping_add(16 * i as u32))? };
        }

        out[0] = unsafe { vmx::vmaddfp(group_gain[0], sample[0], accum[0]) }; // vmaddfp v25,v0,v12,v4
        next = unsafe { vmx::vmaddfp(scale_step, step_v, gain) }; // vmaddfp v26,v14,v13,v0
        out[1] = unsafe { vmx::vmaddfp(group_gain[1], sample[1], accum[1]) }; // vmaddfp v24,v2,v11,v24
        out[2] = unsafe { vmx::vmaddfp(group_gain[2], sample[2], accum[2]) }; // vmaddfp v1,v1,v10,v23
        out[3] = unsafe { vmx::vmaddfp(group_gain[3], sample[3], accum[3]) }; // vmaddfp v2,v31,v9,v22
        out[6] = unsafe { vmx::vmaddfp(group_gain[6], sample[6], accum[6]) }; // vmaddfp v11,v28,v6,v12
        out[4] = unsafe { vmx::vmaddfp(group_gain[4], sample[4], accum[4]) }; // vmaddfp v9,v30,v8,v3
        out[5] = unsafe { vmx::vmaddfp(group_gain[5], sample[5], accum[5]) }; // vmaddfp v10,v29,v7,v4
        out[7] = unsafe { vmx::vmaddfp(group_gain[7], sample[7], accum[7]) }; // vmaddfp v12,v27,v5,v0

        for &i in &ACCUMULATE_RAMP_STORE_ORDER {
            unsafe { vmx::stvx128_ps(g, dst_cursor.wrapping_add(16 * i as u32), out[i])? };
        }
        dst_cursor = dst_cursor.wrapping_add(BLOCK_BYTES); // addi r11,r11,128
        gain = next; // vor v0,v26,v26
    }

    // The ramp is over at sample 64: every remaining single takes the held gain.
    fpscr.disable_flush_mode(); // the *guarded* form, emitted at stfs f12,-96(r1)
    let held_v = unsafe { _mm_set1_ps(held as f32) }; // stfs f12 x4 ; lvx128 v0,r0,r29

    // Second loop, loc_82B44EEC: six blocks, 192 singles, constant gain.
    for _ in 0..HOLD_BLOCKS {
        // lvx128 v12, v10, v9, v5, v4, v3, v2, v1.
        let mut sample = [zero; VECTORS_PER_BLOCK];
        for (i, s) in sample.iter_mut().enumerate() {
            *s = unsafe { vmx::lvx128_ps(g, src_cursor.wrapping_add(16 * i as u32))? };
        }
        src_cursor = src_cursor.wrapping_add(BLOCK_BYTES); // addi r10,r10,128

        // lvx128 v13, v13, v11, v12, v13, v11, v12, v13: all eight before the first store here too.
        let mut hold_accum = [zero; VECTORS_PER_BLOCK];
        for (i, a) in hold_accum.iter_mut().enumerate() {
            *a = unsafe { vmx::lvx128_ps(g, dst_cursor.wrapping_add(16 * i as u32))? };
        }

        fpscr.enable_flush_mode_unconditional(); // emitted at vmaddfp v6,v0,v12,v13
        let mut hold_out = [zero; VECTORS_PER_BLOCK];
        for (i, o) in hold_out.iter_mut().enumerate() {
            // vmaddfp v6,v0,v12,v13 ... v13,v0,v1,v13: the held gain is the FIRST operand.
            *o = unsafe { vmx::vmaddfp(held_v, sample[i], hold_accum[i]) };
        }
        for (i, o) in hold_out.iter().enumerate() {
            unsafe { vmx::stvx128_ps(g, dst_cursor.wrapping_add(16 * i as u32), *o)? };
        }
        dst_cursor = dst_cursor.wrapping_add(BLOCK_BYTES); // addi r11,r11,128
    }

    let registers = [
        scale_step,    // v14
        scale7,        // v15
        scale6,        // v16
        scale5,        // v17
        scale4,        // v18
        scale3,        // v19
        scale2,        // v20
        scale1,        // v21
        accum[3],      // v22: lvx128 v22,r11,r5, the accumulator at +48
        accum[2],      // v23: lvx128 v23,r11,r3, the accumulator at +32
        out[1],        // v24: vmaddfp v24,v2,v11,v24
        out[0],        // v25: vmaddfp v25,v0,v12,v4
        next,          // v26: vmaddfp v26,v14,v13,v0
        group_gain[7], // v27
        group_gain[6], // v28
        group_gain[5], // v29
        group_gain[4], // v30
        group_gain[3], // v31
    ];
    let mut vr = [[0u32; 4]; 18];
    for (slot, v) in vr.iter_mut().zip(registers) {
        *slot = unsafe { lanes_ps(v) };
    }
    Ok(AccumulateClobbers { vr })
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn lanes_ps(v: __m128) -> [u32; 4] {
    let mut out = [0u32; 4];
    unsafe { _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, _mm_castps_si128(v)) };
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const DST: u32 = BASE + 0x0200;
    const SRC: u32 = BASE + 0x1800;
    const RODATA: u32 = 0x8206_0000;

    /// A guest map holding the four rodata singles and the eight pool vectors, with the values
    /// read out of the validated image dump.
    fn image() -> Guest {
        let mut g = Guest::from_segments(vec![
            crate::Segment {
                base: BASE,
                bytes: vec![0u8; 0x4000],
            },
            // 0x82060C50 and 0x82063B08.
            crate::Segment {
                base: RODATA,
                bytes: vec![0u8; 0x4000],
            },
            // 0x82257308.
            crate::Segment {
                base: 0x8225_7000,
                bytes: vec![0u8; 0x1000],
            },
            // 0x820ED958.
            crate::Segment {
                base: 0x820E_D000,
                bytes: vec![0u8; 0x1000],
            },
            // 0x8231BA20 .. 0x8231BA9F.
            crate::Segment {
                base: 0x8231_BA00,
                bytes: vec![0u8; 0x100],
            },
        ]);
        g.set_u32(STEP_SCALE, 4.0f32.to_bits()).unwrap();
        g.set_u32(RAMP_SPAN, 64.0f32.to_bits()).unwrap();
        g.set_u32(LANE2_SCALE, 2.0f32.to_bits()).unwrap();
        g.set_u32(LANE3_SCALE, 3.0f32.to_bits()).unwrap();
        for group in 1..8u32 {
            splat(&mut g, SCALE[group as usize], group as f32);
        }
        splat(&mut g, SCALE_STEP, 8.0);
        g
    }

    fn splat(g: &mut Guest, base: u32, v: f32) {
        for i in 0..4 {
            g.set_u32(base + 4 * i, v.to_bits()).unwrap();
        }
    }

    fn put(g: &mut Guest, base: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            g.set_u32(base + 4 * i as u32, v.to_bits()).unwrap();
        }
    }

    fn get(g: &Guest, base: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| g.f32(base + 4 * i as u32).unwrap())
            .collect()
    }

    fn source() -> Vec<f32> {
        (0..256).map(|i| ((i % 17) as f32) * 0.25 - 2.0).collect()
    }

    /// An independent model, written from what the kernel is *for*: 256 singles scaled by a gain
    /// that ramps for 64 samples and then holds. The rounding structure is reproduced — `fmuls`,
    /// `fadds` and `fmadds` narrow to single, `fmadds` rounds once, and the vector multiply-adds round
    /// twice, as the recomp's unfused `vmaddfp` does (corrected 2026-09-13) — but nothing
    /// about the loop shape, the group multipliers or the lane order is taken from the body.
    fn model(src: &[f32], f1: f64, f2: f64) -> Vec<f32> {
        let step = ((f2 * 4.0) as f32) as f64;
        let held = f2.mul_add(64.0, f1) as f32;
        let start = [
            f2.mul_add(3.0, f1) as f32, // host lane 0 = sample 3
            f2.mul_add(2.0, f1) as f32,
            (f1 + f2) as f32,
            f1 as f32, // host lane 3 = sample 0
        ];

        let mut out = vec![0f32; 256];
        let mut gain = start;
        for block in 0..RAMP_BLOCKS {
            let next = gain.map(|l| 8.0f32 * step as f32 + l);
            for group in 0..VECTORS_PER_BLOCK {
                let gg = if group == 0 {
                    gain
                } else {
                    gain.map(|l| group as f32 * step as f32 + l)
                };
                for j in 0..4 {
                    let k = block * 32 + group * 4 + j;
                    out[k] = gg[3 - j] * src[k]; // sample j of a group is host lane 3-j
                }
            }
            gain = next;
        }
        for k in 64..256 {
            out[k] = held * src[k];
        }
        out
    }

    /// [`model`] evaluated under the mode the kernel runs in — both of its modes carry `FZ|DAZ`,
    /// so a model run outside them can keep denormals the kernel flushes.
    fn model_ftz(src: &[f32], f1: f64, f2: f64) -> Vec<f32> {
        let mut f = Fpscr::capture();
        f.enable_flush_mode_unconditional();
        let r = model(src, f1, f2);
        drop(f);
        r
    }

    #[test]
    fn the_rodata_addresses_are_computed_from_the_lis_immediates() {
        assert_eq!(STEP_SCALE, 0x8225_0000 + 29448);
        assert_eq!(RAMP_SPAN, 0x820F_0000 - 9896);
        assert_eq!(LANE2_SCALE, 0x8206_0000 + 3152);
        assert_eq!(LANE3_SCALE, 0x8206_0000 + 15112);
        // The pool is eight consecutive 16-byte vectors, and the group order is *not* the address
        // order: groups 5, 6, 7 and the block step come first, then groups 1 to 4.
        assert_eq!(SCALE[5], 0x8232_0000 - 17888);
        assert_eq!(SCALE[1], 0x8232_0000 - 17824);
        let mut addresses: Vec<u32> = SCALE[1..].to_vec();
        addresses.push(SCALE_STEP);
        addresses.sort_unstable();
        assert_eq!(addresses[0], 0x8231_BA20);
        assert_eq!(addresses[7], 0x8231_BA90);
        for pair in addresses.windows(2) {
            assert_eq!(pair[1] - pair[0], 16, "the pool is contiguous");
        }
    }

    #[test]
    fn the_gain_ramps_for_64_samples_then_holds() {
        // The headline behaviour. A source of all ones makes the output *be* the gain curve.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        gain_ramp_copy(&mut g, DST, SRC, 0.0, 0.01).unwrap();
        let out = get(&g, DST, 256);
        for k in 0..64 {
            assert!(
                (out[k] - k as f32 * 0.01).abs() < 1e-5,
                "sample {k}: gain {} should be k*0.01",
                out[k]
            );
        }
        let held = 64.0 * 0.01f32;
        for k in 64..256 {
            assert!(
                (out[k] - held).abs() < 1e-5,
                "sample {k}: gain {} should be held",
                out[k]
            );
        }
        // The ramp is strictly increasing over its 64 samples and then dead flat, which is what
        // distinguishes it from a ramp that runs the whole block.
        assert!(out[63] > out[32] && out[32] > out[0]);
        assert_eq!(out[64], out[255]);
    }

    #[test]
    fn the_per_lane_starting_gains_are_in_the_reversed_order() {
        // This is the `stfs`-then-`lvx128` fact the whole per-sample ordering rests on: the single
        // written lowest comes back as host lane 3, which is sample 0 of the group. If the four
        // were built in host order instead, samples 0..3 would get gains 3f2, 2f2, f2, 0.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        gain_ramp_copy(&mut g, DST, SRC, 100.0, 1.0).unwrap();
        let out = get(&g, DST, 256);
        assert_eq!(
            &out[0..4],
            &[100.0, 101.0, 102.0, 103.0],
            "ascending, not descending"
        );
        // And across a group boundary: group 1 continues from 104, so the group multipliers are
        // applied to the right groups too.
        assert_eq!(&out[4..8], &[104.0, 105.0, 106.0, 107.0]);
        // Group 7 of block 0, then group 0 of block 1 — the whole-block step.
        assert_eq!(&out[28..32], &[128.0, 129.0, 130.0, 131.0]);
        assert_eq!(&out[32..36], &[132.0, 133.0, 134.0, 135.0]);
    }

    /// The strongest test here: an independently written model compared on the bits.
    ///
    /// **Four of the seventeen multiply-adds are invisible to it, and that is arithmetic rather
    /// than a weak test.** The group multipliers are splats of 1, 2, 3, 4, 5, 6, 7 and the
    /// whole-block step is 8, and a product by a power of two is exact — so at groups 1, 2, 4 and at
    /// the block step, `fma(c, step, gain)` and `fl(fl(c*step) + gain)` are the *same function*, not
    /// two answers that happen to agree. Measured, not assumed: breaking group 1 or the block step
    /// into a separate multiply and add leaves every test in this crate passing, while the same
    /// break at group 3 or group 5 fails this one. (Measured when the layer was fused.) Those four
    /// sites go through the same `vmx::vmaddfp` as the rest, and nothing here would catch it if
    /// they did not.
    #[test]
    fn it_matches_the_independent_model_bit_for_bit() {
        let src = source();
        for (f1, f2) in [
            (0.0, 0.001),
            (1.0, -0.0025),
            (0.5, 0.0),
            (-2.0, 0.125),
            (0.25, 1e-5),
        ] {
            let mut g = image();
            put(&mut g, SRC, &src);
            gain_ramp_copy(&mut g, DST, SRC, f1, f2).unwrap();
            assert_eq!(
                get(&g, DST, 256),
                model_ftz(&src, f1, f2),
                "f1 = {f1}, f2 = {f2}"
            );
        }
    }

    #[test]
    fn a_zero_step_is_a_plain_scaled_copy() {
        let src = source();
        let mut g = image();
        put(&mut g, SRC, &src);
        gain_ramp_copy(&mut g, DST, SRC, 0.5, 0.0).unwrap();
        assert_eq!(
            get(&g, DST, 256),
            src.iter().map(|x| x * 0.5).collect::<Vec<_>>()
        );
    }

    #[test]
    fn it_touches_exactly_1024_bytes_and_the_length_is_not_a_parameter() {
        let mut g = image();
        put(&mut g, DST, &vec![7.0f32; 512]); // twice as much destination as it will write
        put(&mut g, SRC, &vec![1.0f32; 512]); // twice as much source as it will read
        gain_ramp_copy(&mut g, DST, SRC, 1.0, 0.0).unwrap();
        assert_eq!(get(&g, DST, 256), vec![1.0f32; 256]);
        assert_eq!(
            get(&g, DST + 1024, 256),
            vec![7.0f32; 256],
            "nothing past 1024 bytes"
        );
        assert_eq!(TOTAL_BYTES, 1024);
    }

    #[test]
    fn every_block_is_line_cleared_before_it_is_written() {
        // Eight `dcbzl`, one per 128-byte block. With a 128-byte aligned destination the clear is
        // invisible because the eight stores cover it — so the visible case is an *unaligned*
        // destination, where the clear reaches backwards to the start of the line.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        let unaligned = DST + 16; // 16-byte aligned, not 128
        assert_ne!(unaligned & 127, 0);
        put(&mut g, DST, &vec![7.0f32; 4]); // the 16 bytes below `unaligned`, inside its line
        gain_ramp_copy(&mut g, unaligned, SRC, 1.0, 0.0).unwrap();
        assert_eq!(
            get(&g, DST, 4),
            vec![0.0f32; 4],
            "the dcbzl cleared the head of the line"
        );
        // And it is 128 bytes, not 32 or 64. A destination 124 bytes into a line has its whole
        // line cleared; the first `stvx128` then masks its own address back down to `DST + 112`,
        // so the last four floats of that line are output rather than zero. Both halves of that
        // sentence are the address masking working, and getting either wrong shows up here.
        let mut h = image();
        put(&mut h, SRC, &vec![1.0f32; 256]);
        let far = DST + 124;
        put(&mut h, DST, &vec![7.0f32; 31]);
        gain_ramp_copy(&mut h, far, SRC, 1.0, 0.0).unwrap();
        assert_eq!(
            get(&h, DST, 28),
            vec![0.0f32; 28],
            "the dcbzl reached back 124 bytes"
        );
        assert_eq!(
            get(&h, DST + 112, 4),
            vec![1.0f32; 4],
            "stvx128 masked its address to +112"
        );
    }

    #[test]
    fn the_clobbered_preserved_registers_are_the_pool_constants() {
        let mut g = image();
        put(&mut g, SRC, &vec![0.0f32; 256]);
        let c = gain_ramp_copy(&mut g, DST, SRC, 1.0, 0.1).unwrap();
        assert_eq!(c.v28, [8.0f32.to_bits(); 4], "v28 is the whole-block step");
        assert_eq!(c.v29, [7.0f32.to_bits(); 4]);
        assert_eq!(c.v30, [6.0f32.to_bits(); 4]);
        assert_eq!(c.v31, [5.0f32.to_bits(); 4]);
    }

    #[test]
    fn the_measured_pool_confirms_the_register_plumbing() {
        // The note for this port inferred the pool mapping from which register each `addi` fed and
        // said outright that no image dump had been taken. It has now: the multiplier for group g
        // really is a splat of g, so a call whose gain ramps by one per sample produces the integers
        // in order across all eight groups, which only holds if every address maps to its own group.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        gain_ramp_copy(&mut g, DST, SRC, 0.0, 1.0).unwrap();
        let out = get(&g, DST, 256);
        for group in 0..8 {
            assert_eq!(
                out[group * 4],
                (group * 4) as f32,
                "group {group}'s multiplier is not {group}"
            );
        }
    }

    #[test]
    fn a_block_reads_all_eight_vectors_before_it_writes_any() {
        // The ordering claim is **within a block**: eight `lvx128`, then the arithmetic, then the
        // `dcbzl` and eight `stvx128`. A destination 16 bytes *ahead* of the source puts block 0's
        // stores — and its line clear — straight on top of block 0's own source, so hoisting either
        // above the loads changes the answer.
        //
        // The first version of this test put the destination a whole block *behind* the source,
        // where every store lands on bytes the block has already consumed. It could not have failed,
        // and it passed. Recorded, because that is what a vacuous test looks like.
        let src = source();
        let mut g = image();
        put(&mut g, SRC, &src);
        assert_eq!(
            SRC & 127,
            0,
            "so the dcbzl line is exactly block 0's source"
        );
        gain_ramp_copy(&mut g, SRC + 16, SRC, 1.0, 0.0).unwrap();
        assert_eq!(
            get(&g, SRC + 16, 28),
            src[0..28],
            "block 0 must see the pre-call source in all eight vectors"
        );
        // The last four floats of block 0's output are *not* asserted, because block 1's `dcbzl`
        // takes them back: with the destination 16 bytes into a line, block 1 clears
        // `SRC+128 .. SRC+256` and only stores from `SRC+144`, so `SRC+128 .. SRC+144` is left
        // zeroed. That is the original's behaviour for an unaligned destination and it is checked
        // here rather than written around.
        assert_eq!(
            get(&g, SRC + 128, 4),
            vec![0.0f32; 4],
            "block 1's line clear reached back"
        );
        // Blocks 1 onward also read source bytes this call has already written, which the original
        // does too, so nothing is asserted about their output.
    }

    #[test]
    fn a_missing_constant_is_an_error_not_a_zero() {
        let mut bare = Guest::single(BASE, 0x1000);
        let err = gain_ramp_copy(&mut bare, DST, SRC, 1.0, 0.0).unwrap_err();
        assert_eq!(err.address, STEP_SCALE, "the first cell it reaches");
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        let before = vmx::get_mxcsr();
        gain_ramp_copy(&mut g, DST, SRC, 1.0, 0.01).unwrap();
        assert_eq!(vmx::get_mxcsr(), before);
    }

    // ================================================================ sub_82B44D18

    /// `dst` then `src`, both 256 singles, with few enough significant bits that **every product and
    /// every sum below is exact in single precision**.
    ///
    /// That is deliberate, and it is what these tests may and may not claim. Whether a `vmaddfp` rounds
    /// once or twice is [`crate::vmx`]'s question — it is under measurement, and the answer is owned
    /// there. With exact products the two readings are the *same function*, so every expected value
    /// here holds under either, and nothing in this module asserts one of them.
    fn accumulator() -> Vec<f32> {
        (0..256)
            .map(|i| ((i * 7 % 23) as f32) * 0.125 - 1.0)
            .collect()
    }

    /// The independent model: `dst[k] + src[k]*gain(k)` with `gain(k) = f1 + k*f2` for 64 samples and
    /// `f1 + 64*f2` after. Written from what the kernel is for, not from its loop shape.
    ///
    /// The multiply-add is written `a * b + c` in `f32` — a separate multiply and add, never
    /// `mul_add` — which is how the recomp's `vmaddfp` computes (built without `-mfma`, so SIMDe's
    /// fallback). Only the callers' exact inputs are used with it, and on those the product never rounds,
    /// so the model and the kernel agree whichever way `crate::vmx` lowers the operation. The callers
    /// assert that exactness rather than assume it.
    fn accumulate_model(dst: &[f32], src: &[f32], f1: f64, f2: f64) -> Vec<f32> {
        (0..256)
            .map(|k| {
                let gain = if k < 64 {
                    f1 + k as f64 * f2
                } else {
                    f1 + 64.0 * f2
                };
                src[k] * (gain as f32) + dst[k]
            })
            .collect()
    }

    /// True when `a*b` needs no rounding in single precision.
    fn exact_product(a: f32, b: f32) -> bool {
        (a as f64 * b as f64) as f32 as f64 == a as f64 * b as f64
    }

    /// Host lanes of a vector loaded from four guest words: `lvx128` reverses all sixteen bytes, so
    /// guest element `e` is host lane `3 - e`.
    fn lanes_of(words: &[f32]) -> [u32; 4] {
        [
            words[3].to_bits(),
            words[2].to_bits(),
            words[1].to_bits(),
            words[0].to_bits(),
        ]
    }

    #[test]
    fn the_accumulate_adds_the_ramped_source_onto_the_destination() {
        // The headline behaviour, in values that are exact under any rounding: a source of ones onto
        // a destination of tens, ramping by 1/64 a sample, so sample k gains k/64 for 64 samples and
        // then a flat 1.0.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        put(&mut g, DST, &vec![10.0f32; 256]);
        gain_ramp_accumulate(&mut g, DST, SRC, 0.0, 1.0 / 64.0).unwrap();
        let out = get(&g, DST, 256);
        for k in 0..64 {
            assert_eq!(out[k], 10.0 + k as f32 / 64.0, "sample {k}");
        }
        for k in 64..256 {
            assert_eq!(out[k], 11.0, "sample {k} holds");
        }
    }

    #[test]
    fn accumulate_matches_the_independent_model_on_exact_products() {
        let src = source();
        let dst = accumulator();
        for (f1, f2) in [
            (0.5, 1.0 / 1024.0),
            (1.0, -1.0 / 2048.0),
            (-2.0, 0.125),
            (0.25, 0.0),
            (3.0, 1.0 / 256.0),
        ] {
            // The guard this test's neutrality rests on, checked rather than assumed: every gain the
            // kernel forms is exact, and every product of a gain and a sample is exact.
            for k in 0..=64 {
                let gain = f1 + k as f64 * f2;
                assert_eq!(gain as f32 as f64, gain, "gain {k} is exact");
                for &s in &src {
                    assert!(exact_product(gain as f32, s), "gain {gain} * {s} is exact");
                }
            }
            let mut g = image();
            put(&mut g, SRC, &src);
            put(&mut g, DST, &dst);
            gain_ramp_accumulate(&mut g, DST, SRC, f1, f2).unwrap();
            assert_eq!(
                get(&g, DST, 256),
                accumulate_model(&dst, &src, f1, f2),
                "f1 = {f1}, f2 = {f2}"
            );
        }
    }

    #[test]
    fn there_is_no_line_clear_so_the_bytes_below_an_unaligned_destination_survive() {
        // The one write-set difference from gain_ramp_copy. A destination 16 bytes into a 128-byte
        // line: the copy's dcbzl would zero the 16 bytes below it; the accumulate must not.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        let unaligned = DST + 16;
        assert_ne!(unaligned & 127, 0);
        put(&mut g, DST, &vec![7.0f32; 4 + 256 + 4]);
        gain_ramp_accumulate(&mut g, unaligned, SRC, 1.0, 0.0).unwrap();
        assert_eq!(
            get(&g, DST, 4),
            vec![7.0f32; 4],
            "the head of the line is untouched"
        );
        assert_eq!(
            get(&g, unaligned, 256),
            vec![8.0f32; 256],
            "7 + 1*1 everywhere"
        );
        assert_eq!(
            get(&g, unaligned + 1024, 4),
            vec![7.0f32; 4],
            "nothing past 1024 bytes"
        );
    }

    #[test]
    fn an_unaligned_destination_is_accumulated_at_its_masked_address() {
        // stvx128 and lvx128 both mask to 16 bytes, so a destination 4 bytes into a vector reads and
        // writes the aligned run below it: the word below the pointer is accumulated and the last
        // three words of the nominal run are not touched.
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        put(&mut g, DST, &vec![7.0f32; 260]);
        gain_ramp_accumulate(&mut g, DST + 4, SRC, 1.0, 0.0).unwrap();
        assert_eq!(
            get(&g, DST, 256),
            vec![8.0f32; 256],
            "the aligned run below the pointer"
        );
        assert_eq!(
            get(&g, DST + 1024, 4),
            vec![7.0f32; 4],
            "the nominal tail is not written"
        );
    }

    #[test]
    fn a_ramp_block_reads_all_its_source_and_accumulator_before_it_stores() {
        // A destination one vector ahead of its source: block 0's first store lands on the source
        // vector group 1 reads. Every load of the block precedes every store in the original, so all
        // 32 outputs of block 0 are formed from pre-call values: out[k] = src[k+4] + src[k] at gain 1.
        let src = source();
        let mut g = image();
        put(&mut g, SRC, &src);
        put(&mut g, SRC + 1024, &vec![0.5f32; 64]); // what the destination reaches past the source
        gain_ramp_accumulate(&mut g, SRC + 16, SRC, 1.0, 0.0).unwrap();
        let out = get(&g, SRC + 16, 32);
        for k in 0..32 {
            assert_eq!(out[k], src[k + 4] + src[k], "block 0, sample {k}");
        }
    }

    #[test]
    fn the_clobbered_vector_registers_are_the_pool_and_the_second_ramp_blocks_values() {
        // Every one of v14-v31 is named, and the ten data-dependent ones are checked against values
        // computed here from the inputs. Exact dyadic inputs again, so rounding cannot enter.
        let src = source();
        let dst = accumulator();
        let (f1, f2) = (0.5f64, 1.0 / 1024.0);
        let mut g = image();
        put(&mut g, SRC, &src);
        put(&mut g, DST, &dst);
        let c = gain_ramp_accumulate(&mut g, DST, SRC, f1, f2).unwrap();
        let out = get(&g, DST, 256);

        for (n, value) in [
            (14, 8.0f32),
            (15, 7.0),
            (16, 6.0),
            (17, 5.0),
            (18, 4.0),
            (19, 3.0),
            (20, 2.0),
            (21, 1.0),
        ] {
            assert_eq!(c.v(n), [value.to_bits(); 4], "v{n} is a pool multiplier");
        }
        // The second ramp block covers samples 32..64, i.e. destination words 32..64.
        assert_eq!(
            c.v(22),
            lanes_of(&dst[32 + 12..32 + 16]),
            "v22: the accumulator at +48"
        );
        assert_eq!(
            c.v(23),
            lanes_of(&dst[32 + 8..32 + 12]),
            "v23: the accumulator at +32"
        );
        assert_eq!(
            c.v(24),
            lanes_of(&out[32 + 4..32 + 8]),
            "v24: group 1's result"
        );
        assert_eq!(c.v(25), lanes_of(&out[32..32 + 4]), "v25: group 0's result");
        let gain = |k: usize| (f1 + k as f64 * f2) as f32;
        let gains = |first: usize| {
            lanes_of(&[
                gain(first),
                gain(first + 1),
                gain(first + 2),
                gain(first + 3),
            ])
        };
        assert_eq!(
            c.v(26),
            gains(64),
            "v26: where a third ramp block would start"
        );
        for (n, group) in [(27, 7usize), (28, 6), (29, 5), (30, 4), (31, 3)] {
            assert_eq!(c.v(n), gains(32 + 4 * group), "v{n}: group {group}'s gain");
        }
    }

    #[test]
    fn accumulate_a_missing_constant_is_an_error_not_a_zero() {
        let mut bare = Guest::single(BASE, 0x1000);
        let err = gain_ramp_accumulate(&mut bare, DST, SRC, 1.0, 0.0).unwrap_err();
        assert_eq!(err.address, STEP_SCALE, "the first cell it reaches");
    }

    #[test]
    fn accumulate_restores_the_entry_flush_mode() {
        let mut g = image();
        put(&mut g, SRC, &vec![1.0f32; 256]);
        let before = vmx::get_mxcsr();
        gain_ramp_accumulate(&mut g, DST, SRC, 1.0, 0.01).unwrap();
        assert_eq!(vmx::get_mxcsr(), before);
    }
}
