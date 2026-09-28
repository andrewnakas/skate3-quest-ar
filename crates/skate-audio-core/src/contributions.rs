//! `sub_82B225A0` — republish one contribution into its owner's running total.
//!
//! Ported from `recomp/src/audio_ports/sub_82B225A0.inc`, **STATUS: verified** — 116,945 calls in a
//! played session on `RwAudioCore Dac`, compared against the original under the shadow harness.
//!
//! Replayed against **3,000 recorded calls, 0 disagreements**, 2,021 of them on the correction path
//! through [`crate::mathlib::log10`] — which makes them 2,021 more bit-exact checks of that
//! transcription too — and compared live against the original 77,379 times in the same session.
//!
//! The object is unnamed: `docs/rw_audio_structs.h` has no entry for it, so the offsets below keep
//! numbers and descriptive names. What the function does is bookkeeping on three words, and it is the
//! *difference* that makes it worth reading carefully — the owner's total at `+40` is updated by the
//! **change since the last publish**, not by the new value, so calling this twice is not the same as
//! calling it once with a doubled value.
//!
//! | mode at `+64` | what happens |
//! |---|---|
//! | exactly 1 | the length at `+140`, converted to a float, becomes the value; if the coefficient at `+92` is not the image's zero it is corrected through [`crate::mathlib::log10`]; the *uncorrected* value is published at `+28` and the corrected one at `+32` |
//! | anything else | withdraw: the owner's total loses whatever was last published, and both `+28` and `+32` settle at the image's zero |
//!
//! Four details that a plausible rewrite gets wrong, each with a test:
//!
//! - **`+28` and `+32` disagree on the correction path.** `+28` gets the raw converted length; `+32`
//!   gets the value after the correction is subtracted. A port that stored one value twice passes
//!   every test where the coefficient is zero, which is most of them.
//! - **The "is it zero" test compares against a word in the image**, not against a literal, and it is
//!   an *unordered* compare — so a NaN coefficient is "not equal" and takes the correction path.
//! - **The conversion is `fcfid` then `frsp`**: a signed 64-bit convert of the sign-extended `i32`,
//!   then rounded to single. Lengths past 2^24 lose their low bits.
//! - **The correction is two single-rounded operations**, `fmuls` then `fdivs`, not one expression.
//!
//! The withdrawal arm negates by flipping the sign **bit** (`fneg`) and then adds, which is not the
//! same as subtracting from zero: withdrawing a `+0.0` publish from a `-0.0` total leaves `-0.0`
//! this way, where `total + (0 - published)` would give `+0.0`.
//!
//! The 112-byte frame the original opens is not reproduced: it exists so `sub_82F55068`'s own frame
//! and its callee's scratch land below this one, and [`crate::mathlib::log10`] keeps all of that in
//! registers. Nor are the `r12`/`r31`/`f31` spills, since nothing here writes them.

use crate::{Guest, Result, fp};

/// `lwz r11,12(r31)` — the accumulator this contribution belongs to.
pub const OWNER: u32 = 12;
/// `stfs f31,28(r31)` — the uncorrected value, published on both arms.
pub const VALUE: u32 = 28;
/// `stfs f0,32(r31)` — what was last added into the owner's total.
pub const PUBLISHED: u32 = 32;
/// `lwz r11,64(r3)` — 1 selects the computed arm; anything else withdraws. Signed.
pub const MODE: u32 = 64;
/// `lfs f13,92(r3)` — zero means no correction.
pub const COEFFICIENT: u32 = 92;
/// `lwz r11,140(r3)` — the length, converted to the raw value.
pub const LENGTH: u32 = 140;
/// `lfs f11,40(r11)` — the running total on the owner.
pub const OWNER_TOTAL: u32 = 40;

