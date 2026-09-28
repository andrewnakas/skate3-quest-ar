//! The evaluator's stateful slots: eight ops that hold something across calls.
//!
//! All eight are transcriptions of `STATUS: verified` bodies. These are where the evaluator stops
//! being a calculator — a hysteresis latch, a timer, two cursors, a ramp, a delay line, a shuffle
//! bag and a draw from the shared counter — and they are also where the store ordering starts to
//! matter, because several of them reload a word they have just written.
//!
//! Three of them reach outside their own object: [`op_random_in_range`], [`op_weighted_cursor`] and
//! [`op_shuffle_bag`] all call [`crate::counter::advance`], whose 24-byte global is the only state
//! in the evaluator that is not per-instance. The C++ `Windows()` predicates for all three *refuse*
//! the call when the op's own record overlaps that global, because then the fields reloaded after
//! the call would no longer derive from entry state; there is no analogue of that refusal here,
//! since nothing in Rust is being bracketed, but the overlap remains a real hazard for a caller that
//! lays objects out itself.

use crate::counter;
use crate::eval::{
    HALF_SINGLE, MINUS_ONE_SINGLE, RATE_UNIT_SINGLE, TICK_SCALE_GLOBAL, ZERO_SINGLE,
};
use crate::fp;
use crate::{Guest, Result};

/// Slot 10 — `sub_82B1C778`. A window trigger with hysteresis.
///
/// Layout: `s32` entry window `[+0, +4]`, `s32` exit window `[+8, +12]`, `u8 latched` at `+16`,
/// `u8 edge` at `+17`, `s32 value` at `+20`.
///
/// The latch is set when the value enters the entry window and cleared when it enters the *exit*
/// window; between the two windows it holds. `+17` is a one-call pulse: it is 1 for exactly the
/// call that latches and 0 on every other, and it is also what comes back in `r3`.
///
/// Writes: `+16` on the latch and un-latch paths, `+17` on every path.
pub fn op_window_latch(g: &mut Guest, object: u32) -> Result<u64> {
    let value = g.u32(object + 20)? as i32;
    let enter_lo = g.u32(object)? as i32;

    let mut in_entry_window = false;
    if value >= enter_lo {
        let enter_hi = g.u32(object + 4)? as i32;
        in_entry_window = value <= enter_hi;
    }

    if in_entry_window {
        if g.u8(object + 16)? == 0 {
            g.set_u8(object + 16, 1)?;
            g.set_u8(object + 17, 1)?;
            return Ok(1);
        }
        // Already latched: no store at +16, fall through to the clear-edge path.
    } else {
        let exit_lo = g.u32(object + 8)? as i32;
        if value >= exit_lo {
            let exit_hi = g.u32(object + 12)? as i32;
            if value <= exit_hi {
                g.set_u8(object + 16, 0)?;
            }
        }
    }

    g.set_u8(object + 17, 0)?;
    Ok(0)
}

/// Slot 6 — `sub_82B1C4B8`. A stepping cursor with wraparound.
///
/// Layout: `s32 low` at `+0`, `s32 high` at `+4`, `s32 current` at `+8`, `s8 step` at `+12`,
/// `s32 enabled` at `+16`, `s32 probe` at `+20`.
///
/// When the probe lies inside `[low, high]` it is returned unchanged and nothing is written.
/// Otherwise, and only when `enabled > 0`, the cursor advances by the signed byte step, wrapping
/// over `high` to `low` and under `low` to `high`. Writes: `+8` only, on three paths.
///
/// The step's add is 64-bit but only its low word is stored and compared, so a cursor near
/// `i32::MAX` wraps at bit 32 exactly as the guest's store does.
pub fn op_stepping_cursor(g: &mut Guest, object: u32) -> Result<u64> {
    let probe = g.u32(object + 20)?;
    let low = g.u32(object)?;

    if !((probe as i32) < (low as i32)) {
        let high = g.u32(object + 4)?;
        if (probe as i32) <= (high as i32) {
            return Ok(probe as u64); // blelr cr6 — r3 already holds the zero-extended probe
        }
    }

    let enabled = g.u32(object + 16)? as i32;
    if enabled > 0 {
        let step = (g.u8(object + 12)? as i8) as i32; // lbz ; extsb
        let current = g.u32(object + 8)?;
        let high = g.u32(object + 4)?;
        let next = (step as u32).wrapping_add(current);
        g.set_u32(object + 8, next)?;
        if (next as i32) > (high as i32) {
            g.set_u32(object + 8, low)?;
            return Ok(low as u64);
        }
        if (next as i32) < (low as i32) {
            g.set_u32(object + 8, high)?;
        }
    }

    Ok(g.u32(object + 8)? as u64) // reloaded after the store(s)
}

/// Slot 11 — `sub_82B1C7E8`. A single-precision timer that fires once and parks.
///
/// Layout: `f32 accumulator` at `+0`, `u8 fired` at `+4`, `s32 reset` at `+8`, `s32 threshold` at
/// `+12`.
///
/// A non-zero `reset` zeroes the accumulator before the test. A *negative* accumulator, which is
/// the parked state after a previous fire, returns 0 immediately without advancing. Otherwise the
/// accumulator is compared against `float(threshold)`: at or past it, `fired` becomes 1, the
/// accumulator is parked at the guest's `-1.0f` cell and 1 is returned; short of it, one step from
/// the tick-scale global is added and 0 is returned.
///
/// **NaN fires.** Both comparisons are `fcmpu ... ; blt`, and an unordered compare does not
/// branch, so a NaN accumulator skips the parked-state early return and then passes the threshold
/// test. Writes: `+0` on three of four paths, `+4` on all four.
pub fn op_timer(g: &mut Guest, object: u32) -> Result<u64> {
    let reset = g.u32(object + 8)? as i32;
    let zero = fp::load_single(g, ZERO_SINGLE)?;

    if reset != 0 {
        fp::store_single(g, object, zero)?;
    } else {
        let accumulator = fp::load_single(g, object)?;
        if accumulator < zero {
            g.set_u8(object + 4, 0)?;
            return Ok(0);
        }
    }

    let threshold_word = g.u32(object + 12)?;
    let accumulator = fp::load_single(g, object)?; // reloaded after the store
    let threshold = fp::word_to_single(threshold_word);
    if !(accumulator < threshold) {
        g.set_u8(object + 4, 1)?;
        let parked = fp::load_single(g, MINUS_ONE_SINGLE)?;
        fp::store_single(g, object, parked)?;
        return Ok(1);
    }

    let step = fp::load_single(g, TICK_SCALE_GLOBAL)?;
    fp::store_single(g, object, fp::add_single(accumulator, step))?;
    g.set_u8(object + 4, 0)?;
    Ok(0)
}

