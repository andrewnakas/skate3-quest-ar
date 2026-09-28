//! One mixer filter stage: the five-gain kernel, and the dispatcher that feeds it.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`one_pole_stage`] | `sub_82B399D0` | verified | 901 | 303,420 | 359,364 |
//! | [`run_stage`] | `sub_82B39FA0` | verified | 92 | 151,710 | 179,682 |
//!
//! Both `.inc` headers lead with `// STATUS: verified` and `docs/ports.md` agrees. `sub_82B399D0` is
//! **901 lifted lines**, the largest body translated into this crate, and its C++ port measured zero
//! divergence over 141,462 comparisons with `skipped=0`.
//!
//! **They share a file because they cannot be separated.** `sub_82B399D0` has no `stwu` of its own and
//! reads three of its arguments off `84(r1)`, `92(r1)` and `100(r1)` — which is `sub_82B39FA0`'s frame.
//! Porting the kernel alone would have meant inventing a signature for those three, and porting the
//! dispatcher alone would have left its only interesting path a stub. Here [`run_stage`] builds the
//! frame and [`one_pole_stage`] reads it out of guest memory, exactly as the pair does in the original,
//! so the coupling is exercised rather than modelled.
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green: the C++ was
//! compared call-for-call against the original under the shadow harness on real inputs at zero
//! divergence, and the Rust has no recorded vectors of its own. A fault here is a transcription error
//! rather than a misreading of the kernel; it is not a number.
//!
//! ## What the kernel computes
//!
//! Three interleaved passes over the same run, which is why reading the lifted body linearly is so
//! hard. Written out, with `i` the sample index and the operations in the precision each uses:
//!
//! ```text
//! mixed[i] = addend[i] - feed·source[i+1]                    f32 vnmsubfp, four lanes at a time
//! y[i]     = mixed[i] - pole·y[i-1]                          f64 fnmsubs, in place over mixed
//! copy[i] += bus_gain·(bus_coeff·source[i] + source[i+1])     f32 vmaddfp, two of them
//! ```
//!
//! `y[-1]` is the `f5` argument, and the result is `y[count-1]` — **read back out of memory**, not kept
//! in a register. [`run_stage`] latches it into its gain block's `+32`, which is where the next call's
//! `f5` comes from, so the recursion is continuous across calls.
//!
//! `source` is read at both `i` and `i+1`, so it holds `count + 1` singles. Every group is therefore
//! built from **two** `lvx128` and a `vperm128`: the run is only 4-byte aligned in the census
//! (`0x…388` and `0x…3C0` both appear), so an aligned vector load of it would be at the wrong offset.
//!
//! ## The software pipeline, and why the covering works out
//!
//! The kernel writes the first sixteen samples of `mixed` before its loop, and each iteration then
//! (a) runs the scalar recursion over the sixteen samples the *previous* pass wrote, (b) accumulates
//! the bus from the *previous* pass's source windows, and (c) computes the *next* sixteen `mixed`
//! samples and new windows. A tail block after the loop finishes the last sixteen.
//!
//! For a count that is a multiple of sixteen the three cursors tile the run exactly:
//! `blocks = count/4 - 4`, `iters = ceil(blocks/4) = count/16 - 1`, and the tail sits at
//! `16·blocks = count - 16`. Every sample of `mixed` and of `copy` is visited once. **For any other
//! count they do not tile** — a count of 20 gives one iteration covering samples 0…15, a `mixed`
//! write of samples 16…31 past the end of the run, and a tail that re-filters samples 4…19. That is
//! reproduced rather than guarded, and it is why the tests here only model multiples of sixteen: the
//! census logs `count = 0x100` on every call.
//!
//! ## Flush-to-zero is toggled twenty-one times mid-body
//!
//! `docs/vmx128-exactness.md` rule 2, at its sharpest. The recursion's `fnmsubs` and the bus's
//! `vmaddfp` are **interleaved instruction by instruction**, and the lifted body emits a
//! `disableFlushMode` before each scalar operation and an `enableFlushMode` before each vector one. A
//! single global choice would flush a denormal the recomp keeps, or keep one it flushes, on any sample
//! near the bottom of the range. Every toggle is reproduced at its own site through
//! [`crate::vmx::Fpscr`], including the guarded `enable_flush_mode`/`disable_flush_mode` forms where
//! the lifted line uses those rather than the unconditional ones.
//!
//! ## Rule 4, once
//!
//! Every `vnmsubfp` takes the `feed` **splat** as its first operand and every `vmaddfp` takes the
//! window or the product first, as lifted. That is necessary and not sufficient: rule 4 records that
//! with two NaN operands the surviving payload is a register-allocation decision in clang-20 as much as
//! in GCC, so it cannot be derived from source in either language. If NaN reaches this kernel its
//! output is not predictable from this code *or* from the C++.
//!
//! ## Reproduced rather than fixed
//!
//! - **The 32-byte permute table is rebuilt in `.data` on every call**, at the fixed address
//!   [`PERM_TABLE`], and read straight back through four `lwzx` to form each control. It is a real
//!   write to guest memory — the C++ `Windows()` declares it — and it is a write to a *shared* address,
//!   so two threads in this kernel at once would race. Written as eight `stw` in the lifted order,
//!   which is not ascending.
//! - **`dcbzl` is a real 128-byte store**, not a hint, and it starts from the 128-byte **floor** of
//!   `copy` and covers `ceil(4·count / 128)` whole lines — so it can zero bytes past the end of the
//!   run. `dcbt`, which sits next to it in the same loops, *is* a hint and lifts to nothing.
//! - **The state for each block is reloaded from memory** (`lfsu f0,64(r11)`) rather than carried in a
//!   register, so a misaligned `mixed` whose `stvx128` clobbered the low bytes of `run+60` would feed
//!   the next block the clobbered value.
//! - **The second single of each block is stored before the first** (`stfs f0,8(r11)` then
//!   `stfs f12,4(r11)`).
//!
//! ## Not reproduced, and one of them is a real output
//!
//! The kernel's own frame — three `stfd` of `f29`-`f31` at `-96`…`-112(r1)` and `__savegprlr_22`'s
//! spill at `-8`…`-88(r1)` — is not reproduced. The body leaves those registers untouched, so the
//! epilogue's restores are no-ops, and there is no register file here in any case.
//!
//! **`v25`-`v31` are a different matter.** The kernel clobbers seven vector registers in the
//! ABI-preserved range without saving any, and the C++ port tracks exactly what the last block through
//! the pipeline left in each so that the harness's unconditional `v25`-`v31` comparison passes. This
//! crate has no vector register file, so **that output is simply absent here** — a difference from the
//! reference, listed rather than buried. It costs nothing for replay: the vector recorder captures
//! `r3`…`r8`, `f1`…`f4` and the returned `f1`, and the kernel's mask is `kReturnF1`, so `f1` is the
//! whole of what a recorded comparison can check.
//!
//! [`run_stage`]'s two register spills (`stw r12,-8(r1)` for the link register and `std r31,-16(r1)`)
//! are absent for the same reason. Its frame's **back chain** is written, because that value is
//! `entry_sp` and this crate does have it.

#![allow(unused_unsafe)] // see the note at the top of `crate::vmx`

use crate::vmx::{self, Fpscr};
use crate::{Guest, Result, fp, mem};
use core::arch::x86_64::*;

// ------------------------------------------------------- sub_82B399D0: the three stack arguments

/// `lwz r8,84(r1)` — the in/out run, which is the descriptor's `+16`.
pub const ARG_MIXED: u32 = 84;
/// `lwz r26,92(r1)` — the accumulate bus, the descriptor's `+20`.
pub const ARG_COPY: u32 = 92;
/// `lwz r11,100(r1)` — **zero means establish the bus as zeroes first.** [`run_stage`] puts its own
/// `r5` here, which its `.inc` calls an opaque pass-through; from the kernel's side it is the clear
/// flag, and that is worth saying once in plain words because the two names describe one word.
pub const ARG_CLEAR: u32 = 100;

const _: () = assert!(
    ARG_COPY == ARG_MIXED + 8 && ARG_CLEAR == ARG_COPY + 8,
    "84, 92, 100"
);

/// `((imm & 0xFFFF) << 16)` — the `lis` half of an address, **computed**.
const fn lis(imm: i32) -> u32 {
    ((imm as u32) & 0xFFFF) << 16
}
const _: () = assert!(lis(-31987) == 0x830D_0000, "lis r11,-31987");

/// `lis -31987 ; stw 5412` — the 32-byte permute table the kernel rebuilds on every call.
///
/// A **fixed** `.data` address, not a frame slot, computed as `((imm & 0xFFFF) << 16) + offset` and
/// asserted below rather than read off the disassembly. It holds the byte indices `0x00`…`0x1F` and is
/// the compiler's inline stand-in for `lvsl`: indexing it by `source & 0xC` produces the two
/// `vperm128` controls that build an unaligned window out of two aligned lines.
pub const PERM_TABLE: u32 = lis(-31987).wrapping_add(5412);
const _: () = assert!(PERM_TABLE == 0x830D_1524, "lis -31987 ; stw 5412(r6)");
/// Eight words of byte indices.
pub const PERM_TABLE_BYTES: u32 = 32;