/// `lis -32234 ; lfs 23056` — the image's zero, the same cell [`crate::mix::ZERO_SINGLE`] names.
pub const ZERO_FLOAT: u32 = (((-32234i32 as u32) & 0xFFFF) << 16).wrapping_add(23056);
/// `lis -32225 ; lfs 6032` — the numerator scale of the correction.
pub const CORRECTION_SCALE: u32 = (((-32225i32 as u32) & 0xFFFF) << 16).wrapping_add(6032);

const _: () = assert!(ZERO_FLOAT == 0x8216_5A10, "lis -32234 ; lfs 23056");
const _: () = assert!(CORRECTION_SCALE == 0x821F_1790, "lis -32225 ; lfs 6032");
const _: () = assert!(
    ZERO_FLOAT == crate::mix::ZERO_SINGLE,
    "the same word, reached twice"
);

/// Republish this contribution (`sub_82B225A0`). `self_object` is the guest's `r3`; the original
/// returns nothing.
pub fn republish(g: &mut Guest, self_object: u32) -> Result<()> {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at the first float touch

    let published;

    // lwz r11,64(r3) ; cmpwi cr6,r11,1 -- signed, against 1 exactly.
    if g.u32(self_object + MODE)? as i32 == 1 {
        let length = g.u32(self_object + LENGTH)? as i32; // lwz r11,140(r3)
        let coefficient = fp::load_single(g, self_object + COEFFICIENT)?; // lfs f13,92(r3)
        // extsw ; std ; lfd ; fcfid f12,f0 ; frsp f31,f12 -- the spill is a bit copy, the converts
        // are not: a length past 2^24 loses its low bits here.
        let value = fp::frsp(fp::fcfid(i64::from(length)));
        let mut settled = value; // fmr f0,f31

        // fcmpu cr6,f13,f0 ; beq -- unordered, so a NaN coefficient takes the correction path.
        if !(coefficient == fp::load_single(g, ZERO_FLOAT)?) {
            let magnitude = fp::abs_double(coefficient); // fabs f1,f13
            let transformed = fp::frsp(crate::mathlib::log10(g, magnitude)?); // bl 82f55068 ; frsp
            let scale = fp::load_single(g, CORRECTION_SCALE)?;
            // fmuls f12,f31,f0 ; fdivs f11,f12,f13 -- two single-rounded operations, not one
            let correction = fp::div_single(fp::mul_single(value, scale), transformed);
            settled = fp::sub_single(value, correction); // fsubs f0,f31,f11
        }

        // loc_82B22614
        fp::store_single(g, self_object + VALUE, value)?; // stfs f31,28(r31)
        // lwz r11,12(r31) -- loaded after that store, which cannot alias +12
        let owner = g.u32(self_object + OWNER)?;
        let previous = fp::load_single(g, self_object + PUBLISHED)?; // lfs f13,32(r31)
        let delta = fp::sub_single(settled, previous); // fsubs f12,f0,f13
        let total = fp::load_single(g, owner + OWNER_TOTAL)?; // lfs f11,40(r11)
        fp::store_single(g, owner + OWNER_TOTAL, fp::add_single(delta, total))?;
        published = settled;
    } else {
        // loc_82B22634 -- withdraw whatever was last published and settle at the image's zero.
        let previous = fp::load_single(g, self_object + PUBLISHED)?; // lfs f0,32(r31)
        let negated = fp::neg_double(previous); // fneg f13,f0 -- a sign-bit flip, so -0 survives
        let owner = g.u32(self_object + OWNER)?; // lwz r10,12(r31)
        published = fp::load_single(g, ZERO_FLOAT)?; // lfs f0,23056(r11)
        fp::store_single(g, self_object + VALUE, published)?; // stfs f0,28(r31)
        let total = fp::load_single(g, owner + OWNER_TOTAL)?; // lfs f12,40(r10)
        fp::store_single(g, owner + OWNER_TOTAL, fp::add_single(negated, total))?;
    }

    // loc_82B22658
    fp::store_single(g, self_object + PUBLISHED, published) // stfs f0,32(r31)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const SELF: u32 = BASE + 0x40;
    const OWNER_AT: u32 = BASE + 0x200;
    /// The measured scale is read live, so tests set their own and say which.
    const SCALE: f32 = 20.0;

    fn guest(mode: i32, length: i32, coefficient: f32, previous: f32, total: f32) -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        crate::mathlib::tests::with_log_pool(&mut g);
        g.put(ZERO_FLOAT, 0.0f32.to_bits().to_be_bytes().to_vec());
        g.put(CORRECTION_SCALE, SCALE.to_bits().to_be_bytes().to_vec());
        g.set_u32(SELF + MODE, mode as u32).unwrap();
        g.set_u32(SELF + LENGTH, length as u32).unwrap();
        g.set_u32(SELF + COEFFICIENT, coefficient.to_bits())
            .unwrap();
        g.set_u32(SELF + VALUE, 0xDEAD_BEEF).unwrap();
        g.set_u32(SELF + PUBLISHED, previous.to_bits()).unwrap();
        g.set_u32(SELF + OWNER, OWNER_AT).unwrap();
        g.set_u32(OWNER_AT + OWNER_TOTAL, total.to_bits()).unwrap();
        g
    }

    fn value(g: &Guest) -> f32 {
        g.f32(SELF + VALUE).unwrap()
    }
    fn published(g: &Guest) -> f32 {
        g.f32(SELF + PUBLISHED).unwrap()
    }
    fn total(g: &Guest) -> f32 {
        g.f32(OWNER_AT + OWNER_TOTAL).unwrap()
    }

    #[test]
    fn mode_one_with_no_coefficient_publishes_the_length_and_adds_the_change() {
        // Previous publish 30, new value 50: the owner's total moves by 20, not by 50.
        let mut g = guest(1, 50, 0.0, 30.0, 100.0);
        republish(&mut g, SELF).unwrap();
        assert_eq!(value(&g), 50.0);
        assert_eq!(published(&g), 50.0);
        assert_eq!(
            total(&g),
            120.0,
            "the total gained the difference, not the value"
        );
    }

    #[test]
    fn republishing_the_same_value_twice_moves_the_total_once() {
        // The property the delta exists for. A port that added the value would double it here.
        let mut g = guest(1, 50, 0.0, 0.0, 0.0);
        republish(&mut g, SELF).unwrap();
        assert_eq!(total(&g), 50.0);
        republish(&mut g, SELF).unwrap();
        assert_eq!(
            total(&g),
            50.0,
            "the second publish is a no-op for the total"
        );
    }

    #[test]
    fn the_correction_path_leaves_different_values_at_the_two_words() {
        // `+28` keeps the raw converted length; `+32` keeps it minus the correction. A port that
        // stored one value twice passes every zero-coefficient test and fails this one.
        let mut g = guest(1, 50, -100.0, 0.0, 0.0);
        republish(&mut g, SELF).unwrap();

        // The correction, computed the way the body does: two single-rounded operations over
        // log10(|coefficient|) rounded to single.
        let mut h = guest(1, 50, -100.0, 0.0, 0.0);
        let transformed = fp::frsp(crate::mathlib::log10(&mut h, 100.0).unwrap());
        let correction = fp::div_single(fp::mul_single(50.0, f64::from(SCALE)), transformed);
        let want = fp::sub_single(50.0, correction) as f32;

        assert_eq!(value(&g), 50.0, "+28 is the raw value");
        assert_eq!(published(&g), want, "+32 is the corrected one");
        assert_ne!(
            value(&g),
            published(&g),
            "and they differ, which is the point"
        );
        assert_eq!(total(&g), want, "the total took the corrected value");
    }

    #[test]
    fn the_coefficient_is_compared_against_a_word_in_the_image() {
        // Not against a literal zero: patch the cell to 7 and a coefficient of 7 now counts as
        // "no correction", while a coefficient of 0 takes the correction path.
        // `set_u32`, not a second `put`, so the patch cannot be shadowed by the existing segment.
        let mut g = guest(1, 50, 7.0, 0.0, 0.0);
        g.set_u32(ZERO_FLOAT, 7.0f32.to_bits()).unwrap();
        republish(&mut g, SELF).unwrap();
        assert_eq!(published(&g), 50.0, "equal to the cell, so no correction");

        // The same coefficient against the unpatched cell is corrected, so the cell decided it.
        let mut h = guest(1, 50, 7.0, 0.0, 0.0);
        republish(&mut h, SELF).unwrap();
        assert_ne!(
            published(&h),
            50.0,
            "7 is not the image's zero, so it is corrected"
        );

        // And a zero coefficient against a patched cell does take the correction path, but
        // log10(0) is -inf, so the correction is 1000 / -inf = -0 and the value is unchanged. This
        // test first asserted the opposite; the arithmetic, not the port, was what it got wrong.
        let mut k = guest(1, 50, 0.0, 0.0, 0.0);
        k.set_u32(ZERO_FLOAT, 7.0f32.to_bits()).unwrap();
        republish(&mut k, SELF).unwrap();
        assert_eq!(published(&k), 50.0, "corrected by -0");
    }

    #[test]
    fn a_nan_coefficient_takes_the_correction_path() {
        // `fcmpu` is unordered: NaN is not equal to the cell, so the correction runs — and
        // log10(NaN) is a NaN, so the published value is one too.
        let mut g = guest(1, 50, f32::NAN, 0.0, 0.0);
        republish(&mut g, SELF).unwrap();
        assert_eq!(value(&g), 50.0, "+28 is unaffected");
        assert!(published(&g).is_nan(), "+32 came through the correction");
    }

    #[test]
    fn any_other_mode_withdraws_what_was_last_published() {
        for mode in [0i32, 2, -1, 1000] {
            let mut g = guest(mode, 50, 0.0, 30.0, 100.0);
            republish(&mut g, SELF).unwrap();
            assert_eq!(
                value(&g),
                0.0,
                "mode {mode}: +28 settles at the image's zero"
            );
            assert_eq!(published(&g), 0.0, "mode {mode}: and so does +32");
            assert_eq!(
                total(&g),
                70.0,
                "mode {mode}: the total lost the last publish"
            );
        }
    }

    #[test]
    fn the_withdrawal_negates_by_the_sign_bit_then_adds() {
        // The case that separates `fneg` + add from subtract-from-zero + add: a +0 publish
        // withdrawn from a -0 total. fneg(+0) is -0, and (-0) + (-0) is -0. The other form,
        // `total + (0 - published)`, is (-0) + (+0) = +0 under round-to-nearest.
        let mut g = guest(0, 0, 0.0, 0.0, -0.0);
        republish(&mut g, SELF).unwrap();
        assert_eq!(
            g.u32(OWNER_AT + OWNER_TOTAL).unwrap(),
            (-0.0f32).to_bits(),
            "still -0"
        );
    }

    #[test]
    fn the_length_conversion_rounds_to_single_and_sign_extends() {
        // `fcfid` converts the sign-extended i32 exactly; `frsp` then rounds to single, so 2^24 + 1
        // loses its low bit. A port that used `as f32` on the u32 would disagree on both counts.
        let mut g = guest(1, 16_777_217, 0.0, 0.0, 0.0);
        republish(&mut g, SELF).unwrap();
        assert_eq!(value(&g), 16_777_216.0, "the low bit is rounded away");

        let mut h = guest(1, -5, 0.0, 0.0, 0.0);
        republish(&mut h, SELF).unwrap();
        assert_eq!(value(&h), -5.0, "and the length is signed");
    }

    #[test]
    fn the_addresses_come_from_the_lis_immediates() {
        assert_eq!(ZERO_FLOAT, 0x8216_0000 + 23056);
        assert_eq!(CORRECTION_SCALE, 0x821F_0000 + 6032);
        assert_eq!(
            ZERO_FLOAT,
            crate::mix::ZERO_SINGLE,
            "the same cell the mixer reads"
        );
    }
}
