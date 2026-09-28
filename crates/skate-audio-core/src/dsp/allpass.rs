//! `sub_82B389A0` — the four-lane one-multiply allpass stage, over an **unaligned** tap.
//!
//! Ported from `recomp/src/audio_ports/sub_82B389A0.inc`, **STATUS: verified** — 59,894 calls in a
//! played session on `RwAudioCore Dac`, compared against the original under the shadow harness.
//!
//! Per lane it is the one-multiply allpass pair, and then a gain-weighted accumulate:
//!
//! ```text
//! d   = c − a·t          (vnmsubfp, a first)
//! y   = a·d + t          (vmaddfp,  a first)
//! acc = y·g + acc        (vmaddfp,  y first — the operand order flips here)
//! ```
//!
//! Calling it an allpass is an inference from that shape; no symbol or plug-in metadata covers this
//! function. What the port reproduces is the arithmetic.
//!
//! ## Four things that decide whether a rewrite is right
//!
//! **The tap is read unaligned.** Every 16-byte group of `t` is `lvlx(p) | lvrx(p + 16)` — two
//! partial loads or'd together — while `c`, `d` and `acc` all go through aligned `lvx128`/`stvx128`.
//! So the tap may sit at any byte offset and the kernel still reads four consecutive floats per
//! lane group. [`the_tap_may_be_unaligned`] is that test.
//!
//! **The trip count is a ceiling, so the loop runs past `n`.** It is `ceil(quads / 4)` where
//! `quads = n / 4` truncating, which means a count whose quarter is not a multiple of four is
//! processed up to the next multiple of 16 samples. Reproduced — the C++ window builder declares the
//! same over-run — and [`the_trip_count_is_a_ceiling_and_overruns_the_count`] pins it.
//!
//! **`r10` decides whether the accumulator is cleared first**, and the clear is `dcbzl`: whole
//! 128-byte lines from `acc & !127`, so with an unaligned accumulator it reaches *below* the
//! pointer. [`the_clear_is_by_cache_line_from_the_floor`] asserts that.
//!
//! **Its result registers are `r3` and `v2`,** because its one call site is a tail call: `r3` is the
//! 64-bit `acc − c` delta and `v2` is the last tap group, both of which leave here as the caller's
//! own ABI results. `v29`, `v30` and `v31` are clobbered too — the last `c` group, the last `y`, and
//! the last accumulator group *before* its store — and the harness compares those unconditionally,
//! so they are returned rather than dropped.
//!
//! ## Two things not reproduced, with the reason
//!
//! The eight `stfs` that splat `a` and `g` into the frame at `r1-128` and `r1-112`, read straight
//! back by two `lvx128`: kept in registers here, which is [`crate::dsp::scale`]'s established
//! precedent and holds for the same measured reason — the guest `r1` at these entries is 16-byte
//! aligned, so the two loads see exactly four copies of each single. And `__savegprlr_21`'s spills,
//! since nothing here writes `r21`-`r31`.
//!
//! Note the `.inc`'s comments call each multiply-add "a single vmaddfp" and say the pair must not
//! become two rounded operations. That was the intent; the build does not deliver it. The recomp has
//! no FMA, so `vmaddfp` and `vnmsubfp` each round twice ([`crate::vmx`] rule 1), and this port goes
//! through the same layer as every other kernel here.
//!
//! **This kernel is the strongest evidence for that rule anywhere in the project.** Replayed against
//! 800 recorded calls it passes all 800 with two roundings and fails **701 of them** with one —
//! three multiply-adds per lane per sample will do that, where the sine kernel's tiny reduced
//! arguments disagreed on only 4 calls in 2,000. Measured by patching the layer and re-running, not
//! argued.

use core::arch::x86_64::*;

use crate::vmx::{self, Fpscr};
use crate::{Guest, Result, fp};

