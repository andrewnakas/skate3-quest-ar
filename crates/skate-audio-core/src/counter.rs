//! `sub_82B1F360`: the six-word cascading counter at guest `0x830775F0`.
//!
//! Ported from `recomp/src/audio_ports/sub_82B1F360.inc`, **STATUS: verified** — 6,624 calls
//! per boot on `RwAudioCore Dac`, compared against the original under the shadow harness.
//!
//! This is the engine's only source of variation: no timebase, no lock, no allocation, no
//! indirect call, just six words that accumulate into each other and a low word that ticks by
//! one per call. Two evaluator ops draw from it — `eval::state::op_random_in_range` (slot 7)
//! and `eval::state::op_weighted_cursor` (slot 9) — which is why it lives in its own module
//! rather than inside `eval`: it is shared state those ops reach through, not an opcode.
//!
//! **Not a PRNG in the usual sense.** Each call sums the words upward with carry and returns
//! the top total, so successive draws are strongly correlated; slot 9 takes it modulo 100 and
//! slot 7 modulo an arbitrary range. Whatever its statistical merits, reproducing it exactly is
//! the requirement, and its exactness has a specific history: see [`advance`].

use crate::{Guest, Result};

/// `lis r11,-31993 ; addi r11,r11,30192` — computed as `((imm & 0xFFFF) << 16) + offset`, the
/// way the lifted body forms it, not read off a dump.
pub const COUNTER: u32 = 0x8307_75F0;
/// Six `u32` words, `+0` through `+20`. Every call writes all six.
pub const COUNTER_BYTES: u32 = 24;

const _: () = assert!(COUNTER == 0x8307_0000 + 30192, "lis -31993 ; addi 30192");

