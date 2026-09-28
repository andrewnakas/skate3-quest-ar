//! The evaluator's accessor, stack and reducer slots: seven ops whose whole job is to move a
//! word between the operand block and somewhere else.
//!
//! All seven are transcriptions of `STATUS: verified` bodies. Three of them **write**, which is
//! what separates them from `arith`: the take-and-clear trio leave a zero behind, and the stack
//! push and the flag-table select publish into the block. Where a store lands on a word the
//! function later reloads, the order is kept: the C++ notes call that out explicitly for
//! `op_stack_push`, whose first store can land on the very fields that decide its own branch.

use crate::{Guest, Result};

/// Slot 0 — `sub_82B1BF68`. Return the word at `+16` and leave zero in its place.
///
/// Writes: the four bytes at `+16`. Load then store, in that order and to the same word: hoisting
/// the store would return zero. Zero-extended into all 64 bits of `r3`.
pub fn op_take_word_16(g: &mut Guest, object: u32) -> Result<u64> {
    let value = g.u32(object + 16)? as u64; // lwz r3,16(r3)
    g.set_u32(object + 16, 0)?; // stw r10,16(r11)
    Ok(value)
}

/// Slot 3 — `sub_82B1BF80`. The same over the word at `+0`.
///
/// Writes: the four bytes at `+0`. `sub_82B1BF60`, its neighbour, is a bare `blr`: this is a
/// family of small method slots over one object, reached by function pointer.
pub fn op_take_word_0(g: &mut Guest, object: u32) -> Result<u64> {
    let value = g.u32(object)? as u64;
    g.set_u32(object, 0)?;
    Ok(value)
}

/// Slot 1 — `sub_82832BA8`. Return the word at `+20`; no store.
///
/// **Not shadow-verified**, unlike the rest of this table: the function is outside the audio corpus
/// (identical code folding shares it with game code), so no `.inc` exists. It is one instruction,
/// `lwz r3,20(r3); blr`, so the transcription is the whole of it. Zero-extended into `r3`.
pub fn op_word_20(g: &mut Guest, object: u32) -> Result<u64> {
    Ok(g.u32(object + 20)? as u64)
}

/// Slot 2 — `sub_82C8CDC8`. Return the word at `+24`; no store. The same standing as slot 1:
/// `lwz r3,24(r3); blr`, not shadow-verified.
pub fn op_word_24(g: &mut Guest, object: u32) -> Result<u64> {
    Ok(g.u32(object + 24)? as u64)
}

/// Slot 37 — `sub_82B1D7D0`. Return the byte flag at `+25` and clear it.
///
/// Writes: the one byte at `+25`. Zero-extended into `r3`.
pub fn op_take_flag_25(g: &mut Guest, object: u32) -> Result<u64> {
    let value = g.u8(object + 25)? as u64; // lbz r3,25(r3)
    g.set_u8(object + 25, 0)?; // stb r10,25(r11)
    Ok(value)
}

/// Slot 13 — `sub_82B1C8D0`. 1 if any of `count` words from `+4` is non-zero, else 0.
///
/// Layout: `u8 count` at `+0` — the **top byte** of the big-endian word there — and `u32
/// entries[]` at `+4`. The scan stops at the first non-zero word and the count is *not* reloaded
/// inside the loop. Writes: none.
pub fn op_any_nonzero(g: &mut Guest, object: u32) -> Result<u64> {
    let count = g.u8(object)? as u32;
    // A zero-extended byte is never negative, so the original's signed test is just `!= 0`.
    if count != 0 {
        let mut entry = object + 4;
        let mut index: i32 = 0;
        loop {
            if g.u32(entry)? != 0 {
                return Ok(1);
            }
            index += 1;
            entry = entry.wrapping_add(4);
            if !(index < count as i32) {
                break;
            }
        }
    }
    Ok(0)
}