/// The window arithmetic's guard: the harness's 32 KB budget refuses anything past about 2,700
/// samples long before this, since the kernel writes 12 bytes per sample.
pub const MAX_ENUMERABLE_SAMPLES: i32 = 0x0100_0000;
/// One `dcbzl` line.
pub const CLEAR_LINE: u32 = 128;
/// Bytes of each buffer one iteration covers: four 16-byte groups.
pub const GROUP_BYTES: u32 = 64;

/// What the kernel leaves in the registers a caller can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllpassClobbers {
    /// `v2` — the last tap group, and a result register: the call site is a tail call.
    pub v2: [u32; 4],
    /// `v29` — the last `c` group loaded.
    pub v29: [u32; 4],
    /// `v30` — the last `y` computed.
    pub v30: [u32; 4],
    /// `v31` — the last accumulator group loaded, *before* the store to that same address.
    pub v31: [u32; 4],
}

/// The registers the original leaves behind: `r3` always, the vectors only if the loop ran.
#[derive(Clone, Copy, Debug)]
pub struct AllpassResult {
    /// `r3`: the 64-bit `acc − c` delta, or the incoming count when the loop did not run — the
    /// `subf` sits *after* the `ble`, so an early exit leaves `r3` as it arrived.
    pub r3: u64,
    /// Present only when the loop ran at least once.
    pub clobbers: Option<AllpassClobbers>,
}

#[inline]
fn words(value: __m128i) -> [u32; 4] {
    let mut out = [0u32; 4];
    unsafe { _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, value) };
    out
}

