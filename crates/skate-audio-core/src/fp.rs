//! The guest's scalar floating-point idioms, as the lifted PowerPC performs them.
//!
//! Nothing here is a port of a guest function. These are the recurring *instruction* shapes
//! that the verified C++ bodies spell out inline — `lfs`, `stfs`, `extsw`/`fcfid`/`frsp`, the
//! single-rounded arithmetic forms, and `fctiwz` — factored out so that every op in `eval`
//! rounds in the same places the original does. Each helper is named after the instruction
//! sequence it stands for, and none of them is allowed to be "simplified" into a plain `f32`
//! computation: the guest holds intermediates in 64-bit FPRs and narrows only where a `.s`
//! form appears, which is not the same thing as computing in `f32`.
//!
//! **Flush-to-zero. Corrected 2026-09-12 — this paragraph previously said the opposite.** It
//! claimed `ctx.fpscr.disableFlushModeUnconditional()` writes MXCSR `0x0000`, denormals
//! preserved. It does not. `rex/ppc/context.h` sets `fpu_csr |= FlushMask` when it initialises
//! the host FPU (line 214) and re-applies that mask on every rounding-mode change (line 227), so
//! the **scalar** side carries flush-to-zero and denormals-are-zero exactly as the vector side
//! does. The two "flush mode" calls differ only in *rounding mode*.
//!
//! What that changes here: nothing yet, and the reason is worth stating rather than assuming.
//! Every operation in this module is a conversion or a single arithmetic step on values the game
//! supplies, and no test in this crate drives one with a denormal. A denormal input or an
//! intermediate that underflows to one **would** differ between this code and the recomp, and
//! nothing here would catch it. Treat that as an open gap, not as a cleared one. `vmx.rs`
//! implements the real behaviour and models the CSR directly.
//!
//! Rust's `f32`/`f64` operators also run under the host default's *masked* FP exceptions where
//! the guest leaves them unmasked. Masking changes whether a trap fires, never the value
//! produced. (The recomp's own hooks have to care — an unmasked SIGFPE inside a guest call
//! killed the first capture tap.)

use crate::{Guest, Result};

/// `lfs`: the 32-bit word reinterpreted as a single, widened to double.
///
/// Widening quiets a signalling NaN on the guest and in Rust alike; the payload it leaves is
/// the hardware's in both cases and is not pinned here.
#[inline]
pub fn single_from_bits(bits: u32) -> f64 {
    f32::from_bits(bits) as f64
}

/// `stfs`: round the double to single and take its bits.
#[inline]
pub fn single_to_bits(value: f64) -> u32 {
    (value as f32).to_bits()
}

/// `lfs` from guest memory.
#[inline]
pub fn load_single(g: &Guest, ea: u32) -> Result<f64> {
    Ok(single_from_bits(g.u32(ea)?))
}

/// `lfd` from guest memory: eight big-endian bytes reinterpreted as a double, no rounding.
///
/// Only `sub_82F4DE80` needs it so far — its two pool constants are `lfd`, not `lfs`, and the
/// second of them is `1e18`, which is not representable as a single at all.
#[inline]
pub fn load_double(g: &Guest, ea: u32) -> Result<f64> {
    Ok(f64::from_bits(g.u64(ea)?))
}

/// `stfs` to guest memory.
#[inline]
pub fn store_single(g: &mut Guest, ea: u32, value: f64) -> Result<()> {
    g.set_u32(ea, single_to_bits(value))
}

/// `extsw ; std ; lfd ; fcfid ; frsp`: a 32-bit field sign-extended, converted exactly to
/// double, then rounded to single and kept in a double.
///
/// The red-zone spill the original uses to move the word into an FPR is a bit copy and has no
/// effect on the value, so it is not reproduced. The `frsp` does: word magnitudes above 2^24
/// lose low bits here, and dropping the narrowing would make this op disagree with the guest
/// on ordinary sample counts.
#[inline]
pub fn word_to_single(word: u32) -> f64 {
    let wide = (word as i32) as i64; // extsw
    let converted = wide as f64; // fcfid
    (converted as f32) as f64 // frsp
}

