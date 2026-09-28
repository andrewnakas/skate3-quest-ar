//! `sub_824531C8` — four-lane sine: range reduction, then an 11-term odd polynomial.
//!
//! The hottest function in the audio set by two orders of magnitude: **9,572,672 calls in a boot
//! session and 9,906,721 in a played one**, on `RwAudioCore Dac`. `STATUS: verified` with zero
//! divergence over 6,994,118 compared calls, which makes it the strongest evidence in the project.
//! It is also the function that motivated the harness's `ShadowResults` mask: it has **no stores at
//! all**, its result lives in `v1`, and before the mask existed comparing it checked nothing on
//! eight million calls — a total vacuous green (`probe/ports/notes/sub_824531C8.md`).
//!
//! It is a leaf, it takes no pointer, and it reads only four rodata vectors. So the translation
//! risk is concentrated in exactly three places, all of which the tests below aim at:
//!
//! 1. **The `vspltw128` immediates are reversed** and every coefficient comes through one. A
//!    single wrong immediate picks a different coefficient and the function still runs.
//! 2. **Every `vmaddfp` rounds twice, as the recomp computes it.** The recomp is built without FMA,
//!    so each Horner step is a multiply and then an add. *Corrected 2026-09-13:* this said
//!    single-rounding, and under that reading 1,996 of 2,000 recorded calls replayed and 4 did not;
//!    with two roundings all 2,000 do. The chain is eleven steps, alternating with eleven
//!    `vmulfp128`, and any step written as a fused `mul_add` rounds once where the recomp rounds twice.
//! 3. **It clobbers `v31`**, which the ABI preserves and the harness compares unconditionally, so
//!    the clobber is part of the observable result.
//!
//! ## The constants, read live and checked against the image
//!
//! The four coefficient vectors are at `0x82300000` minus 26544 / 26688 / 26672 / 26656. Each
//! address is asserted below as `((lis_imm & 0xFFFF) << 16) + offset`, **computed** and never read
//! off the disassembly by eye — one misread digit of a `lis`-based address in `sub_82B2FE00` cost
//! this project its first shadow divergence.
//!
//! Their values, read out of the validated image dump (`probe/harness/out/image/g_8220.bin`), are
//! the Taylor series of sine and `1/2π`:
//!
//! ```text
//! 0x822F9850  A  pi          2*pi        1/pi        1/(2*pi)
//! 0x822F97C0  B  1.0         -1/3!       1/5!        -1/7!
//! 0x822F97D0  C  1/9!        -1/11!      1/13!       -1/15!
//! 0x822F97E0  D  1/17!       -1/19!      1/21!       -1/23!
//! ```
//!
//! (guest word order, most significant first). The body loads every one of them **through the guest
//! map** rather than folding the numbers in, because that is what the original does and because a
//! patched image should reach the port. The dump values are used only by the tests, and only to
//! give them a genuinely independent oracle — `f64::sin` — which is what makes a wrong splat
//! immediate or a swapped lane visible instead of merely self-consistent.

use crate::vmx::{self, Fpscr};
use crate::{Guest, Result};
use core::arch::x86_64::*;