/// Slot 29 — `sub_82B1D5D0`. Ramp a float toward an integer target, re-deriving the rate whenever
/// the target or the duration changes.
///
/// Layout: `f32 value` at `+0`, `f32 rate` at `+4`, `s32 last_target` at `+8`, `s32 last_duration`
/// at `+12`, `s32 duration` at `+16`, `s32 steps` at `+20`, `s32 target` at `+24`.
///
/// The rate is `((target - value) / duration) * G * U`, where `G` is the tick-scale global the
/// scheduler writes and `U` is the `1/4096` unit single. A non-positive duration snaps the value
/// to the target instead. The returned value is the ramp position rounded half away from zero;
/// both early returns hand back the **target**, zero-extended.
///
/// Writes: `+0` on most paths, and `+4`, `+8`, `+12` on a re-derive.
///
/// One equivalence worth recording: after the `fmadds` store the original reloads `+0` to make its
/// overshoot comparison, and this code compares the stored value directly. The two are the same
/// because `fmadd_single` already returns a single-exact value, so the store and reload are
/// lossless — not because the reload was dropped as unimportant.
pub fn op_ramp(g: &mut Guest, object: u32) -> Result<u64> {
    let target = g.u32(object + 24)?;
    let target_single = fp::word_to_single(target);
    let current = fp::load_single(g, object)?;
    if target_single == current {
        return Ok(target as u64); // beqlr cr6 — unordered is not eq, so a NaN falls through
    }

    let mut rederive = true;
    let last_target = g.u32(object + 8)? as i32;
    if (target as i32) == last_target {
        let duration = g.u32(object + 16)? as i32;
        let last_duration = g.u32(object + 12)? as i32;
        if duration == last_duration {
            rederive = false;
        }
    }

    if rederive {
        let duration = g.u32(object + 16)?;
        g.set_u32(object + 8, target)?;
        g.set_u32(object + 12, duration)?;
        if !((duration as i32) > 0) {
            fp::store_single(g, object, target_single)?;
            return Ok(target as u64);
        }
        let delta = fp::sub_single(target_single, current);
        let frames = fp::word_to_single(duration);
        let scale = fp::load_single(g, TICK_SCALE_GLOBAL)?;
        let unit = fp::load_single(g, RATE_UNIT_SINGLE)?;
        let per_frame = fp::div_single(delta, frames);
        let scaled = fp::mul_single(per_frame, scale);
        fp::store_single(g, object + 4, fp::mul_single(scaled, unit))?;
    }

    let steps = g.u32(object + 20)?;
    let rate = fp::load_single(g, object + 4)?; // reloaded: it may have just been stored
    let step_count = fp::word_to_single(steps);
    let zero = fp::load_single(g, ZERO_SINGLE)?;
    let falling = rate < zero;
    let next = fp::fmadd_single(step_count, rate, current);
    fp::store_single(g, object, next)?;
    let overshot = if falling {
        next < target_single
    } else {
        next > target_single
    };
    if overshot {
        fp::store_single(g, object, target_single)?;
    }

    let value = fp::load_single(g, object)?;
    let half = fp::load_single(g, HALF_SINGLE)?;
    let rounded = if value < zero {
        fp::sub_single(value, half)
    } else {
        fp::add_single(value, half)
    };
    Ok(fp::fctiwz_low_word(rounded) as u64)
}

/// Slot 7 — `sub_82B1C528`. A value drawn into a range, cached until the next draw is asked for.
///
/// Layout: `u32 base` at `+0`, `u32 range` at `+4`, `u32 current` at `+8`, `u32 enabled` at `+12`.
///
/// A zero `enabled` returns the cached `+8` with no draw and no store. Otherwise the shared
/// counter advances and `base + draw % range` is stored into `+8` and returned.
///
/// **A 64-bit chain with a 32-bit divide inside it.** The division takes the counter's *low* word,
/// but the multiply-back and both the subtract and the final add are 64-bit, so the draw's high
/// half survives into the returned `r3` while only the low word reaches memory. RexGlue defines
/// `divwu` by zero as 0 rather than faulting, and that definition is reproduced; the `twllei`
/// beside it only warns.
///
/// Writes: `+8`, plus the whole 24-byte counter global.
pub fn op_random_in_range(g: &mut Guest, object: u32) -> Result<u64> {
    let enabled = g.u32(object + 12)?;
    if enabled == 0 {
        return Ok(g.u32(object + 8)? as u64);
    }

    let draw = counter::advance(g)?;
    let range = g.u32(object + 4)?; // read AFTER the draw, as the original does
    let start = g.u32(object)?;
    let quotient = if range != 0 { (draw as u32) / range } else { 0 };
    let product = ((quotient as i32 as i64) * (range as i32 as i64)) as u64;
    let result = draw.wrapping_sub(product).wrapping_add(start as u64);
    g.set_u32(object + 8, result as u32)?;
    Ok(result)
}

