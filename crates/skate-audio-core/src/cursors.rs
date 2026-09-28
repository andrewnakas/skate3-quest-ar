//! The three verified cursor advances: which slot, which entry, which segment comes next.
//!
//! These are the scalar half of the engine's timing layer — the small functions that decide, once
//! per tick or once per block, what the next thing to play is. They sit beside
//! [`crate::scheduler`] rather than inside it because they run on stream objects, not on the
//! `rw_scheduler`, but they answer the same kind of question and none of them is large enough to
//! justify a module of its own.
//!
//! | function | guest | `docs/ports.md` status | lifted lines | calls/boot |
//! |---|---|---|---|---|
//! | [`claim_ring_slot`] | `sub_82B32550` | verified | 50 | 2,935 |
//! | [`advance_ring_cursor`] | `sub_82B349A8` | verified | 91 | 865 |
//! | [`advance_segment_position`] | `sub_82B3C9D8` | verified | 70 | 119,335 |
//!
//! All three were compared call-for-call against the original under the shadow harness at zero
//! divergence, all three are leaves, and none of them makes an indirect call, takes a lock, reads
//! the timebase or allocates. **The Rust has no recorded vectors of its own**: the harness
//! brackets these three but does not record per-call inputs for them, so the tests below are
//! "unit-tested against a verified reference", not "replayed" — see the crate README's "two kinds
//! of green".
//!
//! **`sub_82B32DC8`, the segment-ring *append*, is deliberately absent.** It is the function that
//! fills the slots [`claim_ring_slot`] hands out, and it is `gate-1`: it allocates the request's
//! name through the holder's vtable (`bctrl` at `0x82B33090`) and reaches two more unreplayable
//! callees. There is no verified reference for it in either language.
//!
//! Nothing in `docs/rw_audio_structs.h` names any of the three objects below; every field name is
//! a reading of what the code does with the cell, and the offsets are the ones the verified C++
//! bodies use.
//!
//! **Four things here are reproduced rather than tidied, and no test in this crate catches their
//! absence.** Each was checked by breaking it and watching the whole suite still pass:
//!
//! - [`advance_ring_cursor`] reloads the cursor byte after writing it, instead of keeping the
//!   value it just computed. The two differ only if the entry table overlaps the cursor byte;
//! - [`advance_ring_cursor`] stores zero into `+432` a second time on the latch path. It is the
//!   same value to the same address, so no single-threaded comparison can see it in either
//!   direction — the same argument the crate README makes for the command ring's publish order;
//! - [`advance_segment_position`] reloads `+49` after zeroing the retired segment's end word, and
//!   reloads `+49` and `+36` again before addressing the new segment. All three differ only for a
//!   segment table that overlaps the object's own header.
//!
//! They are kept because the originals have them, and because the aliasing cases they guard are
//! real hazards for a caller that lays memory out itself — not because any evidence here
//! distinguishes them.

use crate::fp;
use crate::{Guest, Result};

/// `sub_82B32550`: the ring index byte, `u8` at `object + 473`.
pub const RING_INDEX: u32 = 473;
/// `rotlwi r8,r11,4` — 16 bytes per slot, measured from the object itself.
pub const SLOT_STRIDE: u32 = 16;
/// The byte inside a slot that says it is still in use.
pub const SLOT_BUSY: u32 = 113;
/// The index wraps here rather than at a power of two.
pub const RING_ENTRIES: u32 = 20;

/// `sub_82B349A8`: `u16` byte offset, from the object, of the entry table.
pub const ENTRY_TABLE: u32 = 464;
/// `u8`, advanced on every call.
pub const ENTRY_CURSOR: u32 = 469;
/// `u8`, the value the advanced cursor wraps at.
pub const ENTRY_LIMIT: u32 = 470;
/// `u8`, cleared on every call.
pub const ENTRY_BUSY: u32 = 471;
/// `f32`, latched from the entry's `+12`.
pub const LATCHED_A: u32 = 424;
/// `f32`, latched from the entry's `+16`.
pub const LATCHED_B: u32 = 428;
/// `u32`, cleared on every call and never written with anything else.
pub const ENTRY_FLAG_WORD: u32 = 432;
/// `u32`, cleared on every call and re-written from the entry's `+20` on the latch path.
pub const LATCHED_WORD: u32 = 436;
/// `(i + i*2) * 16` — 48 bytes per entry.
pub const ENTRY_STRIDE: u32 = 48;
/// The entry's own state byte, at `entry + 46`.
pub const ENTRY_STATE: u32 = 46;

