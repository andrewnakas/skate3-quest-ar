//! The evaluator's numeric slots: thirteen ops that read an operand block and write nothing.
//!
//! Every function here is a transcription of a `STATUS: verified` body in
//! `recomp/src/audio_ports/`. None of them stores anything, so the whole of each op's observable
//! effect is the value it returns, which is why the returns are `u64` and not `i32`: the
//! interpreter keeps only the low word (`stwx r3,dst,block`), but the shadow harness compares all
//! 64 bits of `r3`, and several of these ops legitimately leave bits above 31 set.
//!
//! **The 64-bit rule, which cost a real divergence.** RexGlue's `add` and `mullw` operate on
//! 64-bit registers holding zero-extended 32-bit loads. A sum of two `u32` fields carries into
//! bit 32 instead of wrapping; a `subf` that borrows fills the upper word with ones; `mullw`
//! keeps the full 64-bit product of two sign-extended words. Truncating any of those chains to 32
//! bits leaves memory byte-identical and the return value wrong, which is exactly the failure the
//! C++ hit in `counter::advance`. Intermediates stay 64-bit here and narrow only at a store or a
//! comparison that the original performs on low words.
//!
//! **The `tw` traps are not reproduced.** Four of these ops carry `twllei`/`twlgei` guards around
//! their divides. In RexGlue a type-0 `ppc_trap` logs a warning and returns: it changes no
//! register and no memory, so omitting it is exact rather than approximate. Each is noted where it
//! occurred, because it marks the input that reaches the divider's overflow case.

use crate::eval::{HALF_SINGLE, ZERO_SINGLE};
use crate::fp;
use crate::{Guest, Result};

/// Slot 36 — `sub_82B1D198`. `block[0] + block[4]`, as a 64-bit sum of zero-extended words.
///
/// Writes: none. Verified in C++; 698,485 calls per boot.
pub fn op_add(g: &mut Guest, block: u32) -> Result<u64> {
    // The original loads +4 first. Order is immaterial with no stores, and is kept anyway.
    let second = g.u32(block + 4)? as u64;
    let first = g.u32(block)? as u64;
    Ok(second.wrapping_add(first)) // add r3,r11,r10
}

/// Slot 23 — `sub_82B1D1A8`. `block[0] - block[4]`, as a 64-bit subtract.
///
/// When the second field exceeds the first, the borrow runs into the upper word
/// (`0xFFFFFFFF_xxxxxxxx`) rather than wrapping at bit 32. Writes: none.
pub fn op_sub(g: &mut Guest, block: u32) -> Result<u64> {
    let first = g.u32(block)? as u64;
    let second = g.u32(block + 4)? as u64;
    Ok(first.wrapping_sub(second)) // subf r3,r10,r11
}

/// Slot 24 — `sub_82B1D1B8`. `block[4] * block[0]` as signed words, full 64-bit product.
///
/// `mullw` multiplies the low 32 bits of each operand as signed and RexGlue keeps the whole
/// product, so this is a widening multiply and not a wrapping 32-bit one. Writes: none.
pub fn op_mul(g: &mut Guest, block: u32) -> Result<u64> {
    let second = g.u32(block + 4)? as i32;
    let first = g.u32(block)? as i32;
    Ok(((second as i64) * (first as i64)) as u64) // mullw r3,r11,r10
}

/// Slot 25 — `sub_82B1D1C8`. Signed `block[0] / block[4]`, and 0 when the divisor is 0.
///
/// RexGlue defines the `INT32_MIN / -1` overflow as **0**, where real `divw` leaves
/// `0x80000000`; the harness compares against RexGlue, so that is what is reproduced. The
/// quotient is zero-extended into `r3`, so the high word is always 0. Writes: none.
///
/// The `twllei` after the branch is unreachable (the branch already took the zero case) and the
/// `twlgei` fires only for `INT32_MIN / -1`; neither changes state.
pub fn op_div(g: &mut Guest, block: u32) -> Result<u64> {
    let divisor = g.u32(block + 4)?;
    if divisor as i32 == 0 {
        return Ok(0); // li r3,0
    }
    let dividend = g.u32(block)? as i32;
    let divisor = divisor as i32;
    let quotient = if dividend == i32::MIN && divisor == -1 {
        0
    } else {
        dividend / divisor
    };
    Ok(quotient as u32 as u64)
}