/// Run the stage (`sub_82B389A0`).
///
/// The arguments are the guest's, in its order: `n` is `r3`, `c` is `r6` and `acc` is `r9` at full
/// width (their difference is the 64-bit result), `t` is `r7`, `d` is `r8`, `primed` is `r10`, `a`
/// is `f1` and `gain` is `f2`.
pub fn allpass_stage(
    g: &mut Guest,
    n: i32,
    c: u64,
    t: u32,
    d: u32,
    acc: u64,
    primed: i32,
    a: f64,
    gain: f64,
) -> Result<AllpassResult> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    let c_base = c as u32;
    let acc_base = acc as u32;

    // cmpwi cr6,r10,0 reads the INCOMING r10, before `rlwinm r10,r3,2,0,29` overwrites it.
    let clear_accumulator = primed == 0;
    let pass_bytes = ((n as u32) << 2) & 0xFFFF_FFFC;

    // loc_82B389C0: clear the accumulator a cache line at a time. The `dcbt` prefetches alongside
    // have no memory effect, and the r10 != 0 path is prefetch only, so it has nothing to reproduce.
    if clear_accumulator && pass_bytes != 0 {
        let mut off = 0u32;
        while off < pass_bytes {
            let line = (acc_base.wrapping_add(off)) & !(CLEAR_LINE - 1); // dcbzl r11,r9
            g.fill(line, 0, CLEAR_LINE)?;
            off += CLEAR_LINE;
        }
    }

    // srawi r11,r3,2 ; addze. r11,r11 -- a signed divide by four that truncates toward zero.
    let quads = n / 4;

    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted by the first stfs of the splats

    // The eight `stfs` into the frame and the two `lvx128` back are one splat each; see the module
    // note on why they stay in registers.
    let a_v = unsafe { _mm_castsi128_ps(_mm_set1_epi32(fp::single_to_bits(a) as i32)) };
    let g_v = unsafe { _mm_castsi128_ps(_mm_set1_epi32(fp::single_to_bits(gain) as i32)) };

    if quads <= 0 {
        // ble 0x82b38b60 -- and the `subf` that sets r3 is below this, so r3 is untouched.
        return Ok(AllpassResult {
            r3: n as i64 as u64,
            clobbers: None,
        });
    }

    // subf r3,r6,r9 -- a 64-bit subtract of the two full-width registers.
    let r3 = acc.wrapping_sub(c);

    // addi r11,r11,-1 ; rlwinm r11,r11,30,2,31 ; addi r5,r11,1 -- ceil(quads / 4).
    let iterations = (((quads as u32) - 1) >> 2) + 1;

    let mut last = AllpassClobbers {
        v2: [0; 4],
        v29: [0; 4],
        v30: [0; 4],
        v31: [0; 4],
    };

    for k in 0..iterations {
        let u = k * GROUP_BYTES; // every cursor advances by 64 per iteration
        unsafe {
            // The tap's eight partial loads, interleaved in the lifted order because they share
            // address registers; `c`'s four aligned loads are interleaved between them.
            let t0_left = vmx::lvlx128(g, t.wrapping_add(u))?;
            let c2 = vmx::lvx128(g, c_base.wrapping_add(32 + u))?;
            let c0 = vmx::lvx128(g, c_base.wrapping_add(u))?;
            let c1 = vmx::lvx128(g, c_base.wrapping_add(16 + u))?;
            let c3 = vmx::lvx128(g, c_base.wrapping_add(48 + u))?;

            let t2_left = vmx::lvlx128(g, t.wrapping_add(32 + u))?;
            let t2_right = vmx::lvrx128(g, t.wrapping_add(48 + u))?;
            let t1_right = vmx::lvrx128(g, t.wrapping_add(32 + u))?;
            let t2 = _mm_castsi128_ps(vmx::vor(t2_left, t2_right)); // vor128 v5,v62,v61
            let t0_right = vmx::lvrx128(g, t.wrapping_add(16 + u))?;
            let t1_left = vmx::lvlx128(g, t.wrapping_add(16 + u))?;
            let t0 = _mm_castsi128_ps(vmx::vor(t0_left, t0_right)); // vor128 v4,v63,v60
            let t3_left = vmx::lvlx128(g, t.wrapping_add(48 + u))?;
            let t1 = _mm_castsi128_ps(vmx::vor(t1_left, t1_right)); // vor128 v3,v58,v59
            let t3_right = vmx::lvrx128(g, t.wrapping_add(64 + u))?;

            // The flush mode is set here, once per iteration, immediately before the first vnmsubfp.
            fpscr.enable_flush_mode_unconditional();

            // `a` is the first factor in every vnmsubfp and in the four vmaddfp below; the
            // accumulate pass flips the order and puts `y` first. Rule 4: the slot is part of the
            // semantics on NaN, so every order is the lifted one.
            let d2 = vmx::vnmsubfp(a_v, t2, _mm_castsi128_ps(c2));
            let t3_bits = vmx::vor(t3_left, t3_right); // vor128 v2,v57,v56
            let t3 = _mm_castsi128_ps(t3_bits);
            let acc1 = vmx::lvx128(g, acc_base.wrapping_add(16 + u))?;
            let d0 = vmx::vnmsubfp(a_v, t0, _mm_castsi128_ps(c0));
            let acc2 = vmx::lvx128(g, acc_base.wrapping_add(32 + u))?;
            let d1 = vmx::vnmsubfp(a_v, t1, _mm_castsi128_ps(c1));
            let acc3 = vmx::lvx128(g, acc_base.wrapping_add(48 + u))?;
            let acc0 = vmx::lvx128(g, acc_base.wrapping_add(u))?;
            let d3 = vmx::vnmsubfp(a_v, t3, _mm_castsi128_ps(c3));

            // The four `y` fold each `d` back over its tap, with the `d` stores interleaved in the
            // lifted order: d2, d0, d1, d3.
            let y2 = vmx::vmaddfp(a_v, d2, t2);
            vmx::stvx128_ps(g, d.wrapping_add(32 + u), d2)?;
            vmx::stvx128_ps(g, d.wrapping_add(u), d0)?;
            let y0 = vmx::vmaddfp(a_v, d0, t0);
            let y1 = vmx::vmaddfp(a_v, d1, t1);
            vmx::stvx128_ps(g, d.wrapping_add(16 + u), d1)?;
            let y3 = vmx::vmaddfp(a_v, d3, t3);
            vmx::stvx128_ps(g, d.wrapping_add(48 + u), d3)?;

            let out2 = vmx::vmaddfp(y2, g_v, _mm_castsi128_ps(acc2));
            let out0 = vmx::vmaddfp(y0, g_v, _mm_castsi128_ps(acc0));
            let out1 = vmx::vmaddfp(y1, g_v, _mm_castsi128_ps(acc1));
            let out3 = vmx::vmaddfp(y3, g_v, _mm_castsi128_ps(acc3));

            vmx::stvx128_ps(g, acc_base.wrapping_add(32 + u), out2)?;
            vmx::stvx128_ps(g, acc_base.wrapping_add(u), out0)?;
            vmx::stvx128_ps(g, acc_base.wrapping_add(16 + u), out1)?;
            vmx::stvx128_ps(g, acc_base.wrapping_add(48 + u), out3)?;

            last = AllpassClobbers {
                v2: words(t3_bits),
                v29: words(c3),
                v30: words(_mm_castps_si128(y2)),
                v31: words(acc2),
            };
        }
    }

    Ok(AllpassResult {
        r3,
        clobbers: Some(last),
    })
}