/// Advance the counter and return the draw — the **full 64-bit** value the guest leaves in `r3`.
///
/// **Why 64-bit intermediates.** Every `add` in the original is a 64-bit add on zero-extended
/// words, so a sum carries into bit 32. The stores keep only each low word, but `r3` keeps all
/// 64 bits. Truncating the chain to 32 bits in the C++ made every *store* match and `r3` diverge
/// on the 120th call — registers `94BF290A01000000` against `00000000`. That is the concrete
/// reason this module does not use `u32` arithmetic, and why callers that only need the low word
/// (the two evaluator ops both store `u32`) still receive `u64` here.
///
/// Store order is the original's — 16, 12, 8, 4, 20, then 0 — followed by the ripple, which
/// re-stores each word it increments. The order is observable only to a concurrent reader, and
/// is preserved rather than tidied.
pub fn advance(g: &mut Guest) -> Result<u64> {
    let w20 = g.u32(COUNTER + 20)?;
    let w16 = g.u32(COUNTER + 16)?;
    let w12 = g.u32(COUNTER + 12)?;
    let w8 = g.u32(COUNTER + 8)?;
    let w4 = g.u32(COUNTER + 4)?;
    let w0 = g.u32(COUNTER)?;

    // Each word accumulates the one below it, carrying upward. The carry tests are the
    // `subfc`/`subfe` pairs of the original, which look only at the low words.
    let s16 = (w16 as u64).wrapping_add(w20 as u64);
    let carry16 = if (s16 as u32) < w20 || (s16 as u32) < w16 {
        1u64
    } else {
        0
    };
    let s12 = (w12 as u64).wrapping_add(s16).wrapping_add(carry16);
    let carry12 = if (s12 as u32) < w12 { 1u64 } else { 0 };
    let s8 = (w8 as u64).wrapping_add(s12).wrapping_add(carry12);
    let carry8 = if (s8 as u32) < w8 { 1u64 } else { 0 };
    let s4 = (w4 as u64).wrapping_add(s8).wrapping_add(carry8);
    let carry4 = if (s4 as u32) < w4 { 1u64 } else { 0 };
    let s0 = (w0 as u64).wrapping_add(s4).wrapping_add(carry4);
    let next20 = w20.wrapping_add(1);

    g.set_u32(COUNTER + 16, s16 as u32)?;
    g.set_u32(COUNTER + 12, s12 as u32)?;
    g.set_u32(COUNTER + 8, s8 as u32)?;
    g.set_u32(COUNTER + 4, s4 as u32)?;
    g.set_u32(COUNTER + 20, next20)?;
    g.set_u32(COUNTER, s0 as u32)?;

    // `add r3,r5,r4` — the full 64-bit sum, taken before the first `bnelr`.
    if next20 != 0 {
        return Ok(s0);
    }

    // The low word wrapped: ripple a single increment upward, stopping at the first word whose
    // own low word does not wrap. Each `addic.` tests only the low word.
    let r16 = s16.wrapping_add(1);
    g.set_u32(COUNTER + 16, r16 as u32)?;
    if r16 as u32 != 0 {
        return Ok(s0);
    }
    let r12 = s12.wrapping_add(1);
    g.set_u32(COUNTER + 12, r12 as u32)?;
    if r12 as u32 != 0 {
        return Ok(s0);
    }
    let r8 = s8.wrapping_add(1);
    g.set_u32(COUNTER + 8, r8 as u32)?;
    if r8 as u32 != 0 {
        return Ok(s0);
    }
    let r4 = s4.wrapping_add(1);
    g.set_u32(COUNTER + 4, r4 as u32)?;
    if r4 as u32 != 0 {
        return Ok(s0);
    }
    // `addi r3,r3,1` — 64-bit, like the sum it increments.
    let r0 = s0.wrapping_add(1);
    g.set_u32(COUNTER, r0 as u32)?;
    Ok(r0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guest window holding just the counter, at its real address.
    fn guest() -> Guest {
        Guest::single(COUNTER, COUNTER_BYTES as usize)
    }

    fn words(g: &Guest) -> [u32; 6] {
        let mut out = [0u32; 6];
        for (i, w) in out.iter_mut().enumerate() {
            *w = g.u32(COUNTER + 4 * i as u32).unwrap();
        }
        out
    }

    #[test]
    fn from_zero_the_first_draw_only_ticks_the_low_word() {
        let mut g = guest();
        assert_eq!(advance(&mut g).unwrap(), 0);
        assert_eq!(words(&g), [0, 0, 0, 0, 0, 1]);
        // Second call: +20 is now 1, so it cascades into +16 and upward.
        assert_eq!(advance(&mut g).unwrap(), 1);
        assert_eq!(words(&g), [1, 1, 1, 1, 1, 2]);
        assert_eq!(advance(&mut g).unwrap(), 7);
        assert_eq!(words(&g), [7, 6, 5, 4, 3, 3]);
    }

    #[test]
    fn the_draw_keeps_bits_above_31_that_the_stores_drop() {
        // The regression the C++ hit on its 120th call. With every word at its maximum the sum
        // carries well past bit 32: the stores keep low words, the return value does not.
        let mut g = guest();
        for i in 0..6 {
            g.set_u32(COUNTER + 4 * i, 0xFFFF_FFFF).unwrap();
        }
        let draw = advance(&mut g).unwrap();
        assert!(
            draw > u32::MAX as u64,
            "the draw must not be truncated: {draw:#x}"
        );
        // s16 = 0x1FFFFFFFE and each level adds one more maximal word plus its carry, so the
        // top total is 0x5FFFFFFFE with only 0xFFFFFFFE reaching memory.
        assert_eq!(draw, 0x5_FFFF_FFFE);
        assert_eq!(
            words(&g)[0],
            0xFFFF_FFFE,
            "the store keeps the low word only"
        );
        // +20 wrapped, so the ripple ran: it incremented +16 to 0xFFFFFFFF and stopped there
        // because that low word is non-zero.
        assert_eq!(words(&g)[5], 0, "+20 wrapped");
        assert_eq!(words(&g)[4], 0xFFFF_FFFF, "+16 took the ripple's increment");
    }

    #[test]
    fn a_ripple_stops_at_the_first_word_whose_low_half_survives() {
        let mut g = guest();
        // +20 at 0xFFFFFFFF makes next20 wrap to 0 and the ripple run. With the other words at
        // 1, s16's low word is already 0, so the increment lands on 1 and the walk stops.
        g.set_u32(COUNTER + 20, 0xFFFF_FFFF).unwrap();
        for off in [0u32, 4, 8, 12, 16] {
            g.set_u32(COUNTER + off, 1).unwrap();
        }
        let draw = advance(&mut g).unwrap();
        assert_eq!(draw, 0x1_0000_0005);
        assert_eq!(words(&g), [5, 4, 3, 2, 1, 0]);
    }

    #[test]
    fn a_full_ripple_reaches_the_top_word_and_changes_the_returned_draw() {
        // +20 = 0xFFFFFFFF with every other word 0 makes each sum's low word 0xFFFFFFFF, so
        // every increment wraps, the walk runs to the end, and the final `addi r3,r3,1` is what
        // the caller sees — a value the stored words no longer contain.
        let mut g = guest();
        g.set_u32(COUNTER + 20, 0xFFFF_FFFF).unwrap();
        let draw = advance(&mut g).unwrap();
        assert_eq!(words(&g), [0, 0, 0, 0, 0, 0], "every word wrapped to zero");
        assert_eq!(draw, 0x1_0000_0000, "the 64-bit addi, invisible in memory");
    }
}