/// `fadds`: the double sum rounded to single.
#[inline]
pub fn add_single(a: f64, b: f64) -> f64 {
    ((a + b) as f32) as f64
}

/// `fsubs`.
#[inline]
pub fn sub_single(a: f64, b: f64) -> f64 {
    ((a - b) as f32) as f64
}

/// `fmuls`.
#[inline]
pub fn mul_single(a: f64, b: f64) -> f64 {
    ((a * b) as f32) as f64
}

/// `fdivs`.
#[inline]
pub fn div_single(a: f64, b: f64) -> f64 {
    ((a / b) as f32) as f64
}

/// `fmadds`: one fused double-precision multiply-add, then rounded to single.
///
/// Fused because the lifted line is `fmadds`. A `mul_single` followed by an `add_single` would
/// round twice and is a different function; the operand order is kept for NaN propagation.
#[inline]
pub fn fmadd_single(a: f64, b: f64, c: f64) -> f64 {
    (a.mul_add(b, c) as f32) as f64
}

/// `fnmsubs`: `c - a*b`, one fused multiply-add negated, then rounded to single.
///
/// PowerPC's `fnmsub` computes `-(a*b - c)`, so the fusion is over the whole expression and the
/// product's low bits reach the subtraction. Written as the negation of a single `mul_add` for
/// that reason — `sub_single(c, mul_single(a, b))` rounds twice and is a different function. The
/// negation is exact, so applying it before or after the narrowing is the same value; it is
/// written before, as the lifted `double(float(-std::fma(a, b, -c)))` has it.
///
/// The operand order is part of the semantics: `a` is the value, `b` the coefficient, `c` the
/// accumulator, matching every `fnmsubs` site in `sub_82B43AF8`.
#[inline]
pub fn nmsub_single(a: f64, b: f64, c: f64) -> f64 {
    ((-(a.mul_add(b, -c))) as f32) as f64
}

/// `fsqrts`: the double square root rounded to single.
///
/// `f64::sqrt` lowers to `sqrtsd`, which is what `std::sqrt(double)` compiles to in the reference,
/// and both are correctly rounded — so the pair rounds exactly twice, in the same two places. It
/// runs under whatever MXCSR the caller established, which is why the bodies that use it hold a
/// [`crate::vmx::Fpscr`]: a denormal operand is flushed by the recomp and would not be here.
#[inline]
pub fn sqrt_single(a: f64) -> f64 {
    (a.sqrt() as f32) as f64
}

/// `frsp`: a double rounded to single precision and kept in a double.
///
/// Distinct from [`single_to_bits`] in that the value stays an `f64`. It is what every call site of
/// `sub_82F4DE80`, `sub_82F4DED0` and `sub_82F4DFB0` does to the returned `f1` before using it, so
/// the double-precision tail of those results never reaches the arithmetic that follows.
#[inline]
pub fn frsp(a: f64) -> f64 {
    (a as f32) as f64
}

/// `fmsubs`: `a*b - c`, one fused multiply-add, then rounded to single.
///
/// The mirror of [`nmsub_single`] without the negation. `sub_82B45788` uses it once, to take the
/// reduced angle out of the wrapped fraction, and the fusion is visible there: the product is
/// `fraction * 2π`, whose low bits the subtraction of the sector bound keeps.
#[inline]
pub fn fmsub_single(a: f64, b: f64, c: f64) -> f64 {
    (a.mul_add(b, -c) as f32) as f64
}

/// `fabs` on the double representation: the sign bit cleared, nothing else touched.
///
/// Written through the bits rather than as `f64::abs` because that is what the lifted line does —
/// `temp.u64 &= ~0x8000000000000000` — and because the two are only documented to agree on
/// non-NaN values. A NaN keeps its payload here, and `sub_82B454B8` compares the result of this
/// against a rodata epsilon on every call.
#[inline]
pub fn abs_double(value: f64) -> f64 {
    f64::from_bits(value.to_bits() & 0x7FFF_FFFF_FFFF_FFFF)
}