/// Slot 26 — `sub_82B1D200`. Signed `block[0] % block[4]`, and 0 when the divisor is 0.
///
/// The remainder is formed as `dividend - quotient * divisor` in **64 bits**: `mullw` keeps the
/// full product of the sign-extended operands and the `subf` takes it from the *zero-extended*
/// dividend, so a negative remainder leaves bit 32 set. Dividend `0xFFFFFFFA` over 3 returns
/// `0x1_00000000`, not `0xFFFFFFFE`. A 32-bit chain would diverge on every negative dividend.
/// Writes: none.
pub fn op_rem(g: &mut Guest, block: u32) -> Result<u64> {
    let divisor_bits = g.u32(block + 4)?;
    let divisor = divisor_bits as i32;
    if divisor == 0 {
        return Ok(0);
    }
    let dividend_bits = g.u32(block)?;
    let dividend = dividend_bits as i32;
    let quotient = if dividend == i32::MIN && divisor == -1 {
        0
    } else {
        dividend / divisor
    };
    let product = ((quotient as i64) * (divisor as i64)) as u64;
    Ok((dividend_bits as u64).wrapping_sub(product))
}

/// Slot 33 — `sub_82B1CEE0`. The smaller of the two signed words at `+0` and `+4`.
///
/// The comparison is signed on the low words; the winner is returned still **zero**-extended,
/// not sign-extended, because both loads were `lwz`. Ties return `+4`. Writes: none.
pub fn op_min(g: &mut Guest, block: u32) -> Result<u64> {
    let first = g.u32(block)? as u64;
    let second = g.u32(block + 4)? as u64;
    if (first as u32 as i32) < (second as u32 as i32) {
        return Ok(first); // mr r3,r11
    }
    Ok(second)
}

/// Slot 34 — `sub_82B1CF38`. The larger of the two signed words at `+0` and `+4`; ties return
/// `+4`. Zero-extended, like [`op_min`]. Writes: none.
pub fn op_max(g: &mut Guest, block: u32) -> Result<u64> {
    let first = g.u32(block)? as u64;
    let second = g.u32(block + 4)? as u64;
    if (first as u32 as i32) <= (second as u32 as i32) {
        return Ok(second); // blelr cr6
    }
    Ok(first)
}

/// Slot 31 — `sub_82B1D790`. `max(block[4] - block[8], block[0])`.
///
/// The subtract is 64-bit, so an underflow leaves `0xFFFFFFFF` in the high word, and on the
/// taken branch that full value is returned. The comparison against the floor looks at the low
/// words only, signed. Writes: none.
pub fn op_sub_floor(g: &mut Guest, block: u32) -> Result<u64> {
    let minuend = g.u32(block + 4)? as u64;
    let subtrahend = g.u32(block + 8)? as u64;
    let floor_word = g.u32(block)? as u64;
    let difference = minuend.wrapping_sub(subtrahend);
    if (difference as u32 as i32) >= (floor_word as u32 as i32) {
        return Ok(difference); // bgelr cr6 — the full 64-bit subf result
    }
    Ok(floor_word)
}