/// Slot 8 — `sub_82B1C598`. Draw the next entry from a shuffle bag: pick a random slot from the
/// part of the array that has not been drawn yet, swap it down to the cursor, advance the cursor and
/// wrap at the end.
///
/// Layout: `u16 gate_offset` at `+0` (a **self-relative** byte offset of a word; a zero there makes
/// the bag inert), `u8 width` at `+2` (1 selects byte elements, anything else halfwords), `s8
/// wrap_flag` at `+3`, `u32 base` at `+4`, `u16 cursor` at `+8`, `u16 count` at `+10`, `u32 result`
/// at `+12`, the element array at `+16`.
///
/// The undrawn range is `count - wrap_flag - cursor`, so the flag does double duty: it records that
/// the previous draw wrapped *and* shortens the next range by one. The swap is routed **through the
/// result word at `+12`**, which is the function's only temporary — so `+12` briefly holds the
/// drawn element before it holds `base + element`.
///
/// Writes: two element slots, `+3`, `+8` (twice on a wrap), `+12` (twice), and the counter global.
///
/// **The one input the C++ `Windows()` refuses.** A zero range traps and then indexes with the raw
/// counter word, so the store lands wherever that points; the harness declines to bracket it and it
/// was therefore never compared in either language. Here it surfaces as an out-of-segment `Error`
/// rather than a wild write, which is a different behaviour from the guest's and is the right one:
/// nothing is known about what the guest does there.
pub fn op_shuffle_bag(g: &mut Guest, bag: u32) -> Result<u64> {
    /// `addi rX,rY,8 ; rlwinm rX,rX,1,0,30` — the value doubled, keeping only the low word with
    /// bit 0 cleared. Halfword element *k* therefore sits at `bag + 16 + 2k`.
    fn half_offset(index_plus_eight: u64) -> u32 {
        ((index_plus_eight as u32) << 1) & 0xFFFF_FFFE
    }

    let gate_off = g.u16(bag)? as u32;
    if g.u32(gate_off.wrapping_add(bag))? == 0 {
        return Ok(g.u32(bag + 12)? as u64); // inert: hand back the cached result
    }

    // The cursor is read BEFORE the draw; every other field after it.
    let cursor = g.u16(bag + 8)? as u32;
    let draw = counter::advance(g)?;

    let bias = ((g.u8(bag + 3)? as i8) as i64) as u64; // lbz ; extsb
    let count = g.u16(bag + 10)? as u64;
    let width_sel = g.u8(bag + 2)?;
    // 64-bit on a sign-extended byte and two zero-extended halfwords; only the LOW word of the
    // result is the divisor and the multiplicand.
    let range = count.wrapping_sub(bias).wrapping_sub(cursor as u64) as u32;
    let quotient = if range != 0 { (draw as u32) / range } else { 0 };
    let product = ((quotient as i32 as i64) * (range as i32 as i64)) as u64;
    let slot = (cursor as u64).wrapping_add(draw.wrapping_sub(product));

    if width_sel == 1 {
        let picked = (slot as u32).wrapping_add(bag);
        let at_cursor = cursor.wrapping_add(bag);
        let value = g.u8(picked.wrapping_add(16))? as u32;
        g.set_u32(bag + 12, value)?; // the swap's temporary, stored as a whole word
        let held = g.u8(at_cursor.wrapping_add(16))?;
        g.set_u8(picked.wrapping_add(16), held)?;
        let temp = g.u32(bag + 12)?;
        let cursor_now = g.u16(bag + 8)? as u32;
        g.set_u8(cursor_now.wrapping_add(bag).wrapping_add(16), temp as u8)?;
    } else {
        let picked = half_offset(slot.wrapping_add(8));
        let at_cursor = half_offset((cursor as u64).wrapping_add(8));
        let value = g.u16(picked.wrapping_add(bag))? as u32;
        g.set_u32(bag + 12, value)?;
        let held = g.u16(at_cursor.wrapping_add(bag))?;
        g.set_u16(picked.wrapping_add(bag), held)?;
        let temp = g.u32(bag + 12)?;
        let cursor_now = g.u16(bag + 8)? as u64;
        g.set_u16(
            half_offset(cursor_now.wrapping_add(8)).wrapping_add(bag),
            temp as u16,
        )?;
    }

    // Every field reloaded after the swap's stores.
    let cursor_next = g.u16(bag + 8)? as u32;
    let drawn = g.u32(bag + 12)?;
    let base_value = g.u32(bag + 4)?;
    let limit = g.u16(bag + 10)? as u32;
    let advanced = (cursor_next + 1) & 0xFFFF; // addi ; clrlwi
    // A 64-bit sum of two zero-extended words: a carry lands in bit 32, which r3 keeps and the
    // store drops.
    let result = (base_value as u64).wrapping_add(drawn as u64);
    g.set_u16(bag + 8, advanced as u16)?;
    g.set_u32(bag + 12, result as u32)?;
    if advanced < limit {
        g.set_u8(bag + 3, 0)?;
    } else {
        g.set_u16(bag + 8, 0)?;
        g.set_u8(bag + 3, 1)?;
    }
    Ok(result)
}

/// Slot 9 — `sub_82B1C6C8`. Pick an index by accumulating signed per-step weights until the sum
/// passes the counter's value modulo 100.
///
/// Layout: `u32 source` at `+0` (the weight table lives 16 bytes into it), `u32 base_index` at
/// `+4`, `s32 step_count` at `+8`, `u32 result` at `+12`, `s32 enable` at `+16`.
///
/// A zero `enable` reads `+12` back untouched and makes no draw. Otherwise the counter advances,
/// its value is reduced modulo 100 by the usual `0x51EB851F` reciprocal, and the signed weight
/// bytes are summed until the running total exceeds that remainder — at which point
/// `base_index + index` is published into `+12`.
///
/// Two asymmetries are preserved: the weight total is summed in 64 bits from **signed** bytes but
/// compared **unsigned** on low words, so a negative running total compares as enormous and ends
/// the walk on the first step; and the loop bound is reloaded from `+8` every iteration, so a
/// weight table that overlaps the record can change its own trip count.
///
/// Writes: `+12`, plus the whole counter global.
pub fn op_weighted_cursor(g: &mut Guest, record: u32) -> Result<u64> {
    /// `lis r11,20971 ; ori r8,r11,34079` — the unsigned divide-by-100 reciprocal, computed the
    /// way the lifted body forms it.
    const RECIPROCAL: u64 = 0x51EB_851F;
    const TABLE_OFFSET: u32 = 16;

    if g.u32(record + 16)? as i32 != 0 {
        let now = counter::advance(g)?;
        let steps = g.u32(record + 8)? as i32;
        let source = g.u32(record)?;

        // mulhwu ; rlwinm 27,5,31 ; mulli 100 ; subf — the remainder, formed on the FULL counter
        // value even though the reciprocal only sees its low word.
        let high = ((now as u32 as u64) * RECIPROCAL) >> 32;
        let quotient = fp::rlwinm(high, 27, 0x7FF_FFFF);
        let remainder = now.wrapping_sub(quotient.wrapping_mul(100));

        // Tested signed against the count read BEFORE the division, so a negative count skips.
        if steps > 0 {
            let table = source.wrapping_add(TABLE_OFFSET);
            let mut index: i64 = 0;
            let mut total: i64 = 0;
            loop {
                total += (g.u8(table.wrapping_add(index as u32))? as i8) as i64;
                if (total as u32) > (remainder as u32) {
                    let base_index = g.u32(record + 4)?;
                    let published = (base_index as u64).wrapping_add(index as u64);
                    g.set_u32(record + 12, published as u32)?;
                    break;
                }
                let limit = g.u32(record + 8)? as i32; // reloaded every iteration
                index += 1;
                if (index as i32) >= limit {
                    break;
                }
            }
        }
    }

    Ok(g.u32(record + 12)? as u64) // reloaded on every path, including those that never wrote it
}