/// `fneg` on the double representation: the sign bit flipped.
#[inline]
pub fn neg_double(value: f64) -> f64 {
    f64::from_bits(value.to_bits() ^ 0x8000_0000_0000_0000)
}

/// `fctidz`: truncate toward zero into a 64-bit integer.
///
/// Written branch for branch, for the same reason [`crate::player::truncated_low_byte`] is — that
/// one is the `f32` entry point to the same instruction and this is the `f64` one, kept separate
/// rather than refactored together because the two lifted lines differ in their operand widths and
/// nothing is gained by making one call the other.
///
/// Rust's `as i64` **saturates** and `cvttsd2si` does not. The three edges, in the lifted order:
/// NaN yields the integer indefinite `i64::MIN`; anything strictly above `2^63` yields `i64::MAX`,
/// because the lifted test is `x > (double)LLONG_MAX` and takes that arm; and exactly `2^63` falls
/// through to `cvttsd2si`, which yields the indefinite — where `as i64` would give `i64::MAX`.
/// That single input is the whole reason this is not a cast.
#[inline]
pub fn fctidz(value: f64) -> i64 {
    const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
    if value.is_nan() {
        i64::MIN
    } else if value > TWO_POW_63 {
        i64::MAX
    } else if value >= TWO_POW_63 || value < -TWO_POW_63 {
        i64::MIN // cvttsd2si's integer indefinite, which saturation also reaches at the low end
    } else {
        value as i64
    }
}

/// `fctid`: round to nearest even, then to a 64-bit integer -- the lifted line branch for branch.
///
/// The same edges as [`fctidz`] and for the same reason: at exactly 2^63 the guest yields
/// `cvtsd2si`'s integer indefinite, `i64::MIN`, where Rust's saturating cast would give
/// `i64::MAX`. Only the in-range arm differs, rounding half to even as the recomp's MXCSR does.
#[inline]
pub fn fctid(value: f64) -> i64 {
    const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
    if value.is_nan() {
        i64::MIN
    } else if value > TWO_POW_63 {
        i64::MAX
    } else if value >= TWO_POW_63 || value < -TWO_POW_63 {
        i64::MIN
    } else {
        value.round_ties_even() as i64
    }
}

/// `fcfid`: a 64-bit integer converted to double, round to nearest even.
///
/// Exact below `2^53` and correctly rounded above it, in the guest and in Rust alike.
#[inline]
pub fn fcfid(value: i64) -> f64 {
    value as f64
}

/// `fsel fD,fA,fB,fC`: `fA >= 0.0 ? fB : fC`, with **NaN taking the else arm**.
///
/// A select, not a branch: both arms are values the caller has already computed, so reproducing it
/// as an `if` changes nothing except that the untaken arm is not evaluated. What it does pin is the
/// NaN direction — `>=` is false for NaN, so a NaN selector takes `c`, which is the whole of how
/// `sub_82F4DE80` returns a NaN unchanged.
#[inline]
pub fn fsel(a: f64, b: f64, c: f64) -> f64 {
    if a >= 0.0 { b } else { c }
}

/// `fctiwz f13,fX ; stfd f13,-k(r1) ; lwz rN,-k+4(r1)`: truncate toward zero into a word, spill
/// big-endian, read the low word back.
///
/// Written branch for branch because the guest's conversion and Rust's saturating `as i32`
/// disagree on NaN: RexGlue's lowering yields the x86 "integer indefinite" `0x80000000`, while
/// `f64 as i32` in Rust yields 0. Everywhere else the two agree, and
/// `tests::naive_cast_agrees_except_on_nan` pins that — the saturation Rust performs below
/// `i32::MIN` produces the same `0x80000000` the hardware's indefinite value has, and the
/// `value > i32::MAX` arm covers the other end before the cast can be reached.
///
/// The result is the *low word only*: the original's `lwz` takes four bytes out of an eight-byte
/// spill, so the caller sees it zero-extended, never sign-extended.
#[inline]
pub fn fctiwz_low_word(value: f64) -> u32 {
    if value.is_nan() {
        0x8000_0000
    } else if value > i32::MAX as f64 {
        i32::MAX as u32
    } else {
        (value as i32) as u32
    }
}