/// Slot 12 — `sub_82B1C878`. Find the first non-zero entry of a flag table and publish the
/// matching value into `+4`; return `+4` either way.
///
/// Layout: `u16 flags_offset` at `+0` (the table lives at `object + flags_offset`), `u8 count` at
/// `+2`, `u32 current` at `+4`, `u32 values[]` at `+8`. Entry *i* of the flag table selects
/// `values[i]`, which the original reaches as `object + 4 * (i + 2)`.
///
/// Writes: `+4`, on the found path only. The count **is** reloaded every iteration here, unlike
/// [`op_any_nonzero`] — a difference between two neighbouring functions that is preserved rather
/// than harmonised, because a value table that overlaps the count byte would expose it.
pub fn op_select_first_flag(g: &mut Guest, object: u32) -> Result<u64> {
    let flags = (g.u16(object)? as u32).wrapping_add(object);
    if g.u8(object + 2)? != 0 {
        let mut index: u32 = 0;
        loop {
            if g.u32(flags.wrapping_add(4 * index))? != 0 {
                let value = g.u32(object.wrapping_add(4 * (index + 2)))?;
                g.set_u32(object + 4, value)?; // stw r9,4(r3)
                break;
            }
            let count = g.u8(object + 2)? as i32; // reloaded, as the original does
            index += 1;
            if !((index as i32) < count) {
                break;
            }
        }
    }
    Ok(g.u32(object + 4)? as u64) // lwz r3,4(r3) — reloaded on every path
}

/// Slot 17 — `sub_82B1CE18`. The top entry of a one-based stack, or 0 when it is empty or
/// overfull.
///
/// Layout: `u8 capacity` at `+0`, `s32 depth` at `+4`, `u32 entries[]` at `+8` addressed
/// one-based, so entry *k* lives at `4 * (k + 1)` and the top is at `4 * (depth + 1)`. Writes:
/// none.
///
/// At 1.83 million calls per boot this is the second most frequently executed function in the
/// whole audio corpus.
pub fn op_stack_top(g: &mut Guest, object: u32) -> Result<u64> {
    let depth = g.u32(object + 4)? as i32;
    if !(depth > 0) {
        return Ok(0);
    }
    let capacity = g.u8(object)? as i32;
    if depth > capacity {
        return Ok(0);
    }
    let index = (depth as u32) + 1;
    Ok(g.u32((index << 2).wrapping_add(object))? as u64) // lwzx r3,r10,r3
}