/// `sub_82B3C9D8`: `u32` position, advanced by the call's second argument.
pub const SEG_POSITION: u32 = 28;
/// `u32` byte offset, from the object, of the segment array.
pub const SEG_TABLE: u32 = 36;
/// `u8` current segment.
pub const SEG_INDEX: u32 = 49;
/// `u8` segment count; the index wraps to 0 when it reaches this.
pub const SEG_COUNT: u32 = 50;
/// 24 bytes per segment.
pub const SEGMENT_STRIDE: u32 = 24;
/// `u32` start position, at `segment + 8`.
pub const SEGMENT_START: u32 = 8;
/// `u32` end position, at `segment + 12`; zeroed when the position reaches it.
pub const SEGMENT_END: u32 = 12;

const _: () = assert!(SLOT_STRIDE == 1 << 4, "rotlwi r8,r11,4");
const _: () = assert!(ENTRY_STRIDE == 3 * 16, "rotlwi ; add ; rlwinm 4,0,27");
const _: () = assert!(SEGMENT_STRIDE == 3 * 8, "rotlwi ; add ; rlwinm 3,0,28");
const _: () = assert!(ENTRY_LIMIT == ENTRY_CURSOR + 1, "lbz r11,469 ; lbz r9,470");
const _: () = assert!(SEG_COUNT == SEG_INDEX + 1, "lbz r11,49 ; lbz r8,50");

/// `sub_82B32550`: claim the ring's current slot if it is free, publishing the claimed index
/// through `out` and advancing the index byte. Returns 1 on a claim and 0 on a refusal.
///
/// `object` is `r3` and `out` is `r4`, a pointer to one word the caller owns.
///
/// The slot is `object + rotl(index, 4)` — a byte rotated left four is `index * 16` for any index
/// below 2^28, so the rotate and a multiply are the same thing here; it is written as the rotate
/// the original performs. Note that the slots are measured from the object itself while the busy
/// byte sits at `+113` inside one, so consecutive slots' busy bytes are 16 bytes apart in a region
/// that overlaps the object's own header — that is what the arithmetic says, and nothing here
/// establishes a nicer reading.
///
/// Writes: `out + 0` and `object + 473`, both only on the claim path. **A busy slot writes
/// nothing at all**, including the index, so a refusal leaves the ring exactly where it was.
///
/// The wrap is `subfic`/`subfe` in the original — a branchless select of 0 when the incremented
/// index equals 20 and the index otherwise. It is written as the wrap it implements.
pub fn claim_ring_slot(g: &mut Guest, object: u32, out: u32) -> Result<u64> {
    let index = g.u8(object + RING_INDEX)? as u32; // lbz r11,473(r3)
    // rotlwi r8,r11,4 ; add r8,r8,r3
    let slot = object.wrapping_add(index.rotate_left(4));
    // lbz r7,113(r8) ; bne -> li r3,0
    if g.u8(slot + SLOT_BUSY)? != 0 {
        return Ok(0);
    }
    g.set_u32(out, index)?; // stw r9,0(r4) — the claimed index, before advancing
    let next = (index + 1) & 0xFF; // addi ; clrlwi
    let wrapped = if next == RING_ENTRIES { 0 } else { next };
    g.set_u8(object + RING_INDEX, wrapped as u8)?;
    Ok(1) // li r3,1
}

/// `entry_table + (cursor + cursor*2) * 16`, relative to the object.
///
/// `rotlwi r8,r11,1 ; add r11,r11,r8 ; rlwinm r11,r11,4,0,27`. The cursor is a zero-extended
/// byte, so the rotate is a plain doubling and `3 * cursor * 16` never reaches bit 32; the mask
/// the `rlwinm` applies is therefore inert and is not written out.
fn entry_address(g: &Guest, object: u32, cursor: u32) -> Result<u32> {
    let scaled = (cursor.wrapping_add(cursor.rotate_left(1))).wrapping_mul(16);
    Ok(object
        .wrapping_add(g.u16(object + ENTRY_TABLE)? as u32)
        .wrapping_add(scaled))
}