// ------------------------------------------------------------------- the stage dispatcher above it

/// `lwz r6,0(r11)` — the stage descriptor's `c` buffer.
pub const STAGE_TAP_SOURCE: u32 = 0;
/// `lwz r7,4(r7)` — the tap, read unaligned by the stage.
pub const STAGE_TAP_DELAY: u32 = 4;
/// `lwz r9,8(r7)` — non-zero bypasses the stage and clears its accumulator instead.
pub const STAGE_BYPASS: u32 = 8;
/// `lwz r8,16(r7)` — the stage's `d` output.
pub const STAGE_OUT: u32 = 16;
/// `lwz r9,20(r7)` — the accumulator, and the clear's target.
pub const STAGE_ACCUMULATOR: u32 = 20;
/// `lfs f1,16(r6)` — the coefficient block's tap gain, the stage's `a`.
pub const COEFF_TAP_GAIN: u32 = 16;
/// `lfs f2,20(r3)` — its stage gain, the stage's `g`.
pub const COEFF_STAGE_GAIN: u32 = 20;

/// What [`dispatch_stage`] did.
#[derive(Clone, Copy, Debug)]
pub enum DispatchOutcome {
    /// The stage ran, and these are the registers it left — the dispatcher tail-branches, so they
    /// are the dispatcher's results too.
    Ran(AllpassResult),
    /// The stage was bypassed and its accumulator cleared.
    Cleared,
}