/// 16-byte groups per block: the four cursors `r10`/`r5`/`r29`/`r28` take off `source`.
pub const GROUPS: usize = 4;
/// One loop iteration: `addi r7,r7,64` and friends.
pub const BLOCK_BYTES: u32 = 64;
/// The scalar recursion's unrolled run length.
pub const BLOCK_SINGLES: usize = 16;

const _: () = assert!(BLOCK_BYTES as usize == 4 * BLOCK_SINGLES);
const _: () = assert!(BLOCK_SINGLES == 4 * GROUPS);

// ----------------------------------------------------------- sub_82B39FA0: the gain block

/// `lfs f1,16(r31)`; also the address `stfs f1,32(r31)` latches the kernel's result into.
pub const GAIN0: u32 = 16;
/// `lfs f2,20(r31)` — the kernel's `feed`.
pub const GAIN1: u32 = 20;
/// `lfs f3,24(r31)` — the kernel's `bus_coeff`.
pub const GAIN2: u32 = 24;
/// `lfs f4,28(r31)` — the kernel's `bus_gain`.
pub const GAIN3: u32 = 28;
/// `lfs f5,32(r31)` — the recursion's carried `y[-1]`, and the **only** word written back. Running
/// state, not a parameter.
pub const GAIN_STATE: u32 = 32;
/// Five singles at `+16`…`+32`, all read, one written.
pub const GAIN_BYTES: u32 = 20;

/// `lwz r9,0(r7)` — the first input run.
pub const DESC_ADDEND: u32 = 0;
/// `lwz r10,4(r7)` — the second, read at `i` **and** `i+1`.
pub const DESC_SOURCE: u32 = 4;
/// `lwz r11,8(r7)` — zero picks the kernel; anything else clears the `+20` buffer instead.
pub const DESC_SELECT: u32 = 8;
/// `lwz r8,16(r7)` — the in/out run, handed on as `84(r1)`.
pub const DESC_MIXED: u32 = 16;
/// `lwz r11,20(r7)` — the accumulate bus, handed on as `92(r1)`, and the buffer the other path clears.
pub const DESC_COPY: u32 = 20;
/// The whole descriptor, which is what the C++ `Windows()` declares as a read span.
pub const DESC_BYTES: u32 = 24;

/// `stwu r1,-128(r1)`.
pub const FRAME_BYTES: u32 = 128;

// ------------------------------------------------------------------------------ shared idioms

/// `srawi r6,r3,2 ; addze r11,r6` — `count / 4` truncated **toward zero**.
///
/// The `srawi`'s carry-out is the rounding correction and `addze` folds it back in, which is a signed
/// division and not `>> 2`: a negative count divides toward zero, not down.
fn quarter_to_zero(count: i32) -> i32 {
    let carry = count < 0 && (count & 3) != 0;
    (count >> 2) + if carry { 1 } else { 0 }
}

/// Four `lwzx` off [`PERM_TABLE`], assembled into a `vperm128` control register.
///
/// The original writes the four words into its frame with `stw` and reads them back with one `lvx128`.
/// `stw` is big-endian and `lvx128` reverses all sixteen bytes, so the word stored at the **lowest**
/// address lands in host lane 3 — which is what `_mm_set_epi32(w0, w1, w2, w3)` produces. That lane
/// order is `sub_82B3C098`'s verified precedent, and it is why the control is built as a register here
/// and the guest stack is never written.
///
/// # Safety
/// Requires SSE4.1.
#[target_feature(enable = "sse4.1")]
unsafe fn perm_control(g: &Guest, offset: u32) -> Result<__m128i> {
    let w0 = g.u32(PERM_TABLE.wrapping_add(offset))? as i32; // lwzx r11,r4,r11
    let w1 = g.u32(PERM_TABLE.wrapping_add(offset).wrapping_add(4))? as i32; // lwzx r25,r4,r27
    let w2 = g.u32(PERM_TABLE.wrapping_add(offset).wrapping_add(8))? as i32; // lwzx r7,r4,r7
    let w3 = g.u32(PERM_TABLE.wrapping_add(offset).wrapping_add(12))? as i32; // lwzx r6,r4,r31
    Ok(unsafe { _mm_set_epi32(w0, w1, w2, w3) })
}

/// `vperm128 vD,vLo,vHi,vControl` — the sixteen bytes starting `offset` into the pair, as float lanes.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn window(lo: __m128i, hi: __m128i, control: __m128i) -> __m128 {
    unsafe { _mm_castsi128_ps(vmx::perm_epi8(lo, hi, control)) }
}

// ================================================== sub_82B399D0: the five-gain kernel