/// Slot 18 — `sub_82B1CE48`. Clear the slot the previous call recorded; then, if the depth is in
/// range, copy `+8` into the slot for that depth and record it. Return `+12`.
///
/// Layout: `u8 capacity` at `+0`, `s16 recorded_slot` at `+2`, `s32 depth` at `+4`, `u32 value` at
/// `+8`, `u32 result` at `+12`. Slot *k* is at `4 * (k + 2)`, so slots 0 and 1 alias `+8` and
/// `+12` and the *negative* recorded indices -2 and -1 alias `+0` and `+4`.
///
/// **The store order is not cosmetic.** The first store happens before the depth and capacity are
/// read, so a recorded index of -1 zeroes the depth word this call is about to test, and -2 zeroes
/// the capacity byte. Both then take the not-taken branch. The C++ `Windows()` predicate reasons
/// about exactly this, and moving the loads above the store would change the outcome.
///
/// Writes: the cleared slot always; the new slot and `+2` on the taken branch.
pub fn op_stack_push(g: &mut Guest, object: u32) -> Result<u64> {
    /// `addi rX,rY,2 ; rlwinm rZ,rX,2,0,29` — a 32-bit shift, so a negative index wraps the
    /// offset below the object exactly as the original does.
    fn slot_offset(index: i32) -> u32 {
        (index as u32).wrapping_add(2) << 2
    }

    let recorded = (g.u16(object + 2)? as i16) as i32; // lhz ; extsh
    g.set_u32(slot_offset(recorded).wrapping_add(object), 0)?; // stwx r9,r7,r3

    // Loaded after that store, deliberately.
    let depth = g.u32(object + 4)? as i32;
    if depth > 0 {
        let capacity = g.u8(object)? as i32;
        if !(depth > capacity) {
            let value = g.u32(object + 8)?;
            g.set_u32(slot_offset(depth).wrapping_add(object), value)?;
            let depth_again = g.u32(object + 4)?; // reloaded
            g.set_u16(object + 2, depth_again as u16)?; // sth r7,2(r3)
        }
    }

    Ok(g.u32(object + 12)? as u64) // reloaded after the stores, either of which can land here
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::testutil::*;

    #[test]
    fn the_folded_accessors_read_without_clearing() {
        let mut g = block_guest();
        g.set_u32(BLOCK + 20, 0x8000_0014).unwrap();
        g.set_u32(BLOCK + 24, 0x0000_0018).unwrap();
        assert_eq!(
            op_word_20(&mut g, BLOCK).unwrap(),
            0x8000_0014,
            "zero-extended, not sign-extended"
        );
        assert_eq!(op_word_24(&mut g, BLOCK).unwrap(), 0x18);
        assert_eq!(g.u32(BLOCK + 20).unwrap(), 0x8000_0014, "no store");
    }

    #[test]
    fn the_take_and_clear_trio_return_the_old_value() {
        let mut g = block_guest();
        g.set_u32(BLOCK + 16, 0xDEAD_BEEF).unwrap();
        assert_eq!(op_take_word_16(&mut g, BLOCK).unwrap(), 0xDEAD_BEEF);
        assert_eq!(g.u32(BLOCK + 16).unwrap(), 0, "cleared in place");
        assert_eq!(
            op_take_word_16(&mut g, BLOCK).unwrap(),
            0,
            "idempotent after the first"
        );

        g.set_u32(BLOCK, 0x8000_0001).unwrap();
        assert_eq!(
            op_take_word_0(&mut g, BLOCK).unwrap(),
            0x8000_0001,
            "zero-extended, not signed"
        );
        assert_eq!(g.u32(BLOCK).unwrap(), 0);

        g.set_u8(BLOCK + 25, 0xFF).unwrap();
        assert_eq!(op_take_flag_25(&mut g, BLOCK).unwrap(), 0xFF);
        assert_eq!(g.u8(BLOCK + 25).unwrap(), 0);
        // Only that byte: the neighbours are untouched.
        assert_eq!(g.u8(BLOCK + 24).unwrap(), 0);
        assert_eq!(g.u8(BLOCK + 26).unwrap(), 0);
    }

    #[test]
    fn any_nonzero_scans_exactly_count_entries() {
        let mut g = block_guest();
        // The count is the top byte of the word at +0, not the word itself.
        put_words(&mut g, &[count_word(3), 0, 0, 0, 0xFFFF_FFFF]);
        assert_eq!(
            op_any_nonzero(&mut g, BLOCK).unwrap(),
            0,
            "the fourth entry is out of range"
        );

        put_words(&mut g, &[count_word(4), 0, 0, 0, 0xFFFF_FFFF]);
        assert_eq!(op_any_nonzero(&mut g, BLOCK).unwrap(), 1);

        // A count of zero scans nothing, even with a non-zero entry sitting there.
        put_words(&mut g, &[count_word(0), 1]);
        assert_eq!(op_any_nonzero(&mut g, BLOCK).unwrap(), 0);

        // The first entry short-circuits: the rest need not even be mapped.
        put_words(&mut g, &[count_word(255), 7]);
        assert_eq!(op_any_nonzero(&mut g, BLOCK).unwrap(), 1);
    }

    #[test]
    fn select_first_flag_publishes_the_matching_value() {
        let mut g = block_guest();
        // Flag table 0x40 bytes into the object; three entries; values at +8.
        g.set_u16(BLOCK, 0x40).unwrap();
        g.set_u8(BLOCK + 2, 3).unwrap();
        g.set_u32(BLOCK + 4, 0xAAAA_AAAA).unwrap(); // the stale current value
        for (i, v) in [0x1111u32, 0x2222, 0x3333].iter().enumerate() {
            g.set_u32(BLOCK + 8 + 4 * i as u32, *v).unwrap();
        }
        // Flags: entry 0 clear, entry 1 set.
        g.set_u32(BLOCK + 0x40, 0).unwrap();
        g.set_u32(BLOCK + 0x44, 1).unwrap();
        g.set_u32(BLOCK + 0x48, 1).unwrap();

        assert_eq!(
            op_select_first_flag(&mut g, BLOCK).unwrap(),
            0x2222,
            "entry 1 wins"
        );
        assert_eq!(g.u32(BLOCK + 4).unwrap(), 0x2222, "published into +4");

        // No flag set: nothing is published and the stale value comes back.
        g.set_u32(BLOCK + 4, 0xAAAA_AAAA).unwrap();
        g.set_u32(BLOCK + 0x44, 0).unwrap();
        g.set_u32(BLOCK + 0x48, 0).unwrap();
        assert_eq!(op_select_first_flag(&mut g, BLOCK).unwrap(), 0xAAAA_AAAA);
        assert_eq!(g.u32(BLOCK + 4).unwrap(), 0xAAAA_AAAA, "untouched");

        // A zero count skips the scan entirely, even with flags set.
        g.set_u8(BLOCK + 2, 0).unwrap();
        g.set_u32(BLOCK + 0x40, 1).unwrap();
        assert_eq!(op_select_first_flag(&mut g, BLOCK).unwrap(), 0xAAAA_AAAA);
    }

    #[test]
    fn stack_top_reads_one_based_and_rejects_empty_or_overfull() {
        let mut g = block_guest();
        g.set_u8(BLOCK, 4).unwrap(); // capacity
        // Entry k at 4*(k+1): entry 1 at +8, entry 2 at +12, entry 3 at +16.
        for k in 1u32..=4 {
            g.set_u32(BLOCK + 4 * (k + 1), 0x100 + k).unwrap();
        }

        g.set_u32(BLOCK + 4, 0).unwrap();
        assert_eq!(op_stack_top(&mut g, BLOCK).unwrap(), 0, "empty");
        g.set_u32(BLOCK + 4, (-1i32) as u32).unwrap();
        assert_eq!(
            op_stack_top(&mut g, BLOCK).unwrap(),
            0,
            "a negative depth is empty too"
        );

        g.set_u32(BLOCK + 4, 1).unwrap();
        assert_eq!(op_stack_top(&mut g, BLOCK).unwrap(), 0x101);
        g.set_u32(BLOCK + 4, 4).unwrap();
        assert_eq!(
            op_stack_top(&mut g, BLOCK).unwrap(),
            0x104,
            "depth == capacity is in range"
        );
        g.set_u32(BLOCK + 4, 5).unwrap();
        assert_eq!(op_stack_top(&mut g, BLOCK).unwrap(), 0, "past the capacity");
    }

    #[test]
    fn stack_push_clears_the_recorded_slot_before_it_reads_its_own_fields() {
        let mut g = block_guest();
        g.set_u8(BLOCK, 8).unwrap(); // capacity
        g.set_u16(BLOCK + 2, 5).unwrap(); // the slot a previous call recorded
        g.set_u32(BLOCK + 4, 3).unwrap(); // depth
        g.set_u32(BLOCK + 8, 0xC0FF_EE00).unwrap(); // the value to push
        g.set_u32(BLOCK + 12, 0x1234_5678).unwrap(); // the result word
        // Slot k at 4*(k+2): slot 5 at +28, slot 3 at +20.
        g.set_u32(BLOCK + 28, 0xFFFF_FFFF).unwrap();

        assert_eq!(op_stack_push(&mut g, BLOCK).unwrap(), 0x1234_5678);
        assert_eq!(g.u32(BLOCK + 28).unwrap(), 0, "slot 5 was cleared");
        assert_eq!(
            g.u32(BLOCK + 20).unwrap(),
            0xC0FF_EE00,
            "slot 3 took the value"
        );
        assert_eq!(
            g.u16(BLOCK + 2).unwrap(),
            3,
            "the depth was recorded as the new slot"
        );
    }

    #[test]
    fn a_recorded_index_of_minus_one_zeroes_the_depth_it_is_about_to_test() {
        // The aliasing case the C++ Windows() predicate is built around. Slot -1 is at
        // 4*(-1+2) = +4, which is the depth word: clearing it turns a would-be push into a
        // no-op on the very same call.
        let mut g = block_guest();
        g.set_u8(BLOCK, 8).unwrap();
        g.set_u16(BLOCK + 2, (-1i16) as u16).unwrap();
        g.set_u32(BLOCK + 4, 3).unwrap();
        g.set_u32(BLOCK + 8, 0xC0FF_EE00).unwrap();
        g.set_u32(BLOCK + 12, 0x1234_5678).unwrap();

        assert_eq!(op_stack_push(&mut g, BLOCK).unwrap(), 0x1234_5678);
        assert_eq!(
            g.u32(BLOCK + 4).unwrap(),
            0,
            "the depth was the slot that got cleared"
        );
        assert_eq!(g.u32(BLOCK + 20).unwrap(), 0, "so no push happened");
        assert_eq!(
            g.u16(BLOCK + 2).unwrap(),
            (-1i16) as u16,
            "and nothing was recorded"
        );
    }

    #[test]
    fn slot_zero_and_one_alias_the_value_and_result_words() {
        // Slot 0 is at +8 and slot 1 at +12, so a depth of 1 copies the value word onto the
        // result word and the return value changes with it.
        let mut g = block_guest();
        g.set_u8(BLOCK, 8).unwrap();
        g.set_u16(BLOCK + 2, 4).unwrap();
        g.set_u32(BLOCK + 4, 1).unwrap();
        g.set_u32(BLOCK + 8, 0xC0FF_EE00).unwrap();
        g.set_u32(BLOCK + 12, 0x1234_5678).unwrap();

        assert_eq!(
            op_stack_push(&mut g, BLOCK).unwrap(),
            0xC0FF_EE00,
            "the reload sees the push"
        );
        assert_eq!(g.u32(BLOCK + 12).unwrap(), 0xC0FF_EE00);
    }
}