/// `lis -32208` — the one high half all four addresses are formed from, computed from the
/// immediate rather than transcribed.
const LIS_82300000: u32 = ((-32208i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82300000 == 0x8230_0000, "lis -32208");

/// `addi r9,r11,-26544` — the `[pi, 2pi, 1/pi, 1/(2pi)]` vector.
pub const VECTOR_A: u32 = LIS_82300000.wrapping_add(-26544i32 as u32);
/// `addi r8,r10,-26688` — `[1, -1/3!, 1/5!, -1/7!]`.
pub const VECTOR_B: u32 = LIS_82300000.wrapping_add(-26688i32 as u32);
/// `addi r5,r7,-26672` — `[1/9!, -1/11!, 1/13!, -1/15!]`.
pub const VECTOR_C: u32 = LIS_82300000.wrapping_add(-26672i32 as u32);
/// `addi r4,r6,-26656` — `[1/17!, -1/19!, 1/21!, -1/23!]`.
pub const VECTOR_D: u32 = LIS_82300000.wrapping_add(-26656i32 as u32);

const _: () = assert!(VECTOR_A == 0x822F_9850);
const _: () = assert!(VECTOR_B == 0x822F_97C0);
const _: () = assert!(VECTOR_C == 0x822F_97D0);
const _: () = assert!(VECTOR_D == 0x822F_97E0);
// All four are 16-byte aligned, which is what makes `lvx128`'s masking of the low address bits a
// no-op on them. If that ever stopped holding the loads would silently read the containing block.
const _: () =
    assert!(VECTOR_A % 16 == 0 && VECTOR_B % 16 == 0 && VECTOR_C % 16 == 0 && VECTOR_D % 16 == 0);

/// Everything `sub_824531C8` leaves in a vector register, as raw lane bits.
///
/// The function writes no memory, so this *is* its whole observable effect. Twenty registers are
/// listed because the lifted body leaves twenty behind and the harness compares `v14`-`v31`
/// unconditionally — `v31` in particular, which the ABI would preserve and this function clobbers
/// without saving. A Rust caller normally wants [`SineResult::v1`] and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SineResult {
    /// `v1` — the result, `sin(x)` per lane. The only register the port's result mask names.
    pub v1: [u32; 4],
    /// `v0` — the last power of `t` computed, `t^23`.
    pub v0: [u32; 4],
    /// `v12` — the Horner sum after the `c9` term.
    pub v12: [u32; 4],
    /// `v13` — the Horner sum after the `c10` term.
    pub v13: [u32; 4],
    /// `v59` — `t^2`.
    pub v59: [u32; 4],
    /// `v60` — `x * (1/2pi)` before rounding.
    pub v60: [u32; 4],
    /// `v61` — the splat of `1/(2pi)`.
    pub v61: [u32; 4],
    /// `v62` — vector D as loaded.
    pub v62: [u32; 4],
    /// `v63` — vector C as loaded.
    pub v63: [u32; 4],
    /// `v31`, `v2` … `v11` — the eleven coefficient splats, in coefficient order `c1` … `c11`.
    /// `c1` is the one that lands in `v31`, the preserved register the harness always compares.
    pub coefficients: [[u32; 4]; 11],
}

/// `sub_824531C8`. `x` is `v1` on entry, in **host lane order** — lane 0 is the guest word a
/// `lvx128` would have read from `ea + 12`.
///
/// Returns `sin(x)` per lane, plus every other register the original leaves behind; see
/// [`SineResult`]. Reads four rodata vectors and writes nothing.
pub fn sine4(g: &Guest, x: [u32; 4]) -> Result<SineResult> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    unsafe { sine4_impl(g, x) }
}