/// `sub_82B399D0` — one filter stage over `count` singles. Returns the guest's `f1`.
///
/// Arguments, by register. Only these are live on entry; `r4`-`r8` are passed by [`run_stage`] and
/// never read (`r8` is reloaded from `84(r1)`, which holds the same value).
///
/// | register | parameter | what it is |
/// |---|---|---|
/// | `r3` | `count` | singles to filter — `0x100` in every logged call |
/// | `r9` | `addend` | the first input run, read only |
/// | `r10` | `source` | the second, read at `i` **and** `i+1`, so `count + 1` singles |
/// | `r1` | `sp` | the **caller's** frame; `84`/`92`/`100` off it are arguments seven to nine |
/// | `f1` | `pole` | the one-pole coefficient. `f1` is also the result register |
/// | `f2` | `feed` | scales `source[i+1]` out of `addend[i]` |
/// | `f3` | `bus_coeff` | scales `source[i]` into the bus product |
/// | `f4` | `bus_gain` | scales the bus product into the copy bus |
/// | `f5` | `state` | the recursion's `y[-1]` |
///
/// `count`, `addend` and `source` arrive as full 64-bit registers and are truncated here exactly where
/// the original truncates them — `ctx.r3.s32`, `ctx.r9.u32`, `ctx.r10.u32` — so a replay passes the
/// recorded register unmodified. `sp` is a `u32` because the original never touches its high half.
///
/// **The mask is `kReturnF1`**, and the returned value is `mixed[count-1]` loaded back **out of
/// memory** after the last store. It is the only register the caller reads.
///
/// **Writes:** [`PERM_TABLE`]'s 32 bytes; `mixed` over the union of the first block at its 16-byte
/// floor, sixteen unmasked `stfs` plus four masked `stvx128` per iteration, and sixteen `stfs` at
/// `mixed + 16·blocks`; and `copy` over four `stvx128` per iteration, four for the tail block, and —
/// when the clear flag is zero — `ceil(4·count / 128)` whole 128-byte lines from `copy & ~127`.
/// **Reads:** the three stack arguments as one 20-byte span at `sp + 84`, [`PERM_TABLE`] straight back,
/// `mixed` and `copy` over the spans it writes, four `lvx128` of `addend` per block, and five lines of
/// `source` per block — one past the block, because every group is read a single further along.
///
/// Nothing outside those is read. There is no rodata constant anywhere in this body.
#[allow(clippy::too_many_arguments)]
pub fn one_pole_stage(
    g: &mut Guest,
    count: u64,
    addend: u64,
    source: u64,
    sp: u32,
    pole: f64,
    feed: f64,
    bus_coeff: f64,
    bus_gain: f64,
    state: f64,
) -> Result<f64> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    unsafe {
        kernel(
            g,
            count as u32 as i32,
            addend as u32,
            source as u32,
            sp,
            pole,
            feed,
            bus_coeff,
            bus_gain,
            state,
        )
    }
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn kernel(
    g: &mut Guest,
    count: i32,
    addend: u32,
    source: u32,
    sp: u32,
    pole: f64,
    feed: f64,
    bus_coeff: f64,
    bus_gain: f64,
    state_in: f64,
) -> Result<f64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at stfd f29,-112(r1)
    let clear = g.u32(sp.wrapping_add(ARG_CLEAR))?; // lwz r11,100(r1)
    let mut state = state_in; // fmr f0,f5
    let copy = g.u32(sp.wrapping_add(ARG_COPY))?; // lwz r26,92(r1)
    let span = ((count as u32) << 2) & 0xFFFF_FFFC; // rlwinm r22,r3,2,0,29
    let mixed = g.u32(sp.wrapping_add(ARG_MIXED))?; // lwz r8,84(r1)

    // cmpwi cr6,r11,0 ; bne cr6 — a non-zero flag takes loc_82B39A2C, which is eight `dcbt`: prefetch
    // hints only, no state, so that branch lifts to nothing at all. Zero means this is the first tap
    // into the bus and loc_82B39A0C establishes it as zeroes, a 128-byte line at a time.
    if clear == 0 {
        // cmplwi cr6,r22,0 ; beq cr6 guards the loop, which steps r11 by 128 while r11 <u r22.
        let mut off: u32 = 0;
        while off < span {
            vmx::dcbzl(g, copy.wrapping_add(off))?; // dcbzl r11,r26
            off = off.wrapping_add(128);
        }
    }

    // loc_82B39A5C. Three splats the original materialises through its own frame — four copies of
    // each single at -176/-144/-128(r1), each reloaded with one `lvx128`. The four words are
    // identical, so the load's lane reversal is a no-op on them and they are built as registers.
    fpscr.disable_flush_mode_unconditional(); // emitted at stfs f2,-176(r1)
    let feed_v = unsafe { _mm_set1_ps(feed as f32) }; // v63
    let bus_coeff_v = unsafe { _mm_set1_ps(bus_coeff as f32) }; // v13
    let bus_gain_v = unsafe { _mm_set1_ps(bus_gain as f32) }; // v12

    // Eight lis/ori pairs written into .data in the lifted order — 0, 4, 12, 16, 8, 24, 28, 20.
    for (off, word) in [
        (0u32, 0x0001_0203u32),
        (4, 0x0405_0607),
        (12, 0x0C0D_0E0F),
        (16, 0x1011_1213),
        (8, 0x0809_0A0B),
        (24, 0x1819_1A1B),
        (28, 0x1C1D_1E1F),
        (20, 0x1415_1617),
    ] {
        g.set_u32(PERM_TABLE.wrapping_add(off), word)?;
    }

    // rlwinm r4,r10,0,28,29 — the source run is only 4-byte aligned, so every read of it is an
    // unaligned window built from two lines.
    let sel = source & 0xC;
    let shift_hi = unsafe { perm_control(g, sel + 4)? }; // v0: the window one single further on
    let shift_lo = unsafe { perm_control(g, sel)? }; // v7: the window at the group

    // ---- the first block: sixteen singles of `mixed`, from `addend` and the source windows ----
    let mut line = [unsafe { _mm_setzero_si128() }; GROUPS + 1];
    for (i, slot) in line.iter_mut().enumerate() {
        // lvx128 v59/v62/v61/v60/v58 at source + 0, 16, 32, 48, 64
        *slot = unsafe { vmx::lvx128(g, source.wrapping_add(16 * i as u32))? };
    }

    // win_hi[j] is source[16j + 4 .. 16j + 19]; win_lo[j] is source[16j .. 16j + 15].
    let mut win_hi = [unsafe { _mm_setzero_ps() }; GROUPS];
    let mut win_lo = [unsafe { _mm_setzero_ps() }; GROUPS];
    for j in 0..GROUPS {
        win_hi[j] = unsafe { window(line[j], line[j + 1], shift_hi) }; // vperm128 v2/v1/v31/v30
    }
    for j in 0..GROUPS {
        win_lo[j] = unsafe { window(line[j], line[j + 1], shift_lo) }; // vperm128 v11/v10/v9/v8
    }

    let mut first = [unsafe { _mm_setzero_ps() }; GROUPS];
    for (j, slot) in first.iter_mut().enumerate() {
        *slot = unsafe { vmx::lvx128_ps(g, addend.wrapping_add(16 * j as u32))? }; // v28/v27/v26/v25
    }

    // vnmsubfp vD,vA,vB,vC = -(A·B) + C, one rounding. The splat is the FIRST operand of every one, as
    // lifted; rule 4 applies and the order is copied rather than normalised. The compute order is
    // 0, 2, 1, 3 and the store order is 0, 32, 16, 48.
    fpscr.enable_flush_mode(); // at vnmsubfp v28 — the *guarded* form, as lifted
    let mut mix = [unsafe { _mm_setzero_ps() }; GROUPS];
    mix[0] = unsafe { vmx::vnmsubfp(feed_v, win_hi[0], first[0]) };
    mix[2] = unsafe { vmx::vnmsubfp(feed_v, win_hi[2], first[2]) };
    mix[1] = unsafe { vmx::vnmsubfp(feed_v, win_hi[1], first[1]) };
    mix[3] = unsafe { vmx::vnmsubfp(feed_v, win_hi[3], first[3]) };

    unsafe { vmx::stvx128_ps(g, mixed, mix[0])? };
    unsafe { vmx::stvx128_ps(g, mixed.wrapping_add(32), mix[2])? };
    unsafe { vmx::stvx128_ps(g, mixed.wrapping_add(16), mix[1])? };
    unsafe { vmx::stvx128_ps(g, mixed.wrapping_add(48), mix[3])? };

    // srawi/addze/addi r3,r11,-4, then addi/rlwinm/addi/mtctr: ceil(blocks/4) iterations of 64 bytes.
    // `ble cr6` skips the loop entirely.
    let blocks = quarter_to_zero(count) - 4;
    let iters = if blocks > 0 {
        ((blocks as u32 - 1) >> 2) + 1
    } else {
        0
    };

    // ---- loc_82B39C4C: one block per iteration, pipelined one behind the windows ----
    for i in 0..iters {
        // r11 runs four bytes behind the block it filters, so `4+4k(r11)` is this run + 4k.
        let run = mixed.wrapping_add(BLOCK_BYTES.wrapping_mul(i));

        // The one-pole recursion over the sixteen singles the previous block wrote, with the four bus
        // products interleaved into it exactly as lifted. Every toggle is at its own site: a denormal
        // single loaded on the wrong side of one would flush.
        fpscr.disable_flush_mode_unconditional(); // at lfs f13,4(r11)
        let mut x = [0f64; BLOCK_SINGLES];
        let mut y = [0f64; BLOCK_SINGLES];
        x[0] = fp::load_single(g, run)?;
        fpscr.enable_flush_mode(); // at vmaddfp v10,v10,v13,v1
        let mut prod = [unsafe { _mm_setzero_ps() }; GROUPS];
        prod[1] = unsafe { vmx::vmaddfp(win_lo[1], bus_coeff_v, win_hi[1]) };
        fpscr.disable_flush_mode(); // at fnmsubs f12,f0,f1,f13
        y[0] = fp::nmsub_single(state, pole, x[0]);
        x[1] = fp::load_single(g, run.wrapping_add(4))?; // lfs f11,8(r11)
        x[2] = fp::load_single(g, run.wrapping_add(8))?; // lfs f10,12(r11)
        fpscr.enable_flush_mode(); // at vmaddfp v6,v11,v13,v2
        prod[0] = unsafe { vmx::vmaddfp(win_lo[0], bus_coeff_v, win_hi[0]) };
        fpscr.disable_flush_mode(); // at lfs f9,16(r11)
        x[3] = fp::load_single(g, run.wrapping_add(12))?;
        fpscr.enable_flush_mode(); // at vmaddfp v9,v9,v13,v31
        prod[2] = unsafe { vmx::vmaddfp(win_lo[2], bus_coeff_v, win_hi[2]) };
        fpscr.disable_flush_mode(); // at lfs f8,20(r11)
        x[4] = fp::load_single(g, run.wrapping_add(16))?;
        fpscr.enable_flush_mode(); // at vmaddfp v8,v8,v13,v30
        prod[3] = unsafe { vmx::vmaddfp(win_lo[3], bus_coeff_v, win_hi[3]) };
        fpscr.disable_flush_mode(); // at lfs f7,24(r11)

        // The last eleven loads. The original interleaves five of them after the first two stores
        // below; hoisted here because run+44..run+60 cannot alias run+0 or run+4, and because every
        // one of the sixteen singles is read before any of the sixteen is written either way.
        for k in 5..BLOCK_SINGLES {
            x[k] = fp::load_single(g, run.wrapping_add(4 * k as u32))?;
        }

        y[1] = fp::nmsub_single(y[0], pole, x[1]); // fnmsubs f0,f12,f1,f11
        fp::store_single(g, run.wrapping_add(4), y[1])?; // stfs f0,8(r11) — the second single first
        fp::store_single(g, run, y[0])?; // stfs f12,4(r11)
        for k in 2..BLOCK_SINGLES {
            y[k] = fp::nmsub_single(y[k - 1], pole, x[k]); // fnmsubs f10,f0,f1,f10 ...
            fp::store_single(g, run.wrapping_add(4 * k as u32), y[k])?; // stfs f10,12(r11) ...
        }

        // The next block. Thirteen `lvx128`, every one of them before this block's first store, which
        // is what makes an overlapping mixed/copy/addend/source behave as the original does.
        let src = source
            .wrapping_add(BLOCK_BYTES)
            .wrapping_add(BLOCK_BYTES.wrapping_mul(i));
        let bus = copy.wrapping_add(BLOCK_BYTES.wrapping_mul(i));
        let add = addend
            .wrapping_add(BLOCK_BYTES)
            .wrapping_add(BLOCK_BYTES.wrapping_mul(i));
        let out = mixed
            .wrapping_add(BLOCK_BYTES)
            .wrapping_add(BLOCK_BYTES.wrapping_mul(i));

        line[1] = unsafe { vmx::lvx128(g, src.wrapping_add(16))? }; // lvx128 v62,r0,r6
        line[2] = unsafe { vmx::lvx128(g, src.wrapping_add(32))? }; // lvx128 v61,r6,r29
        win_hi[1] = unsafe { window(line[1], line[2], shift_hi) }; // vperm128 v1,v62,v61,v0
        let mut acc = [unsafe { _mm_setzero_ps() }; GROUPS];
        acc[1] = unsafe { vmx::lvx128_ps(g, bus.wrapping_add(16))? }; // lvx128 v11,r7,r10
        fpscr.enable_flush_mode(); // at vmaddfp v5,v10,v12,v11
        let mut sum = [unsafe { _mm_setzero_ps() }; GROUPS];
        sum[1] = unsafe { vmx::vmaddfp(prod[1], bus_gain_v, acc[1]) };
        line[0] = unsafe { vmx::lvx128(g, src)? }; // lvx128 v59,r6,r25
        win_lo[1] = unsafe { window(line[1], line[2], shift_lo) }; // vperm128 v57,v62,v61,v7
        line[3] = unsafe { vmx::lvx128(g, src.wrapping_add(48))? }; // lvx128 v60,r6,r28
        line[4] = unsafe { vmx::lvx128(g, src.wrapping_add(64))? }; // lvx128 v58,r6,r27
        acc[3] = unsafe { vmx::lvx128_ps(g, bus.wrapping_add(48))? }; // lvx128 v10,r7,r30
        acc[0] = unsafe { vmx::lvx128_ps(g, bus)? }; // lvx128 v11,r7,r9
        sum[0] = unsafe { vmx::vmaddfp(prod[0], bus_gain_v, acc[0]) }; // vmaddfp v6,v6,v12,v11
        acc[2] = unsafe { vmx::lvx128_ps(g, bus.wrapping_add(32))? }; // lvx128 v11,r0,r7
        sum[2] = unsafe { vmx::vmaddfp(prod[2], bus_gain_v, acc[2]) }; // vmaddfp v4,v9,v12,v11
        let mut next = [unsafe { _mm_setzero_ps() }; GROUPS];
        next[0] = unsafe { vmx::lvx128_ps(g, add)? }; // lvx128 v3,r6,r24
        win_hi[0] = unsafe { window(line[0], line[1], shift_hi) }; // vperm128 v2,v59,v62,v0
        next[1] = unsafe { vmx::lvx128_ps(g, add.wrapping_add(16))? }; // lvx128 v29,r4,r10
        win_hi[2] = unsafe { window(line[2], line[3], shift_hi) }; // vperm128 v31,v61,v60,v0
        next[2] = unsafe { vmx::lvx128_ps(g, add.wrapping_add(32))? }; // lvx128 v28,r0,r4
        next[3] = unsafe { vmx::lvx128_ps(g, add.wrapping_add(48))? }; // lvx128 v27,r4,r30

        unsafe { vmx::stvx128_ps(g, bus, sum[0])? }; // stvx128 v6,r7,r9
        win_hi[3] = unsafe { window(line[3], line[4], shift_hi) }; // vperm128 v30,v60,v58,v0
        sum[3] = unsafe { vmx::vmaddfp(prod[3], bus_gain_v, acc[3]) }; // vmaddfp v26,v8,v12,v10
        unsafe { vmx::stvx128_ps(g, bus.wrapping_add(16), sum[1])? }; // stvx128 v5,r7,r10
        win_lo[0] = unsafe { window(line[0], line[1], shift_lo) }; // vperm128 v59,v59,v62,v7
        unsafe { vmx::stvx128_ps(g, bus.wrapping_add(32), sum[2])? }; // stvx128 v4,r0,r7
        mix[3] = unsafe { vmx::vnmsubfp(feed_v, win_hi[3], next[3]) }; // vnmsubfp v27,v6,v30,v27
        mix[0] = unsafe { vmx::vnmsubfp(feed_v, win_hi[0], next[0]) }; // vnmsubfp v25,v11,v2,v3
        win_lo[2] = unsafe { window(line[2], line[3], shift_lo) }; // vperm128 v56,v61,v60,v7
        mix[2] = unsafe { vmx::vnmsubfp(feed_v, win_hi[2], next[2]) }; // vnmsubfp v28,v9,v31,v28
        win_lo[3] = unsafe { window(line[3], line[4], shift_lo) }; // vperm128 v55,v60,v58,v7
        unsafe { vmx::stvx128_ps(g, bus.wrapping_add(48), sum[3])? }; // stvx128 v26,r7,r30
        mix[1] = unsafe { vmx::vnmsubfp(feed_v, win_hi[1], next[1]) }; // vnmsubfp v6,v4,v1,v29

        unsafe { vmx::stvx128_ps(g, out.wrapping_add(48), mix[3])? }; // stvx128 v27,r0,r31
        unsafe { vmx::stvx128_ps(g, out, mix[0])? }; // stvx128 v25,r6,r5
        unsafe { vmx::stvx128_ps(g, out.wrapping_add(32), mix[2])? }; // stvx128 v28,r4,r23
        unsafe { vmx::stvx128_ps(g, out.wrapping_add(16), mix[1])? }; // stvx128 v6,r31,r9

        // lfsu f0,64(r11) — the state for the next block is RELOADED from the single just stored at
        // run+60, not carried in a register.
        fpscr.disable_flush_mode();
        state = fp::load_single(g, run.wrapping_add(4 * (BLOCK_SINGLES as u32 - 1)))?;
    }

    // ---- loc_82B39DE8: the last block, sixteen singles at mixed + 16·blocks ----
    fpscr.enable_flush_mode_unconditional(); // at vmaddfp v6,v11,v13,v2
    let mut tail_prod = [unsafe { _mm_setzero_ps() }; GROUPS];
    tail_prod[0] = unsafe { vmx::vmaddfp(win_lo[0], bus_coeff_v, win_hi[0]) }; // vmaddfp v6
    tail_prod[2] = unsafe { vmx::vmaddfp(win_lo[2], bus_coeff_v, win_hi[2]) }; // vmaddfp v11
    tail_prod[1] = unsafe { vmx::vmaddfp(win_lo[1], bus_coeff_v, win_hi[1]) }; // vmaddfp v7
    tail_prod[3] = unsafe { vmx::vmaddfp(win_lo[3], bus_coeff_v, win_hi[3]) }; // vmaddfp v9

    // rlwinm r10,r3,4,0,27 ; add r10,r10,r8, and the same 16·blocks off `copy`. The shift is masked
    // and the adds are 32-bit, so the offset wraps exactly as the guest's does.
    let tail_off = ((blocks as u32) << 4) & 0xFFFF_FFF0;
    let last_run = mixed.wrapping_add(tail_off); // add r10,r10,r8
    let last_bus = copy.wrapping_add(tail_off); // add r11,r11,r26

    // Sixteen lfs/lfsx, sixteen fnmsubs, sixteen stfs/stfsx. The loads are hoisted: the original
    // interleaves them with the stores, but no single is ever read after it is written in this block.
    fpscr.disable_flush_mode(); // at lfs f13,0(r10)
    let mut x = [0f64; BLOCK_SINGLES];
    for (k, slot) in x.iter_mut().enumerate() {
        *slot = fp::load_single(g, last_run.wrapping_add(4 * k as u32))?;
    }
    for (k, xk) in x.iter().enumerate() {
        state = fp::nmsub_single(state, pole, *xk); // fnmsubs f12,f0,f1,f13 ...
        fp::store_single(g, last_run.wrapping_add(4 * k as u32), state)?; // stfs / stfsx
    }

    // The bus block, in the lifted load/compute/store order: +0, then +32 and +48, then +16.
    let acc0 = unsafe { vmx::lvx128_ps(g, last_bus)? }; // lvx128 v0,r0,r11
    fpscr.enable_flush_mode(); // at vmaddfp v10,v6,v12,v0
    let out0 = unsafe { vmx::vmaddfp(tail_prod[0], bus_gain_v, acc0) };
    let acc3 = unsafe { vmx::lvx128_ps(g, last_bus.wrapping_add(48))? }; // lvx128 v0,r0,r7
    let acc2 = unsafe { vmx::lvx128_ps(g, last_bus.wrapping_add(32))? }; // lvx128 v13,r0,r9
    let out2 = unsafe { vmx::vmaddfp(tail_prod[2], bus_gain_v, acc2) }; // vmaddfp v11,v11,v12,v13
    let out3 = unsafe { vmx::vmaddfp(tail_prod[3], bus_gain_v, acc3) }; // vmaddfp v13,v9,v12,v0
    unsafe { vmx::stvx128_ps(g, last_bus, out0)? }; // stvx128 v10,r0,r11
    unsafe { vmx::stvx128_ps(g, last_bus.wrapping_add(32), out2)? }; // stvx128 v11,r0,r9
    unsafe { vmx::stvx128_ps(g, last_bus.wrapping_add(48), out3)? }; // stvx128 v13,r0,r7
    let acc1 = unsafe { vmx::lvx128_ps(g, last_bus.wrapping_add(16))? }; // lvx128 v0,r0,r10
    let out1 = unsafe { vmx::vmaddfp(tail_prod[1], bus_gain_v, acc1) }; // vmaddfp v0,v7,v12,v0
    unsafe { vmx::stvx128_ps(g, last_bus.wrapping_add(16), out1)? }; // stvx128 v0,r0,r10

    // add r5,r22,r8 ; lfs f1,-4(r5) — the result is the LAST filtered single, read back out of memory.
    fpscr.disable_flush_mode();
    fp::load_single(g, mixed.wrapping_add(span).wrapping_sub(4))
}