/// `sub_82B349A8`: advance the entry cursor, wrapping at its limit, then clear the two published
/// words and — unless the new entry's state byte says otherwise — latch that entry's two floats
/// and its word.
///
/// Writes, in order: `+469` (twice on a wrap), `+432`, `+436`, then on the latch path `+432`
/// again, `+424`, `+428` and `+436` again, and finally `+471` on every path.
///
/// **The cursor is reloaded rather than reused.** The original writes `+469`, conditionally
/// writes it again, and then loads it back to index the entry table instead of keeping either
/// value in a register — the two stores could alias the byte the compiler would have kept. Both
/// the reload and the doubled store are reproduced.
///
/// **States 0, 1 and 4 skip the latch.** The original tests 4 and 0 together and 1 separately,
/// which is a redundant shape the compiler emitted from something like an enum switch; the effect
/// is that any state outside `{0, 1, 4}` latches. Reproduced as three tests rather than
/// simplified to a set membership, so the correspondence with the lifted branches survives.
///
/// The two floats go through an `lfs`/`stfs` pair, which is a bit copy for everything except a
/// signalling NaN — those are quieted by the widening load on the guest and by
/// [`fp::load_single`] here alike, with a hardware-chosen payload that this port does not pin.
pub fn advance_ring_cursor(g: &mut Guest, object: u32) -> Result<()> {
    let cursor = g.u8(object + ENTRY_CURSOR)? as u32; // lbz r11,469(r3)
    let limit = g.u8(object + ENTRY_LIMIT)? as u32; // lbz r9,470(r3)
    let advanced = (cursor + 1) & 0xFF; // addi ; clrlwi
    g.set_u8(object + ENTRY_CURSOR, advanced as u8)?; // stb r7,469(r3)
    if advanced == limit {
        g.set_u8(object + ENTRY_CURSOR, 0)?; // stb r10,469(r3) — a second store, not a select
    }

    // Reloaded, because the two stores above could alias the byte.
    let current = g.u8(object + ENTRY_CURSOR)? as u32;
    g.set_u32(object + ENTRY_FLAG_WORD, 0)?; // stw r10,432(r3)
    g.set_u32(object + LATCHED_WORD, 0)?; // stw r10,436(r3)

    let entry = entry_address(g, object, current)?;
    let state = g.u8(entry + ENTRY_STATE)? as u32; // lbz r9,46(r11)
    let latch = !(state as i32 == 4 || state as i32 == 0);
    if latch && state != 1 {
        g.set_u32(object + ENTRY_FLAG_WORD, 0)?; // stw r10,432(r3) again, as lifted
        let a = fp::load_single(g, entry + 12)?; // lfs f0,12(r11)
        fp::store_single(g, object + LATCHED_A, a)?;
        let b = fp::load_single(g, entry + 16)?; // lfs f13,16(r11)
        fp::store_single(g, object + LATCHED_B, b)?;
        let word = g.u32(entry + 20)?;
        g.set_u32(object + LATCHED_WORD, word)?; // stw r11,436(r3)
    }
    g.set_u8(object + ENTRY_BUSY, 0)?; // stb r10,471(r3)
    Ok(())
}

/// `object + table + 24 * index`.
///
/// `rotlwi ; add ; rlwinm rA,rA,3,0,28 ; add ; add`. The index is a zero-extended byte, so the
/// scaled term is exactly `24 * index` and the mask never bites. The two adds are 64-bit in the
/// lifted form, but the segment is only ever *addressed* through the low word, so wrapping at 32
/// bits is identical — unlike [`crate::scheduler::detach_instance`], nothing here returns the
/// register.
fn segment_address(object: u32, table: u32, index: u32) -> u32 {
    index
        .wrapping_mul(SEGMENT_STRIDE)
        .wrapping_add(table)
        .wrapping_add(object)
}