/// Slot 32 — `sub_82B1D7B0`. `block[8] * block[4]`, capped at `block[0]`.
///
/// The product is the full 64-bit `mullw` result and is returned whole when it is not capped;
/// only its **low word** is compared against the cap, so a product that overflows 32 bits can
/// come back uncapped. Writes: none.
pub fn op_mul_cap(g: &mut Guest, block: u32) -> Result<u64> {
    let third = g.u32(block + 8)? as i32;
    let second = g.u32(block + 4)? as i32;
    let cap = g.u32(block)? as u64;
    let product = (third as i64) * (second as i64);
    if (product as i32) > (cap as u32 as i32) {
        return Ok(cap); // mr r3,r11 — zero-extended, as loaded
    }
    Ok(product as u64)
}

/// Slot 30 — `sub_82B1D700`. The sum of `block[0]`'s count of words from `+8`, capped at
/// `block[4]`.
///
/// Layout: `u8 count` at `+0`, `u32 cap` at `+4`, `u32 words[count]` at `+8`.
///
/// **Bug for bug.** A count of 0 or 1 yields `words[0]` alone, and count 0 *still reads*
/// `words[0]`. The original unrolls the middle in pairs into two separate accumulators and folds
/// them in last; wrapping 64-bit addition is associative, so the grouping is reproduced without
/// changing the value, and the odd-count tail word is picked up by the same `lwzx` index the
/// original computes. Writes: none.
pub fn op_sum_capped(g: &mut Guest, block: u32) -> Result<u64> {
    let count = g.u8(block)? as u32;
    let mut total = g.u32(block + 8)? as u64; // words[0], read unconditionally
    if count as i32 > 1 {
        let mut odd = 0u64;
        let mut even = 0u64;
        let mut next = 1u32;
        if (count.wrapping_sub(1)) as i32 >= 2 {
            let pairs = (count.wrapping_sub(3) >> 1) + 1;
            next = (pairs << 1) + 1;
            let mut cursor = block + 8;
            for _ in 0..pairs {
                let a = g.u32(cursor + 4)? as u64;
                cursor = cursor.wrapping_add(8); // lwzu
                let b = g.u32(cursor)? as u64;
                odd = odd.wrapping_add(a);
                even = even.wrapping_add(b);
            }
        }
        if (next as i32) < count as i32 {
            // words[next] at (next + 2) * 4 from the block
            total = total.wrapping_add(g.u32(((next + 2) << 2).wrapping_add(block))? as u64);
        }
        total = total.wrapping_add(even.wrapping_add(odd));
    }
    let cap = g.u32(block + 4)? as u64;
    if (total as u32 as i32) <= (cap as u32 as i32) {
        return Ok(total); // the full 64-bit sum
    }
    Ok(cap)
}

/// Slot 22 — `sub_82B1D118`. The sum of `block[0]`'s count of words from `+4`, uncapped.
///
/// The same shape as [`op_sum_capped`] one field earlier: `u8 count` at `+0`, `u32 elements[]` at
/// `+4`, element 0 read even when the count is 0. Writes: none.
pub fn op_sum(g: &mut Guest, block: u32) -> Result<u64> {
    let count = g.u8(block)? as u32;
    let mut sum = g.u32(block + 4)? as u64;
    if !(count as i32 > 1) {
        return Ok(sum);
    }
    let mut odd = 0u64;
    let mut even = 0u64;
    let mut next = 1u32;
    if !((count.wrapping_sub(1) as i32) < 2) {
        let pairs = (count.wrapping_sub(3) >> 1) + 1;
        next = (pairs << 1) + 1;
        let mut cursor = block + 4;
        for _ in 0..pairs {
            odd = odd.wrapping_add(g.u32(cursor + 4)? as u64);
            cursor = cursor.wrapping_add(8); // lwzu
            even = even.wrapping_add(g.u32(cursor)? as u64);
        }
    }
    if (next as i32) < count as i32 {
        // elements[next] at (next + 1) * 4 from the block
        sum = sum.wrapping_add(g.u32(((next + 1) << 2).wrapping_add(block))? as u64);
    }
    Ok(sum.wrapping_add(even.wrapping_add(odd)))
}