/// Slot 16 — `sub_82B1CD28`. A word-wide delay line whose length is re-derived when the source's
/// delay changes.
///
/// Layout: `u16 record_offset` at `+0` (the source record is at `object + record_offset`), `u16
/// capacity` at `+2`, `u16 write` at `+4`, `u16 read` at `+6`, `u32 last_delay` at `+8`, `u32
/// ring[]` at `+12` with slot *k* at `4 * (k + 3)`. The source record is `{u32 word, s32 delay}`.
///
/// On a changed delay the write index is re-placed at `read + round(delay / G)` — `G` being the
/// tick-scale global — clamped to `capacity - 1`, and a negative delay is **clamped to zero in the
/// source record itself**, which is the only write this op makes outside its own object. Both
/// indices then wrap, the source word is pushed at the write slot, the word at the read slot is
/// returned, and both indices advance by one.
///
/// Writes: `+4` and `+6` always; `+8` and one ring slot on the changed-delay path; `record + 4`
/// when the delay was negative.
pub fn op_delay_ring(g: &mut Guest, object: u32) -> Result<u64> {
    let record_offset = g.u16(object)? as u32;
    let last_delay = g.u32(object + 8)?;
    let record = record_offset.wrapping_add(object);
    let delay_seen = g.u32(record + 4)?;

    if (delay_seen as i32) != (last_delay as i32) {
        g.set_u32(object + 8, delay_seen)?;
        if (g.u32(record + 4)? as i32) < 0 {
            g.set_u32(record + 4, 0)?;
        }
        let delay = g.u32(record + 4)?;
        let capacity = g.u16(object + 2)? as u64;
        let wide = ((delay as i32) as i64) as f64; // extsw ; fcfid
        let divisor = fp::load_single(g, TICK_SCALE_GLOBAL)?;
        let narrow = (wide as f32) as f64; // frsp
        let half = fp::load_single(g, HALF_SINGLE)?;
        let quotient = fp::div_single(narrow, divisor);
        let rounded = fp::add_single(quotient, half);
        let mut offset = fp::fctiwz_low_word(rounded) as u64;
        if !((offset as u32 as i32) < (capacity as u32 as i32)) {
            offset = capacity.wrapping_sub(1); // 64-bit, so capacity 0 gives an all-ones offset
        }
        let read_index = g.u16(object + 6)? as u64;
        g.set_u16(object + 4, read_index.wrapping_add(offset) as u16)?;
    }

    let capacity = g.u16(object + 2)? as u32;
    let write_index = g.u16(object + 4)? as u32;
    if !(write_index < capacity) {
        g.set_u16(object + 4, write_index.wrapping_sub(capacity) as u16)?;
    }
    if !((g.u16(object + 6)? as u32) < capacity) {
        g.set_u16(object + 6, 0)?;
    }

    let write_slot = g.u16(object + 4)? as u32;
    let word = g.u32(record)?;
    g.set_u32((write_slot.wrapping_add(3) << 2).wrapping_add(object), word)?;
    let write_after = g.u16(object + 4)? as u32;
    let read_slot = g.u16(object + 6)? as u32;
    let result = g.u32((read_slot.wrapping_add(3) << 2).wrapping_add(object))?;
    g.set_u16(object + 6, read_slot.wrapping_add(1) as u16)?;
    g.set_u16(object + 4, write_after.wrapping_add(1) as u16)?;
    Ok(result as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::testutil::*;

    #[test]
    fn the_window_latch_holds_between_the_two_windows() {
        let mut g = block_guest();
        // Entry window [100, 200], exit window [0, 50].
        put_words(&mut g, &[100, 200, 0, 50]);

        let set_value = |g: &mut Guest, v: i32| g.set_u32(BLOCK + 20, v as u32).unwrap();

        // Below both windows and above the exit window: nothing latches, no edge.
        set_value(&mut g, 75);
        assert_eq!(op_window_latch(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.u8(BLOCK + 16).unwrap(), 0);

        // Entering the entry window latches and pulses.
        set_value(&mut g, 150);
        assert_eq!(op_window_latch(&mut g, BLOCK).unwrap(), 1);
        assert_eq!(g.u8(BLOCK + 16).unwrap(), 1, "latched");
        assert_eq!(g.u8(BLOCK + 17).unwrap(), 1, "the rising edge");

        // Staying inside holds the latch but the pulse is gone.
        assert_eq!(op_window_latch(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.u8(BLOCK + 16).unwrap(), 1);
        assert_eq!(g.u8(BLOCK + 17).unwrap(), 0, "one call only");

        // The gap between the windows holds the latch: this is the hysteresis.
        set_value(&mut g, 75);
        op_window_latch(&mut g, BLOCK).unwrap();
        assert_eq!(g.u8(BLOCK + 16).unwrap(), 1, "still latched in the gap");

        // The exit window clears it.
        set_value(&mut g, 25);
        op_window_latch(&mut g, BLOCK).unwrap();
        assert_eq!(g.u8(BLOCK + 16).unwrap(), 0);

        // Re-entering pulses again.
        set_value(&mut g, 200);
        assert_eq!(
            op_window_latch(&mut g, BLOCK).unwrap(),
            1,
            "the bound is inclusive"
        );
    }

    #[test]
    fn the_stepping_cursor_returns_the_probe_when_it_is_in_range() {
        let mut g = block_guest();
        put_words(&mut g, &[10, 20, 15, 0, 1, 12]); // low, high, current, step, enabled, probe
        g.set_u8(BLOCK + 12, 3).unwrap(); // the step is a signed BYTE at +12

        assert_eq!(
            op_stepping_cursor(&mut g, BLOCK).unwrap(),
            12,
            "the probe is inside [10,20]"
        );
        assert_eq!(g.u32(BLOCK + 8).unwrap(), 15, "and nothing was written");

        // Out of range: the cursor steps.
        g.set_u32(BLOCK + 20, 99).unwrap();
        assert_eq!(op_stepping_cursor(&mut g, BLOCK).unwrap(), 18);
        assert_eq!(
            op_stepping_cursor(&mut g, BLOCK).unwrap(),
            10,
            "21 is over high, so it wraps"
        );

        // A negative step wraps the other way.
        g.set_u8(BLOCK + 12, (-3i8) as u8).unwrap();
        g.set_u32(BLOCK + 8, 11).unwrap();
        assert_eq!(
            op_stepping_cursor(&mut g, BLOCK).unwrap(),
            20,
            "8 is under low, so it wraps"
        );

        // Disabled: the cursor is read back untouched.
        g.set_u32(BLOCK + 16, 0).unwrap();
        g.set_u32(BLOCK + 8, 17).unwrap();
        assert_eq!(op_stepping_cursor(&mut g, BLOCK).unwrap(), 17);
        assert_eq!(g.u32(BLOCK + 8).unwrap(), 17);
    }

    #[test]
    fn the_timer_fires_once_then_parks_negative() {
        let mut g = block_guest();
        put_rodata(&mut g);
        // The tick-scale global is the timer's step. Give it 0.25 so four calls reach 1.0.
        g.set_u32(TICK_SCALE_GLOBAL, 0.25f32.to_bits()).unwrap();
        fp::store_single(&mut g, BLOCK, 0.0).unwrap();
        g.set_u32(BLOCK + 8, 0).unwrap(); // no reset
        g.set_u32(BLOCK + 12, 1).unwrap(); // threshold 1

        // 0.0 is already below the threshold, so the first three calls accumulate.
        for expect in [0.25f32, 0.5, 0.75, 1.0] {
            assert_eq!(op_timer(&mut g, BLOCK).unwrap(), 0);
            assert_eq!(g.u8(BLOCK + 4).unwrap(), 0);
            assert_eq!(g.f32(BLOCK).unwrap(), expect);
        }
        // At 1.0 the threshold is reached: it fires and parks.
        assert_eq!(op_timer(&mut g, BLOCK).unwrap(), 1);
        assert_eq!(g.u8(BLOCK + 4).unwrap(), 1);
        assert_eq!(
            g.f32(BLOCK).unwrap(),
            -1.0,
            "parked at the guest's -1.0f cell"
        );

        // Parked: the negative accumulator short-circuits and does not advance.
        assert_eq!(op_timer(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.f32(BLOCK).unwrap(), -1.0);

        // A non-zero reset zeroes the accumulator first, which un-parks it.
        g.set_u32(BLOCK + 8, 1).unwrap();
        assert_eq!(op_timer(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.f32(BLOCK).unwrap(), 0.25);
    }

    #[test]
    fn a_nan_accumulator_fires_the_timer_because_an_unordered_compare_does_not_branch() {
        let mut g = block_guest();
        put_rodata(&mut g);
        g.set_u32(BLOCK, f32::NAN.to_bits()).unwrap();
        g.set_u32(BLOCK + 8, 0).unwrap();
        g.set_u32(BLOCK + 12, 1_000_000).unwrap();
        assert_eq!(
            op_timer(&mut g, BLOCK).unwrap(),
            1,
            "NaN is neither below zero nor below the threshold"
        );
        assert_eq!(g.f32(BLOCK).unwrap(), -1.0);
    }

    #[test]
    fn the_ramp_derives_a_rate_then_clamps_at_the_target() {
        let mut g = block_guest();
        put_rodata(&mut g);
        // G * U scales the per-frame rate. Set G so that G/4096 == 1, i.e. G == 4096: then the
        // rate is exactly (target - value) / duration and the arithmetic is checkable by hand.
        g.set_u32(TICK_SCALE_GLOBAL, 4096.0f32.to_bits()).unwrap();

        fp::store_single(&mut g, BLOCK, 0.0).unwrap();
        g.set_u32(BLOCK + 8, 0xFFFF_FFFF).unwrap(); // last_target, deliberately stale
        g.set_u32(BLOCK + 12, 0).unwrap(); // last_duration
        g.set_u32(BLOCK + 16, 4).unwrap(); // duration
        g.set_u32(BLOCK + 20, 1).unwrap(); // steps
        g.set_u32(BLOCK + 24, 100).unwrap(); // target

        // First call re-derives: rate = 100/4 = 25, value = 0 + 1*25 = 25.
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 25);
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 25.0, "the derived rate");
        assert_eq!(g.u32(BLOCK + 8).unwrap(), 100, "last_target latched");
        assert_eq!(g.u32(BLOCK + 12).unwrap(), 4, "last_duration latched");
        assert_eq!(g.f32(BLOCK).unwrap(), 25.0);

        // Subsequent calls reuse the rate: no re-derive, because nothing changed.
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 50);
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 75);
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 100);
        // Arriving exactly on the target: the next call takes the equality early return.
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 100);
        assert_eq!(g.f32(BLOCK).unwrap(), 100.0);

        // Overshoot clamps rather than passing the target.
        fp::store_single(&mut g, BLOCK, 90.0).unwrap();
        g.set_u32(BLOCK + 20, 4).unwrap(); // four steps of 25 would reach 190
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 100);
        assert_eq!(
            g.f32(BLOCK).unwrap(),
            100.0,
            "clamped at the target, not 190"
        );
    }

    #[test]
    fn a_non_positive_duration_snaps_the_ramp_and_returns_the_target() {
        let mut g = block_guest();
        put_rodata(&mut g);
        fp::store_single(&mut g, BLOCK, 5.0).unwrap();
        g.set_u32(BLOCK + 4, 0x7F7F_FFFF).unwrap(); // a rate that must NOT be touched
        g.set_u32(BLOCK + 16, 0).unwrap(); // duration 0
        g.set_u32(BLOCK + 24, (-7i32) as u32).unwrap(); // target -7

        assert_eq!(
            op_ramp(&mut g, BLOCK).unwrap(),
            0xFFFF_FFF9,
            "the target, zero-extended"
        );
        assert_eq!(g.f32(BLOCK).unwrap(), -7.0, "snapped");
        assert_eq!(
            g.u32(BLOCK + 4).unwrap(),
            0x7F7F_FFFF,
            "the rate was left alone"
        );
    }

    #[test]
    fn a_falling_ramp_clamps_from_the_other_side() {
        let mut g = block_guest();
        put_rodata(&mut g);
        g.set_u32(TICK_SCALE_GLOBAL, 4096.0f32.to_bits()).unwrap();
        fp::store_single(&mut g, BLOCK, 100.0).unwrap();
        g.set_u32(BLOCK + 16, 4).unwrap(); // duration
        g.set_u32(BLOCK + 20, 9).unwrap(); // nine steps, far past the target
        g.set_u32(BLOCK + 24, 0).unwrap(); // target 0
        // rate = (0 - 100) / 4 = -25, which is negative, so the falling branch decides.
        assert_eq!(op_ramp(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.f32(BLOCK + 4).unwrap(), -25.0);
        assert_eq!(g.f32(BLOCK).unwrap(), 0.0, "clamped up to the target");
    }

    #[test]
    fn random_in_range_caches_when_disabled_and_draws_when_enabled() {
        let mut g = block_guest();
        put_rodata(&mut g);
        put_words(&mut g, &[1000, 7, 0xABCD, 0]); // base, range, current, enabled=0

        assert_eq!(op_random_in_range(&mut g, BLOCK).unwrap(), 0xABCD, "cached");
        assert_eq!(
            g.u32(counter::COUNTER + 20).unwrap(),
            0,
            "the counter did not advance"
        );

        g.set_u32(BLOCK + 12, 1).unwrap();
        // From a zeroed counter the first draw is 0, so the result is base + 0 % 7.
        assert_eq!(op_random_in_range(&mut g, BLOCK).unwrap(), 1000);
        assert_eq!(g.u32(BLOCK + 8).unwrap(), 1000, "published into +8");
        assert_eq!(
            g.u32(counter::COUNTER + 20).unwrap(),
            1,
            "the counter advanced"
        );
        // Second draw is 1, third is 7 -> 7 % 7 == 0.
        assert_eq!(op_random_in_range(&mut g, BLOCK).unwrap(), 1001);
        assert_eq!(op_random_in_range(&mut g, BLOCK).unwrap(), 1000);
    }

    #[test]
    fn random_in_range_keeps_the_draws_high_half_in_r3_but_not_in_memory() {
        let mut g = block_guest();
        put_rodata(&mut g);
        put_words(&mut g, &[0, 1, 0, 1]); // base 0, range 1, enabled
        // Force a draw with bits above 31: every counter word maximal gives 0x5FFFFFFFE.
        for i in 0..6 {
            g.set_u32(counter::COUNTER + 4 * i, 0xFFFF_FFFF).unwrap();
        }
        // range 1: quotient = low word, product = quotient * 1 sign-extended, so the 64-bit
        // subtract leaves the draw's high half plus the sign-extension in r3.
        let r3 = op_random_in_range(&mut g, BLOCK).unwrap();
        assert!(
            r3 > u32::MAX as u64,
            "the high half survives into r3: {r3:#x}"
        );
        assert_eq!(
            g.u32(BLOCK + 8).unwrap(),
            r3 as u32,
            "memory keeps the low word only"
        );
    }

    #[test]
    fn a_zero_range_divides_to_zero_instead_of_faulting() {
        let mut g = block_guest();
        put_rodata(&mut g);
        put_words(&mut g, &[500, 0, 0, 1]); // range 0
        g.set_u32(counter::COUNTER + 20, 41).unwrap();
        // RexGlue's divwu by zero is 0, so the multiply-back is 0 too and the draw passes through
        // undivided: base + draw. With only +20 seeded, the cascade leaves the draw at 41.
        let r3 = op_random_in_range(&mut g, BLOCK).unwrap();
        assert_eq!(r3, 500 + 41, "the draw passes through undivided");
        assert_eq!(g.u32(BLOCK + 8).unwrap(), 541);
    }

    #[test]
    fn the_weighted_cursor_picks_the_bucket_the_remainder_falls_in() {
        let mut g = block_guest();
        put_rodata(&mut g);
        // The weight table lives 16 bytes into the source record; put the source at BLOCK + 0x80
        // so the table is at BLOCK + 0x90.
        let source = BLOCK + 0x80;
        g.set_u32(BLOCK, source).unwrap();
        g.set_u32(BLOCK + 4, 500).unwrap(); // base_index
        g.set_u32(BLOCK + 8, 3).unwrap(); // three weights
        g.set_u32(BLOCK + 12, 0xDEAD).unwrap(); // the stale result
        g.set_u32(BLOCK + 16, 0).unwrap(); // disabled

        for (i, w) in [30u8, 30, 40].iter().enumerate() {
            g.set_u8(source + 16 + i as u32, *w).unwrap();
        }

        assert_eq!(
            op_weighted_cursor(&mut g, BLOCK).unwrap(),
            0xDEAD,
            "disabled: read back"
        );
        assert_eq!(g.u32(counter::COUNTER + 20).unwrap(), 0, "and no draw");

        // Enabled. From a zeroed counter the draw is 0, so the remainder is 0 and the first
        // weight (30 > 0) wins immediately: index 0, published as base + 0.
        g.set_u32(BLOCK + 16, 1).unwrap();
        assert_eq!(op_weighted_cursor(&mut g, BLOCK).unwrap(), 500);
        assert_eq!(g.u32(BLOCK + 12).unwrap(), 500);

        // Drive the counter to a value whose remainder mod 100 lands in the second bucket.
        g.set_u32(counter::COUNTER + 20, 0).unwrap();
        for off in [0u32, 4, 8, 12, 16] {
            g.set_u32(counter::COUNTER + off, 0).unwrap();
        }
        g.set_u32(counter::COUNTER, 145).unwrap();
        // The draw is w0 + (cascade of zeros) = 145, so the remainder is 45: 30 is not > 45,
        // 60 is, so index 1 wins.
        assert_eq!(op_weighted_cursor(&mut g, BLOCK).unwrap(), 501);
    }

    #[test]
    fn a_negative_weight_ends_the_walk_because_the_compare_is_unsigned() {
        let mut g = block_guest();
        put_rodata(&mut g);
        let source = BLOCK + 0x80;
        g.set_u32(BLOCK, source).unwrap();
        g.set_u32(BLOCK + 4, 0).unwrap();
        g.set_u32(BLOCK + 8, 3).unwrap();
        g.set_u32(BLOCK + 16, 1).unwrap();
        // A weight of -1 sums to -1, whose low word 0xFFFFFFFF compares UNSIGNED above any
        // remainder, so the walk stops on the first step with index 0.
        g.set_u8(source + 16, (-1i8) as u8).unwrap();
        g.set_u8(source + 17, 50).unwrap();
        g.set_u8(source + 18, 50).unwrap();
        assert_eq!(op_weighted_cursor(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.u32(BLOCK + 12).unwrap(), 0);
    }

    #[test]
    fn a_weight_table_that_never_passes_the_remainder_leaves_the_result_alone() {
        let mut g = block_guest();
        put_rodata(&mut g);
        let source = BLOCK + 0x80;
        g.set_u32(BLOCK, source).unwrap();
        g.set_u32(BLOCK + 8, 2).unwrap();
        g.set_u32(BLOCK + 12, 0x1234).unwrap();
        g.set_u32(BLOCK + 16, 1).unwrap();
        g.set_u32(counter::COUNTER, 99).unwrap(); // remainder 99
        g.set_u8(source + 16, 1).unwrap();
        g.set_u8(source + 17, 1).unwrap(); // total 2, never above 99
        assert_eq!(
            op_weighted_cursor(&mut g, BLOCK).unwrap(),
            0x1234,
            "the stale result"
        );
    }

    /// A halfword shuffle bag over `count` elements, with the gate word live.
    fn shuffle_bag(g: &mut Guest, count: u16, elements: &[u16]) {
        g.set_u16(BLOCK, 0x40).unwrap(); // the gate word lives at BLOCK + 0x40
        g.set_u32(BLOCK + 0x40, 1).unwrap(); // live
        g.set_u8(BLOCK + 2, 2).unwrap(); // halfword elements
        g.set_u8(BLOCK + 3, 0).unwrap(); // wrap flag clear
        g.set_u32(BLOCK + 4, 1000).unwrap(); // base
        g.set_u16(BLOCK + 8, 0).unwrap(); // cursor
        g.set_u16(BLOCK + 10, count).unwrap();
        g.set_u32(BLOCK + 12, 0).unwrap();
        for (i, e) in elements.iter().enumerate() {
            g.set_u16(BLOCK + 16 + 2 * i as u32, *e).unwrap();
        }
    }

    fn bag_elements(g: &Guest, count: u16) -> Vec<u16> {
        (0..count)
            .map(|i| g.u16(BLOCK + 16 + 2 * i as u32).unwrap())
            .collect()
    }

    #[test]
    fn an_inert_shuffle_bag_hands_back_its_cached_result() {
        let mut g = block_guest();
        put_rodata(&mut g);
        shuffle_bag(&mut g, 4, &[10, 20, 30, 40]);
        g.set_u32(BLOCK + 0x40, 0).unwrap(); // the gate word reads zero
        g.set_u32(BLOCK + 12, 0x1234).unwrap();

        assert_eq!(op_shuffle_bag(&mut g, BLOCK).unwrap(), 0x1234);
        assert_eq!(g.u32(counter::COUNTER + 20).unwrap(), 0, "no draw was made");
        assert_eq!(g.u16(BLOCK + 8).unwrap(), 0, "and the cursor did not move");
    }

    #[test]
    fn the_shuffle_bag_permutes_rather_than_losing_elements() {
        let mut g = block_guest();
        put_rodata(&mut g);
        let original = [11u16, 22, 33, 44, 55];
        shuffle_bag(&mut g, 5, &original);

        // Five draws consume the bag exactly once each, and the array stays a permutation of
        // itself throughout: the algorithm swaps, it does not overwrite.
        let mut drawn = Vec::new();
        let mut last = 0u64;
        for i in 0..5 {
            let r = op_shuffle_bag(&mut g, BLOCK).unwrap();
            assert!(r >= 1000, "base is added: {r}");
            last = r;
            drawn.push((r - 1000) as u16);
            let mut present = bag_elements(&g, 5);
            present.sort_unstable();
            let mut expect = original;
            expect.sort_unstable();
            assert_eq!(
                present,
                expect.to_vec(),
                "still a permutation after draw {i}"
            );
        }
        let mut sorted = drawn.clone();
        sorted.sort_unstable();
        let mut expect = original;
        expect.sort_unstable();
        assert_eq!(
            sorted,
            expect.to_vec(),
            "each element was drawn exactly once"
        );

        // The fifth draw wrapped: the cursor is back at 0 and the flag is set.
        assert_eq!(g.u16(BLOCK + 8).unwrap(), 0, "wrapped");
        assert_eq!(g.u8(BLOCK + 3).unwrap(), 1, "and recorded the wrap");
        // +12 ends up holding base + element, not the bare element it briefly held mid-swap.
        assert_eq!(g.u32(BLOCK + 12).unwrap() as u64, last);
    }

    #[test]
    fn the_wrap_flag_shortens_the_next_range_by_one() {
        // The flag does double duty. With it set and the cursor at 0, the range is count - 1, so
        // the last element can never be picked on that draw — the bag deals count-1 of its
        // elements before the flag is cleared again.
        let mut g = block_guest();
        put_rodata(&mut g);
        shuffle_bag(&mut g, 2, &[7, 9]);
        g.set_u8(BLOCK + 3, 1).unwrap();
        // range = 2 - 1 - 0 = 1, so the draw is forced to slot 0 whatever the counter says.
        g.set_u32(counter::COUNTER, 12345).unwrap();
        assert_eq!(op_shuffle_bag(&mut g, BLOCK).unwrap(), 1007);
        assert_eq!(
            bag_elements(&g, 2),
            vec![7, 9],
            "a self-swap leaves the array alone"
        );
        assert_eq!(g.u16(BLOCK + 8).unwrap(), 1, "the cursor advanced");
        assert_eq!(g.u8(BLOCK + 3).unwrap(), 0, "and the flag was cleared");
    }

    #[test]
    fn a_byte_wide_shuffle_bag_uses_the_narrow_element_path() {
        let mut g = block_guest();
        put_rodata(&mut g);
        shuffle_bag(&mut g, 3, &[0, 0, 0]);
        g.set_u8(BLOCK + 2, 1).unwrap(); // byte elements
        for (i, e) in [5u8, 6, 7].iter().enumerate() {
            g.set_u8(BLOCK + 16 + i as u32, *e).unwrap();
        }
        let mut drawn = Vec::new();
        for _ in 0..3 {
            drawn.push((op_shuffle_bag(&mut g, BLOCK).unwrap() - 1000) as u8);
        }
        drawn.sort_unstable();
        assert_eq!(
            drawn,
            vec![5, 6, 7],
            "each byte element was drawn exactly once"
        );
    }

    #[test]
    fn the_delay_ring_pushes_and_pops_with_the_derived_offset() {
        let mut g = block_guest();
        put_rodata(&mut g);
        // G is the divisor for the delay, so G = 1 makes the offset the delay itself.
        g.set_u32(TICK_SCALE_GLOBAL, 1.0f32.to_bits()).unwrap();

        let source = BLOCK + 0x80;
        g.set_u16(BLOCK, 0x80).unwrap(); // the source record's offset
        g.set_u16(BLOCK + 2, 4).unwrap(); // capacity 4
        g.set_u16(BLOCK + 4, 0).unwrap(); // write
        g.set_u16(BLOCK + 6, 0).unwrap(); // read
        g.set_u32(BLOCK + 8, 0xFFFF_FFFF).unwrap(); // last_delay, forcing a re-derive
        g.set_u32(source + 4, 2).unwrap(); // delay 2

        // Re-derive: offset = round(2/1) = 2, so write = read + 2 = 2. Push word, pop slot 0.
        g.set_u32(source, 0xAA).unwrap();
        assert_eq!(
            op_delay_ring(&mut g, BLOCK).unwrap(),
            0,
            "slot 0 is still empty"
        );
        assert_eq!(g.u32(BLOCK + 8).unwrap(), 2, "the delay was latched");
        assert_eq!(g.u32(BLOCK + 12 + 4 * 2).unwrap(), 0xAA, "pushed at slot 2");
        assert_eq!(g.u16(BLOCK + 4).unwrap(), 3, "write advanced");
        assert_eq!(g.u16(BLOCK + 6).unwrap(), 1, "read advanced");

        // Two more pushes with the delay unchanged, then the first word comes out.
        g.set_u32(source, 0xBB).unwrap();
        assert_eq!(op_delay_ring(&mut g, BLOCK).unwrap(), 0);
        g.set_u32(source, 0xCC).unwrap();
        assert_eq!(
            op_delay_ring(&mut g, BLOCK).unwrap(),
            0xAA,
            "two calls of delay"
        );
    }

    #[test]
    fn a_negative_delay_is_clamped_in_the_source_record_itself() {
        let mut g = block_guest();
        put_rodata(&mut g);
        g.set_u32(TICK_SCALE_GLOBAL, 1.0f32.to_bits()).unwrap();
        let source = BLOCK + 0x80;
        g.set_u16(BLOCK, 0x80).unwrap();
        g.set_u16(BLOCK + 2, 4).unwrap();
        g.set_u32(BLOCK + 8, 5).unwrap(); // last_delay, different from the source
        g.set_u32(source + 4, (-3i32) as u32).unwrap();

        op_delay_ring(&mut g, BLOCK).unwrap();
        assert_eq!(
            g.u32(source + 4).unwrap(),
            0,
            "clamped in the caller's own record"
        );
        // +8 latched the value seen BEFORE the clamp, which is the negative one.
        assert_eq!(
            g.u32(BLOCK + 8).unwrap(),
            (-3i32) as u32,
            "latched before the clamp"
        );
        // offset = round(0/1) = 0, so write = read = 0 and the push and pop hit the same slot.
        assert_eq!(g.u16(BLOCK + 4).unwrap(), 1);
        assert_eq!(g.u16(BLOCK + 6).unwrap(), 1);
    }

    #[test]
    fn the_delay_offset_is_clamped_to_the_last_slot() {
        let mut g = block_guest();
        put_rodata(&mut g);
        g.set_u32(TICK_SCALE_GLOBAL, 1.0f32.to_bits()).unwrap();
        let source = BLOCK + 0x80;
        g.set_u16(BLOCK, 0x80).unwrap();
        g.set_u16(BLOCK + 2, 4).unwrap(); // capacity 4
        g.set_u16(BLOCK + 6, 0).unwrap();
        g.set_u32(BLOCK + 8, 0).unwrap();
        g.set_u32(source + 4, 1000).unwrap(); // a delay far past the ring

        op_delay_ring(&mut g, BLOCK).unwrap();
        // offset clamps to capacity - 1 == 3, so write = 0 + 3 = 3, then advances to 4.
        assert_eq!(g.u16(BLOCK + 4).unwrap(), 4);
        assert_eq!(g.u32(BLOCK + 12 + 4 * 3).unwrap(), g.u32(source).unwrap());
    }
}