/// `sub_82B3C9D8`: advance the object's position by `amount`, and when that lands exactly on the
/// current segment's end, retire the segment and reload the position from the next one's start.
///
/// `object` is `r3`; `amount` is `r4`, of which only the low word matters.
///
/// Writes: `+28` on every path (twice on a rollover), and on a rollover the current segment's end
/// word and `+49` (twice when the index wraps).
///
/// **The end test is equality, not a threshold.** `cmpw` against the segment's end word followed
/// by `bnelr` means a position that steps *over* the end never rolls over and the cursor runs on
/// into whatever follows. That is what the original does; it is reproduced rather than turned
/// into a `>=`, and no test here establishes that overshooting cannot happen — it establishes
/// only that this code behaves as the guest does when it does.
///
/// **Three reloads are reproduced.** `+49` is read again after the segment's end word is
/// zeroed, and `+49` and `+36` are both read again before the new segment is addressed. The
/// original's compiler could not prove those stores miss the header, and neither can this: a
/// segment table that overlapped the header would change its own index mid-call.
pub fn advance_segment_position(g: &mut Guest, object: u32, amount: u32) -> Result<()> {
    let index = g.u8(object + SEG_INDEX)? as u32; // lbz r11,49(r3)
    let table = g.u32(object + SEG_TABLE)?; // lwz r9,36(r3)
    let position = g.u32(object + SEG_POSITION)?; // lwz r8,28(r3)
    // add r11,r8,r4 — 64-bit in the lifted form; only its low word is stored and compared.
    let advanced = position.wrapping_add(amount);
    g.set_u32(object + SEG_POSITION, advanced)?; // stw r11,28(r3)

    let segment = segment_address(object, table, index);
    // lwz r9,12(r10) — after the store, as the original: the segment could alias +28.
    let end = g.u32(segment + SEGMENT_END)?;
    if advanced != end {
        return Ok(()); // bnelr cr6
    }

    g.set_u32(segment + SEGMENT_END, 0)?; // stw r9,12(r10)
    let next = (g.u8(object + SEG_INDEX)? as u32 + 1) & 0xFF; // reloaded after the store
    g.set_u8(object + SEG_INDEX, next as u8)?; // stb r10,49(r3)
    let count = g.u8(object + SEG_COUNT)? as u32; // lbz r8,50(r3)
    if !(next < count) {
        // cmplw cr6,r10,r8 ; blt — unsigned
        g.set_u8(object + SEG_INDEX, 0)?; // stb r9,49(r3)
    }

    // loc_82B3CA34: both reloaded, as the original.
    let current = g.u8(object + SEG_INDEX)? as u32;
    let table_now = g.u32(object + SEG_TABLE)?;
    let start = g.u32(segment_address(object, table_now, current) + SEGMENT_START)?;
    g.set_u32(object + SEG_POSITION, start)?; // stw r9,28(r3)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT: u32 = 0x4000_0000;
    const OUT: u32 = 0x4000_0F00;
    /// Where the tests put the entry / segment tables, as a byte offset from the object.
    const TABLE: u32 = 0x600;

    fn guest() -> Guest {
        Guest::single(OBJECT, 0x1000)
    }

    #[test]
    fn a_free_slot_is_claimed_and_the_index_advances() {
        let mut g = guest();
        g.set_u8(OBJECT + RING_INDEX, 3).unwrap();
        g.set_u32(OUT, 0xDEAD_BEEF).unwrap();

        assert_eq!(claim_ring_slot(&mut g, OBJECT, OUT).unwrap(), 1);
        assert_eq!(
            g.u32(OUT).unwrap(),
            3,
            "the claimed index, before advancing"
        );
        assert_eq!(g.u8(OBJECT + RING_INDEX).unwrap(), 4);
    }

    #[test]
    fn a_busy_slot_refuses_and_writes_nothing() {
        let mut g = guest();
        g.set_u8(OBJECT + RING_INDEX, 3).unwrap();
        // Slot 3's busy byte: object + 3*16 + 113.
        g.set_u8(OBJECT + 3 * SLOT_STRIDE + SLOT_BUSY, 1).unwrap();
        g.set_u32(OUT, 0xDEAD_BEEF).unwrap();

        assert_eq!(claim_ring_slot(&mut g, OBJECT, OUT).unwrap(), 0);
        assert_eq!(
            g.u32(OUT).unwrap(),
            0xDEAD_BEEF,
            "the out word is untouched"
        );
        assert_eq!(g.u8(OBJECT + RING_INDEX).unwrap(), 3, "and so is the index");
    }

    #[test]
    fn the_busy_byte_is_read_from_this_slot_and_not_another() {
        // Marking slot 4 busy must not stop slot 3 being claimed, and vice versa. A stride of the
        // wrong size, or a missing one, shows up here and nowhere else.
        let mut g = guest();
        g.set_u8(OBJECT + RING_INDEX, 3).unwrap();
        g.set_u8(OBJECT + 4 * SLOT_STRIDE + SLOT_BUSY, 1).unwrap();
        assert_eq!(
            claim_ring_slot(&mut g, OBJECT, OUT).unwrap(),
            1,
            "slot 3 is still free"
        );
        assert_eq!(g.u8(OBJECT + RING_INDEX).unwrap(), 4);
        // Now the index names slot 4, which is the busy one.
        assert_eq!(claim_ring_slot(&mut g, OBJECT, OUT).unwrap(), 0);
        assert_eq!(
            g.u8(OBJECT + RING_INDEX).unwrap(),
            4,
            "and the index did not move"
        );
    }

    #[test]
    fn the_index_wraps_at_twenty_not_at_a_power_of_two() {
        let mut g = guest();
        g.set_u8(OBJECT + RING_INDEX, 18).unwrap();
        assert_eq!(claim_ring_slot(&mut g, OBJECT, OUT).unwrap(), 1);
        assert_eq!(g.u8(OBJECT + RING_INDEX).unwrap(), 19);
        assert_eq!(claim_ring_slot(&mut g, OBJECT, OUT).unwrap(), 1);
        assert_eq!(
            g.u32(OUT).unwrap(),
            19,
            "the last entry is still handed out"
        );
        assert_eq!(g.u8(OBJECT + RING_INDEX).unwrap(), 0, "20 becomes 0");
    }

    /// One 48-byte entry at table index `k`: two floats, a word, and a state byte.
    fn entry(g: &mut Guest, k: u32, state: u8, a: f32, b: f32, word: u32) {
        let at = OBJECT + TABLE + k * ENTRY_STRIDE;
        g.set_u32(at + 12, a.to_bits()).unwrap();
        g.set_u32(at + 16, b.to_bits()).unwrap();
        g.set_u32(at + 20, word).unwrap();
        g.set_u8(at + ENTRY_STATE, state).unwrap();
    }

    fn ring(g: &mut Guest, cursor: u8, limit: u8) {
        g.set_u16(OBJECT + ENTRY_TABLE, TABLE as u16).unwrap();
        g.set_u8(OBJECT + ENTRY_CURSOR, cursor).unwrap();
        g.set_u8(OBJECT + ENTRY_LIMIT, limit).unwrap();
        g.set_u8(OBJECT + ENTRY_BUSY, 1).unwrap();
    }

    #[test]
    fn the_cursor_advances_and_latches_the_new_entrys_parameters() {
        let mut g = guest();
        ring(&mut g, 0, 4);
        entry(&mut g, 0, 2, 11.0, 12.0, 0xAAAA);
        entry(&mut g, 1, 2, 0.25, -0.5, 0x1234_5678);
        g.set_u32(OBJECT + ENTRY_FLAG_WORD, 0xFFFF).unwrap();

        advance_ring_cursor(&mut g, OBJECT).unwrap();

        assert_eq!(
            g.u8(OBJECT + ENTRY_CURSOR).unwrap(),
            1,
            "the cursor advanced"
        );
        // Entry 1's parameters, not entry 0's: the latch reads the entry the cursor now names.
        assert_eq!(g.f32(OBJECT + LATCHED_A).unwrap(), 0.25);
        assert_eq!(g.f32(OBJECT + LATCHED_B).unwrap(), -0.5);
        assert_eq!(g.u32(OBJECT + LATCHED_WORD).unwrap(), 0x1234_5678);
        assert_eq!(
            g.u32(OBJECT + ENTRY_FLAG_WORD).unwrap(),
            0,
            "the flag word is cleared"
        );
        assert_eq!(g.u8(OBJECT + ENTRY_BUSY).unwrap(), 0);
    }

    #[test]
    fn states_zero_one_and_four_clear_the_published_words_without_latching() {
        for state in [0u8, 1, 4] {
            let mut g = guest();
            ring(&mut g, 0, 9);
            entry(&mut g, 1, state, 7.5, 8.5, 0xBEEF);
            // Stale values that a latch would overwrite and a skip must leave or clear.
            g.set_u32(OBJECT + LATCHED_A, 1.5f32.to_bits()).unwrap();
            g.set_u32(OBJECT + LATCHED_B, 2.5f32.to_bits()).unwrap();
            g.set_u32(OBJECT + LATCHED_WORD, 0xC0DE).unwrap();
            g.set_u32(OBJECT + ENTRY_FLAG_WORD, 0xC0DE).unwrap();

            advance_ring_cursor(&mut g, OBJECT).unwrap();

            assert_eq!(
                g.u8(OBJECT + ENTRY_CURSOR).unwrap(),
                1,
                "state {state}: still advances"
            );
            assert_eq!(
                g.f32(OBJECT + LATCHED_A).unwrap(),
                1.5,
                "state {state}: no latch"
            );
            assert_eq!(
                g.f32(OBJECT + LATCHED_B).unwrap(),
                2.5,
                "state {state}: no latch"
            );
            assert_eq!(
                g.u32(OBJECT + LATCHED_WORD).unwrap(),
                0,
                "state {state}: cleared"
            );
            assert_eq!(
                g.u32(OBJECT + ENTRY_FLAG_WORD).unwrap(),
                0,
                "state {state}: cleared"
            );
            assert_eq!(g.u8(OBJECT + ENTRY_BUSY).unwrap(), 0);
        }
        // The three skipping states are not a range: 2, 3 and 5 all latch.
        for state in [2u8, 3, 5] {
            let mut g = guest();
            ring(&mut g, 0, 9);
            entry(&mut g, 1, state, 7.5, 8.5, 0xBEEF);
            advance_ring_cursor(&mut g, OBJECT).unwrap();
            assert_eq!(
                g.u32(OBJECT + LATCHED_WORD).unwrap(),
                0xBEEF,
                "state {state} latches"
            );
        }
    }

    #[test]
    fn the_cursor_wraps_when_the_advanced_value_reaches_the_limit() {
        let mut g = guest();
        ring(&mut g, 3, 4); // 3 + 1 == the limit
        entry(&mut g, 0, 2, 1.0, 2.0, 0x1111);
        entry(&mut g, 4, 2, 9.0, 9.0, 0x9999);

        advance_ring_cursor(&mut g, OBJECT).unwrap();

        assert_eq!(
            g.u8(OBJECT + ENTRY_CURSOR).unwrap(),
            0,
            "wrapped to the start"
        );
        // And the entry that was latched is entry 0, so the wrap happened before the read.
        assert_eq!(g.u32(OBJECT + LATCHED_WORD).unwrap(), 0x1111);
        assert_eq!(g.f32(OBJECT + LATCHED_A).unwrap(), 1.0);
    }

    #[test]
    fn the_entry_stride_is_forty_eight_bytes() {
        // Entry 2 must be read at table + 96. A stride of 16 or 32 would land inside entry 0 or 1,
        // where this test puts values that would be latched instead.
        let mut g = guest();
        ring(&mut g, 1, 9);
        entry(&mut g, 0, 2, -1.0, -1.0, 0x0000_0001);
        entry(&mut g, 1, 2, -2.0, -2.0, 0x0000_0002);
        entry(&mut g, 2, 2, 4.0, 8.0, 0x0000_0003);

        advance_ring_cursor(&mut g, OBJECT).unwrap();

        assert_eq!(
            g.u32(OBJECT + LATCHED_WORD).unwrap(),
            3,
            "entry 2, at table + 96"
        );
        assert_eq!(g.f32(OBJECT + LATCHED_A).unwrap(), 4.0);
        assert_eq!(g.f32(OBJECT + LATCHED_B).unwrap(), 8.0);
    }

    /// Segment `k`: `[start, end)`, in the object's 24-byte segment array.
    fn segment(g: &mut Guest, k: u32, start: u32, end: u32) {
        let at = OBJECT + TABLE + k * SEGMENT_STRIDE;
        g.set_u32(at + SEGMENT_START, start).unwrap();
        g.set_u32(at + SEGMENT_END, end).unwrap();
    }

    fn segments(g: &mut Guest, count: u8, index: u8, position: u32) {
        g.set_u32(OBJECT + SEG_TABLE, TABLE).unwrap();
        g.set_u8(OBJECT + SEG_COUNT, count).unwrap();
        g.set_u8(OBJECT + SEG_INDEX, index).unwrap();
        g.set_u32(OBJECT + SEG_POSITION, position).unwrap();
    }

    #[test]
    fn an_advance_short_of_the_end_writes_only_the_position() {
        let mut g = guest();
        segments(&mut g, 3, 0, 100);
        segment(&mut g, 0, 100, 200);
        segment(&mut g, 1, 500, 600);

        advance_segment_position(&mut g, OBJECT, 40).unwrap();

        assert_eq!(g.u32(OBJECT + SEG_POSITION).unwrap(), 140);
        assert_eq!(
            g.u8(OBJECT + SEG_INDEX).unwrap(),
            0,
            "the segment did not change"
        );
        assert_eq!(
            g.u32(OBJECT + TABLE + SEGMENT_END).unwrap(),
            200,
            "nor did its end word"
        );
    }

    #[test]
    fn landing_on_the_end_retires_the_segment_and_reloads_from_the_next_one() {
        let mut g = guest();
        segments(&mut g, 3, 0, 180);
        segment(&mut g, 0, 100, 200);
        segment(&mut g, 1, 500, 600);

        advance_segment_position(&mut g, OBJECT, 20).unwrap();

        assert_eq!(
            g.u32(OBJECT + TABLE + SEGMENT_END).unwrap(),
            0,
            "segment 0's end is zeroed"
        );
        assert_eq!(g.u8(OBJECT + SEG_INDEX).unwrap(), 1);
        assert_eq!(
            g.u32(OBJECT + SEG_POSITION).unwrap(),
            500,
            "segment 1's start, not 200"
        );
    }

    #[test]
    fn stepping_over_the_end_does_not_roll_over_because_the_test_is_equality() {
        // Reproduced, not fixed: `cmpw ; bnelr` means one byte too far runs straight past the
        // segment boundary. A `>=` here would be a different program.
        let mut g = guest();
        segments(&mut g, 3, 0, 180);
        segment(&mut g, 0, 100, 200);
        segment(&mut g, 1, 500, 600);

        advance_segment_position(&mut g, OBJECT, 21).unwrap();

        assert_eq!(
            g.u32(OBJECT + SEG_POSITION).unwrap(),
            201,
            "the position ran on"
        );
        assert_eq!(
            g.u8(OBJECT + SEG_INDEX).unwrap(),
            0,
            "and the segment did not change"
        );
        assert_eq!(g.u32(OBJECT + TABLE + SEGMENT_END).unwrap(), 200);
    }

    #[test]
    fn the_index_wraps_to_zero_at_the_count_and_reloads_the_first_segment() {
        let mut g = guest();
        segments(&mut g, 2, 1, 550);
        segment(&mut g, 0, 100, 200);
        segment(&mut g, 1, 500, 600);

        advance_segment_position(&mut g, OBJECT, 50).unwrap();

        assert_eq!(
            g.u8(OBJECT + SEG_INDEX).unwrap(),
            0,
            "1 + 1 is not below the count of 2"
        );
        assert_eq!(
            g.u32(OBJECT + SEG_POSITION).unwrap(),
            100,
            "segment 0's start"
        );
        // Segment 1 is the one that was retired, at table + 24.
        assert_eq!(
            g.u32(OBJECT + TABLE + SEGMENT_STRIDE + SEGMENT_END)
                .unwrap(),
            0
        );
        assert_eq!(
            g.u32(OBJECT + TABLE + SEGMENT_END).unwrap(),
            200,
            "segment 0 still has its"
        );
    }

    #[test]
    fn the_position_advance_wraps_at_thirty_two_bits() {
        // `add r11,r8,r4` is 64-bit in the lifted form, but only the low word is stored and only
        // the low word is compared against the segment's end, so the wrap is the guest's.
        let mut g = guest();
        segments(&mut g, 2, 0, 0xFFFF_FFF0);
        segment(&mut g, 0, 100, 0x20);
        segment(&mut g, 1, 777, 900);

        advance_segment_position(&mut g, OBJECT, 0x30).unwrap();

        // 0xFFFFFFF0 + 0x30 == 0x20 on 32 bits, which is exactly segment 0's end.
        assert_eq!(
            g.u8(OBJECT + SEG_INDEX).unwrap(),
            1,
            "the wrapped sum matched the end"
        );
        assert_eq!(g.u32(OBJECT + SEG_POSITION).unwrap(), 777);
    }
}