/// Slot 35 — `sub_82B1D098`. `round(f32[+0] * s32[+4] * s32[+8])`, half away from zero.
///
/// Every narrowing the original performs is kept: each integer field becomes a single before it
/// is multiplied, the two products are single-rounded, and the sign test that chooses `+0.5` or
/// `-0.5` is `fcmpu` against the guest's own `0.0f` cell — so a NaN, which sets neither `lt` nor
/// `gt`, takes the *add* branch. Writes: none.
///
/// Reads two `.rdata` singles live from guest memory, as the original does: see
/// [`crate::eval::ZERO_SINGLE`] and [`crate::eval::HALF_SINGLE`].
pub fn op_round_scaled(g: &mut Guest, block: u32) -> Result<u64> {
    let second = g.u32(block + 4)? as i32;
    let mut value = fp::load_single(g, block)?; // lfs f0,0(r3)
    let third = g.u32(block + 8)? as i32;

    // extsw ; std ; lfd ; fcfid — the spill is a bit copy; the value converted is sign-extended.
    let second_f64 = (second as i64) as f64;
    let third_f64 = (third as i64) as f64;
    let zero = fp::load_single(g, ZERO_SINGLE)?;
    let second_f32 = (second_f64 as f32) as f64; // frsp
    let third_f32 = (third_f64 as f32) as f64; // frsp
    let product = fp::mul_single(third_f32, second_f32);
    value = fp::mul_single(product, value);
    let below_zero = value < zero; // fcmpu; unordered clears lt
    let offset = fp::load_single(g, HALF_SINGLE)?;
    value = if below_zero {
        fp::sub_single(value, offset)
    } else {
        fp::add_single(value, offset)
    };
    Ok(fp::fctiwz_low_word(value) as u64)
}