// ================================================== sub_82B39FA0: the dispatcher

/// `sub_82B39FA0` — run [`one_pole_stage`] over a descriptor, or just clear the descriptor's `+20`
/// buffer.
///
/// Arguments, by register: `gains` is `r3` (the five-single gain block), `count` is `r4`, `opaque` is
/// `r5` — which becomes the kernel's clear flag at `100(r1)`, see [`ARG_CLEAR`] — `desc` is `r7`, and
/// `sp` is the entry `r1`. `r6` and `r8`-`r10` are not read on entry. There is no return value
/// (`kReturnNone`).
///
/// `count` and `opaque` are full 64-bit registers: `count` is handed to the kernel as its `r3` and also
/// drives the clear path's `rlwinm` on the low word, and `opaque` is stored as a word but passed along
/// as a register. `gains`, `desc` and `sp` are `u32` because the original addresses all three through
/// their low words.
///
/// **The selector at `+8` decides the path from entry state**, which is why the C++ `Windows()` can
/// enumerate both without running anything:
///
/// | `desc[8]` | what happens |
/// |---|---|
/// | `0` | the kernel, then `stfs f1,32(r31)` latches its result into [`GAIN_STATE`] |
/// | anything else | `memset(desc[20], 0, 4·count)` and nothing else |
///
/// **Writes**, on the kernel path: this frame's three argument slots at `+84`/`+92`/`+100` and its back
/// chain, the four bytes at `gains + 32`, and everything [`one_pole_stage`] writes. On the clear path:
/// exactly `4·count` bytes at `desc[20]`. **Reads** the whole 24-byte descriptor and the five gains at
/// `gains + 16`.
///
/// The frame is reproduced — most ports in this crate skip their own — because those three slots *are*
/// the kernel's seventh, eighth and ninth arguments. The two register spills below `sp` are not; see
/// the module note.
pub fn run_stage(
    g: &mut Guest,
    gains: u32,
    count: u64,
    opaque: u64,
    desc: u32,
    sp: u32,
) -> Result<()> {
    // stwu r1,-128(r1): the back chain, then r1 moves. The `stw r12,-8(r1)` and `std r31,-16(r1)`
    // that precede it spill the link register and r31, which this crate has no model of.
    let frame = sp.wrapping_sub(FRAME_BYTES);
    g.set_u32(frame, sp)?;

    let select = g.u32(desc.wrapping_add(DESC_SELECT))?; // lwz r11,8(r7)

    // cmplwi cr6,r11,0 ; bne cr6,0x82b3a010
    if select == 0 {
        let copy = g.u32(desc.wrapping_add(DESC_COPY))?; // lwz r11,20(r7)
        let mixed = g.u32(desc.wrapping_add(DESC_MIXED))?; // lwz r8,16(r7)
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        // The five gains and the three stores are interleaved exactly as lifted.
        let state = fp::load_single(g, gains.wrapping_add(GAIN_STATE))?; // lfs f5,32(r31)
        let source = g.u32(desc.wrapping_add(DESC_SOURCE))?; // lwz r10,4(r7)
        let bus_gain = fp::load_single(g, gains.wrapping_add(GAIN3))?; // lfs f4,28(r31)
        let addend = g.u32(desc.wrapping_add(DESC_ADDEND))?; // lwz r9,0(r7)
        let bus_coeff = fp::load_single(g, gains.wrapping_add(GAIN2))?; // lfs f3,24(r31)
        let feed = fp::load_single(g, gains.wrapping_add(GAIN1))?; // lfs f2,20(r31)
        g.set_u32(frame.wrapping_add(ARG_CLEAR), opaque as u32)?; // stw r5,100(r1)
        let pole = fp::load_single(g, gains.wrapping_add(GAIN0))?; // lfs f1,16(r31)
        g.set_u32(frame.wrapping_add(ARG_COPY), copy)?; // stw r11,92(r1)
        g.set_u32(frame.wrapping_add(ARG_MIXED), mixed)?; // stw r8,84(r1)

        // bl 0x82b399d0 — r3 = count, r9 = addend, r10 = source, r1 = this frame.
        let result = one_pole_stage(
            g,
            count,
            addend as u64,
            source as u64,
            frame,
            pole,
            feed,
            bus_coeff,
            bus_gain,
            state,
        )?;
        // stfs f1,32(r31) — the kernel's result, latched into the gain block as the next call's f5.
        fpscr.disable_flush_mode_unconditional();
        fp::store_single(g, gains.wrapping_add(GAIN_STATE), result)?;
    } else {
        // loc_82B3A010: rlwinm r5,r4,2,0,29 — four bytes per element, the rotate's wrap masked off.
        let length = ((count as u32 as u64) << 2) & 0xFFFF_FFFC;
        let target = g.u32(desc.wrapping_add(DESC_COPY))?; // lwz r3,20(r7)
        // li r4,0 ; bl 0x82f52040
        mem::memset_82f52040(g, target, 0, length)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const GAINS: u32 = BASE + 0x0100;
    const DESC: u32 = BASE + 0x0140;
    /// The caller's `r1`; [`run_stage`]'s frame is 128 bytes below it, and the three argument slots
    /// land at `SP - 44`, `SP - 36` and `SP - 28`.
    const SP: u32 = BASE + 0x0300;
    const FRAME: u32 = SP - FRAME_BYTES;
    const ADDEND: u32 = BASE + 0x0400;
    const SOURCE: u32 = BASE + 0x0800;
    const MIXED: u32 = BASE + 0x1000;
    const COPY: u32 = BASE + 0x1800;

    /// A guest with the buffers and a 32-byte segment at the permute table's fixed `.data` address.
    ///
    /// The table gets its own segment so that a body computing the wrong address gets an `Err` out of
    /// [`Guest`] rather than a plausible neighbouring word.
    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x2000);
        g.put(PERM_TABLE, vec![0u8; PERM_TABLE_BYTES as usize]);
        g
    }

    fn put(g: &mut Guest, at: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            g.set_u32(at + 4 * i as u32, v.to_bits()).unwrap();
        }
    }

    fn get(g: &Guest, at: u32, n: u32) -> Vec<f32> {
        (0..n).map(|i| g.f32(at + 4 * i).unwrap()).collect()
    }

    fn words(g: &Guest, at: u32, n: u32) -> Vec<u32> {
        (0..n).map(|i| g.u32(at + 4 * i).unwrap()).collect()
    }

    /// The three stack arguments, written where [`run_stage`] would have put them.
    fn stack_args(g: &mut Guest, mixed: u32, copy: u32, clear: u32) {
        g.set_u32(FRAME + ARG_MIXED, mixed).unwrap();
        g.set_u32(FRAME + ARG_COPY, copy).unwrap();
        g.set_u32(FRAME + ARG_CLEAR, clear).unwrap();
    }

    fn source_ramp(n: u32) -> Vec<f32> {
        (0..n).map(|i| ((i % 17) as f32) * 0.125 - 1.0).collect()
    }

    fn addend_ramp(n: u32) -> Vec<f32> {
        (0..n).map(|i| 2.0 - ((i % 11) as f32) * 0.25).collect()
    }

    /// The five gains used by most tests. `pole` is small enough that the recursion stays bounded over
    /// 128 samples, which keeps the model and the port comparable in ordinary floats.
    const POLE: f64 = -0.5;
    const FEED: f64 = 0.25;
    const BUS_COEFF: f64 = 2.0;
    const BUS_GAIN: f64 = 0.5;

    /// An independent model of all three passes, written from the algorithm in the module note rather
    /// than from the loop structure — which is the point: it knows nothing about blocks, pipelining or
    /// cursors, so agreeing with it is evidence the covering is right.
    ///
    /// Each pass is written in the precision its instruction uses: the two vector passes as f32
    /// multiply-then-add, the recomp's two roundings (corrected 2026-09-13; this said FMA),
    /// **but nothing here pins that choice**: `FEED`, `BUS_COEFF` and `BUS_GAIN` are 0.25, 2 and 0.5,
    /// so every product is exact and the fused and unfused forms agree. Measured: the suite passed
    /// with this model fused and the layer unfused. Nor does the recorded replay pin it — stage.tsv
    /// replays clean under both layers.
    /// the recursion as `fnmsubs` through [`fp::nmsub_single`] with the `mixed` value round-tripped
    /// through an f32 store and load the way the port's memory does it.
    fn model(
        count: usize,
        addend: &[f32],
        source: &[f32],
        bus_in: &[f32],
        state: f64,
    ) -> (Vec<f32>, Vec<f32>, f64) {
        let (feed, bc, bg) = (FEED as f32, BUS_COEFF as f32, BUS_GAIN as f32);
        let mut filtered = Vec::with_capacity(count);
        let mut bus = Vec::with_capacity(count);
        let mut y = state;
        for i in 0..count {
            // vnmsubfp: addend[i] - feed·source[i+1], the product rounded first.
            let mixed = addend[i] - feed * source[i + 1];
            // fnmsubs over the value as it comes back out of memory.
            y = fp::nmsub_single(y, POLE, mixed as f64);
            filtered.push(y as f32);
            // Two vmaddfp, two roundings each: bus_gain·(bus_coeff·source[i] + source[i+1]) + bus[i].
            let prod = source[i] * bc + source[i + 1];
            bus.push(prod * bg + bus_in[i]);
        }
        (filtered, bus, y)
    }

    /// Set up and run one kernel call, returning `(mixed, copy, result)`.
    #[allow(clippy::type_complexity)]
    fn run(
        count: u32,
        source_at: u32,
        clear: u32,
        bus_in: &[f32],
        state: f64,
    ) -> (Guest, Vec<f32>, Vec<f32>, f64) {
        let mut g = guest();
        let source = source_ramp(count + 1);
        let addend = addend_ramp(count);
        put(&mut g, source_at, &source);
        put(&mut g, ADDEND, &addend);
        put(&mut g, COPY, bus_in);
        stack_args(&mut g, MIXED, COPY, clear);
        let result = one_pole_stage(
            &mut g,
            count as u64,
            ADDEND as u64,
            source_at as u64,
            FRAME,
            POLE,
            FEED,
            BUS_COEFF,
            BUS_GAIN,
            state,
        )
        .unwrap();
        let mixed = get(&g, MIXED, count);
        let copy = get(&g, COPY, count);
        (g, mixed, copy, result)
    }

    // ------------------------------------------------------------------ sub_82B399D0

    #[test]
    fn one_block_matches_the_three_pass_model() {
        // count = 16 is the smallest count the three cursors tile: blocks = 0, so there is no loop
        // iteration at all and the tail block does the whole recursion and the whole bus accumulate.
        let bus_in = vec![7.0f32; 16];
        let (_, mixed, copy, result) = run(16, SOURCE, 1, &bus_in, 0.0);
        let (want_mixed, want_copy, want_state) =
            model(16, &addend_ramp(16), &source_ramp(17), &bus_in, 0.0);
        assert_eq!(mixed, want_mixed);
        assert_eq!(copy, want_copy);
        assert_eq!(result, want_state, "the return is y[count-1]");
    }

    #[test]
    fn every_count_that_tiles_matches_the_model() {
        // 16, 32, 48, 64 and 128 exercise 0, 1, 2, 3 and 7 loop iterations. The pipeline hands the
        // recursion one block behind the windows and the bus one behind that, so a transcription that
        // got any of the three cursors wrong would cover the run with a gap or an overlap — which is
        // exactly what a model that knows nothing about blocks detects.
        for count in [16u32, 32, 48, 64, 128] {
            let bus_in: Vec<f32> = (0..count).map(|i| (i as f32) * 0.5).collect();
            let (_, mixed, copy, result) = run(count, SOURCE, 1, &bus_in, 0.25);
            let (want_mixed, want_copy, want_state) = model(
                count as usize,
                &addend_ramp(count),
                &source_ramp(count + 1),
                &bus_in,
                0.25,
            );
            assert_eq!(mixed, want_mixed, "count {count}: mixed");
            assert_eq!(copy, want_copy, "count {count}: bus");
            assert_eq!(result, want_state, "count {count}: result");
        }
    }

    #[test]
    fn the_source_window_is_built_at_every_four_byte_alignment() {
        // `rlwinm r4,r10,0,28,29` picks one of four 32-byte permute controls, and the census shows the
        // run arriving at more than one of them. All four have to give the same answer: a control
        // built at the wrong offset would shift the whole source run by one to three singles, which
        // the model catches immediately.
        for slot in 0..4u32 {
            let at = SOURCE + 4 * slot;
            assert_eq!(
                at & 0xC,
                4 * slot,
                "the test has to reach all four selectors"
            );
            let bus_in = vec![0.0f32; 32];
            let (_, mixed, copy, _) = run(32, at, 1, &bus_in, 0.0);
            let (want_mixed, want_copy, _) =
                model(32, &addend_ramp(32), &source_ramp(33), &bus_in, 0.0);
            assert_eq!(mixed, want_mixed, "selector {slot}: mixed");
            assert_eq!(copy, want_copy, "selector {slot}: bus");
        }
    }

    #[test]
    fn the_source_is_read_one_single_beyond_the_run() {
        // `source` holds count + 1 singles, and the last one only ever reaches `mixed[count-1]` and
        // the bus. Moving it has to move both and nothing else — which is what pins the `+4` control
        // against the `+0` one.
        let bus_in = vec![0.0f32; 16];
        let (a, ma, ca, _) = run(16, SOURCE, 1, &bus_in, 0.0);
        let mut g = a.clone();
        g.set_u32(SOURCE + 4 * 16, 99.0f32.to_bits()).unwrap();
        stack_args(&mut g, MIXED, COPY, 1);
        put(&mut g, COPY, &bus_in);
        let _ = one_pole_stage(
            &mut g,
            16,
            ADDEND as u64,
            SOURCE as u64,
            FRAME,
            POLE,
            FEED,
            BUS_COEFF,
            BUS_GAIN,
            0.0,
        )
        .unwrap();
        let mb = get(&g, MIXED, 16);
        let cb = get(&g, COPY, 16);
        assert_eq!(
            ma[..15],
            mb[..15],
            "only the last sample depends on source[count]"
        );
        assert_ne!(ma[15], mb[15]);
        assert_eq!(ca[..15], cb[..15]);
        assert_ne!(ca[15], cb[15]);
    }

    #[test]
    fn the_state_argument_seeds_the_recursion_and_the_result_carries_it_out() {
        // f5 in, f1 out, and the two are the ends of one chain. With pole = -0.5 a change in the seed
        // decays but never vanishes over sixteen samples, so every output moves.
        let bus_in = vec![0.0f32; 16];
        let (_, a, _, ra) = run(16, SOURCE, 1, &bus_in, 0.0);
        let (_, b, _, rb) = run(16, SOURCE, 1, &bus_in, 1.0);
        assert_ne!(a[0], b[0], "the first output sees f5 directly");
        assert_ne!(
            a[15], b[15],
            "and it is still visible at the end of the block"
        );
        assert_ne!(ra, rb);
        assert_eq!(
            ra, a[15] as f64,
            "the result is the last single, read back from memory"
        );
        assert_eq!(rb, b[15] as f64);
    }

    #[test]
    fn the_recursion_is_fused_and_subtracted() {
        // `fnmsubs f12,f0,f1,f13` is `mixed - pole·y`, one rounding. Two properties: the sign (a
        // `fmadds` would run the filter away rather than alternating) and the fusion (a separate
        // multiply then subtract rounds twice and is a different filter).
        //
        // The distinguishing input needs a product whose low bits the subtraction keeps. Searching a
        // deterministic set for one is what makes this a check rather than a hope.
        let mut xorshift = 0x2468_ACE0u32;
        let mut next = || {
            xorshift ^= xorshift << 13;
            xorshift ^= xorshift >> 17;
            xorshift ^= xorshift << 5;
            f32::from_bits((xorshift & 0x007F_FFFF) | 0x3F80_0000)
        };
        let mut found = 0;
        for _ in 0..2000 {
            let (pole, y, mixed) = (next(), next(), next());
            let fused = fp::nmsub_single(y as f64, pole as f64, mixed as f64) as f32;
            let split = fp::sub_single(mixed as f64, fp::mul_single(y as f64, pole as f64)) as f32;
            if fused == split {
                continue;
            }
            found += 1;

            // feed = 0 makes mixed[i] = addend[i] exactly, so the first output is
            // `fnmsubs(state, pole, addend[0])` and nothing else.
            let mut g = guest();
            put(&mut g, SOURCE, &vec![0.0f32; 17]);
            let mut addend = vec![0.0f32; 16];
            addend[0] = mixed;
            put(&mut g, ADDEND, &addend);
            stack_args(&mut g, MIXED, COPY, 1);
            let got = one_pole_stage(
                &mut g,
                16,
                ADDEND as u64,
                SOURCE as u64,
                FRAME,
                pole as f64,
                0.0,
                0.0,
                0.0,
                y as f64,
            )
            .unwrap();
            assert_eq!(
                g.f32(MIXED).unwrap(),
                fused,
                "pole {pole} y {y} mixed {mixed}"
            );
            assert_ne!(
                g.f32(MIXED).unwrap(),
                split,
                "the two forms have to differ here"
            );
            let _ = got;
            if found == 6 {
                break;
            }
        }
        assert!(
            found > 0,
            "no distinguishing input found: this test would prove nothing"
        );

        // And the sign: with pole = 1 and a constant feed-forward of 1, the outputs alternate 1, 0
        // rather than running away.
        let mut g = guest();
        put(&mut g, SOURCE, &vec![0.0f32; 17]);
        put(&mut g, ADDEND, &vec![1.0f32; 16]);
        stack_args(&mut g, MIXED, COPY, 1);
        one_pole_stage(
            &mut g,
            16,
            ADDEND as u64,
            SOURCE as u64,
            FRAME,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
        )
        .unwrap();
        assert_eq!(
            get(&g, MIXED, 8),
            vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]
        );
    }

    #[test]
    fn a_zero_clear_flag_establishes_the_bus_through_dcbzl_whole_lines_at_a_time() {
        // `dcbzl` is a real 128-byte store from the 128-byte **floor** of `copy`, and it covers
        // `ceil(4·count / 128)` whole lines — so for count = 16 it zeroes 128 bytes where the run is
        // only 64. That over-write is declared by the C++ `Windows()` and is reproduced here; a port
        // that cleared only `4·count` bytes, or that treated `dcbzl` as a hint, fails this.
        //
        // Two lines of 9.0, not one. The first version seeded 32 singles -- exactly the 128 bytes
        // of the first line -- so the single at `COPY + 128` was never written and read as 0.0 for
        // any port at all. The last assertion could not pass, which also means it could never have
        // caught a port that cleared a line too many.
        let bus_in = vec![9.0f32; 64];
        let mut g = guest();
        put(&mut g, SOURCE, &source_ramp(17));
        put(&mut g, ADDEND, &addend_ramp(16));
        put(&mut g, COPY, &bus_in);
        stack_args(&mut g, MIXED, COPY, 0); // the clear flag

        one_pole_stage(
            &mut g,
            16,
            ADDEND as u64,
            SOURCE as u64,
            FRAME,
            POLE,
            FEED,
            BUS_COEFF,
            BUS_GAIN,
            0.0,
        )
        .unwrap();

        // The run accumulated onto zeroes, not onto the 9.0s.
        let (_, want_copy, _) = model(
            16,
            &addend_ramp(16),
            &source_ramp(17),
            &vec![0.0f32; 16],
            0.0,
        );
        assert_eq!(get(&g, COPY, 16), want_copy);
        // And the second half of the 128-byte line was zeroed even though no sample reaches it.
        assert_eq!(
            get(&g, COPY + 64, 16),
            vec![0.0f32; 16],
            "the rest of the cache line"
        );
        assert_eq!(g.f32(COPY + 128).unwrap(), 9.0, "and not the line after it");
    }

    #[test]
    fn a_span_of_exactly_one_line_clears_that_line_and_no_more() {
        // The loop is `r11 <u r22` stepping by 128, and it only tells `<` from `<=` when the span is
        // an exact multiple of 128. At 16 samples the span is 64 and both bounds stop after one
        // line, so no other test here can see the difference: a negative control that turned the
        // `<` into `<=` passed all 334 tests before this one existed. 32 samples is a span of
        // exactly 128 bytes, where `<=` would establish a second line of zeroes.
        let bus_in = vec![9.0f32; 64];
        let mut g = guest();
        put(&mut g, SOURCE, &source_ramp(33));
        put(&mut g, ADDEND, &addend_ramp(32));
        put(&mut g, COPY, &bus_in);
        stack_args(&mut g, MIXED, COPY, 0); // the clear flag

        one_pole_stage(
            &mut g,
            32,
            ADDEND as u64,
            SOURCE as u64,
            FRAME,
            POLE,
            FEED,
            BUS_COEFF,
            BUS_GAIN,
            0.0,
        )
        .unwrap();

        assert_eq!(
            g.f32(COPY + 128).unwrap(),
            9.0,
            "the line after a one-line span"
        );
    }

    #[test]
    fn a_non_zero_clear_flag_accumulates_onto_whatever_the_bus_held() {
        // The other side of the same branch: the flag is not a boolean the port may normalise, it is
        // a `cmpwi` against zero, and a non-zero value skips the establish entirely.
        for flag in [1u32, 0xFFFF_FFFF, 0x8000_0000] {
            let bus_in = vec![9.0f32; 16];
            let (_, _, copy, _) = run(16, SOURCE, flag, &bus_in, 0.0);
            let (_, want_copy, _) = model(16, &addend_ramp(16), &source_ramp(17), &bus_in, 0.0);
            assert_eq!(copy, want_copy, "flag {flag:#x}");
            assert_ne!(
                copy[0],
                want_copy[0] - 9.0,
                "the 9.0 has to be visible in the answer"
            );
        }
    }

    #[test]
    fn the_permute_table_is_rebuilt_in_data_on_every_call() {
        // Eight `stw` to a fixed `.data` address, read straight back to form the two controls. It is a
        // write the C++ `Windows()` declares, so it is reproduced — and because the table is read back
        // in the same call, a port that skipped the stores would compute its windows from whatever was
        // there. The poison below is what makes that visible.
        let mut g = guest();
        for k in 0..8u32 {
            g.set_u32(PERM_TABLE + 4 * k, 0xA5A5_A5A5).unwrap();
        }
        put(&mut g, SOURCE, &source_ramp(17));
        put(&mut g, ADDEND, &addend_ramp(16));
        stack_args(&mut g, MIXED, COPY, 1);
        one_pole_stage(
            &mut g,
            16,
            ADDEND as u64,
            SOURCE as u64,
            FRAME,
            POLE,
            FEED,
            BUS_COEFF,
            BUS_GAIN,
            0.0,
        )
        .unwrap();

        assert_eq!(
            words(&g, PERM_TABLE, 8),
            vec![
                0x0001_0203,
                0x0405_0607,
                0x0809_0A0B,
                0x0C0D_0E0F,
                0x1011_1213,
                0x1415_1617,
                0x1819_1A1B,
                0x1C1D_1E1F
            ],
            "byte indices 0x00..0x1F, ascending"
        );
        // And the windows it formed from the rebuilt table are the right ones.
        let (want_mixed, _, _) = model(
            16,
            &addend_ramp(16),
            &source_ramp(17),
            &vec![0.0f32; 16],
            0.0,
        );
        assert_eq!(get(&g, MIXED, 16), want_mixed);
    }

    #[test]
    fn the_three_arguments_come_off_the_callers_frame() {
        // `lwz r8,84(r1)`, `lwz r26,92(r1)`, `lwz r11,100(r1)`: the kernel has no `stwu`, so these are
        // the caller's slots. Pointing `mixed` and `copy` somewhere else through the frame alone has to
        // move every write.
        let alt_mixed = MIXED + 0x200;
        let alt_copy = COPY + 0x200;
        let mut g = guest();
        put(&mut g, SOURCE, &source_ramp(17));
        put(&mut g, ADDEND, &addend_ramp(16));
        stack_args(&mut g, alt_mixed, alt_copy, 1);
        for k in 0..16u32 {
            g.set_u32(MIXED + 4 * k, 0x7F7F_7F7F).unwrap();
            g.set_u32(COPY + 4 * k, 0x7F7F_7F7F).unwrap();
        }

        one_pole_stage(
            &mut g,
            16,
            ADDEND as u64,
            SOURCE as u64,
            FRAME,
            POLE,
            FEED,
            BUS_COEFF,
            BUS_GAIN,
            0.0,
        )
        .unwrap();

        let (want_mixed, want_copy, _) = model(
            16,
            &addend_ramp(16),
            &source_ramp(17),
            &vec![0.0f32; 16],
            0.0,
        );
        assert_eq!(get(&g, alt_mixed, 16), want_mixed);
        assert_eq!(get(&g, alt_copy, 16), want_copy);
        assert_eq!(
            words(&g, MIXED, 16),
            vec![0x7F7F_7F7F; 16],
            "the default was written"
        );
        assert_eq!(words(&g, COPY, 16), vec![0x7F7F_7F7F; 16]);
    }

    #[test]
    fn the_kernel_restores_the_entry_flush_mode() {
        let bus_in = vec![0.0f32; 64];
        let before = crate::vmx::get_mxcsr();
        run(64, SOURCE, 1, &bus_in, 0.0);
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }

    // ------------------------------------------------------------------ sub_82B39FA0

    /// The gain block and descriptor [`run_stage`] reads.
    fn dispatcher(g: &mut Guest, select: u32) {
        put(
            g,
            GAINS + GAIN0,
            &[
                POLE as f32,
                FEED as f32,
                BUS_COEFF as f32,
                BUS_GAIN as f32,
                0.0,
            ],
        );
        g.set_u32(DESC + DESC_ADDEND, ADDEND).unwrap();
        g.set_u32(DESC + DESC_SOURCE, SOURCE).unwrap();
        g.set_u32(DESC + DESC_SELECT, select).unwrap();
        g.set_u32(DESC + DESC_MIXED, MIXED).unwrap();
        g.set_u32(DESC + DESC_COPY, COPY).unwrap();
    }

    #[test]
    fn a_zero_selector_runs_the_kernel_and_latches_its_result_into_the_gain_block() {
        let mut g = guest();
        dispatcher(&mut g, 0);
        put(&mut g, SOURCE, &source_ramp(17));
        put(&mut g, ADDEND, &addend_ramp(16));
        put(&mut g, COPY, &vec![3.0f32; 16]);

        run_stage(&mut g, GAINS, 16, 1, DESC, SP).unwrap();

        let (want_mixed, want_copy, want_state) = model(
            16,
            &addend_ramp(16),
            &source_ramp(17),
            &vec![3.0f32; 16],
            0.0,
        );
        assert_eq!(get(&g, MIXED, 16), want_mixed);
        assert_eq!(get(&g, COPY, 16), want_copy);
        // stfs f1,32(r31): the kernel's f1, narrowed to a single, is the next call's f5.
        assert_eq!(g.f32(GAINS + GAIN_STATE).unwrap(), want_state as f32);
        // The four gains it read are untouched.
        assert_eq!(
            get(&g, GAINS + GAIN0, 4),
            vec![POLE as f32, FEED as f32, BUS_COEFF as f32, BUS_GAIN as f32]
        );
    }

    #[test]
    fn the_latched_state_is_what_the_next_call_carries_in() {
        // `+32` is running state, not a parameter: two calls of sixteen samples have to equal one call
        // of thirty-two over the same data. That is the only thing that pins the latch's direction.
        let source = source_ramp(33);
        let addend = addend_ramp(32);

        let mut whole = guest();
        dispatcher(&mut whole, 0);
        put(&mut whole, SOURCE, &source);
        put(&mut whole, ADDEND, &addend);
        run_stage(&mut whole, GAINS, 32, 1, DESC, SP).unwrap();

        let mut halves = guest();
        dispatcher(&mut halves, 0);
        put(&mut halves, SOURCE, &source);
        put(&mut halves, ADDEND, &addend);
        run_stage(&mut halves, GAINS, 16, 1, DESC, SP).unwrap();
        // The second half: the source and addend runs sixteen singles on, the output sixteen on too.
        halves.set_u32(DESC + DESC_SOURCE, SOURCE + 64).unwrap();
        halves.set_u32(DESC + DESC_ADDEND, ADDEND + 64).unwrap();
        halves.set_u32(DESC + DESC_MIXED, MIXED + 64).unwrap();
        halves.set_u32(DESC + DESC_COPY, COPY + 64).unwrap();
        run_stage(&mut halves, GAINS, 16, 1, DESC, SP).unwrap();

        assert_eq!(get(&whole, MIXED, 32), get(&halves, MIXED, 32));
        assert_eq!(
            whole.f32(GAINS + GAIN_STATE).unwrap(),
            halves.f32(GAINS + GAIN_STATE).unwrap()
        );
    }

    #[test]
    fn the_three_stack_slots_are_written_at_84_92_and_100_of_the_new_frame() {
        // The whole of the coupling between the two bodies. The slots are poisoned first so that a
        // port writing them at the wrong offsets — or into the caller's frame rather than the new
        // one — is caught rather than accidentally working.
        let mut g = guest();
        dispatcher(&mut g, 0);
        put(&mut g, SOURCE, &source_ramp(17));
        put(&mut g, ADDEND, &addend_ramp(16));
        for k in 0..32u32 {
            g.set_u32(FRAME + 4 * k, 0xDEAD_BEEF).unwrap();
        }

        run_stage(&mut g, GAINS, 16, 0x5A5A, DESC, SP).unwrap();

        assert_eq!(
            g.u32(FRAME + ARG_MIXED).unwrap(),
            MIXED,
            "desc[16] -> 84(r1)"
        );
        assert_eq!(g.u32(FRAME + ARG_COPY).unwrap(), COPY, "desc[20] -> 92(r1)");
        assert_eq!(g.u32(FRAME + ARG_CLEAR).unwrap(), 0x5A5A, "r5 -> 100(r1)");
        // stwu r1,-128(r1) writes the back chain at the new frame's first word.
        assert_eq!(g.u32(FRAME).unwrap(), SP, "the back chain");
    }

    #[test]
    fn the_opaque_fifth_argument_is_the_kernels_clear_flag() {
        // `r5` is documented as an opaque pass-through by `sub_82B39FA0` and read as the clear flag by
        // `sub_82B399D0`. They are one word, and passing zero has to establish the bus as zeroes.
        let mut zero = guest();
        dispatcher(&mut zero, 0);
        put(&mut zero, SOURCE, &source_ramp(17));
        put(&mut zero, ADDEND, &addend_ramp(16));
        put(&mut zero, COPY, &vec![9.0f32; 16]);
        run_stage(&mut zero, GAINS, 16, 0, DESC, SP).unwrap();

        let mut one = guest();
        dispatcher(&mut one, 0);
        put(&mut one, SOURCE, &source_ramp(17));
        put(&mut one, ADDEND, &addend_ramp(16));
        put(&mut one, COPY, &vec![9.0f32; 16]);
        run_stage(&mut one, GAINS, 16, 1, DESC, SP).unwrap();

        let (_, cleared, _) = model(
            16,
            &addend_ramp(16),
            &source_ramp(17),
            &vec![0.0f32; 16],
            0.0,
        );
        let (_, kept, _) = model(
            16,
            &addend_ramp(16),
            &source_ramp(17),
            &vec![9.0f32; 16],
            0.0,
        );
        assert_eq!(get(&zero, COPY, 16), cleared, "r5 == 0 establishes the bus");
        assert_eq!(get(&one, COPY, 16), kept, "r5 != 0 accumulates onto it");
    }

    #[test]
    fn a_non_zero_selector_clears_the_copy_buffer_and_runs_no_kernel() {
        // `bne cr6,0x82b3a010`: a memset of exactly `4·count` bytes at desc[20], and nothing else at
        // all — no kernel, no permute table, no latch.
        let mut g = guest();
        dispatcher(&mut g, 1);
        put(&mut g, SOURCE, &source_ramp(17));
        put(&mut g, ADDEND, &addend_ramp(16));
        put(&mut g, COPY, &vec![9.0f32; 20]);
        for k in 0..16u32 {
            g.set_u32(MIXED + 4 * k, 0x7F7F_7F7F).unwrap();
        }
        g.set_u32(GAINS + GAIN_STATE, 0x1234_5678).unwrap();
        for k in 0..8u32 {
            g.set_u32(PERM_TABLE + 4 * k, 0xA5A5_A5A5).unwrap();
        }

        run_stage(&mut g, GAINS, 16, 0, DESC, SP).unwrap();

        assert_eq!(get(&g, COPY, 16), vec![0.0f32; 16], "the buffer is cleared");
        assert_eq!(
            g.f32(COPY + 64).unwrap(),
            9.0,
            "and not one byte past 4·count"
        );
        assert_eq!(words(&g, MIXED, 16), vec![0x7F7F_7F7F; 16], "no kernel ran");
        assert_eq!(g.u32(GAINS + GAIN_STATE).unwrap(), 0x1234_5678, "no latch");
        assert_eq!(
            words(&g, PERM_TABLE, 8),
            vec![0xA5A5_A5A5; 8],
            "no permute table"
        );
    }

    #[test]
    fn the_clear_length_is_the_count_scaled_by_four_on_the_low_word() {
        // `rlwinm r5,r4,2,0,29` — a 32-bit shift of the low word with the bottom two bits masked, so a
        // count whose scaled length wraps clears a *short* buffer rather than a huge one. The high
        // half of `r4` is dropped before the shift, which is the part a `u64` multiply would get wrong.
        let mut g = guest();
        dispatcher(&mut g, 1);
        put(&mut g, COPY, &vec![9.0f32; 20]);
        run_stage(&mut g, GAINS, 0xDEAD_0000_0000_0003, 0, DESC, SP).unwrap();
        assert_eq!(
            get(&g, COPY, 3),
            vec![0.0f32; 3],
            "three words of the low half"
        );
        assert_eq!(
            g.f32(COPY + 12).unwrap(),
            9.0,
            "and the high half was dropped"
        );

        // A zero count clears nothing and never touches the address, which is `crate::mem`'s contract.
        let mut h = guest();
        dispatcher(&mut h, 1);
        h.set_u32(DESC + DESC_COPY, 0).unwrap();
        assert!(run_stage(&mut h, GAINS, 0, 0, DESC, SP).is_ok());
    }

    #[test]
    fn the_five_gains_are_read_from_plus_sixteen_and_only_the_fifth_is_written() {
        // The offsets, one at a time: each gain is patched alone and has to change the answer, which
        // is what rules out a port that read them in the wrong order.
        let baseline = |g: &mut Guest| {
            dispatcher(g, 0);
            put(g, SOURCE, &source_ramp(17));
            put(g, ADDEND, &addend_ramp(16));
            put(g, COPY, &vec![0.0f32; 16]);
        };
        let mut base_g = guest();
        baseline(&mut base_g);
        run_stage(&mut base_g, GAINS, 16, 1, DESC, SP).unwrap();
        let base_mixed = get(&base_g, MIXED, 16);
        let base_copy = get(&base_g, COPY, 16);

        for (slot, name) in [
            (GAIN0, "pole"),
            (GAIN1, "feed"),
            (GAIN2, "bus_coeff"),
            (GAIN3, "bus_gain"),
        ] {
            let mut g = guest();
            baseline(&mut g);
            g.set_u32(GAINS + slot, 0.375f32.to_bits()).unwrap();
            run_stage(&mut g, GAINS, 16, 1, DESC, SP).unwrap();
            let moved = get(&g, MIXED, 16) != base_mixed || get(&g, COPY, 16) != base_copy;
            assert!(moved, "{name} at +{slot} changed nothing");
        }

        // And the fifth is the one that also seeds the recursion.
        let mut seeded = guest();
        baseline(&mut seeded);
        seeded
            .set_u32(GAINS + GAIN_STATE, 2.0f32.to_bits())
            .unwrap();
        run_stage(&mut seeded, GAINS, 16, 1, DESC, SP).unwrap();
        assert_ne!(
            get(&seeded, MIXED, 16),
            base_mixed,
            "f5 seeds the recursion"
        );
    }

    #[test]
    fn the_dispatcher_restores_the_entry_flush_mode() {
        for select in [0u32, 1] {
            let mut g = guest();
            dispatcher(&mut g, select);
            put(&mut g, SOURCE, &source_ramp(17));
            put(&mut g, ADDEND, &addend_ramp(16));
            let before = crate::vmx::get_mxcsr();
            run_stage(&mut g, GAINS, 16, 1, DESC, SP).unwrap();
            assert_eq!(crate::vmx::get_mxcsr(), before, "selector {select}");
        }
    }

    #[test]
    fn quarter_to_zero_truncates_toward_zero() {
        assert_eq!(quarter_to_zero(256), 64);
        assert_eq!(quarter_to_zero(7), 1);
        assert_eq!(quarter_to_zero(-7), -1, ">> 2 would give -2");
        assert_eq!(
            quarter_to_zero(-8),
            -2,
            "an exact multiple needs no correction"
        );
        assert_eq!(quarter_to_zero(0), 0);
    }
}