/// `rlwinm rA,rS,SH,MB,ME` as RexGlue lowers it: the low word is rotated into *both* halves of
/// a 64-bit value before the mask is applied.
///
/// Written out rather than treated as a 32-bit shift because the two are only equal for masks
/// that stay inside the low word. `sub_82B1C6C8`'s divide-by-100 uses it with a 27-bit rotate
/// and a 27-bit mask, where the doubled operand is what feeds the result.
#[inline]
pub fn rlwinm(value: u64, sh: u32, mask: u64) -> u64 {
    let lo = value & 0xFFFF_FFFF;
    (lo | (lo << 32)).rotate_left(sh) & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naive_cast_agrees_except_on_nan() {
        // The one case that forces the branch-for-branch form.
        assert_eq!(fctiwz_low_word(f64::NAN), 0x8000_0000);
        assert_eq!(
            (f64::NAN as i32) as u32,
            0,
            "the naive cast, which would diverge"
        );

        // Both ends of the range, where saturation happens to reproduce the hardware.
        assert_eq!(fctiwz_low_word(f64::INFINITY), i32::MAX as u32);
        assert_eq!(fctiwz_low_word(f64::NEG_INFINITY), 0x8000_0000);
        assert_eq!(fctiwz_low_word(3e9), i32::MAX as u32);
        assert_eq!(fctiwz_low_word(-3e9), 0x8000_0000);
        assert_eq!(fctiwz_low_word(i32::MAX as f64), 0x7FFF_FFFF);
        assert_eq!(fctiwz_low_word(i32::MIN as f64), 0x8000_0000);

        // Truncation toward zero, not rounding: this is what makes the callers add or subtract
        // a half before converting.
        assert_eq!(fctiwz_low_word(1.9), 1);
        assert_eq!(fctiwz_low_word(-1.9), (-1i32) as u32);
        assert_eq!(fctiwz_low_word(-0.5), 0);
        assert_eq!(fctiwz_low_word(0.0), 0);
    }

    #[test]
    fn word_to_single_narrows_where_the_guest_narrows() {
        assert_eq!(word_to_single(0), 0.0);
        assert_eq!(word_to_single(48_000), 48_000.0);
        // Sign extension, not zero extension: the field is a signed word.
        assert_eq!(word_to_single(0xFFFF_FFFF), -1.0);
        assert_eq!(word_to_single(0x8000_0000), i32::MIN as f64);

        // The frsp matters past 2^24. 16_777_217 is not representable as f32 and rounds to
        // even; a version that skipped the narrowing would return the exact integer.
        assert_eq!(word_to_single(16_777_217), 16_777_216.0);
        assert_ne!(
            16_777_217i32 as f64, 16_777_216.0,
            "the un-narrowed value differs"
        );
    }

    #[test]
    fn single_rounded_forms_round_once_at_the_end() {
        // The plain forms: a double operation whose result is narrowed. The distinguishing case
        // for "rounds once" is the fused multiply-add below; these pin the shape.
        let tiny = 2f64.powi(-25);
        assert_eq!(add_single(1.0, tiny), 1.0);
        assert_eq!(sub_single(1.0, tiny), 1.0);
        assert_eq!(mul_single(1.5, 2.0), 3.0);
        assert_eq!(div_single(1.0, 3.0), (1.0f32 / 3.0f32) as f64);

        // fmadds is fused: the product is not rounded before the add. Multiplying two f32-exact
        // values whose product needs 48 bits and adding its negation leaves 0 only if the
        // product was kept intact.
        let a = (1.0f32 + f32::EPSILON) as f64;
        let b = (1.0f32 - f32::EPSILON) as f64;
        assert_eq!(fmadd_single(a, b, -(a * b)), 0.0);
        // The unfused form rounds a*b to single first and cannot cancel.
        assert_ne!(add_single(mul_single(a, b), -(a * b)), 0.0);
    }

    #[test]
    fn fnmsubs_is_one_fused_step_and_not_a_multiply_then_a_subtract() {
        // `c - a*b` with the product needing 48 bits and `c` equal to the product's f32 rounding.
        // Fused, the discarded low bits survive and the result is non-zero; unfused, the multiply
        // throws them away first and the subtraction cancels exactly.
        let a = (1.0f32 + f32::EPSILON) as f64;
        let b = (1.0f32 - f32::EPSILON) as f64;
        let c = ((a * b) as f32) as f64; // == 1.0
        let fused = nmsub_single(a, b, c);
        assert_ne!(fused, 0.0, "the fusion has to reach the subtraction");
        assert_eq!(
            fused,
            ((c - a * b) as f32) as f64,
            "and it is c - a*b, not a*b - c"
        );
        assert_eq!(
            sub_single(c, mul_single(a, b)),
            0.0,
            "the unfused form, which cancels"
        );
        // The scope of that, stated rather than left to be assumed. The realistic mistranscription
        // is the line above — two `.s` roundings — and this test catches it. What it does *not*
        // catch is a transcription that keeps the intermediate in **double**: `((c - a*b) as f32)`,
        // asserted equal to the fused answer two lines up. For two `f32` operands the product needs
        // at most 48 bits and is exact in an `f64`, so the two agree here and differ only through
        // double rounding, which this input does not reach. Measured, not argued: breaking
        // `nmsub_single` to that form leaves this test green.

        // The sign is the other half of the instruction: fnmsub negates, fmsub does not.
        assert_eq!(nmsub_single(2.0, 3.0, 10.0), 4.0);
        assert_eq!(nmsub_single(2.0, 3.0, 0.0), -6.0);
    }

    #[test]
    fn lfs_and_stfs_are_a_bit_round_trip_for_singles() {
        let bits = 0x3F80_0000; // 1.0f
        assert_eq!(load_single_bits_round_trip(bits), bits);
        assert_eq!(single_from_bits(bits), 1.0);
        assert_eq!(single_to_bits(1.0), bits);
        // A double that does not fit a single is rounded by the store, as `stfs` does.
        assert_eq!(single_to_bits(1.0 + 2f64.powi(-30)), bits);
    }

    fn load_single_bits_round_trip(bits: u32) -> u32 {
        single_to_bits(single_from_bits(bits))
    }

    #[test]
    fn fctidz_disagrees_with_a_saturating_cast_at_exactly_two_to_the_63() {
        const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
        // The one input that forces the branch-for-branch form, and the reason is the lifted test
        // being `>` rather than `>=`: 2^63 itself falls through to cvttsd2si's indefinite.
        assert_eq!(fctidz(TWO_POW_63), i64::MIN);
        assert_eq!(
            TWO_POW_63 as i64,
            i64::MAX,
            "the naive cast, which would diverge"
        );
        // One ulp above takes the other arm.
        assert_eq!(fctidz(TWO_POW_63 * 1.0000001), i64::MAX);
        assert_eq!(fctidz(f64::INFINITY), i64::MAX);
        // NaN is the indefinite, where the cast gives 0.
        assert_eq!(fctidz(f64::NAN), i64::MIN);
        assert_eq!(f64::NAN as i64, 0, "the naive cast again");
        // The low end agrees with saturation, and is written out anyway.
        assert_eq!(fctidz(-TWO_POW_63), i64::MIN);
        assert_eq!(fctidz(f64::NEG_INFINITY), i64::MIN);
        // Truncation toward zero, both signs.
        assert_eq!(fctidz(2.9), 2);
        assert_eq!(fctidz(-2.9), -2);
        assert_eq!(fctidz(-0.5), 0);
        // And the round trip through fcfid is exact below 2^53.
        assert_eq!(fcfid(fctidz(-2.9)), -2.0);
        assert_eq!(fcfid(1i64 << 52), 4_503_599_627_370_496.0);
    }

    #[test]
    fn fsel_sends_nan_to_the_else_arm() {
        assert_eq!(fsel(0.0, 1.0, 2.0), 1.0, "+0 is >= 0");
        // -0.0 >= 0.0 is true in IEEE and in the hardware fsel alike.
        assert_eq!(fsel(-0.0, 1.0, 2.0), 1.0);
        assert_eq!(fsel(-1.0, 1.0, 2.0), 2.0);
        assert_eq!(fsel(f64::NAN, 1.0, 2.0), 2.0, "unordered takes c");
        assert_eq!(fsel(f64::INFINITY, 1.0, 2.0), 1.0);
    }

    #[test]
    fn fsqrts_rounds_to_single_and_frsp_is_the_narrowing_alone() {
        // fsqrts is a double sqrt narrowed, not an f32 sqrt: for an input whose exact root needs
        // more than 24 bits the two agree here, and the shape is what is being pinned.
        assert_eq!(sqrt_single(4.0), 2.0);
        assert_eq!(sqrt_single(2.0), (2.0f64.sqrt() as f32) as f64);
        assert_ne!(
            sqrt_single(2.0),
            2.0f64.sqrt(),
            "the narrowing has to be visible"
        );
        assert!(sqrt_single(-1.0).is_nan());

        // frsp keeps the value in a double; it is not `single_to_bits`, which yields the word.
        assert_eq!(frsp(1.0 + 2f64.powi(-30)), 1.0);
        assert_eq!(frsp(0.1), 0.1f32 as f64);
        assert_ne!(frsp(0.1), 0.1, "0.1 as a double is not 0.1 as a single");
    }

    #[test]
    fn fmsubs_is_one_fused_step() {
        // `a*b - c` with the product needing 48 bits and `c` its f32 rounding: fused, the discarded
        // low bits survive; unfused, the multiply throws them away and the subtraction cancels.
        let a = (1.0f32 + f32::EPSILON) as f64;
        let b = (1.0f32 - f32::EPSILON) as f64;
        let c = ((a * b) as f32) as f64; // == 1.0
        assert_ne!(
            fmsub_single(a, b, c),
            0.0,
            "the fusion has to reach the subtraction"
        );
        assert_eq!(
            sub_single(mul_single(a, b), c),
            0.0,
            "the unfused form, which cancels"
        );
        // Sign: fmsub is a*b - c, and fnmsub is its negation.
        assert_eq!(fmsub_single(2.0, 3.0, 10.0), -4.0);
        assert_eq!(nmsub_single(2.0, 3.0, 10.0), 4.0);
    }

    #[test]
    fn fabs_and_fneg_work_on_the_bits_and_keep_nan_payloads() {
        assert_eq!(abs_double(-2.5), 2.5);
        assert_eq!(abs_double(2.5), 2.5);
        assert_eq!(abs_double(-0.0).to_bits(), 0, "-0 becomes +0, bit for bit");
        assert_eq!(neg_double(0.0).to_bits(), 0x8000_0000_0000_0000);
        // A NaN keeps every payload bit; only the sign moves. `f64::abs` is documented the same
        // way, and this is written through the bits because the lifted line is.
        let nan = f64::from_bits(0xFFF8_0000_DEAD_BEEF);
        assert_eq!(abs_double(nan).to_bits(), 0x7FF8_0000_DEAD_BEEF);
        assert_eq!(neg_double(nan).to_bits(), 0x7FF8_0000_DEAD_BEEF);
    }

    #[test]
    fn rlwinm_doubles_the_low_word_before_rotating() {
        // The divide-by-100 shape sub_82B1C6C8 uses: rotate 27, mask 27 bits, which for an
        // operand inside the low word is a logical right shift by 5.
        const MASK: u64 = 0x7FF_FFFF;
        assert_eq!(rlwinm(0x20, 27, MASK), 0x20 >> 5);
        assert_eq!(rlwinm(0xFFFF_FFFF, 27, MASK), 0xFFFF_FFFFu64 >> 5);

        // The doubling is load-bearing, not decoration: rotating the un-doubled 64-bit value by
        // 27 moves the low word's bits clear of the mask and yields zero.
        assert_eq!(0x20u64.rotate_left(27) & MASK, 0);

        // Only the low word participates, so bits above 31 are discarded rather than rotated in.
        assert_eq!(rlwinm(0xDEAD_0000_0000_0020, 27, MASK), 0x20 >> 5);
    }
}