/// Slot 21 — `sub_82B1CF50`. `round(f32 scale[+4] * product of the s32 words from +8)`, half away
/// from zero.
///
/// Layout: `u8 count` at `+0`, `f32 scale` at `+4`, `s32 words[]` at `+8`; word 0 is read even
/// when the count is 0.
///
/// **The multiply order is load-bearing.** Single-rounded multiplication is not associative, so
/// the original's grouping — four words per unrolled iteration, applied in the order
/// `acc*w[k], *w[k+1], *w[k+2], *w[k+3]` with the quad's own words converted before any of them
/// is used — is reproduced step for step. Rearranging it into a tidy fold would change the result
/// on ordinary inputs, not just at the edges. Writes: none.
///
/// **What the tests cannot see, measured rather than assumed.** The *trip count* of the unrolled
/// loop is not observable. Reducing `quads` by one leaves the remainder to the trailing loop,
/// which visits the same words in the same sequence and reads the same addresses, so the result is
/// identical — a deliberately broken `quads` formula passed the whole suite. The only residual
/// difference is which operand of a commutative multiply comes first, and `docs/vmx128-exactness.md`
/// rule 4 records that float ops are **not** NaN-commutative while the winning operand is chosen by
/// register allocation and cannot be derived from source at all. So the unrolling is reproduced
/// because it is what the original does, not because a test could catch its absence.
pub fn op_round_product(g: &mut Guest, block: u32) -> Result<u64> {
    let word0 = g.u32(block + 8)?;
    let mut next = 1u32;
    let count = g.u8(block)? as i32;
    let mut acc = fp::word_to_single(word0);

    if count > 1 {
        if !(count - 1 < 4) {
            // quads = ((count - 5) >> 2) + 1, the loop trip count the original computes
            let quads = ((count as u32).wrapping_sub(5) >> 2) + 1;
            next = (quads << 2) + 1;
            let mut cursor = block.wrapping_sub(4); // addi r11,r3,-4
            for _ in 0..quads {
                let w3 = g.u32(cursor.wrapping_add(28))?;
                let w2 = g.u32(cursor.wrapping_add(24))?;
                let w1 = g.u32(cursor.wrapping_add(20))?;
                cursor = cursor.wrapping_add(16); // lwzu
                let w0 = g.u32(cursor)?;
                // All four conversions happen before the multiplies, as lifted.
                let f2 = fp::word_to_single(w3);
                let f3 = fp::word_to_single(w2);
                let f7 = fp::word_to_single(w1);
                let f4 = fp::word_to_single(w0);
                acc = fp::mul_single(f4, acc);
                acc = fp::mul_single(acc, f7);
                acc = fp::mul_single(acc, f3);
                acc = fp::mul_single(acc, f2);
            }
        }
        if (next as i32) < count {
            let mut cursor = ((next + 1) << 2).wrapping_add(block);
            let remaining = (count as u32).wrapping_sub(next);
            for _ in 0..remaining {
                cursor = cursor.wrapping_add(4); // lwzu
                let w = g.u32(cursor)?;
                acc = fp::mul_single(fp::word_to_single(w), acc);
            }
        }
    }

    let scale = fp::load_single(g, block + 4)?;
    acc = fp::mul_single(scale, acc);
    let zero = fp::load_single(g, ZERO_SINGLE)?;
    let below_zero = acc < zero;
    let half = fp::load_single(g, HALF_SINGLE)?;
    let rounded = if below_zero {
        fp::sub_single(acc, half)
    } else {
        fp::add_single(acc, half)
    };
    Ok(fp::fctiwz_low_word(rounded) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::testutil::*;

    #[test]
    fn the_four_two_word_ops_keep_their_carries_and_borrows() {
        let mut g = block_guest();
        put_words(&mut g, &[7, 5]);
        assert_eq!(op_add(&mut g, BLOCK).unwrap(), 12);
        assert_eq!(op_sub(&mut g, BLOCK).unwrap(), 2);
        assert_eq!(op_mul(&mut g, BLOCK).unwrap(), 35);
        assert_eq!(op_div(&mut g, BLOCK).unwrap(), 1);
        assert_eq!(op_rem(&mut g, BLOCK).unwrap(), 2);
        assert_eq!(op_min(&mut g, BLOCK).unwrap(), 5);
        assert_eq!(op_max(&mut g, BLOCK).unwrap(), 7);

        // The sum carries into bit 32 instead of wrapping.
        put_words(&mut g, &[0xFFFF_FFFF, 1]);
        assert_eq!(op_add(&mut g, BLOCK).unwrap(), 0x1_0000_0000);

        // The subtract borrows into the upper word instead of wrapping.
        put_words(&mut g, &[1, 2]);
        assert_eq!(op_sub(&mut g, BLOCK).unwrap(), 0xFFFF_FFFF_FFFF_FFFF);

        // mullw is a widening signed multiply, and its operands are signed words.
        put_words(&mut g, &[0x0001_0000, 0x0001_0000]);
        assert_eq!(op_mul(&mut g, BLOCK).unwrap(), 0x1_0000_0000);
        put_words(&mut g, &[3, (-4i32) as u32]);
        assert_eq!(op_mul(&mut g, BLOCK).unwrap(), (-12i64) as u64);
    }

    #[test]
    fn min_and_max_compare_signed_but_return_zero_extended() {
        let mut g = block_guest();
        // -1 against 1: the signed compare picks -1, returned as 0x00000000FFFFFFFF.
        put_words(&mut g, &[0xFFFF_FFFF, 1]);
        assert_eq!(op_min(&mut g, BLOCK).unwrap(), 0xFFFF_FFFF);
        assert_eq!(op_max(&mut g, BLOCK).unwrap(), 1);
        // Ties go to +4 in both, which is only observable through which word is returned.
        put_words(&mut g, &[4, 4]);
        assert_eq!(op_min(&mut g, BLOCK).unwrap(), 4);
        assert_eq!(op_max(&mut g, BLOCK).unwrap(), 4);
    }

    #[test]
    fn the_dividers_define_their_own_zero_and_overflow_cases() {
        let mut g = block_guest();
        // A zero divisor returns 0 rather than trapping, on both.
        put_words(&mut g, &[100, 0]);
        assert_eq!(op_div(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(op_rem(&mut g, BLOCK).unwrap(), 0);

        // RexGlue's divw overflow is 0, where the hardware would leave 0x80000000.
        put_words(&mut g, &[0x8000_0000, 0xFFFF_FFFF]);
        assert_eq!(op_div(&mut g, BLOCK).unwrap(), 0);
        // The remainder that follows from that quotient: 0x80000000 - 0*(-1).
        assert_eq!(op_rem(&mut g, BLOCK).unwrap(), 0x8000_0000);

        // Truncation toward zero, and a negative remainder carrying into bit 32 — the case a
        // 32-bit chain would get wrong.
        put_words(&mut g, &[(-6i32) as u32, 3]);
        assert_eq!(op_div(&mut g, BLOCK).unwrap(), (-2i32) as u32 as u64);
        assert_eq!(op_rem(&mut g, BLOCK).unwrap(), 0x1_0000_0000);
        // -7 / 3 truncates to -2, so the remainder is -1; the 64-bit subtract leaves it as
        // 0x00000000FFFFFFFF rather than sign-extending.
        put_words(&mut g, &[(-7i32) as u32, 3]);
        assert_eq!(op_rem(&mut g, BLOCK).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn sub_floor_and_mul_cap_clamp_on_low_words_only() {
        let mut g = block_guest();
        // max(block[4] - block[8], block[0]): 10 - 3 = 7, floor 5, so 7.
        put_words(&mut g, &[5, 10, 3]);
        assert_eq!(op_sub_floor(&mut g, BLOCK).unwrap(), 7);
        // 3 - 10 underflows: the low word is -7, below the floor, so the floor wins.
        put_words(&mut g, &[5, 3, 10]);
        assert_eq!(op_sub_floor(&mut g, BLOCK).unwrap(), 5);
        // With a floor below the underflowed difference, the FULL 64-bit borrow is returned.
        put_words(&mut g, &[(-100i32) as u32, 3, 10]);
        assert_eq!(op_sub_floor(&mut g, BLOCK).unwrap(), 0xFFFF_FFFF_FFFF_FFF9);

        // block[8] * block[4] capped at block[0].
        put_words(&mut g, &[100, 6, 7]);
        assert_eq!(op_mul_cap(&mut g, BLOCK).unwrap(), 42);
        put_words(&mut g, &[10, 6, 7]);
        assert_eq!(op_mul_cap(&mut g, BLOCK).unwrap(), 10);
        // A product past 32 bits is compared by its LOW word: 0x10000 * 0x10000 has low word 0,
        // which is under the cap, so the whole 64-bit product comes back uncapped.
        put_words(&mut g, &[10, 0x0001_0000, 0x0001_0000]);
        assert_eq!(op_mul_cap(&mut g, BLOCK).unwrap(), 0x1_0000_0000);
    }

    #[test]
    fn the_two_summers_walk_their_unrolled_loops_for_every_count() {
        let mut g = block_guest();
        let elements: Vec<u32> = (0..8).map(|k| 1u32 << k).collect();
        let expect = |count: u8| -> u64 {
            if count <= 1 {
                elements[0] as u64
            } else {
                elements[..count as usize].iter().map(|&w| w as u64).sum()
            }
        };

        // op_sum_capped: u8 count at +0, u32 cap at +4, words from +8.
        for count in 0u8..9 {
            let mut words = vec![count_word(count), 0x7FFF_FFFF];
            words.extend_from_slice(&elements);
            put_words(&mut g, &words);
            assert_eq!(
                op_sum_capped(&mut g, BLOCK).unwrap(),
                expect(count),
                "capped at {count}"
            );
        }
        // The cap bites, and is returned zero-extended.
        put_words(&mut g, &[count_word(4), 5, 1, 2, 3, 4]);
        assert_eq!(op_sum_capped(&mut g, BLOCK).unwrap(), 5);

        // op_sum: u8 count at +0, elements from +4, no cap.
        for count in 0u8..9 {
            let mut words = vec![count_word(count)];
            words.extend_from_slice(&elements);
            put_words(&mut g, &words);
            assert_eq!(
                op_sum(&mut g, BLOCK).unwrap(),
                expect(count),
                "sum at {count}"
            );
        }
        // Both accumulate in 64 bits: four maximal words carry past bit 32.
        put_words(
            &mut g,
            &[
                count_word(4),
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
                0xFFFF_FFFF,
            ],
        );
        assert_eq!(op_sum(&mut g, BLOCK).unwrap(), 4 * 0xFFFF_FFFFu64);
    }

    #[test]
    fn the_summers_read_word_zero_even_at_count_zero() {
        // Bug for bug: a count of 0 does not mean an empty sum, it means one word.
        let mut g = block_guest();
        put_words(&mut g, &[count_word(0), 0x7FFF_FFFF, 0xABC]);
        assert_eq!(op_sum_capped(&mut g, BLOCK).unwrap(), 0xABC);
        put_words(&mut g, &[count_word(0), 0xDEF]);
        assert_eq!(op_sum(&mut g, BLOCK).unwrap(), 0xDEF);
    }

    #[test]
    fn round_scaled_narrows_at_each_step_and_rounds_away_from_zero() {
        let mut g = block_guest();
        put_rodata(&mut g);

        // 2.5 * 3 * 4 = 30
        put_words(&mut g, &[2.5f32.to_bits(), 3, 4]);
        assert_eq!(op_round_scaled(&mut g, BLOCK).unwrap(), 30);
        // 0.1 * 1 * 1 rounds to 0; 0.6 rounds to 1; -0.6 rounds to -1 (away from zero).
        put_words(&mut g, &[0.1f32.to_bits(), 1, 1]);
        assert_eq!(op_round_scaled(&mut g, BLOCK).unwrap(), 0);
        put_words(&mut g, &[0.6f32.to_bits(), 1, 1]);
        assert_eq!(op_round_scaled(&mut g, BLOCK).unwrap(), 1);
        put_words(&mut g, &[(-0.6f32).to_bits(), 1, 1]);
        assert_eq!(
            op_round_scaled(&mut g, BLOCK).unwrap(),
            (-1i32) as u32 as u64
        );

        // A NaN sets neither lt nor gt, so the ADD branch runs and fctiwz yields the indefinite.
        put_words(&mut g, &[f32::NAN.to_bits(), 1, 1]);
        assert_eq!(op_round_scaled(&mut g, BLOCK).unwrap(), 0x8000_0000);

        // The integer fields go through frsp: 16_777_217 is not an f32, so it rounds to even
        // before it multiplies.
        put_words(&mut g, &[1.0f32.to_bits(), 16_777_217, 1]);
        assert_eq!(op_round_scaled(&mut g, BLOCK).unwrap(), 16_777_216);
    }

    /// A reference for `op_round_product` that mirrors the original's grouping — four words per
    /// unrolled iteration, applied in the order the lifted registers give — so the loop structure
    /// is what is under test rather than the arithmetic. `order` lets a caller ask for the wrong
    /// order deliberately, which is how the order test proves it can see one.
    fn round_product_reference(
        scale_bits: u32,
        words: &[u32],
        count: usize,
        order: [usize; 4],
    ) -> u64 {
        let mut acc = fp::word_to_single(words[0]);
        if count > 1 {
            let mut next = 1usize;
            if count - 1 >= 4 {
                let quads = ((count - 5) >> 2) + 1;
                next = (quads << 2) + 1;
                for q in 0..quads {
                    let base = 1 + 4 * q;
                    for k in order {
                        acc = fp::mul_single(acc, fp::word_to_single(words[base + k]));
                    }
                }
            }
            for k in next..count {
                acc = fp::mul_single(fp::word_to_single(words[k]), acc);
            }
        }
        let scale = fp::single_from_bits(scale_bits);
        acc = fp::mul_single(scale, acc);
        let rounded = if acc < 0.0 {
            fp::sub_single(acc, 0.5)
        } else {
            fp::add_single(acc, 0.5)
        };
        fp::fctiwz_low_word(rounded) as u64
    }

    /// The order the lifted body applies: `f4` (the quad's first word), then `f7`, `f3`, `f2`.
    const QUAD_ORDER: [usize; 4] = [0, 1, 2, 3];

    #[test]
    fn round_product_multiply_order_is_observable_and_correct() {
        // The multiply order only shows up when an intermediate product has to round, so the
        // inputs are chosen for that: forward and reverse differ by 128 at the end. Without an
        // input like this the order test passes vacuously, which is exactly what happened on the
        // first attempt — a reversed quad over small primes gave the identical answer.
        let mut g = block_guest();
        put_rodata(&mut g);
        let words: Vec<u32> = vec![33257, 14072, 2459, 5634, 28421];
        let scale_bits = 0x2D00_0000; // 2^-37, to bring the product back inside i32
        let mut block = vec![count_word(5), scale_bits];
        block.extend_from_slice(&words);
        put_words(&mut g, &block);

        let forward = round_product_reference(scale_bits, &words, 5, QUAD_ORDER);
        let reverse = round_product_reference(scale_bits, &words, 5, [3, 2, 1, 0]);
        assert_ne!(
            forward, reverse,
            "these inputs must be order-sensitive or the test is vacuous"
        );
        assert_eq!(forward, 1_340_737_536);
        assert_eq!(reverse, 1_340_737_664);

        assert_eq!(op_round_product(&mut g, BLOCK).unwrap(), forward);
    }

    #[test]
    fn round_product_walks_its_unrolled_loop_for_every_count() {
        let mut g = block_guest();
        put_rodata(&mut g);
        let elements: Vec<u32> = vec![3, 5, 7, 11, 13, 2, 17, 19, 23];
        let scale_bits = 1.5f32.to_bits();
        for count in 0u8..=9 {
            let mut words = vec![count_word(count), scale_bits];
            words.extend_from_slice(&elements);
            put_words(&mut g, &words);
            let expect =
                round_product_reference(scale_bits, &elements, count.max(1) as usize, QUAD_ORDER);
            assert_eq!(
                op_round_product(&mut g, BLOCK).unwrap(),
                expect,
                "round_product at count {count}"
            );
        }

        // Something checkable by hand: 1.5 * 3 * 5 = 22.5 -> 23 (half away from zero).
        put_words(&mut g, &[count_word(2), scale_bits, 3, 5]);
        assert_eq!(op_round_product(&mut g, BLOCK).unwrap(), 23);
        // Negative: 1.5 * 3 * -5 = -22.5 -> -23.
        put_words(&mut g, &[count_word(2), scale_bits, 3, (-5i32) as u32]);
        assert_eq!(
            op_round_product(&mut g, BLOCK).unwrap(),
            (-23i32) as u32 as u64
        );
    }

    #[test]
    fn round_product_reads_word_zero_at_count_zero_and_ignores_the_rest() {
        let mut g = block_guest();
        put_rodata(&mut g);
        // count 0: word 0 alone, times the scale. 2.0 * 7 = 14.
        put_words(
            &mut g,
            &[count_word(0), 2.0f32.to_bits(), 7, 0xDEAD, 0xBEEF],
        );
        assert_eq!(op_round_product(&mut g, BLOCK).unwrap(), 14);
    }
}