/// The lane bits of `sin(x)` alone, for callers that do not care about the clobbers.
pub fn sine4_value(g: &Guest, x: [u32; 4]) -> Result<[u32; 4]> {
    Ok(sine4(g, x)?.v1)
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn sine4_impl(g: &Guest, x: [u32; 4]) -> Result<SineResult> {
    unsafe {
        // The four lvx128 come before any float work and are pure integer loads, so no flush mode
        // is established yet. Load order is the lifted one.
        let a = vmx::lvx128(g, VECTOR_A)?;
        let reduce_scale = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W3 }>(a));
        let b = vmx::lvx128(g, VECTOR_B)?;
        let reduce_step = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W1 }>(a));
        let c1 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W1 }>(b)); // -> v31
        let c2 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W2 }>(b));
        let c = vmx::lvx128(g, VECTOR_C)?;
        let x = _mm_loadu_ps(x.as_ptr() as *const f32);

        let mut fpscr = Fpscr::capture();
        fpscr.enable_flush_mode_unconditional(); // emitted before the first vector float op

        let scaled = vmx::vmulfp(x, reduce_scale); // vmulfp128 v60,v1,v61
        let c3 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W3 }>(b));
        let c4 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W0 }>(c));
        let d = vmx::lvx128(g, VECTOR_D)?;
        let c5 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W1 }>(c));
        let c6 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W2 }>(c));
        let c7 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W3 }>(c));
        let c8 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W0 }>(d));
        let c9 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W1 }>(d));
        let c10 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W2 }>(d));
        let c11 = _mm_castsi128_ps(vmx::vspltw128::<{ vmx::SPLAT_W3 }>(d));

        // Range reduction. vrfin128 rounds to nearest; vnmsubfp is `x - 2pi*turns` with the
        // product rounded to single first, as the recomp computes it (vmx.rs, rule 1).
        let turns = vmx::vrfin(scaled); // vrfin128 v12,v60
        let t = vmx::vnmsubfp(reduce_step, turns, x); // vnmsubfp v0,v13,v12,v1
        let t2 = vmx::vmulfp(t, t); // vmulfp128 v59,v0,v0

        // Odd-power Horner: each step multiplies the running power by t^2 and folds in one
        // coefficient with a vmaddfp, which the recomp rounds twice. Writing any step as a fused
        // mul_add changes the answer: on 4 of 2,000 recorded calls, which is how it was found.
        let mut power_a = vmx::vmulfp(t2, t); // vmulfp128 v13,v59,v0   -> t^3
        let mut power_b = vmx::vmulfp(power_a, t2); // vmulfp128 v12,v13,v59  -> t^5
        let mut sum = vmx::vmaddfp(c1, power_a, t); // vmaddfp v13,v31,v13,v0
        power_a = vmx::vmulfp(power_b, t2); // vmulfp128 v0,v12,v59
        sum = vmx::vmaddfp(c2, power_b, sum); // vmaddfp v12,v2,v12,v13
        power_b = vmx::vmulfp(power_a, t2); // vmulfp128 v13,v0,v59
        sum = vmx::vmaddfp(c3, power_a, sum); // vmaddfp v12,v3,v0,v12
        power_a = vmx::vmulfp(power_b, t2); // vmulfp128 v0,v13,v59
        sum = vmx::vmaddfp(c4, power_b, sum); // vmaddfp v12,v4,v13,v12
        power_b = vmx::vmulfp(power_a, t2); // vmulfp128 v13,v0,v59
        sum = vmx::vmaddfp(c5, power_a, sum); // vmaddfp v12,v5,v0,v12
        power_a = vmx::vmulfp(power_b, t2); // vmulfp128 v0,v13,v59
        sum = vmx::vmaddfp(c6, power_b, sum); // vmaddfp v12,v6,v13,v12
        power_b = vmx::vmulfp(power_a, t2); // vmulfp128 v13,v0,v59
        sum = vmx::vmaddfp(c7, power_a, sum); // vmaddfp v12,v7,v0,v12
        power_a = vmx::vmulfp(power_b, t2); // vmulfp128 v0,v13,v59
        sum = vmx::vmaddfp(c8, power_b, sum); // vmaddfp v12,v8,v13,v12
        power_b = vmx::vmulfp(power_a, t2); // vmulfp128 v13,v0,v59
        let sum_9 = vmx::vmaddfp(c9, power_a, sum); // vmaddfp v12,v9,v0,v12
        power_a = vmx::vmulfp(power_b, t2); // vmulfp128 v0,v13,v59
        let sum_10 = vmx::vmaddfp(c10, power_b, sum_9); // vmaddfp v13,v10,v13,v12
        let result = vmx::vmaddfp(c11, power_a, sum_10); // vmaddfp v1,v11,v0,v13

        Ok(SineResult {
            v1: lanes_ps(result),
            v0: lanes_ps(power_a),
            v12: lanes_ps(sum_9),
            v13: lanes_ps(sum_10),
            v59: lanes_ps(t2),
            v60: lanes_ps(scaled),
            v61: lanes_ps(reduce_scale),
            v62: lanes(d),
            v63: lanes(c),
            coefficients: [
                lanes_ps(c1),
                lanes_ps(c2),
                lanes_ps(c3),
                lanes_ps(c4),
                lanes_ps(c5),
                lanes_ps(c6),
                lanes_ps(c7),
                lanes_ps(c8),
                lanes_ps(c9),
                lanes_ps(c10),
                lanes_ps(c11),
            ],
        })
    }
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn lanes(v: __m128i) -> [u32; 4] {
    let mut out = [0u32; 4];
    unsafe { _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, v) };
    out
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn lanes_ps(v: __m128) -> [u32; 4] {
    unsafe { lanes(_mm_castps_si128(v)) }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The four coefficient vectors as they appear in the validated image dump, guest word order
    /// (most significant word first). Used only to furnish a guest map for the tests; the body
    /// reads them live.
    const A_WORDS: [u32; 4] = [0x4049_0FDB, 0x40C9_0FDB, 0x3EA2_F983, 0x3E22_F983];
    const B_WORDS: [u32; 4] = [0x3F80_0000, 0xBE2A_AAAB, 0x3C08_8889, 0xB950_0D01];
    const C_WORDS: [u32; 4] = [0x3638_EF1D, 0xB2D7_322B, 0x2F30_9231, 0xAB57_3F9F];
    const D_WORDS: [u32; 4] = [0x274A_963C, 0xA317_A4DA, 0x1EB8_DC78, 0x9A3B_0DA1];

    pub(crate) fn image() -> Guest {
        // 0x822F97C0 .. 0x822F9860 covers all four vectors in one span.
        let mut g = Guest::single(VECTOR_B, 0xA0);
        for (base, words) in [
            (VECTOR_A, A_WORDS),
            (VECTOR_B, B_WORDS),
            (VECTOR_C, C_WORDS),
            (VECTOR_D, D_WORDS),
        ] {
            for (i, w) in words.iter().enumerate() {
                g.set_u32(base + 4 * i as u32, *w).unwrap();
            }
        }
        g
    }

    /// The eleven coefficients in **series order** — `c1` … `c11`, the multipliers of `t^3` …
    /// `t^23`. Which guest word supplies which is decided entirely by the eleven `vspltw128`
    /// immediates, so this array is the mapping those immediates have to produce.
    const SERIES: [u32; 11] = [
        B_WORDS[1], B_WORDS[2], B_WORDS[3], //  -1/3!,  1/5!,  -1/7!
        C_WORDS[0], C_WORDS[1], C_WORDS[2], C_WORDS[3], //  1/9!, -1/11!, 1/13!, -1/15!
        D_WORDS[0], D_WORDS[1], D_WORDS[2], D_WORDS[3], //  1/17!, -1/19!, 1/21!, -1/23!
    ];

    fn bits(x: f32) -> u32 {
        x.to_bits()
    }

    /// An independent scalar model of the same kernel: one lane, `f32`, single-rounded
    /// multiply-adds, written from the algorithm rather than from the vector code.
    ///
    /// The original forms each power as `previous_power * t^2` and folds in one coefficient per
    /// step, alternating between two registers only because it has two to alternate between; the
    /// sequence of multiplies and their operands is the plain loop below. It exists to be
    /// disagreed with — if the vector body fuses a pair the guest keeps separate, or reaches a
    /// coefficient through the wrong splat immediate, this does not follow it there.
    #[inline(never)]
    fn reference_sine(x: f32) -> f32 {
        let f = |w: u32| f32::from_bits(w);
        let two_pi = f(A_WORDS[1]);
        let inv_two_pi = f(A_WORDS[3]);

        let turns = (x * inv_two_pi).round_ties_even(); // vmulfp128 then vrfin128
        let t = x - two_pi * turns; // vnmsubfp: two roundings in the recomp (corrected 2026-09-13)
        let t2 = t * t;

        let mut power = t2 * t; // t^3
        let mut sum = f(SERIES[0]) * power + t; // vmaddfp, unfused
        for k in 1..11 {
            power *= t2; // t^(2k+3), one multiply from the last, never powi
            sum = f(SERIES[k]) * power + sum;
        }
        sum
    }

    /// [`reference_sine`] evaluated under the same MXCSR the kernel runs under.
    ///
    /// Not fussiness: the kernel's `t^23` goes denormal for a small reduced argument, and under
    /// `FZ|DAZ` that term is zero where an unflushed model keeps it. Running both sides in one mode
    /// is what makes a bit-for-bit comparison mean something.
    fn reference_sine_ftz(x: f32) -> f32 {
        let mut f = Fpscr::capture();
        f.enable_flush_mode_unconditional();
        let r = reference_sine(x);
        drop(f);
        r
    }

    /// `f64::sin` of the argument this kernel actually reduces to — outside the translation
    /// entirely, and free of the f32 constants' own error, so it can be compared tightly.
    fn oracle_sine(x: f32) -> f64 {
        let inv_two_pi = f32::from_bits(A_WORDS[3]);
        let two_pi = f32::from_bits(A_WORDS[1]) as f64;
        let turns = (x * inv_two_pi).round_ties_even() as f64;
        (x as f64 - two_pi * turns).sin()
    }

    #[test]
    fn the_rodata_addresses_are_computed_from_the_lis_immediates() {
        // Not a tautology: the assertions at the top of the module derive each address from
        // `((imm & 0xFFFF) << 16) + offset`, and this repeats the arithmetic here so a reader can
        // see the numbers. Reading a lis-based address by eye caused this project's first shadow
        // divergence.
        assert_eq!(LIS_82300000, 0x8230_0000);
        assert_eq!(VECTOR_A, 0x8230_0000 - 26544);
        assert_eq!(VECTOR_B, 0x8230_0000 - 26688);
        assert_eq!(VECTOR_C, 0x8230_0000 - 26672);
        assert_eq!(VECTOR_D, 0x8230_0000 - 26656);
        // B is the lowest and A the highest; the four are not contiguous in load order.
        assert!(VECTOR_B < VECTOR_C && VECTOR_C < VECTOR_D && VECTOR_D < VECTOR_A);
    }

    #[test]
    fn the_coefficients_come_out_in_the_order_the_series_needs() {
        // This is the splat-immediate test. Every coefficient arrives through a `vspltw128` whose
        // immediate is reversed relative to the PPC element number, and a wrong one picks a
        // neighbouring coefficient — which still produces a plausible curve.
        let g = image();
        let r = sine4(&g, [bits(0.5); 4]).unwrap();

        for (k, want) in SERIES.iter().enumerate() {
            assert_eq!(
                r.coefficients[k],
                [*want; 4],
                "coefficient {} (the multiplier of t^{}) came from the wrong word",
                k + 1,
                2 * k + 3
            );
        }

        // Independently of the bit patterns: they must alternate in sign and be 1/(2k+3)!, which is
        // what says the *ordering* is right rather than just the bytes matching a list I typed.
        let mut factorial = 6.0f64;
        let mut sign = -1.0f64;
        for k in 0..11 {
            let got = f32::from_bits(SERIES[k]) as f64;
            let want = sign / factorial;
            assert!(
                (got - want).abs() <= 1e-5 * want.abs(),
                "coefficient {} is {got:e}, not 1/{}!",
                k + 1,
                2 * k + 3
            );
            let n = (2 * k + 4) as f64;
            factorial *= n * (n + 1.0);
            sign = -sign;
        }

        // And the reduction pair, which comes from a *different* vector with two more immediates.
        assert_eq!(
            f32::from_bits(r.v61[0]),
            (1.0 / std::f64::consts::TAU) as f32,
            "1/(2pi) splat"
        );
    }

    #[test]
    #[allow(clippy::approx_constant)] // the probes below are input angles, not uses of pi
    fn it_computes_sine_against_an_independent_oracle() {
        // f64::sin is outside this translation entirely. A swapped lane, a wrong splat immediate or
        // a reduction that used pi instead of 2pi all survive a self-consistency check and none of
        // them survives this one.
        let g = image();
        let probes: [f32; 12] = [
            0.0, 0.25, 1.0, -1.0, 1.5707964, 3.1415927, -3.1415927, 6.2831855, 10.0, -10.0, 100.0,
            0.1,
        ];
        for x in probes {
            let got = f32::from_bits(sine4(&g, [bits(x); 4]).unwrap().v1[0]) as f64;
            let want = oracle_sine(x);
            assert!(
                (got - want).abs() <= 5e-7,
                "sin({x}): got {got}, f64 oracle says {want}"
            );
        }
        // Tie the oracle itself to actual sine, so a shared mistake in the reduction cannot pass.
        let one = f32::from_bits(sine4(&g, [bits(1.0); 4]).unwrap().v1[0]);
        assert!((one - 0.841_470_98).abs() < 1e-6, "sin(1) = {one}");
        let half_pi = f32::from_bits(sine4(&g, [bits(1.570_796_4); 4]).unwrap().v1[0]);
        assert!((half_pi - 1.0).abs() < 1e-6, "sin(pi/2) = {half_pi}");
    }

    #[test]
    fn every_lane_is_computed_independently() {
        // Four different arguments in one call. If any splat, shuffle or broadcast leaked across
        // lanes, the four results would not each match the single-lane answer.
        let g = image();
        let xs = [0.3f32, -1.25, 2.75, 9.0];
        let together = sine4(&g, [bits(xs[0]), bits(xs[1]), bits(xs[2]), bits(xs[3])]).unwrap();
        for (lane, x) in xs.iter().enumerate() {
            let alone = sine4(&g, [bits(*x); 4]).unwrap();
            assert_eq!(together.v1[lane], alone.v1[0], "lane {lane} for x = {x}");
        }
    }

    #[test]
    fn it_matches_the_scalar_model_bit_for_bit() {
        // The strongest test here: an independently written per-lane model with the same rounding
        // structure, compared on the bits rather than on a tolerance. Every fused/unfused choice and
        // every coefficient is pinned by this.
        let g = image();
        let mut x = 0.013f32;
        for _ in 0..400 {
            let got = sine4(&g, [bits(x); 4]).unwrap().v1[0];
            assert_eq!(got, bits(reference_sine_ftz(x)), "x = {x}");
            x = x * -1.11 + 0.017;
        }
    }

    #[test]
    fn the_range_reduction_uses_2pi_and_rounds_to_nearest() {
        // `vrfin128` is the one operation here the recorded harness does not cover — its table has
        // `vrfiz128`, the round-*toward-zero* form, only. Substituting it is nearly invisible in the
        // output, because sine is periodic and the polynomial is still accurate over a whole turn.
        // `t^2` is where the substitution is loud: it lands on a different branch of the reduction.
        let g = image();
        let inv_two_pi = f32::from_bits(A_WORDS[3]);
        let two_pi = f32::from_bits(A_WORDS[1]) as f64;
        for x in [4.0f32, -4.0, 10.0, -10.0, 100.0, -100.0] {
            let r = sine4(&g, [bits(x); 4]).unwrap();
            let nearest = (x * inv_two_pi).round_ties_even() as f64;
            let truncated = (x * inv_two_pi).trunc() as f64;
            let t_near = x as f64 - two_pi * nearest;
            let t_trunc = x as f64 - two_pi * truncated;
            let got = f32::from_bits(r.v59[0]) as f64;
            assert!(
                (got - t_near * t_near).abs() <= 1e-4 * (1.0 + t_near * t_near),
                "x = {x}: t^2 is {got}, round-to-nearest says {}",
                t_near * t_near
            );
            assert_ne!(
                nearest, truncated,
                "x = {x} was chosen so the two rounds disagree"
            );
            assert!(
                (got - t_trunc * t_trunc).abs() > 1.0,
                "x = {x}: vrfiz would have given t^2 = {}",
                t_trunc * t_trunc
            );
        }

        // And the divisor is a whole turn, not a half one: x = 2pi scales to exactly one.
        let r = sine4(&g, [bits(std::f32::consts::TAU); 4]).unwrap();
        assert!(
            (f32::from_bits(r.v60[0]) - 1.0).abs() < 1e-6,
            "x = 2pi should scale to 1 turn"
        );
        assert!(
            f32::from_bits(r.v59[0]).abs() < 1e-12,
            "and reduce to t ~ 0"
        );

        // Negative arguments still land on the right value of sine, which a reduction that only
        // worked in one direction would not manage.
        for x in [-1.0f32, -3.0, -7.0, -100.0] {
            let got = f32::from_bits(sine4(&g, [bits(x); 4]).unwrap().v1[0]) as f64;
            assert!((got - oracle_sine(x)).abs() <= 5e-7, "sin({x}) = {got}");
        }
    }

    #[test]
    fn the_clobbered_preserved_register_is_reproduced() {
        // v31 holds c1 = -1/6 and the ABI would have preserved it; the original clobbers it without
        // saving, and the harness compares v14-v31 on every call regardless of the result mask. A
        // port that dropped the clobber would be a divergence on eight million calls.
        let g = image();
        let r = sine4(&g, [bits(1.0); 4]).unwrap();
        assert_eq!(
            f32::from_bits(r.coefficients[0][0]),
            -1.0 / 6.0,
            "c1 lands in v31"
        );
        assert_eq!(
            r.coefficients[0], [r.coefficients[0][0]; 4],
            "it is a splat, all four lanes"
        );
        // v62 and v63 are whole vectors, not splats: D and C as loaded.
        assert_eq!(
            r.v63,
            [C_WORDS[3], C_WORDS[2], C_WORDS[1], C_WORDS[0]],
            "v63 = C, lane-reversed"
        );
        assert_eq!(
            r.v62,
            [D_WORDS[3], D_WORDS[2], D_WORDS[1], D_WORDS[0]],
            "v62 = D, lane-reversed"
        );
    }

    #[test]
    fn a_missing_constant_is_an_error_not_a_zero() {
        // The map with no rodata in it at all. Inventing zeroes would turn a gap in coverage into a
        // wrong answer, which is the choice `eval::dispatch` refuses for the same reason.
        let empty = Guest::single(0x4000_0000, 0x100);
        let err = sine4(&empty, [bits(1.0); 4]).unwrap_err();
        assert_eq!(err.address, VECTOR_A);
    }

    #[test]
    fn it_runs_under_flush_to_zero_and_restores_the_entry_mode() {
        let g = image();
        let before = vmx::get_mxcsr();
        let r = sine4(&g, [0x0000_0001u32; 4]).unwrap(); // smallest denormal, as raw lane bits
        assert_eq!(
            vmx::get_mxcsr(),
            before,
            "MXCSR is restored when the body returns"
        );
        // Under FZ/DAZ a denormal argument is zero on the way in, so the answer is exactly +0 —
        // not the denormal itself, which is what a non-flushing translation would return.
        assert_eq!(r.v1[0], 0x0000_0000);
    }
}