/// Run one allpass stage from its descriptor, or clear its accumulator instead (`sub_82B38B68`).
///
/// `coeffs` is `r3`, `count` `r4`, `primed` `r5` and `stage` `r7`, all at full width. When the
/// descriptor's bypass word is zero the loads feed [`allpass_stage`] through a tail branch —
/// `r3 = count`, the four buffers from the descriptor, `r10 = primed`. Otherwise the accumulator is
/// cleared through the guest's own memset, for `count * 4` bytes computed as a 32-bit rotate.
pub fn dispatch_stage(
    g: &mut Guest,
    coeffs: u64,
    count: u64,
    primed: u64,
    stage: u64,
) -> Result<DispatchOutcome> {
    let coeffs = coeffs as u32;
    let stage = stage as u32;
    if g.u32(stage + STAGE_BYPASS)? == 0 {
        // cmplwi cr6,r9,0 -- an unsigned compare of the low word.
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        let gain = fp::load_single(g, coeffs.wrapping_add(COEFF_STAGE_GAIN))?; // lfs f2,20(r3)
        let acc = g.u32(stage + STAGE_ACCUMULATOR)?; // lwz r9,20(r7)
        let a = fp::load_single(g, coeffs.wrapping_add(COEFF_TAP_GAIN))?; // lfs f1,16(r6)
        let out = g.u32(stage + STAGE_OUT)?; // lwz r8,16(r7)
        let delay = g.u32(stage + STAGE_TAP_DELAY)?; // lwz r7,4(r7)
        let source = g.u32(stage + STAGE_TAP_SOURCE)?; // lwz r6,0(r11)
        drop(fpscr);
        // b 0x82b389a0
        let ran = allpass_stage(
            g,
            count as u32 as i32,
            u64::from(source),
            delay,
            out,
            u64::from(acc),
            primed as u32 as i32,
            a,
            gain,
        )?;
        return Ok(DispatchOutcome::Ran(ran));
    }
    // loc_82B38BA0: rlwinm r5,r4,2,0,29 ; lwz r3,20(r11) ; b 0x82f52040
    let bytes = u64::from((count as u32).rotate_left(2) & 0xFFFF_FFFC);
    let target = g.u32(stage + STAGE_ACCUMULATOR)?;
    crate::mem::memset_82f52040(g, target, 0, bytes)?;
    Ok(DispatchOutcome::Cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const C: u32 = BASE + 0x1000;
    const T: u32 = BASE + 0x2000;
    const D: u32 = BASE + 0x3000;
    const ACC: u32 = BASE + 0x4000;
    const POISON: u32 = 0xDEAD_BEEF;
    const A: f64 = 0.75;
    const GAIN: f64 = 0.5;

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x6000);
        for i in 0..64u32 {
            g.set_u32(C + i * 4, (1.0 + i as f32).to_bits()).unwrap();
            g.set_u32(T + i * 4, (0.25 * (i as f32 + 1.0)).to_bits())
                .unwrap();
            g.set_u32(D + i * 4, POISON).unwrap();
            g.set_u32(ACC + i * 4, (100.0 + i as f32).to_bits())
                .unwrap();
        }
        g
    }

    fn at(g: &Guest, base: u32, i: u32) -> f32 {
        g.f32(base + i * 4).unwrap()
    }

    /// The arithmetic, written per lane in the order and precision the kernel uses — two roundings
    /// per multiply-add, as the recomp computes them.
    fn model(c: f32, tap: f32, acc: f32) -> (f32, f32) {
        let a = A as f32;
        let gain = GAIN as f32;
        let d = c - a * tap; // vnmsubfp: the product rounds, then the subtract
        let y = a * d + tap; // vmaddfp
        (d, y * gain + acc) // vmaddfp, y first
    }

    #[test]
    fn it_computes_the_allpass_pair_and_accumulates_at_gain() {
        let mut g = guest();
        let out =
            allpass_stage(&mut g, 16, u64::from(C), T, D, u64::from(ACC), 1, A, GAIN).unwrap();

        assert_eq!(
            out.r3,
            u64::from(ACC) - u64::from(C),
            "r3 is the 64-bit acc - c delta"
        );
        assert!(out.clobbers.is_some(), "the loop ran, so v2/v29-v31 moved");
        for i in 0..16u32 {
            let (d_want, acc_want) =
                model(1.0 + i as f32, 0.25 * (i as f32 + 1.0), 100.0 + i as f32);
            assert_eq!(at(&g, D, i), d_want, "d[{i}]");
            assert_eq!(at(&g, ACC, i), acc_want, "acc[{i}]");
        }
        // Exactly 16 samples: the 17th of each buffer is untouched.
        assert_eq!(g.u32(D + 16 * 4).unwrap(), POISON);
        assert_eq!(at(&g, ACC, 16), 116.0);
    }

    #[test]
    fn a_primed_accumulator_is_kept_and_an_unprimed_one_is_cleared() {
        // r10 != 0 accumulates onto what is there; r10 == 0 clears first, so the same call gives
        // just the stage's own output.
        let mut primed = guest();
        allpass_stage(
            &mut primed,
            16,
            u64::from(C),
            T,
            D,
            u64::from(ACC),
            1,
            A,
            GAIN,
        )
        .unwrap();
        let mut cleared = guest();
        allpass_stage(
            &mut cleared,
            16,
            u64::from(C),
            T,
            D,
            u64::from(ACC),
            0,
            A,
            GAIN,
        )
        .unwrap();

        for i in 0..16u32 {
            let (_, with_prior) = model(1.0 + i as f32, 0.25 * (i as f32 + 1.0), 100.0 + i as f32);
            let (_, from_zero) = model(1.0 + i as f32, 0.25 * (i as f32 + 1.0), 0.0);
            assert_eq!(at(&primed, ACC, i), with_prior, "primed [{i}]");
            assert_eq!(at(&cleared, ACC, i), from_zero, "cleared [{i}]");
        }
        assert_ne!(
            at(&primed, ACC, 0),
            at(&cleared, ACC, 0),
            "the flag has to matter"
        );
    }

    #[test]
    fn the_clear_is_by_cache_line_from_the_floor() {
        // `dcbzl` clears whole 128-byte lines from `acc & !127`, so an accumulator 16 bytes into a
        // line has the 16 bytes below it cleared as well. 4*16 = 64 bytes of samples means one line.
        let mut g = guest();
        let acc = ACC + 16;
        for i in 0..8u32 {
            g.set_u32(ACC + i * 4, POISON).unwrap(); // below the pointer, inside the line
        }
        allpass_stage(&mut g, 16, u64::from(C), T, D, u64::from(acc), 0, A, GAIN).unwrap();
        for i in 0..4u32 {
            assert_eq!(
                g.u32(ACC + i * 4).unwrap(),
                0,
                "word {i} below the pointer was cleared"
            );
        }
    }

    #[test]
    fn the_tap_may_be_unaligned() {
        // Every tap group is `lvlx(p) | lvrx(p + 16)`, so a tap four bytes into a line still reads
        // four consecutive floats per lane. The taps then come from T+4 onwards, and the answer
        // differs from the aligned call at every sample.
        let mut g = guest();
        allpass_stage(
            &mut g,
            16,
            u64::from(C),
            T + 4,
            D,
            u64::from(ACC),
            1,
            A,
            GAIN,
        )
        .unwrap();
        for i in 0..16u32 {
            let tap = 0.25 * (i as f32 + 2.0); // one float further along
            let (d_want, acc_want) = model(1.0 + i as f32, tap, 100.0 + i as f32);
            assert_eq!(at(&g, D, i), d_want, "unaligned d[{i}]");
            assert_eq!(at(&g, ACC, i), acc_want, "unaligned acc[{i}]");
        }
    }

    #[test]
    fn the_trip_count_is_a_ceiling_and_overruns_the_count() {
        // n = 20 gives quads = 5 and ceil(5/4) = 2 iterations, which is 32 samples — twelve past
        // the count. The C++ window builder declares the same over-run, so this is the original's
        // behaviour rather than a bug introduced here.
        let mut g = guest();
        allpass_stage(&mut g, 20, u64::from(C), T, D, u64::from(ACC), 1, A, GAIN).unwrap();
        for i in [19u32, 20, 31] {
            let (d_want, _) = model(1.0 + i as f32, 0.25 * (i as f32 + 1.0), 100.0 + i as f32);
            assert_eq!(at(&g, D, i), d_want, "d[{i}] is written even past n");
        }
        assert_eq!(
            g.u32(D + 32 * 4).unwrap(),
            POISON,
            "and it stops at the next multiple of 16"
        );
    }

    #[test]
    fn a_count_below_four_writes_nothing_and_leaves_r3_alone() {
        for n in [0i32, 1, 3] {
            let mut g = guest();
            let out =
                allpass_stage(&mut g, n, u64::from(C), T, D, u64::from(ACC), 1, A, GAIN).unwrap();
            assert_eq!(
                out.r3, n as u64,
                "n = {n}: the subf is below the ble, so r3 is untouched"
            );
            assert!(out.clobbers.is_none(), "n = {n}: no iteration, no clobbers");
            assert_eq!(g.u32(D).unwrap(), POISON, "n = {n}: nothing written");
        }
    }

    #[test]
    fn the_clobbers_are_the_last_groups_the_loop_touched() {
        let mut g = guest();
        let out =
            allpass_stage(&mut g, 16, u64::from(C), T, D, u64::from(ACC), 1, A, GAIN).unwrap();
        let c = out.clobbers.unwrap();
        // v29 is the last `c` group: samples 12..15, and the register holds them lane-reversed the
        // way every vector load in this crate does.
        assert_eq!(f32::from_bits(c.v29[3]), 13.0, "v29 lane 3 is c[12]");
        assert_eq!(f32::from_bits(c.v29[0]), 16.0, "v29 lane 0 is c[15]");
        // v31 is the accumulator group *before* the store, so it still holds the entry values.
        assert_eq!(f32::from_bits(c.v31[3]), 108.0, "v31 is pre-store: acc[8]");
        // v2 is the last tap group, and v30 the last y.
        assert_eq!(f32::from_bits(c.v2[3]), 0.25 * 13.0, "v2 lane 3 is t[12]");
        let (_, _) = model(1.0, 0.25, 0.0);
        let a = A as f32;
        let expected_y = |i: u32| {
            let c = 1.0 + i as f32;
            let tap = 0.25 * (i as f32 + 1.0);
            a * (c - a * tap) + tap
        };
        assert_eq!(
            f32::from_bits(c.v30[3]),
            expected_y(8),
            "v30 lane 3 is y for sample 8"
        );
    }

    // ------------------------------------------------------------------------- the dispatcher

    const STAGE: u32 = BASE + 0x5000;
    const COEFFS: u32 = BASE + 0x5100;

    fn descriptor(g: &mut Guest, bypass: u32) {
        g.set_u32(STAGE + STAGE_TAP_SOURCE, C).unwrap();
        g.set_u32(STAGE + STAGE_TAP_DELAY, T).unwrap();
        g.set_u32(STAGE + STAGE_BYPASS, bypass).unwrap();
        g.set_u32(STAGE + STAGE_OUT, D).unwrap();
        g.set_u32(STAGE + STAGE_ACCUMULATOR, ACC).unwrap();
        g.set_u32(COEFFS + COEFF_TAP_GAIN, (A as f32).to_bits())
            .unwrap();
        g.set_u32(COEFFS + COEFF_STAGE_GAIN, (GAIN as f32).to_bits())
            .unwrap();
    }

    #[test]
    fn the_dispatcher_runs_the_stage_it_describes() {
        let mut g = guest();
        descriptor(&mut g, 0);
        let mut h = g.clone();
        let out = dispatch_stage(&mut g, u64::from(COEFFS), 16, 1, u64::from(STAGE)).unwrap();
        let direct =
            allpass_stage(&mut h, 16, u64::from(C), T, D, u64::from(ACC), 1, A, GAIN).unwrap();
        match out {
            DispatchOutcome::Ran(r) => assert_eq!(r.r3, direct.r3),
            DispatchOutcome::Cleared => panic!("a zero bypass word must run the stage"),
        }
        for i in 0..16u32 {
            assert_eq!(
                g.u32(D + 4 * i).unwrap(),
                h.u32(D + 4 * i).unwrap(),
                "d[{i}]"
            );
            assert_eq!(
                g.u32(ACC + 4 * i).unwrap(),
                h.u32(ACC + 4 * i).unwrap(),
                "acc[{i}]"
            );
        }
    }

    #[test]
    fn a_bypassed_stage_clears_count_words_of_its_accumulator_and_nothing_else() {
        let mut g = guest();
        descriptor(&mut g, 7);
        let out = dispatch_stage(&mut g, u64::from(COEFFS), 12, 1, u64::from(STAGE)).unwrap();
        assert!(matches!(out, DispatchOutcome::Cleared));
        for i in 0..12u32 {
            assert_eq!(g.u32(ACC + 4 * i).unwrap(), 0, "acc[{i}] cleared");
        }
        assert_eq!(at(&g, ACC, 12), 112.0, "twelve words, not thirteen");
        assert_eq!(
            g.u32(D).unwrap(),
            POISON,
            "and the stage's output is untouched"
        );
    }
}
