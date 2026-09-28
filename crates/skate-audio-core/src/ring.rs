//! The decode ring: cutting one block of samples out of it, and writing one block back into it.
//!
//! Four verified bodies that sit directly on top of one another. `sub_82B3DD90` — which is
//! **not** here, see below — builds a 16-byte ring window on its frame and a one- or two-entry
//! segment array, hands both to [`fill_segments`], and then calls [`fill_tail`] to pad whatever
//! the ring could not supply. [`fill_segments`] in turn calls [`copy_from_ring`] once per segment.
//! [`write_into_ring`] is the other direction: the producer side, which places a finished block back
//! into the same wrapping ring.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`copy_from_ring`] | `sub_82B3DB90` | verified | 109 | 940,743 | 1,306,629 |
//! | [`fill_segments`] | `sub_82B3DC48` | verified | 184 | 533,208 | 759,184 |
//! | [`fill_tail`] | `sub_82B3DF90` | verified | 242 | 533,208 | 759,184 |
//! | [`write_into_ring`] | `sub_82B3DEA8` | verified | 132 | 266,604 | 379,592 |
//!
//! They share a module because they share a structure and a call edge, not because they do similar
//! things: all four take the same stream object in `r3`, and the object's `+20`/`+24` pair is
//! `copy_from_ring`'s wrap span, `fill_segments`'s `kObjSpanHigh`/`kObjSpanLow`, `fill_tail`'s
//! `kEnd` and `write_into_ring`'s divisor and lag. Porting them apart would have meant naming those
//! cells four times and hoping the four readings agreed. Porting them together makes
//! `fill_segments`'s call to `copy_from_ring` a real call rather than a stub, which is the only part
//! of that body a test can actually drive.
//!
//! **[`copy_from_ring`] and [`write_into_ring`] are twins that do not share one line of code**, and
//! that is deliberate. Both end in two `memcpy`s, one reaching the ring's end and one from its base,
//! and both wrap a cursor by the object's `(high - low)` words. But the read side measures its source
//! *backwards* from a cursor held in the ring descriptor, and the write side computes its cursor
//! *forwards* from a word position held in the object at `+52` through a `divw`. The two derivations
//! share no intermediate, so factoring them would have meant inventing a common form neither
//! original has.
//!
//! **`sub_82B3DD90`, their caller, is deliberately absent.** `docs/ports.md` records it
//! `gate-2`: `sub_82B3DF90`'s store bases are the segment output pointers `sub_82B3DC48` produces
//! *during* the call, so its write set does not derive from entry state and the harness cannot
//! bracket it. There is no verified reference for it in either language.
//!
//! ## What the green here means
//!
//! These are **unit-tested against a verified reference** — the crate README's second kind of
//! green. Each C++ body was compared call-for-call against the original under the shadow harness at
//! zero divergence, on the real inputs the call counts above come from. The Rust has no recorded
//! vectors of its own yet. A fault here is a transcription error rather than a misreading of the
//! engine, which is a much smaller search space; it is not a number.
//!
//! ## Nothing in `docs/rw_audio_structs.h` names any of this
//!
//! Every offset below is the raw offset plus the use the lifted body makes of it. No field name
//! here is recovered from RTTI or from plug-in metadata, and none should be read as one.
//!
//! ## Two things reproduced rather than fixed
//!
//! - [`copy_from_ring`] re-reads the ring base **after** its first copy, so a copy that lands on
//!   the ring descriptor feeds the second copy the changed value. The original does; the C++ note
//!   calls it out; it is kept.
//! - [`fill_tail`] reloads the buffer pointer out of the record before **every** store, and reloads
//!   the object's three control fields between its two fills. A fill that aliased either would
//!   change its own target mid-loop. Kept for the same reason.
//!
//! ## One divergence, in the direction of not hanging
//!
//! A length that runs off the guest map is an `Err` here, where the original would run a ~4 GB
//! `memcpy` or walk a fill loop into unmapped memory. The C++ `Windows()` builders refuse exactly
//! that class of input, so nothing is known about what the guest does with it either — an `Err`
//! resolves an unknown rather than contradicting a known. See [`crate::mem`].

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp, mem};

// ------------------------------------------------------------------------------------ the ring

/// `sub_82B3DB90`'s ring descriptor, in `r4`; `sub_82B3DC48`'s 16-byte window, in `r4`.
pub const RING_BASE: u32 = 0;
/// One past the ring's last byte.
pub const RING_END: u32 = 4;
/// The cursor the read is measured *back* from.
pub const RING_CURSOR: u32 = 12;
/// `sub_82B3DD90` builds this on its own frame, so it is 16 bytes and nothing more.
pub const RING_WINDOW_BYTES: u32 = 16;

// --------------------------------------------------------------------------- the stream object

/// The destination buffer the segments are cut from. Read by `sub_82B3DC48` only.
pub const OBJ_DEST: u32 = 8;
/// `fill_tail`'s cap contributor, added to 127 / 255.
pub const OBJ_BLOCK: u32 = 16;
/// The high half of the wrap span, and the end `fill_tail` compares its cursor against.
pub const OBJ_SPAN_HIGH: u32 = 20;
/// The low half of the wrap span. `(high - low)` words is how far a wrapped read moves forward.
pub const OBJ_SPAN_LOW: u32 = 24;
/// `fill_tail`'s cursor.
pub const OBJ_CURSOR: u32 = 36;
/// Buffer 0's fill length counts up to here.
pub const OBJ_LIMIT0: u32 = 40;
/// Buffer 1's fill length counts up to here.
pub const OBJ_LIMIT1: u32 = 44;
/// The running **word position** a written block is placed from. `lwz r7,52(r3)`, read by
/// [`write_into_ring`] only; it is that body's `divw` dividend and the source of its lag.
pub const OBJ_POSITION: u32 = 52;

/// `sub_82B3DF90`'s `kEnd` and `sub_82B3DB90`'s `kSpanHigh` are the same word, read for two
/// different purposes by two functions that take the same object.
pub const OBJ_END: u32 = OBJ_SPAN_HIGH;

// --------------------------------------------------------------------------- the segment array

/// Words wanted. Also the key the two segments are ranked by.
pub const SEG_NEED: u32 = 0;
/// The cap `sub_82B3DD90` derived from the object's block.
pub const SEG_CAP: u32 = 4;
/// Which segment this slot selects; written by [`fill_segments`].
pub const SEG_RANK: u32 = 8;
/// Where this segment's words landed; written by [`fill_segments`].
pub const SEG_OUT: u32 = 12;
/// 16 bytes per segment.
pub const SEG_STRIDE: u32 = 16;
/// `sub_82B3DD90`, the only caller, passes 1 or 2.
pub const MAX_SEGMENTS: u32 = 2;

// --------------------------------------------------------------------------- the buffer record

/// The first float buffer, in `sub_82B3DF90`'s `r5`.
pub const BUFFER0: u32 = 4;
/// The second float buffer. May be null, which ends the call.
pub const BUFFER1: u32 = 8;

const LIS_82160000: u32 = ((-32234i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82160000 == 0x8216_0000, "lis r6,-32234");

/// `lis r6,-32234 ; lfs f0,23056(r6)` — the single [`fill_tail`] fills with, read **live**.
///
/// The address is `((imm & 0xFFFF) << 16) + offset`, **computed** above and asserted below rather
/// than read off by eye: one misread digit in `sub_82B2FE00` cost this project its first shadow
/// divergence.
///
/// This is the same cell [`crate::eval::ZERO_SINGLE`] names, whose dump value is `0.0f`, and the
/// same one `sub_82B31838` and `sub_82B3C668` load. Nothing is assumed about its value: the body
/// loads it through the guest map, as the original does, so a patched image reaches the port.
pub const FILL_CONSTANT_CELL: u32 = LIS_82160000 + 23056;
const _: () = assert!(FILL_CONSTANT_CELL == 0x8216_5A10, "lis -32234 ; lfs 23056");
const _: () = assert!(
    FILL_CONSTANT_CELL == crate::eval::ZERO_SINGLE,
    "the pool's 0.0f cell"
);
const _: () = assert!(SEG_STRIDE == 1 << 4, "rlwinm r11,r11,4,0,27");
const _: () = assert!(
    OBJ_SPAN_LOW == OBJ_SPAN_HIGH + 4,
    "lwz r11,20(r3) ; lwz r10,24(r3)"
);
const _: () = assert!(
    OBJ_LIMIT1 == OBJ_LIMIT0 + 4,
    "lwz r9,40(r3) ; lwz r11,44(r3)"
);
const _: () = assert!(BUFFER1 == BUFFER0 + 4, "lwz 4(r5) ; lwz 8(r5)");

// ------------------------------------------------------------------------------ shared idioms

/// `rlwinm rN,rX,2,0,29` — the **low word** scaled by four, zero-extended back to 64 bits.
///
/// Not `value * 4`: everything above bit 31 is dropped before the shift, and the bottom two bits
/// of the result are cleared. Both matter for a span that has wrapped.
fn words_to_bytes(value: u64) -> u64 {
    ((value as u32 as u64) << 2) & 0xFFFF_FFFC
}

/// `addi rN,rX,31 ; rlwinm rN,rN,0,0,26` — a word count rounded up to a multiple of 32, on the low
/// word.
fn round_up_32(value: u64) -> u64 {
    ((value as u32).wrapping_add(31) & 0xFFFF_FFE0) as u64
}

// ------------------------------------------------------------------- sub_82B3DB90: the two runs

/// Everything the two copies need, all of it derivable from entry state.
///
/// Split out for the same reason the C++ splits it: the body and the harness's window builder both
/// have to compute it, and a second copy of this arithmetic is a second thing to get wrong.
#[derive(Clone, Copy, Debug, Default)]
struct Plan {
    /// `r7 == 0`: nothing is copied and zero is returned.
    empty: bool,
    /// `r31` — the clamped word count, and the return value. All 64 bits.
    want: u64,
    /// `r4` at the first copy.
    src: u64,
    /// `r30` — the first copy's length in bytes.
    run_bytes: u64,
    /// `r5` at the second copy.
    rest_bytes: u64,
}

fn make_plan(g: &Guest, object: u64, ring: u32, limit: u64, want_in: u64) -> Result<Plan> {
    // cmpwi cr6,r7,0 ; bne cr6 — a zero request copies nothing. The test is on the low word.
    if want_in as u32 as i32 == 0 {
        return Ok(Plan {
            empty: true,
            ..Plan::default()
        });
    }
    // cmpw cr6,r31,r6 ; blt cr6 ; mr r31,r6 — clamp the request to the window. The compare is
    // signed on the low words and the `mr` keeps whichever register's full 64 bits.
    let want = if (want_in as u32 as i32) < (limit as u32 as i32) {
        want_in
    } else {
        limit
    };

    let cursor = g.u32(ring.wrapping_add(RING_CURSOR))?; // lwz r11,12(r28)
    let behind = words_to_bytes(limit); // rlwinm r10,r6,2,0,29
    let ring_base = g.u32(ring.wrapping_add(RING_BASE))?; // lwz r9,0(r28)
    // subf r4,r10,r11 is 64-bit: a borrow leaves the high word set, and that value is what the
    // wrap add below and the copy's source register carry on with. Only the compares truncate.
    let mut src = (cursor as u64).wrapping_sub(behind);

    // cmplw cr6,r4,r9 ; blt cr6 — below the base wraps; otherwise the end has to be above it.
    let mut inside = false;
    if !((src as u32) < ring_base) {
        let ring_end = g.u32(ring.wrapping_add(RING_END))?; // lwz r11,4(r28)
        inside = ring_end > src as u32; // cmplw cr6,r11,r4 ; bgt cr6
    }
    if !inside {
        let object = object as u32;
        let high = g.u32(object.wrapping_add(OBJ_SPAN_HIGH))?; // lwz r11,20(r3)
        let low = g.u32(object.wrapping_add(OBJ_SPAN_LOW))?; // lwz r10,24(r3)
        // subf r9,r10,r11 ; rlwinm r11,r9,2,0,29 ; add r4,r11,r4 — all 64-bit but for the rlwinm.
        src = src.wrapping_add(words_to_bytes((high as u64).wrapping_sub(low as u64)));
    }

    let ring_end = g.u32(ring.wrapping_add(RING_END))?; // lwz r11,4(r28) — read a second time
    // subf r10,r4,r11 ; srawi r29,r10,2 — words left to the end, on the low word, signed and
    // sign-extended back to 64 bits.
    let left = (ring_end as u64).wrapping_sub(src) as u32;
    let mut run = (((left as i32) >> 2) as i64) as u64;
    // cmpw cr6,r31,r29 ; blt cr6 ; mr r29,r31 — min, signed on the low words.
    if (want as u32 as i32) < (run as u32 as i32) {
        run = want;
    }
    Ok(Plan {
        empty: false,
        want,
        src,
        run_bytes: words_to_bytes(run), // rlwinm r30,r29,2,0,29
        rest_bytes: words_to_bytes(want.wrapping_sub(run)), // subf r11,r29,r31 ; rlwinm r5
    })
}

/// `sub_82B3DB90` — copy up to `want` words out of a wrapping ring, in two runs, and return how
/// many words the call settled on.
///
/// Arguments, by register: `object` is `r3` (the stream object, read for its `+20`/`+24` wrap
/// span), `ring` is `r4` (the descriptor), `dst` is `r5`, `limit` is `r6` (the window the request
/// is clamped to, and the distance the source is measured back from the cursor) and `want` is `r7`
/// (the request). All six are full 64-bit registers because the caller's chain depends on the high
/// halves: `r5` carries a carry into bit 32 from `run_bytes + dst`, `r7` can arrive with all 64
/// bits set from `sub_82B3DC48`'s branch-free max, and the return is the guest's full `r3`.
///
/// **Writes** exactly the two destination runs — `[dst, dst + run_bytes)` and
/// `[dst + run_bytes, + rest_bytes)` — and nothing else. This function has no stores of its own;
/// both spans are written by `sub_82EDF460`. Reads the ring descriptor's `+0`/`+4`/`+12`, the
/// object's `+20`/`+24`, and the two source runs.
///
/// **The ring base is re-read after the first copy** rather than kept from the plan, so a copy
/// that lands on the ring descriptor feeds the second copy the value it just wrote. That is the
/// original's ordering and it is reproduced.
pub fn copy_from_ring(
    g: &mut Guest,
    object: u64,
    ring: u64,
    dst: u64,
    limit: u64,
    want: u64,
) -> Result<u64> {
    let ring = ring as u32; // mr r28,r4 — the descriptor is addressed through the low word
    let plan = make_plan(g, object, ring, limit, want)?;
    if plan.empty {
        return Ok(0); // li r3,0
    }
    // bl 0x82edf460 — the run that reaches the end of the ring.
    mem::memcpy(g, dst as u32, plan.src as u32, plan.run_bytes)?;
    // add r3,r30,r27 — 64-bit on two zero-extended words, so a carry into bit 32 is kept.
    let tail = plan.run_bytes.wrapping_add(dst);
    let wrapped = g.u32(ring.wrapping_add(RING_BASE))?; // lwz r4,0(r28), after the copy
    // bl 0x82edf460 — the remainder, from the base. Length zero when the run covered it all.
    mem::memcpy(g, tail as u32, wrapped, plan.rest_bytes)?;
    Ok(plan.want) // mr r3,r31
}

// ------------------------------------------------------------- sub_82B3DC48: rank, then fill

/// One trip of the segment loop, up to the call.
#[derive(Clone, Copy, Debug, Default)]
struct Step {
    /// `r11` — `&segments[rank]`.
    entry: u32,
    /// The word stored at `entry + SEG_OUT`.
    out: u64,
    /// `r6` at the call to [`copy_from_ring`].
    limit: u64,
    /// `r7` at the call.
    want: u64,
    /// `r29`.
    need_up: u64,
    /// `cmpw cr6,r29,r31 ; bgt` — the second branch.
    over: bool,
}

fn make_step(g: &Guest, segments_base: u32, rank: u32, dest: u64, running: u64) -> Result<Step> {
    // rlwinm r11,r11,4,0,27 ; add r11,r11,r26 — 64-bit add, but the loads use the low word.
    let entry = ((rank << 4) & 0xFFFF_FFF0).wrapping_add(segments_base);
    let need = g.u32(entry.wrapping_add(SEG_NEED))? as u64; // lwz r10,0(r11)
    let cap = g.u32(entry.wrapping_add(SEG_CAP))? as u64; // lwz r9,4(r11)
    let need_up = round_up_32(need); // addi r8,r10,31 ; rlwinm r29
    let pad = need_up.wrapping_sub(need); // subf r10,r10,r29
    // cmpw cr6,r29,r31 — signed on the low words.
    let over = (need_up as u32 as i32) > (running as u32 as i32);
    // add r9,r9,r10 ; addi r7,r9,31 ; rlwinm r7,r7,0,0,26
    let cap_up = round_up_32(cap.wrapping_add(pad));
    let slack = need_up.wrapping_sub(cap_up); // subf r9,r7,r29

    let (out, limit, want) = if !over {
        // rlwinm r10,r10,2,0,29 ; add r10,r10,r30 ; mr r6,r29 ; r7 still holds roundup32(cap + pad)
        (words_to_bytes(pad).wrapping_add(dest), need_up, cap_up)
    } else {
        let behind = pad.wrapping_sub(need_up); // subf r10,r29,r10
        let left = running.wrapping_sub(slack); // subf r8,r9,r31 ; subf r3,r24,r8
        // add r7,r10,r31 ; rlwinm r9,r7,2,0,29 ; add r4,r9,r30
        let out = words_to_bytes(behind.wrapping_add(running)).wrapping_add(dest);
        // xoris r5,r24,32768 ; addc r11,r3,r5 ; subfe r9,r24,r24 ; and r7,r9,r8 — the branch-free
        // max(left, 0). The carry out of `left + 0x80000000` is set exactly when left's low word is
        // negative, and `subfe` then builds 0 or all-ones from it. All 64 bits of `left` survive.
        let carry = (left as u32).wrapping_add(0x8000_0000) < (left as u32);
        let want = (if carry { 0u64 } else { !0u64 }) & left;
        (out, running, want) // mr r6,r31
    };
    Ok(Step {
        entry,
        out,
        limit,
        want,
        need_up,
        over,
    })
}

/// `sub_82B3DC48` — rank the output segments by size, then fill each one out of the ring. Returns
/// the destination cursor the last copy left behind, as the guest's full `r3`.
///
/// Arguments, by register: `object` is `r3`, `window` is `r4` (the 16-byte ring descriptor),
/// `segments_base` is `r5` (the 16-byte-per-entry segment array) and `segments` is `r6`, the entry
/// count — `sub_82B3DD90`, the only caller, passes 1 or 2. `object`, `window` and `segments` are
/// full 64-bit registers: the first two are handed straight to [`copy_from_ring`], and `segments`
/// is decremented as a 64-bit value whose *low word* ends the loop.
///
/// **Writes** `segments_base + 8` on every path, `segments_base + 24 + 8` when there are two
/// segments, one output word per segment at `entry + 12`, and — through [`copy_from_ring`] — the
/// destination run of each copy. Reads the object's `+8`/`+20`/`+24`, the window, and the segment
/// array.
///
/// **The rank word is reloaded from the slot** each trip rather than kept from the ranking above,
/// because the copy of the previous trip could have landed on it. So is the object's destination,
/// which is read once before the loop and then advanced in a register — that part is the
/// original's too.
///
/// The unconditional `stw` of zero into slot 0's rank happens **before** the two-segment ranking,
/// and the ranking then writes slot 0 again on both of its branches. Three stores to the same word
/// where one would do; reproduced, because a caller whose segment array aliases the object would
/// see all three.
pub fn fill_segments(
    g: &mut Guest,
    object: u64,
    window: u64,
    segments_base: u32,
    segments: u64,
) -> Result<u64> {
    let count = segments as u32 as i32; // cmpwi cr6,r6,2 and ble cr6 both read the low word

    g.set_u32(segments_base.wrapping_add(SEG_RANK), 0)?; // stw r24,8(r5) — r24 is zero throughout
    // cmpwi cr6,r6,2 ; bne cr6 — two segments get ranked, the larger first.
    if count == 2 {
        let first = g.u32(segments_base.wrapping_add(SEG_NEED))?; // lwz r10,0(r5)
        let second = g.u32(segments_base.wrapping_add(SEG_STRIDE + SEG_NEED))?; // lwz r9,16(r5)
        // cmpw cr6,r10,r9 ; bge cr6 — signed on the low words.
        if (first as i32) < (second as i32) {
            g.set_u32(segments_base.wrapping_add(SEG_STRIDE + SEG_RANK), 0)?; // stw r24,24(r5)
            g.set_u32(segments_base.wrapping_add(SEG_RANK), 1)?; // stw r10,0(r11)
        } else {
            g.set_u32(segments_base.wrapping_add(SEG_RANK), 0)?; // stw r24,0(r11) — zero again
            g.set_u32(segments_base.wrapping_add(SEG_STRIDE + SEG_RANK), 1)?; // stw r10,24(r26)
        }
    }

    let lead = g.u32(segments_base.wrapping_add(SEG_RANK))?; // lwz r10,0(r11)
    let mut dest = g.u32((object as u32).wrapping_add(OBJ_DEST))? as u64; // lwz r30,8(r28)
    // rlwinm r9,r10,4,0,27 ; lwzx r10,r9,r26 ; addi r8,r10,31 ; rlwinm r31,r8,0,0,26
    let mut running =
        round_up_32(g.u32(((lead << 4) & 0xFFFF_FFF0).wrapping_add(segments_base))? as u64);
    if count <= 0 {
        // ble cr6,0x82b3dd80 — cr6 is still the cmpwi against zero. Nothing but the unconditional
        // rank store (and, at two segments, the ranking) has happened.
        return Ok(dest); // mr r3,r30
    }

    let mut rank_slot = segments_base.wrapping_add(SEG_RANK); // mr r25,r11
    let mut left = segments; // mr r23,r6 — the full 64-bit register
    loop {
        let rank = g.u32(rank_slot)?; // lwz r11,0(r25)
        let step = make_step(g, segments_base, rank, dest, running)?;
        // stw r10,12(r11) on the first branch, stw r4,12(r11) on the second — both before the call.
        g.set_u32(step.entry.wrapping_add(SEG_OUT), step.out as u32)?;
        // bl 0x82b3db90 — (object, window, dest, limit, want).
        let copied = copy_from_ring(g, object, window, dest, step.limit, step.want)?;
        // subf r31,r3,r29 on the first branch, add r31,r3,r31 on the second.
        running = if step.over {
            copied.wrapping_add(running)
        } else {
            step.need_up.wrapping_sub(copied)
        };
        dest = words_to_bytes(copied).wrapping_add(dest); // rlwinm r11,r3,2,0,29 ; add r30,r11,r30
        left = (left as i64).wrapping_sub(1) as u64; // addic. r23,r23,-1
        rank_slot = rank_slot.wrapping_add(SEG_STRIDE); // addi r25,r25,16
        if left as u32 as i32 == 0 {
            break; // bne 0x82b3dcc4 — the condition register reads the low word
        }
    }
    Ok(dest) // mr r3,r30
}

// ----------------------------------------------------------------- sub_82B3DF90: the constant fill

/// `subfic r10,rN,0 ; rlwinm r9,rN,1,31,31 ; addme r10,r9 ; and r8,r10,rN` — the branch-free
/// `max(n, 0)`.
///
/// `r10` is all-ones only when `n > 0` (no borrow out of the negation, and the sign bit clear), so
/// the AND keeps `n` exactly then and yields 0 otherwise. Only the low 32 bits are consumed
/// afterwards, which is why this returns a `u32`.
fn clamp_non_negative(n: i32) -> u32 {
    if n > 0 { n as u32 } else { 0 }
}

/// How many floats buffer 0 gets. All of it 32-bit wrap arithmetic.
fn count0(limit0: u32, cursor: u32, block: u32, consumed: u32) -> u32 {
    // subf r8,r11,r9 ; subf r11,r4,r8
    let mut n = limit0.wrapping_sub(cursor).wrapping_sub(consumed) as i32;
    let cap = block.wrapping_add(255) as i32; // addi r10,r10,255
    if !(n < cap) {
        n = cap; // cmpw cr6,r11,r10 ; blt cr6 ; mr r11,r10
    }
    clamp_non_negative(n)
}

/// How many floats buffer 1 gets: capped at `block + 127`, then — as the original really does —
/// tested again against `block + 255`.
///
/// The second test is dead. It only fires when the value is already at or above `block + 255`, and
/// the first cap has just put it at `block + 127` or below. `the_two_buffers_are_capped_at_
/// different_lengths` measures that rather than arguing it. Both are written out because the
/// original emits both, and because "dead" is a claim about the arithmetic that a reader should be
/// able to re-check against the instructions.
///
/// The two subtractions are also written in the lifted order, `(limit1 - consumed) - cursor`,
/// which is not the order [`count0`] uses. On wrapping 32-bit arithmetic the two agree for every
/// input; they are kept apart so each reads as its own instruction sequence.
fn count1(limit1: u32, cursor: u32, block: u32, consumed: u32) -> u32 {
    // subf r8,r4,r11 ; subf r11,r10,r8
    let mut n = limit1.wrapping_sub(consumed).wrapping_sub(cursor) as i32;
    let mut cap = block.wrapping_add(127) as i32; // addi r10,r9,127
    if !(n < cap) {
        n = cap;
    }
    cap = block.wrapping_add(255) as i32; // addi r10,r9,255
    if !(n < cap) {
        n = cap;
    }
    clamp_non_negative(n)
}

/// `count` floats of `value` through the pointer at `slot`, four per iteration and then a tail.
///
/// The pointer is reloaded out of the record before **every** store, as the original does (`lwz
/// r9,4(r5)` / `lwz r9,8(r5)` between every `stfs`), so a store that aliases the slot behaves
/// identically.
fn fill(g: &mut Guest, fpscr: &mut Fpscr, slot: u32, count: u32, value: f64) -> Result<()> {
    let mut done: u32 = 0; // li r7,0
    if count as i32 >= 4 {
        // cmpwi cr6,r8,4 ; blt cr6
        // addi r10,r8,-4 ; rlwinm r10,r10,30,2,31 ; addi r10,r10,1 -> quads; rlwinm r7,r10,2,0,29
        let mut quads = (count.wrapping_sub(4) >> 2).wrapping_add(1);
        done = (quads << 2) & 0xFFFF_FFFC;
        let mut off: u32 = 0; // li r11,0
        loop {
            fpscr.disable_flush_mode_unconditional();
            // Four stores, four reloads of the slot. `lwz r9,N(r5)` appears between every `stfs`
            // in the lifted body; it is the reload, not the store, that is easy to lose here.
            let p = g.u32(slot)?;
            fp::store_single(g, p.wrapping_add(off), value)?; // stfsx f0,r11,r9
            let p = g.u32(slot)?;
            fp::store_single(g, p.wrapping_add(off).wrapping_add(4), value)?; // stfs f0,4(r6)
            let p = g.u32(slot)?;
            // stfs f0,-4(rX) off a pointer the original has already advanced by 12 — the same
            // address as `+8`, written the way the instruction pair forms it.
            fp::store_single(
                g,
                p.wrapping_add(off).wrapping_add(12).wrapping_sub(4),
                value,
            )?;
            let p = g.u32(slot)?;
            fp::store_single(g, p.wrapping_add(off).wrapping_add(12), value)?; // stfsx f0,r10,rY
            off = off.wrapping_add(16); // addi r11,r11,16
            quads -= 1;
            if quads == 0 {
                break; // bdnz
            }
        }
    }
    // cmpw cr6,r7,r8 ; bge cr6 — signed, on the float counts rather than on the byte offsets.
    if (done as i32) < (count as i32) {
        let mut remaining = count.wrapping_sub(done); // subf r10,r7,r8 ; mtctr r10
        let mut off = (done << 2) & 0xFFFF_FFFC; // rlwinm r11,r7,2,0,29
        loop {
            fpscr.disable_flush_mode_unconditional();
            let p = g.u32(slot)?; // lwz r10,N(r5) — reloaded here too
            fp::store_single(g, p.wrapping_add(off), value)?; // stfsx f0,r11,r10
            off = off.wrapping_add(4); // addi r11,r11,4
            remaining -= 1;
            if remaining == 0 {
                break; // bdnz
            }
        }
    }
    Ok(())
}

/// `sub_82B3DF90` — fill the tail of one or two float output buffers with a rodata constant.
///
/// Arguments, by register: `object` is `r3`, `consumed` is `r4` (floats the caller has already
/// accounted for, subtracted from both limits) and `buffers` is `r5`, a record whose `+4` and `+8`
/// are the two float pointers. Only the low words of all three are ever used, so they are `u32`.
/// There is no return value.
///
/// **Writes** `4 * count0` bytes through the pointer at `buffers + 4`, and — only when the pointer
/// at `buffers + 8` is non-null — `4 * count1` bytes through that one. Nothing else. Reads the
/// object's `+16`, `+20`, `+36`, `+40`, `+44`, the two pointers, and [`FILL_CONSTANT_CELL`].
///
/// **Both counts are capped, and the caps are asymmetric.** Buffer 0's is `block + 255`; buffer
/// 1's is `block + 127`, tested again against `block + 255` by a second compare that can never
/// fire. That is what the instruction stream does and it is reproduced rather than folded into one
/// `min` — see [`count1`].
///
/// **The object's three control fields are reloaded between the two fills**, as the original does,
/// so a first fill that lands on the object changes the second fill's length.
///
/// The constant is loaded **once**, before either loop, whatever `count0` turns out to be — so a
/// call that fills nothing still reads the cell.
pub fn fill_tail(g: &mut Guest, object: u32, consumed: u32, buffers: u32) -> Result<()> {
    let end = g.u32(object.wrapping_add(OBJ_END))?; // lwz r10,20(r3)
    let cursor = g.u32(object.wrapping_add(OBJ_CURSOR))?; // lwz r11,36(r3)
    if (cursor as i32) >= (end as i32) {
        return Ok(()); // cmpw cr6,r11,r10 ; bgelr cr6 — signed
    }

    let limit0 = g.u32(object.wrapping_add(OBJ_LIMIT0))?; // lwz r9,40(r3)
    let block = g.u32(object.wrapping_add(OBJ_BLOCK))?; // lwz r10,16(r3)
    let n0 = count0(limit0, cursor, block, consumed);

    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let value = fp::load_single(g, FILL_CONSTANT_CELL)?; // lfs f0,23056(r6)

    fill(g, &mut fpscr, buffers.wrapping_add(BUFFER0), n0, value)?;

    if g.u32(buffers.wrapping_add(BUFFER1))? == 0 {
        return Ok(()); // lwz r11,8(r5) ; cmplwi cr6,r11,0 ; beqlr cr6
    }

    // Reloaded after the first fill, in this order (lwz r11,44 ; lwz r10,36 ; lwz r9,16).
    let limit1 = g.u32(object.wrapping_add(OBJ_LIMIT1))?;
    let cursor1 = g.u32(object.wrapping_add(OBJ_CURSOR))?;
    let block1 = g.u32(object.wrapping_add(OBJ_BLOCK))?;
    let n1 = count1(limit1, cursor1, block1, consumed);

    fill(g, &mut fpscr, buffers.wrapping_add(BUFFER1), n1, value)
}

// ------------------------------------------------------------- sub_82B3DEA8: the write side

/// `sub_82B3DEA8`'s output record, in `r6`: `+16` is one past the last word the caller produced.
///
/// The block written is counted **back** from it by `4 * count` bytes, so the record holds an end
/// pointer and never a base. `sub_82B3DD90` writes this word from [`fill_segments`]'s return and the
/// caller advances it block by block.
pub const OUT_END: u32 = 16;

/// The object's `+0`: the float ring's byte base, `lwz r9,0(r11)`.
///
/// The same offset [`RING_BASE`] names on the *descriptor* [`copy_from_ring`] takes in `r4`, and the
/// coincidence is worth a name of its own rather than reusing that constant: here it is read off the
/// stream object in `r3`, which is a different structure that happens to keep its ring base at the
/// same offset.
pub const OBJ_RING: u32 = 0;
const _: () = assert!(
    OBJ_RING == RING_BASE,
    "both structures keep a ring base at +0"
);

/// Everything both copies need, all of it fixed at entry.
///
/// Split out for the same reason [`Plan`] is: in the C++ the body and the harness's window builder
/// both have to compute it, and a second copy of this arithmetic is a second thing to get wrong.
/// Every load the original makes happens before its first `memcpy`, and the second copy's arguments
/// come out of non-volatile registers rather than a reload — so nothing here has to be recomputed
/// between the two.
#[derive(Clone, Copy, Debug, Default)]
struct WritePlan {
    /// `r3` at the `twllei` — `LOAD(obj + 20)`, the `divw` divisor.
    span: u64,
    /// `r8` at the `twlgei`.
    guard: u64,
    /// `r3` at the branch: where the first run is written, with the wrap applied.
    cursor: u64,
    /// `r31` — the base of this channel's window in the ring, and where the wrapped run is written.
    ring_start: u64,
    /// `r27` — the block, counted back from the output record's end.
    source: u64,
    /// False on the `bge`: the call writes nothing at all.
    copies: bool,
    /// `r29` at the first copy.
    first_bytes: u64,
    /// `r5` at the second copy.
    rest_bytes: u64,
}

/// `divw rD,rA,rB` as RexGlue lowers it: zero for a zero divisor and for the one overflowing case.
///
/// PowerPC leaves `rD` undefined there and RexGlue picks zero, so the quotient of a zero span is
/// **0** and the body carries on with it. That is observable — `whole` becomes zero and the
/// remainder is the whole position — which is why it is written out rather than made an error.
fn divw(dividend: u32, divisor: u32) -> u32 {
    let (a, b) = (dividend as i32, divisor as i32);
    if b == 0 || (a == i32::MIN && b == -1) {
        return 0;
    }
    (a / b) as u32
}

/// `sub_82B3DEA8` — write one block of `count` words into the stream's wrapping ring, in two runs.
///
/// Arguments, by register: `object` is `r3`, `channel` is `r4`, `count` is `r5` (words) and `record`
/// is `r6`. All four arrive as full 64-bit registers because the arithmetic below keeps 64-bit
/// intermediates: `object` and `record` are addressed through their low words (`mr r11,r3`,
/// `lwz r6,16(r6)`), `channel` reaches a `mullw` as a **sign-extended** low word, and `count`'s low
/// word drives both lengths while its full value is subtracted from the record's end pointer.
///
/// The ring is divided into one window per channel, the window length in words being the object's
/// `+20`: `ring_start = ring + 4·(+20)·channel` and the window ends `4·(+20)` bytes later. Inside it,
/// the `+52` word position is reduced modulo `+20` by an explicit `divw`/`mullw`/`subf`, `+24` is
/// added, and the result is the cursor the block is placed at — advanced by `(+20) - (+24)` words
/// when it falls outside the window.
///
/// **That wrap is written as the original writes it and no semantics is claimed for it.** `+20` and
/// `+24` are the same `high`/`low` pair [`copy_from_ring`] reads, and here `+20` is doing double duty
/// as the `divw` divisor *and* the window length while `+24` is doing double duty as a lag added to
/// the cursor *and* the subtrahend in the wrap distance. The consequence, stated because a reader
/// will otherwise assume the tidy version: the wrap brings a cursor **below** the window's start back
/// into it, and a cursor at or past the window's **end** is pushed further out unless `+24` exceeds
/// `+20`. Both compares are emitted by the original and both are written out here; no test in this
/// crate establishes which of the two the game actually reaches.
///
/// **Writes** the two runs and nothing else: `[cursor, cursor + first_bytes)` and
/// `[ring_start, ring_start + rest_bytes)`, both performed by `sub_82EDF460` (see [`crate::mem`]).
/// This function has no stores of its own. Reads the object's `+0`, `+20`, `+24` and `+52`, the
/// record's `+16`, and the block at `source`.
///
/// **Returns the guest's full `r3`**, which is not one value on both paths and is worth reading
/// twice. On the `bge` path — a block at least as long as the channel window — it is the wrapped
/// `cursor`. On the copy path there is no `mr r3,…` after the second `memcpy`, so it is whatever
/// `sub_82EDF460` left: its own destination, `ring_start`. That is a claim about the guest routine of
/// the same kind [`crate::mem`] documents, not something this body computes.
///
/// **Two `tw` traps are not represented here.** `twllei r3,0` fires on a zero span and
/// `twlgei r8,-1` on the `divw` overflow guard; RexGlue's `ppc_trap` for type 0 only warns and
/// returns, so in both cases the original carries straight on with [`divw`]'s zero quotient. There is
/// no guest state to reproduce, which is why they appear as this paragraph rather than as an `Err`:
/// turning a warning into a refusal would be a divergence, and inventing a flag would be API no
/// recorded vector can check.
pub fn write_into_ring(
    g: &mut Guest,
    object: u64,
    channel: u64,
    count: u64,
    record: u64,
) -> Result<u64> {
    let plan = make_write_plan(g, object as u32, channel, count, record as u32)?;
    if !plan.copies {
        // bge cr6,0x82b3df88 — straight to the epilogue with the cursor still in r3.
        return Ok(plan.cursor);
    }
    // bl 0x82edf460 — the run that reaches this channel window's end.
    mem::memcpy(g, plan.cursor as u32, plan.source as u32, plan.first_bytes)?;
    // add r4,r29,r27 — 64-bit on two zero-extended words, so a carry into bit 32 is kept.
    let wrapped_source = plan.first_bytes.wrapping_add(plan.source);
    // bl 0x82edf460 — the remainder, from the window's start. Length zero when the first run took
    // the whole block; the guest routine still returns its own r3, which is what this call leaves.
    mem::memcpy(
        g,
        plan.ring_start as u32,
        wrapped_source as u32,
        plan.rest_bytes,
    )?;
    Ok(plan.ring_start)
}

fn make_write_plan(
    g: &Guest,
    object: u32,
    channel: u64,
    count: u64,
    record: u32,
) -> Result<WritePlan> {
    let mut plan = WritePlan::default();

    let position = g.u32(object.wrapping_add(OBJ_POSITION))? as u64; // lwz r7,52(r3)
    let span = g.u32(object.wrapping_add(OBJ_SPAN_HIGH))? as u64; // lwz r3,20(r3)
    // rotlwi r10,r7,1 — the position doubled, on the low word only, so bit 31 rotates into bit 0.
    let doubled = (position as u32).rotate_left(1) as u64;
    let source_end = g.u32(record.wrapping_add(OUT_END))? as u64; // lwz r6,16(r6)
    // mullw r4,r3,r4 — the full 64-bit product of the two sign-extended low words, so a carry into
    // bit 32 survives into the address arithmetic below.
    let block = ((span as u32 as i32 as i64).wrapping_mul(channel as u32 as i32 as i64)) as u64;
    let ring = g.u32(object.wrapping_add(OBJ_RING))? as u64; // lwz r9,0(r11)
    let lag = g.u32(object.wrapping_add(OBJ_SPAN_LOW))? as u64; // lwz r8,24(r11)
    let quotient = divw(position as u32, span as u32) as u64; // divw r31,r7,r3
    let doubled_less_one = (doubled as i64).wrapping_sub(1) as u64; // addi r29,r10,-1
    let block_bytes = words_to_bytes(block); // rlwinm r10,r4,2,0,29
    // mullw r4,r31,r3 — again 64-bit; the remainder below is a 64-bit subtraction of it.
    let whole = ((quotient as u32 as i32 as i64).wrapping_mul(span as u32 as i32 as i64)) as u64;
    plan.ring_start = block_bytes.wrapping_add(ring); // add r31,r10,r9
    let remainder = position.wrapping_sub(whole); // subf r10,r4,r7
    let span_bytes = words_to_bytes(span); // rlwinm r9,r3,2,0,29
    let offset = remainder.wrapping_add(lag); // add r10,r10,r8
    plan.guard = span & !doubled_less_one; // andc r8,r3,r29
    let offset_bytes = words_to_bytes(offset); // rlwinm r10,r10,2,0,29
    let count_bytes = words_to_bytes(count); // rlwinm r5,r5,2,0,29
    let cursor = offset_bytes.wrapping_add(plan.ring_start); // add r10,r10,r31
    plan.span = span; // the twllei r3,0 operand
    plan.source = source_end.wrapping_sub(count_bytes); // subf r27,r5,r6
    let ring_end = span_bytes.wrapping_add(plan.ring_start); // add r9,r9,r31
    plan.cursor = cursor; // mr r3,r10

    // cmplw cr6,r10,r31 ; blt cr6 ; cmplw cr6,r9,r10 ; bgt cr6 — a cursor below the window's start,
    // or at or past its end, is outside it and gets the wrap span added. Both compares are unsigned
    // on the low words.
    if (cursor as u32) < (plan.ring_start as u32) || !((ring_end as u32) > (cursor as u32)) {
        let high = g.u32(object.wrapping_add(OBJ_SPAN_HIGH))? as u64; // lwz r8,20(r11) — re-read
        let low = g.u32(object.wrapping_add(OBJ_SPAN_LOW))? as u64; // lwz r7,24(r11)
        // subf r6,r7,r8 ; rlwinm r11,r6,2,0,29 ; add r3,r11,r10
        plan.cursor = words_to_bytes(high.wrapping_sub(low)).wrapping_add(cursor);
    }

    // subf r11,r31,r9 ; srawi r10,r11,2 ; cmpw cr6,r30,r10 ; bge cr6 — a block at least as long as
    // the channel window itself writes nothing.
    let window_words = ((ring_end.wrapping_sub(plan.ring_start)) as u32 as i32) >> 2;
    if (count as u32 as i32) >= window_words {
        return Ok(plan);
    }
    plan.copies = true;

    // subf r11,r3,r9 ; srawi r28,r11,2 ; cmpw cr6,r30,r28 ; blt cr6 ; mr r28,r30 — the first run
    // reaches the window's end, clamped to the block.
    let mut run = ((((ring_end.wrapping_sub(plan.cursor)) as u32 as i32) >> 2) as i64) as u64;
    if (count as u32 as i32) < (run as u32 as i32) {
        run = count;
    }
    plan.first_bytes = words_to_bytes(run); // rlwinm r29,r28,2,0,29
    plan.rest_bytes = words_to_bytes(count.wrapping_sub(run)); // subf r11,r28,r30 ; rlwinm r5
    Ok(plan)
}

// ------------------------------------------------------------------ sub_82B3DD90: the window

/// `lbz r28,56(r31)` — non-zero means the stream has a second segment. Read twice, the second time
/// after [`fill_segments`] has run.
pub const OBJ_TWO_BUFFERS: u32 = 56;
/// `stwu r1,-176(r1)`. The ring window and the segment array live in this frame, and their
/// addresses are what [`fill_segments`] is handed.
pub const WINDOW_FRAME: u32 = 176;
/// `addi r4,r1,80` — the ring window's place in the frame.
pub const WINDOW_OFFSET: u32 = 80;
/// `addi r5,r1,96` — the segment array's.
pub const SEGMENTS_OFFSET: u32 = 96;
/// `stw r10,88(r1)` — the ring end less the lag. Stored, and read by nobody this call reaches.
pub const WIN_LAGGED: u32 = 8;

/// Build the ring window and the segment array for one block, fill the segments, and zero their
/// tails (`sub_82B3DD90`). Returns 256, the block length, as `li r3,256`.
///
/// `object` is `r3` and `consumed` `r5`, both at full width because the two calls hand them on as
/// full registers; `block_index` is `r4`, whose low word is what `mullw` multiplies; `out` is the
/// caller's record in `r6`; `sp` is `r1`.
///
/// The window is `ring + 4 * span * block_index` to `+ 4 * span`, with the cursor at
/// `(position + consumed) mod span + lag` words into it. The `mod` is a `divw` and a `mullw`, so a
/// zero span divides to a zero quotient — the original guards the divide with `twllei` and `twlgei`,
/// and RexGlue's trap only logs and returns, so both are no-ops here as they are in the recomp.
///
/// Three details worth pinning:
///
/// - **The second buffer word is masked, not branched on.** With one segment the frame slot for
///   segment 1's output is never initialised, and the `subfic`/`subfe`/`and` publishes zero whatever
///   it holds. The flag is re-read after [`fill_segments`] runs.
/// - **Four 64-bit sums**, one per `add`: `position + consumed`, the block offset, the cursor and
///   the ring end all keep their high halves until a store truncates them.
/// - **The frame is real guest memory.** Its back chain is written and the two structures are read
///   by the callees through their addresses, so the frame must be mapped.
pub fn build_window(
    g: &mut Guest,
    object: u64,
    block_index: u64,
    consumed: u64,
    out: u64,
    sp: u32,
) -> Result<u64> {
    let obj = object as u32;
    let record = out as u32;
    let frame = sp.wrapping_sub(WINDOW_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-176(r1)
    let window = frame.wrapping_add(WINDOW_OFFSET);
    let segments = frame.wrapping_add(SEGMENTS_OFFSET);

    let position = u64::from(g.u32(obj.wrapping_add(OBJ_POSITION))?); // lwz r6,52(r3)
    let span = u64::from(g.u32(obj.wrapping_add(OBJ_SPAN_HIGH))?); // lwz r3,20(r3)
    let reach = position.wrapping_add(consumed); // add r7,r6,r30
    // mullw r6,r3,r4 -- the full product of the sign-extended low words.
    let block = (i64::from(span as u32 as i32) * i64::from(block_index as u32 as i32)) as u64;
    let limit0 = g.u32(obj.wrapping_add(OBJ_LIMIT0))?; // lwz r4,40(r31)
    let ring = u64::from(g.u32(obj.wrapping_add(OBJ_RING))?); // lwz r11,0(r31)
    let lag = u64::from(g.u32(obj.wrapping_add(OBJ_SPAN_LOW))?); // lwz r10,24(r31)
    let cap_base = u64::from(g.u32(obj.wrapping_add(OBJ_BLOCK))?); // lwz r9,16(r31)
    let two = g.u8(obj.wrapping_add(OBJ_TWO_BUFFERS))?; // lbz r28,56(r31)
    g.set_u32(segments + SEG_NEED, limit0)?; // stw r4,96(r1)
    g.set_u32(segments + SEG_OUT, 0)?; // stw r5,108(r1)
    // divw r4,r7,r3 -- a zero quotient where the divide is undefined.
    let (dividend, divisor) = (reach as u32 as i32, span as u32 as i32);
    let quotient = if divisor != 0 && !(dividend == i32::MIN && divisor == -1) {
        dividend / divisor
    } else {
        0
    };
    let whole = (i64::from(quotient) * i64::from(divisor)) as u64; // mullw r6,r4,r3
    let ring_start = words_to_bytes(block).wrapping_add(ring); // rlwinm r8 ; add r11,r8,r11
    let remainder = reach.wrapping_sub(whole); // subf r8,r6,r7
    g.set_u32(window + RING_BASE, ring_start as u32)?; // stw r11,80(r1)
    let offset = remainder.wrapping_add(lag); // add r4,r8,r10
    let ring_end = words_to_bytes(span).wrapping_add(ring_start); // add r10,r7,r11
    g.set_u32(window + RING_END, ring_end as u32)?; // stw r10,84(r1)
    // twllei r3,0 -- logs and returns in RexGlue.
    let cursor = words_to_bytes(offset).wrapping_add(ring_start); // add r3,r8,r11
    let cap0 = cap_base.wrapping_add(255); // addi r11,r9,255
    let lagged = ring_end.wrapping_sub(words_to_bytes(lag)); // subf r10,r27,r10
    g.set_u32(window + RING_CURSOR, cursor as u32)?; // stw r3,92(r1)
    let mut count = 1u64; // li r6,1
    g.set_u32(segments + SEG_CAP, cap0 as u32)?; // stw r11,100(r1)
    // twlgei r4,-1 -- the divide-overflow guard, likewise a no-op.
    g.set_u32(window + WIN_LAGGED, lagged as u32)?; // stw r10,88(r1)

    if two != 0 {
        let limit1 = g.u32(obj.wrapping_add(OBJ_LIMIT1))?; // lwz r11,44(r31)
        let cap1 = cap_base.wrapping_add(127); // addi r10,r9,127
        count = 2; // li r6,2
        g.set_u32(segments + SEG_STRIDE + SEG_OUT, 0)?; // stw r5,124(r1)
        g.set_u32(segments + SEG_STRIDE + SEG_CAP, cap1 as u32)?; // stw r10,116(r1)
        g.set_u32(segments + SEG_STRIDE + SEG_NEED, limit1)?; // stw r11,112(r1)
    }

    let end = fill_segments(g, object, u64::from(window), segments, count)?; // bl 0x82b3dc48
    let seg0 = g.u32(segments + SEG_OUT)?; // lwz r11,108(r1)
    g.set_u32(record.wrapping_add(OUT_END), end as u32)?; // stw r3,16(r29)
    let seg1 = g.u32(segments + SEG_STRIDE + SEG_OUT)?; // lwz r10,124(r1)
    g.set_u32(record.wrapping_add(BUFFER0), seg0)?; // stw r11,4(r29)
    let two_again = g.u8(obj.wrapping_add(OBJ_TWO_BUFFERS))?; // lbz r9,56(r31) -- re-read
    // subfic r8,r9,0 ; subfe r6,r7,r7 ; and r11,r6,r10 -- only the carry survives.
    let mask = if two_again == 0 { 0 } else { u32::MAX };
    g.set_u32(record.wrapping_add(BUFFER1), mask & seg1)?; // stw r11,8(r29)

    fill_tail(g, obj, consumed as u32, record)?; // bl 0x82b3df90
    Ok(256) // li r3,256
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const WINDOW: u32 = BASE + 0x100;
    const SEGMENTS: u32 = BASE + 0x140;
    const RECORD: u32 = BASE + 0x180;
    const RING: u32 = BASE + 0x1000;
    const RING_BYTES: u32 = 0x400;
    const DEST: u32 = BASE + 0x2000;
    const BUF0: u32 = BASE + 0x3000;
    const BUF1: u32 = BASE + 0x3400;

    /// A guest with the ring, the buffers and the rodata cell all mapped.
    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x4000);
        g.put(FILL_CONSTANT_CELL & !0xF, vec![0u8; 16]);
        g.set_u32(FILL_CONSTANT_CELL, 0.0f32.to_bits()).unwrap();
        g
    }

    /// The ring descriptor, and the ring filled with an ascending word ramp.
    fn ring(g: &mut Guest, cursor: u32) {
        g.set_u32(WINDOW + RING_BASE, RING).unwrap();
        g.set_u32(WINDOW + RING_END, RING + RING_BYTES).unwrap();
        g.set_u32(WINDOW + RING_CURSOR, cursor).unwrap();
        for i in 0..RING_BYTES / 4 {
            g.set_u32(RING + 4 * i, 0x1000 + i).unwrap();
        }
    }

    fn words(g: &Guest, at: u32, n: u32) -> Vec<u32> {
        (0..n).map(|i| g.u32(at + 4 * i).unwrap()).collect()
    }

    // ------------------------------------------------------------------ sub_82B3DB90

    #[test]
    fn a_read_that_does_not_reach_the_ring_end_is_one_run() {
        // Cursor 64 words in, asking for 8 words behind it: src = cursor - 8*4, entirely inside.
        let mut g = guest();
        ring(&mut g, RING + 256);
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 8, 8).unwrap();
        assert_eq!(got, 8);
        // Words 56..63 of the ring, because the source is measured *back* from the cursor by
        // `limit` words, not forward from the base.
        assert_eq!(
            words(&g, DEST, 8),
            (56..64).map(|i| 0x1000 + i).collect::<Vec<_>>()
        );
        assert_eq!(g.u32(DEST + 32).unwrap(), 0, "nothing past the run");
    }

    #[test]
    fn a_read_that_crosses_the_end_is_split_into_two_runs() {
        // The split only happens on the *wrapped* path, and that is worth stating because it is
        // not obvious: while the source stays inside the ring, `run` is `limit + (end - cursor)/4`,
        // which is never below `want`, so the second copy is always empty. Here the source is
        // pushed below the base, wrapped forward by a whole ring, and lands four words from the
        // end — so four words come from there and four more from the base.
        let mut g = guest();
        ring(&mut g, RING + 16);
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, 256).unwrap(); // a full ring of wrap
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 0).unwrap();
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 8, 8).unwrap();
        assert_eq!(got, 8);
        let last = RING_BYTES / 4; // 256 words
        let expected: Vec<u32> = (last - 4..last).chain(0..4).map(|i| 0x1000 + i).collect();
        assert_eq!(
            words(&g, DEST, 8),
            expected,
            "four from the end, then four from the base"
        );
    }

    #[test]
    fn a_source_below_the_ring_base_is_wrapped_forward_by_the_objects_span() {
        // src = cursor - 4*limit lands below the base, so the object's (high - low) words are
        // added. Without the wrap the copy would read whatever precedes the ring.
        let mut g = guest();
        ring(&mut g, RING + 16); // four words in
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, 100).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 4).unwrap(); // 96 words of wrap
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 8, 4).unwrap();
        assert_eq!(got, 4);
        // src = (RING + 16) - 32 = RING - 16, below the base, so + 96*4 = RING + 368 -> word 92.
        assert_eq!(
            words(&g, DEST, 4),
            (92..96).map(|i| 0x1000 + i).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_source_at_or_past_the_ring_end_is_also_wrapped() {
        // The second half of the same test: not below the base, but not below the end either, so
        // `inside` stays false and the span is added. A transcription that only checked the base
        // would leave this read past the ring.
        let mut g = guest();
        ring(&mut g, RING + RING_BYTES + 64);
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, 0).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 32).unwrap(); // -32 words: moves the source back
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 8, 4).unwrap();
        assert_eq!(got, 4);
        // src = RING + 0x440 - 32 = RING + 0x420, past the end; -32 words -> RING + 0x3A0.
        assert_eq!(
            words(&g, DEST, 4),
            (232..236).map(|i| 0x1000 + i).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_zero_request_copies_nothing_and_returns_zero() {
        let mut g = guest();
        ring(&mut g, RING + 256);
        g.set_u32(DEST, 0xFEED_FACE).unwrap();
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 8, 0).unwrap();
        assert_eq!(got, 0);
        assert_eq!(g.u32(DEST).unwrap(), 0xFEED_FACE, "not one byte written");
    }

    #[test]
    fn the_request_is_clamped_to_the_window_by_a_signed_compare() {
        // r7 > r6 clamps to r6; r7 < r6 keeps r7. The compare is signed on the low words, so a
        // `want` with bit 31 set is *smaller* than any sane limit and survives the clamp — which is
        // what makes the ~4 GB copy the C++ Windows() refuses reachable at all.
        let mut g = guest();
        ring(&mut g, RING + 256);
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 4, 9).unwrap();
        assert_eq!(got, 4);
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 9, 4).unwrap();
        assert_eq!(got, 4);
        // Negative: kept, and the copy then runs off the map rather than hanging.
        let r = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 4, !0u64);
        assert!(
            r.is_err(),
            "a negative word count is an Err here, a 4 GB memcpy in the original"
        );
    }

    #[test]
    fn the_return_value_keeps_all_sixty_four_bits_of_whichever_register_won() {
        // `mr r31,r6` / the untouched r7 are full 64-bit moves, and sub_82B3DC48 consumes the
        // result as `ctx.r3.u64`. Truncating to 32 bits leaves every byte of memory identical and
        // the caller's running total wrong — the exact failure mode CLAUDE.md warns about.
        let mut g = guest();
        ring(&mut g, RING + 256);
        let want = 0xDEAD_0000_0000_0004u64; // low word 4, high half set
        let got = copy_from_ring(&mut g, OBJECT as u64, WINDOW as u64, DEST as u64, 8, want);
        assert_eq!(got.unwrap(), want, "the high half has to survive");
    }

    // ------------------------------------------------------------------ sub_82B3DC48

    /// One 16-byte segment slot.
    fn segment(g: &mut Guest, k: u32, need: u32, cap: u32) {
        let at = SEGMENTS + k * SEG_STRIDE;
        g.set_u32(at + SEG_NEED, need).unwrap();
        g.set_u32(at + SEG_CAP, cap).unwrap();
        g.set_u32(at + SEG_RANK, 0xFFFF_FFFF).unwrap();
        g.set_u32(at + SEG_OUT, 0xFFFF_FFFF).unwrap();
    }

    fn stream(g: &mut Guest) {
        g.set_u32(OBJECT + OBJ_DEST, DEST).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, 0).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 0).unwrap();
    }

    #[test]
    fn one_segment_is_filled_and_its_output_word_published() {
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 32, 32);

        let end = fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 1).unwrap();

        assert_eq!(
            g.u32(SEGMENTS + SEG_RANK).unwrap(),
            0,
            "the rank word is written on every path"
        );
        assert_eq!(
            g.u32(SEGMENTS + SEG_OUT).unwrap(),
            DEST,
            "pad is zero, so out == dest"
        );
        assert_eq!(end, (DEST + 128) as u64, "32 words copied");
        assert_eq!(
            words(&g, DEST, 4),
            (96..100).map(|i| 0x1000 + i).collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_segments_are_ranked_with_the_larger_need_first() {
        // Slot 0's rank word selects which *segment* is served first. With the second segment
        // wanting more, slot 0 must name segment 1.
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 32, 32);
        segment(&mut g, 1, 64, 64);

        fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 2).unwrap();

        assert_eq!(
            g.u32(SEGMENTS + SEG_RANK).unwrap(),
            1,
            "the bigger segment leads"
        );
        assert_eq!(g.u32(SEGMENTS + SEG_STRIDE + SEG_RANK).unwrap(), 0);

        // And the other way round, where the compare goes the other way.
        let mut h = guest();
        stream(&mut h);
        ring(&mut h, RING + 512);
        segment(&mut h, 0, 64, 64);
        segment(&mut h, 1, 32, 32);
        fill_segments(&mut h, OBJECT as u64, WINDOW as u64, SEGMENTS, 2).unwrap();
        assert_eq!(h.u32(SEGMENTS + SEG_RANK).unwrap(), 0);
        assert_eq!(h.u32(SEGMENTS + SEG_STRIDE + SEG_RANK).unwrap(), 1);
    }

    #[test]
    fn the_ranking_compare_is_signed_so_a_negative_need_ranks_lowest() {
        // `cmpw`, not `cmplw`. A need word with bit 31 set is *less* than any positive one, so it
        // loses the lead — under an unsigned compare it would win it.
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 0x8000_0000, 32);
        segment(&mut g, 1, 32, 32);
        fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 2).unwrap();
        assert_eq!(
            g.u32(SEGMENTS + SEG_RANK).unwrap(),
            1,
            "the negative need does not lead"
        );
        assert_eq!(g.u32(SEGMENTS + SEG_STRIDE + SEG_RANK).unwrap(), 0);
    }

    #[test]
    fn a_segment_count_of_zero_writes_only_the_lead_rank_and_returns_the_objects_dest() {
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 32, 32);

        let end = fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 0).unwrap();

        assert_eq!(end, DEST as u64);
        assert_eq!(
            g.u32(SEGMENTS + SEG_RANK).unwrap(),
            0,
            "the unconditional store still ran"
        );
        assert_eq!(
            g.u32(SEGMENTS + SEG_OUT).unwrap(),
            0xFFFF_FFFF,
            "and nothing else did"
        );
        assert_eq!(g.u32(DEST).unwrap(), 0);
    }

    #[test]
    fn the_pad_from_rounding_the_need_up_to_thirty_two_offsets_the_output() {
        // A need of 33 rounds up to 64, so the segment's 31 words of pad push its output word 31
        // words past `dest` — the caller writes into the middle of the block, not at its start.
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 33, 33);

        fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 1).unwrap();

        assert_eq!(g.u32(SEGMENTS + SEG_OUT).unwrap(), DEST + 4 * 31);
    }

    #[test]
    fn the_second_segment_starts_where_the_first_one_stopped() {
        // The destination cursor advances by 4 * the words the copy reported, so the two segments
        // tile the block. A `dest` that did not advance would stack them on top of each other.
        //
        // The lead's cap is *below* its need on purpose. `running` carries the lead's shortfall —
        // `need_up - copied` — into the next trip, so when the lead is satisfied in full the
        // second segment's `need_up > running` and it takes the `over` branch, which asks for
        // nothing. A two-segment test where both are actually filled has to leave a shortfall.
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 64, 32);
        segment(&mut g, 1, 32, 32);

        let end = fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 2).unwrap();

        assert_eq!(
            g.u32(SEGMENTS + SEG_RANK).unwrap(),
            0,
            "the 64-word segment leads"
        );
        assert_eq!(
            g.u32(SEGMENTS + SEG_OUT).unwrap(),
            DEST,
            "segment 0 at the start"
        );
        // Segment 1's output word is 32 words in, which is only true if `dest` moved.
        assert_eq!(
            g.u32(SEGMENTS + SEG_STRIDE + SEG_OUT).unwrap(),
            DEST + 4 * 32
        );
        assert_eq!(end, (DEST + 4 * 64) as u64, "32 words then 32 more");
        // And the two runs abut: ring words 64..127, contiguous, with no gap or overlap.
        assert_eq!(
            words(&g, DEST, 64),
            (64..128).map(|i| 0x1000 + i).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_segment_that_is_over_the_running_budget_asks_for_nothing() {
        // The `over` branch, which is the common shape once the lead has been satisfied in full:
        // `running` is zero, the second segment's rounded need is above it, and the branch-free
        // max clamps its request to zero rather than to a negative word count.
        let mut g = guest();
        stream(&mut g);
        ring(&mut g, RING + 512);
        segment(&mut g, 0, 64, 64); // the lead gets everything it asked for
        segment(&mut g, 1, 32, 32);

        let end = fill_segments(&mut g, OBJECT as u64, WINDOW as u64, SEGMENTS, 2).unwrap();

        assert_eq!(
            end,
            (DEST + 4 * 64) as u64,
            "the second segment contributed nothing"
        );
        assert_eq!(
            words(&g, DEST, 64),
            (64..128).map(|i| 0x1000 + i).collect::<Vec<_>>()
        );
        assert_eq!(
            g.u32(DEST + 4 * 64).unwrap(),
            0,
            "and wrote nothing past the first run"
        );
    }

    // ------------------------------------------------------------------ sub_82B3DF90

    fn tail_object(g: &mut Guest, block: u32, end: u32, cursor: u32, limit0: u32, limit1: u32) {
        g.set_u32(OBJECT + OBJ_BLOCK, block).unwrap();
        g.set_u32(OBJECT + OBJ_END, end).unwrap();
        g.set_u32(OBJECT + OBJ_CURSOR, cursor).unwrap();
        g.set_u32(OBJECT + OBJ_LIMIT0, limit0).unwrap();
        g.set_u32(OBJECT + OBJ_LIMIT1, limit1).unwrap();
    }

    fn poison(g: &mut Guest, at: u32, n: u32) {
        for i in 0..n {
            g.set_u32(at + 4 * i, 0x7F7F_7F7F).unwrap();
        }
    }

    #[test]
    fn a_cursor_at_or_past_the_end_fills_nothing() {
        let mut g = guest();
        tail_object(&mut g, 0, 100, 100, 1000, 1000);
        g.set_u32(RECORD + BUFFER0, BUF0).unwrap();
        poison(&mut g, BUF0, 4);
        fill_tail(&mut g, OBJECT, 0, RECORD).unwrap();
        assert_eq!(words(&g, BUF0, 4), vec![0x7F7F_7F7F; 4], "not one store");
    }

    #[test]
    fn buffer_zero_is_filled_with_the_rodata_constant() {
        let mut g = guest();
        g.set_u32(FILL_CONSTANT_CELL, (-2.5f32).to_bits()).unwrap(); // read live, not assumed
        tail_object(&mut g, 0, 100, 0, 7, 0);
        g.set_u32(RECORD + BUFFER0, BUF0).unwrap();
        g.set_u32(RECORD + BUFFER1, 0).unwrap();
        poison(&mut g, BUF0, 9);

        fill_tail(&mut g, OBJECT, 0, RECORD).unwrap();

        for i in 0..7 {
            assert_eq!(g.f32(BUF0 + 4 * i).unwrap(), -2.5, "float {i}");
        }
        assert_eq!(
            g.u32(BUF0 + 28).unwrap(),
            0x7F7F_7F7F,
            "nothing past the count"
        );
    }

    #[test]
    fn the_quad_loop_and_the_tail_loop_cover_every_count() {
        // 1, 2, 3 take the tail loop alone; 4 and 8 the quad loop alone; 5, 7, 9, 13 both. A
        // mistranscribed trip count shows up as a short or a long fill at exactly one of these.
        for count in [1u32, 2, 3, 4, 5, 7, 8, 9, 13, 16, 17] {
            let mut g = guest();
            g.set_u32(FILL_CONSTANT_CELL, 1.5f32.to_bits()).unwrap();
            tail_object(&mut g, 0, 100, 0, count, 0);
            g.set_u32(RECORD + BUFFER0, BUF0).unwrap();
            g.set_u32(RECORD + BUFFER1, 0).unwrap();
            poison(&mut g, BUF0, count + 2);

            fill_tail(&mut g, OBJECT, 0, RECORD).unwrap();

            for i in 0..count {
                assert_eq!(
                    g.f32(BUF0 + 4 * i).unwrap(),
                    1.5,
                    "count {count}, float {i}"
                );
            }
            assert_eq!(
                g.u32(BUF0 + 4 * count).unwrap(),
                0x7F7F_7F7F,
                "count {count}: overrun"
            );
        }
    }

    #[test]
    fn a_null_second_pointer_ends_the_call_after_the_first_fill() {
        let mut g = guest();
        tail_object(&mut g, 0, 100, 0, 4, 4);
        g.set_u32(RECORD + BUFFER0, BUF0).unwrap();
        g.set_u32(RECORD + BUFFER1, 0).unwrap();
        poison(&mut g, BUF1, 4);
        fill_tail(&mut g, OBJECT, 0, RECORD).unwrap();
        assert_eq!(words(&g, BUF1, 4), vec![0x7F7F_7F7F; 4]);
        assert_eq!(
            words(&g, BUF0, 4),
            vec![0; 4],
            "buffer 0 still got its fill"
        );
    }

    #[test]
    fn both_buffers_are_filled_and_their_counts_are_computed_differently() {
        // Buffer 0's count subtracts the cursor then `consumed`; buffer 1's subtracts `consumed`
        // then the cursor, off a *different* limit. With limit0 != limit1 the two lengths differ,
        // which is what makes the second count's own arithmetic observable.
        let mut g = guest();
        g.set_u32(FILL_CONSTANT_CELL, 3.0f32.to_bits()).unwrap();
        tail_object(&mut g, 1000, 100, 2, 10, 7);
        g.set_u32(RECORD + BUFFER0, BUF0).unwrap();
        g.set_u32(RECORD + BUFFER1, BUF1).unwrap();
        poison(&mut g, BUF0, 10);
        poison(&mut g, BUF1, 10);

        fill_tail(&mut g, OBJECT, 3, RECORD).unwrap();

        // count0 = 10 - 2 - 3 = 5; count1 = 7 - 3 - 2 = 2.
        for i in 0..5 {
            assert_eq!(g.f32(BUF0 + 4 * i).unwrap(), 3.0, "buffer 0 float {i}");
        }
        assert_eq!(g.u32(BUF0 + 20).unwrap(), 0x7F7F_7F7F);
        for i in 0..2 {
            assert_eq!(g.f32(BUF1 + 4 * i).unwrap(), 3.0, "buffer 1 float {i}");
        }
        assert_eq!(g.u32(BUF1 + 8).unwrap(), 0x7F7F_7F7F);
    }

    #[test]
    fn the_two_buffers_are_capped_at_different_lengths() {
        // Buffer 0 is capped at `block + 255`. Buffer 1 is capped at `block + 127` and then, as a
        // second `cmpw ; blt ; mr` the original really does emit, at `block + 255` — which can only
        // *raise* a value, and a value the first cap already lowered to `block + 127` is below
        // `block + 255`, so the second test never fires. **Measured, not assumed**: the assertion
        // below reads 127 and not 255, which is the whole content of "the second cap is dead".
        // Both are written out anyway, because the original has both.
        assert_eq!(count0(100_000, 0, 0, 0), 255, "capped at block + 255");
        assert_eq!(
            count1(100_000, 0, 0, 0),
            127,
            "capped at block + 127, and left there"
        );
        assert_eq!(
            count0(1000, 0, 700, 0),
            955,
            "block + 255 == 955, below the 1000 asked for"
        );
        assert_eq!(
            count1(1000, 0, 700, 0),
            827,
            "block + 127 == 827 for the same block"
        );
        // Below the cap both are the length asked for, so the caps are caps and not constants.
        assert_eq!(count0(40, 0, 1000, 0), 40);
        assert_eq!(count1(40, 0, 1000, 0), 40);
        // And the clamp at the bottom: a negative length fills nothing rather than ~4 GB.
        assert_eq!(count0(0, 10, 1000, 0), 0);
        assert_eq!(count1(0, 10, 1000, 0), 0);
    }

    #[test]
    fn the_pointer_is_reloaded_before_every_store() {
        // The record's own slot is inside the span being filled, so the first store overwrites the
        // pointer and every later store goes through the new value. A body that hoisted the load
        // would keep filling the original buffer. This is the aliasing case the C++ Windows()
        // refuses to bracket, so what it establishes is that the reload survived the
        // transcription — not that the guest agrees with the answer.
        let mut g = guest();
        // The constant is the *address* the second store will use, read as a float.
        let redirect = f32::from_bits(BUF1);
        g.set_u32(FILL_CONSTANT_CELL, redirect.to_bits()).unwrap();
        tail_object(&mut g, 0, 100, 0, 3, 0);
        // Buffer 0 points at the record itself, so store 0 lands on the +4 slot.
        g.set_u32(RECORD + BUFFER0, RECORD + BUFFER0).unwrap();
        g.set_u32(RECORD + BUFFER1, 0).unwrap();
        poison(&mut g, BUF1, 4);

        fill_tail(&mut g, OBJECT, 0, RECORD).unwrap();

        assert_eq!(
            g.u32(RECORD + BUFFER0).unwrap(),
            BUF1,
            "the slot now holds the new pointer"
        );
        // Stores 1 and 2 went through the reloaded pointer, at BUF1 + 4 and BUF1 + 8.
        assert_eq!(
            g.u32(BUF1 + 4).unwrap(),
            BUF1,
            "store 1 followed the reload"
        );
        assert_eq!(g.u32(BUF1 + 8).unwrap(), BUF1, "store 2 followed it too");
        assert_eq!(
            g.u32(BUF1).unwrap(),
            0x7F7F_7F7F,
            "and store 0 did not land here"
        );
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest();
        tail_object(&mut g, 0, 100, 0, 8, 0);
        g.set_u32(RECORD + BUFFER0, BUF0).unwrap();
        g.set_u32(RECORD + BUFFER1, 0).unwrap();
        let before = crate::vmx::get_mxcsr();
        fill_tail(&mut g, OBJECT, 0, RECORD).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }

    // ------------------------------------------------------------------ the shared idioms

    #[test]
    fn words_to_bytes_drops_the_high_half_before_it_scales() {
        assert_eq!(words_to_bytes(8), 32);
        // The high half is discarded, not shifted into the result.
        assert_eq!(words_to_bytes(0xDEAD_0000_0000_0008), 32);
        // And the product is taken modulo 2^32 with the low two bits cleared.
        assert_eq!(words_to_bytes(0x4000_0001), 4);
        assert_eq!(words_to_bytes(0xFFFF_FFFF), 0xFFFF_FFFC);
    }

    #[test]
    fn round_up_32_rounds_up_on_the_low_word_only() {
        assert_eq!(round_up_32(0), 0);
        assert_eq!(round_up_32(1), 32);
        assert_eq!(round_up_32(32), 32);
        assert_eq!(round_up_32(33), 64);
        // The add wraps at 32 bits, so a word near the top rounds to zero rather than to 2^32.
        assert_eq!(round_up_32(0xFFFF_FFFF), 0);
    }

    // ------------------------------------------------------------------ sub_82B3DEA8

    /// One past the last word the caller produced, stored at `RECORD + 16`.
    const SRC_END: u32 = BASE + 0x2900;

    /// The four object cells `write_into_ring` reads, and a poisoned ring.
    ///
    /// The ring is poisoned rather than zeroed so that "nothing was written here" is a real
    /// assertion: a zero-filled ring cannot tell an untouched word from one the call wrote zero to.
    fn write_object(g: &mut Guest, span: u32, lag: u32, position: u32) {
        g.set_u32(OBJECT + OBJ_RING, RING).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, span).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, lag).unwrap();
        g.set_u32(OBJECT + OBJ_POSITION, position).unwrap();
        g.set_u32(RECORD + OUT_END, SRC_END).unwrap();
        for i in 0..0x300 / 4 {
            g.set_u32(RING + 4 * i, 0x7F7F_7F7F).unwrap();
        }
    }

    /// `n` words of ramp ending at [`SRC_END`], which is where the block the call reads lives.
    fn block(g: &mut Guest, n: u32) {
        for i in 0..n {
            g.set_u32(SRC_END - 4 * n + 4 * i, 0x2000 + i).unwrap();
        }
    }

    fn ramp(n: u32) -> Vec<u32> {
        (0..n).map(|i| 0x2000 + i).collect()
    }

    #[test]
    fn a_block_inside_the_channel_window_is_written_as_one_run() {
        // span 64 words, position 16 in, no lag: the cursor is 64 bytes into the window and the
        // whole block fits before its end, so the second copy has length zero.
        let mut g = guest();
        write_object(&mut g, 64, 0, 16);
        block(&mut g, 8);

        let got = write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();

        assert_eq!(words(&g, RING + 64, 8), ramp(8));
        assert_eq!(
            g.u32(RING + 60).unwrap(),
            0x7F7F_7F7F,
            "one word below the cursor"
        );
        assert_eq!(
            g.u32(RING + 96).unwrap(),
            0x7F7F_7F7F,
            "one word above the run"
        );
        assert_eq!(
            g.u32(RING).unwrap(),
            0x7F7F_7F7F,
            "the second copy wrote nothing"
        );
        // No `mr r3` follows the second memcpy, so the return is that call's own destination.
        assert_eq!(got, RING as u64);
    }

    #[test]
    fn a_block_that_reaches_the_window_end_is_split_at_it() {
        // Position 60 of 64 leaves four words before the end; the other four go to the window's
        // start, out of the *later* half of the block — which is what pins `first_bytes + source`.
        let mut g = guest();
        write_object(&mut g, 64, 0, 60);
        block(&mut g, 8);

        let got = write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();

        assert_eq!(
            words(&g, RING + 240, 4),
            ramp(4),
            "the run to the window end"
        );
        assert_eq!(
            words(&g, RING, 4),
            vec![0x2004, 0x2005, 0x2006, 0x2007],
            "and the remainder from its start"
        );
        assert_eq!(
            g.u32(RING + 16).unwrap(),
            0x7F7F_7F7F,
            "nothing past the wrapped run"
        );
        assert_eq!(got, RING as u64);
    }

    #[test]
    fn the_position_is_reduced_modulo_the_window_by_the_divide() {
        // `divw`/`mullw`/`subf` is a modulo written out longhand. Positions 16, 80 and 208 differ by
        // whole spans of 64 and must place the block identically; without the reduction the second
        // and third would run off the window and take the wrap branch.
        let place = |position: u32| {
            let mut g = guest();
            write_object(&mut g, 64, 0, position);
            block(&mut g, 8);
            let got = write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();
            (got, words(&g, RING, 64))
        };
        let base = place(16);
        assert_eq!(place(80), base, "one whole span further on");
        assert_eq!(place(3 * 64 + 16), base, "three spans further on");
        // And the reduction is not the identity: a position inside the first span places elsewhere.
        assert_ne!(place(20), base);
    }

    #[test]
    fn a_zero_span_divides_to_zero_and_the_body_carries_on() {
        // `twllei r3,0` warns and returns in RexGlue, so a zero span is not a refusal. What it is
        // not is a test of the quotient: `whole = quotient * span` multiplies by that same zero, so
        // RexGlue's choice of 0 for `divw`'s undefined case is **invisible** here and this test
        // does not pretend to pin it. What it does pin is that the call completes and that the
        // position reaches the cursor unreduced — window length zero puts the cursor at or past the
        // end, the wrap adds `0 - lag` words, and the two lags cancel.
        let mut g = guest();
        write_object(&mut g, 0, 7, 9);
        block(&mut g, 8);

        let got = write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();

        // window_words is 0, so `count >= 0` takes the bge path: nothing is written at all.
        assert_eq!(words(&g, RING, 16), vec![0x7F7F_7F7F; 16]);
        // The wrap distance is `0 - 7` words, which reaches the final `add r3,r11,r10` as the
        // zero-extended `0xFFFFFFE4` rather than as a negative number — so the 64-bit sum carries
        // into bit 32 and the returned `r3` is *not* a 32-bit address. Only the low word is the
        // cursor. Truncating this chain would leave every written byte identical and this register
        // wrong, which is the whole of `CLAUDE.md`'s 64-bit-intermediates rule in one value.
        assert_eq!(
            got,
            0x1_0000_0000 + (RING + 4 * 9) as u64,
            "position 9 words on, lags cancelled"
        );
        assert_eq!(got as u32, RING + 4 * 9);
    }

    #[test]
    fn the_channel_index_selects_a_window_of_its_own() {
        // r4 scales the window length: channel 1 starts one whole span into the ring. The first
        // window has to come out untouched, which is the half a dropped `mullw` would break.
        let mut g = guest();
        write_object(&mut g, 64, 0, 16);
        block(&mut g, 8);

        let got = write_into_ring(&mut g, OBJECT as u64, 1, 8, RECORD as u64).unwrap();

        assert_eq!(words(&g, RING + 256 + 64, 8), ramp(8));
        assert_eq!(
            words(&g, RING, 64),
            vec![0x7F7F_7F7F; 64],
            "channel 0's window is untouched"
        );
        assert_eq!(got, (RING + 256) as u64);
    }

    #[test]
    fn a_cursor_below_the_window_start_is_wrapped_by_high_minus_low() {
        // Lag -4 with position 0 puts the cursor 16 bytes *below* the window, which takes the first
        // of the two wrap compares. The distance added is `(high - low) = 64 - (-4) = 68` words, so
        // the cursor lands exactly on the window's end: the first run is empty and the whole block
        // goes to the start through the second copy.
        //
        // Without the wrap the cursor stays at RING - 16 and the block is written there instead,
        // which is what the two "untouched" assertions below detect.
        let mut g = guest();
        write_object(&mut g, 64, (-4i32) as u32, 0);
        block(&mut g, 8);
        for i in 0..4u32 {
            g.set_u32(RING - 16 + 4 * i, 0x0BAD_0BAD).unwrap();
        }

        let got = write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();

        assert_eq!(
            words(&g, RING, 8),
            ramp(8),
            "the whole block, at the window's start"
        );
        for i in 0..4u32 {
            assert_eq!(
                g.u32(RING - 16 + 4 * i).unwrap(),
                0x0BAD_0BAD,
                "below the window"
            );
        }
        assert_eq!(
            g.u32(RING + 256).unwrap(),
            0x7F7F_7F7F,
            "and nothing at the end it wrapped to"
        );
        assert_eq!(got, RING as u64);
    }

    #[test]
    fn a_block_as_long_as_the_window_writes_nothing_and_returns_the_cursor() {
        // `cmpw cr6,r30,r10 ; bge cr6` — signed, on the word counts. Eight words into an eight-word
        // window writes nothing; seven into the same window writes.
        let mut g = guest();
        write_object(&mut g, 8, 0, 0);
        block(&mut g, 8);
        let got = write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();
        assert_eq!(words(&g, RING, 8), vec![0x7F7F_7F7F; 8], "not one word");
        assert_eq!(got, RING as u64, "the cursor, not a copy's destination");

        let mut h = guest();
        write_object(&mut h, 8, 0, 0);
        block(&mut h, 7);
        write_into_ring(&mut h, OBJECT as u64, 0, 7, RECORD as u64).unwrap();
        assert_eq!(
            words(&h, RING, 7),
            ramp(7),
            "one word short of the window does write"
        );
    }

    #[test]
    fn the_source_is_counted_back_from_the_records_end_pointer() {
        // `subf r27,r5,r6` — the block is `[end - 4*count, end)`, so moving the record's end word
        // by one slides which words are copied. A port that read a *base* pointer there would be
        // insensitive to this.
        let mut g = guest();
        write_object(&mut g, 64, 0, 16);
        for i in 0..12u32 {
            g.set_u32(SRC_END - 48 + 4 * i, 0x3000 + i).unwrap();
        }
        g.set_u32(RECORD + OUT_END, SRC_END - 16).unwrap();

        write_into_ring(&mut g, OBJECT as u64, 0, 8, RECORD as u64).unwrap();

        // end - 16 means words 0..7 of the twelve, not 4..11.
        assert_eq!(
            words(&g, RING + 64, 8),
            (0..8).map(|i| 0x3000 + i).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_window_base_keeps_a_carry_into_bit_32() {
        // `add r31,r10,r9` is 64-bit on two zero-extended words and the result is the value this
        // function *returns*. With the ring near the top of the map and sixteen windows of eight
        // words below the cursor, `ring + 4*span*channel` passes 2^32: the copy still lands at the
        // truncated address, and the return carries bit 32. Truncating the chain to 32 bits leaves
        // guest memory byte-identical and the returned register wrong — the failure `CLAUDE.md`
        // records surfacing on the 120th call of another function.
        let mut g = Guest::from_segments(vec![
            crate::Segment {
                base: 0x0000_0000,
                bytes: vec![0x7Fu8; 0x200],
            },
            crate::Segment {
                base: BASE,
                bytes: vec![0u8; 0x4000],
            },
        ]);
        g.set_u32(OBJECT + OBJ_RING, 0xFFFF_FF00).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, 8).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 0).unwrap();
        g.set_u32(OBJECT + OBJ_POSITION, 0).unwrap();
        g.set_u32(RECORD + OUT_END, SRC_END).unwrap();
        block(&mut g, 4);

        // channel 16 of 8-word windows: block_bytes = 4*8*16 = 0x200, so ring_start = 0x100000100.
        let got = write_into_ring(&mut g, OBJECT as u64, 16, 4, RECORD as u64).unwrap();

        assert_eq!(got, 0x1_0000_0100, "bit 32 survives into the returned r3");
        assert_eq!(
            words(&g, 0x100, 4),
            ramp(4),
            "and the copy lands at the truncated address"
        );
    }

    // ------------------------------------------------------------------ sub_82B3DD90

    const SP: u32 = BASE + 0x3F00;
    const CONSUMED: u32 = 28;

    /// A stream object whose window lands on the `ring()` fixture: a 256-word span, no lag, and a
    /// position that puts the cursor 128 words in. The frame the call builds is poisoned, so a word
    /// the body does not write shows.
    fn window_stream(g: &mut Guest, two: bool) {
        ring(g, 0);
        g.set_u32(OBJECT + OBJ_RING, RING).unwrap();
        g.set_u32(OBJECT + OBJ_DEST, DEST).unwrap();
        g.set_u32(OBJECT + OBJ_BLOCK, 0).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, RING_BYTES / 4).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 0).unwrap();
        g.set_u32(OBJECT + OBJ_CURSOR, 0).unwrap();
        g.set_u32(OBJECT + OBJ_LIMIT0, 32).unwrap();
        g.set_u32(OBJECT + OBJ_LIMIT1, 16).unwrap();
        g.set_u32(OBJECT + OBJ_POSITION, 100).unwrap();
        g.set_u8(OBJECT + OBJ_TWO_BUFFERS, u8::from(two)).unwrap();
        for i in 0..WINDOW_FRAME / 4 {
            g.set_u32(SP - WINDOW_FRAME + 4 * i, 0xFFFF_FFFF).unwrap();
        }
    }

    /// The same work done by hand: the window and segments written where the tests keep them, then
    /// the two kernels called directly and the record filled in between.
    fn window_reference(g: &mut Guest, count: u32) {
        g.set_u32(WINDOW + RING_BASE, RING).unwrap();
        g.set_u32(WINDOW + RING_END, RING + RING_BYTES).unwrap();
        g.set_u32(WINDOW + RING_CURSOR, RING + 512).unwrap(); // (100 + 28) mod 256 words in
        segment(g, 0, 32, 255);
        segment(g, 1, 16, 127);
        let end = fill_segments(g, OBJECT as u64, WINDOW as u64, SEGMENTS, count as u64).unwrap();
        g.set_u32(RECORD + OUT_END, end as u32).unwrap();
        let s0 = g.u32(SEGMENTS + SEG_OUT).unwrap();
        g.set_u32(RECORD + BUFFER0, s0).unwrap();
        let s1 = if count == 2 {
            g.u32(SEGMENTS + SEG_STRIDE + SEG_OUT).unwrap()
        } else {
            0
        };
        g.set_u32(RECORD + BUFFER1, s1).unwrap();
        fill_tail(g, OBJECT, CONSUMED, RECORD).unwrap();
    }

    #[test]
    fn the_window_builder_feeds_both_kernels_what_it_derives() {
        for two in [false, true] {
            let mut g = guest();
            window_stream(&mut g, two);
            let mut h = g.clone();
            let r = build_window(&mut g, OBJECT as u64, 0, CONSUMED as u64, RECORD as u64, SP);
            assert_eq!(r.unwrap(), 256);
            window_reference(&mut h, if two { 2 } else { 1 });
            assert_eq!(
                words(&g, DEST, 512),
                words(&h, DEST, 512),
                "the copies and fills, two={two}"
            );
            for off in [BUFFER0, BUFFER1, OUT_END] {
                assert_eq!(
                    g.u32(RECORD + off).unwrap(),
                    h.u32(RECORD + off).unwrap(),
                    "+{off}, two={two}"
                );
            }
            assert_eq!(g.u32(SP - WINDOW_FRAME).unwrap(), SP, "the back chain");
            let win = SP - WINDOW_FRAME + WINDOW_OFFSET;
            assert_eq!(
                g.u32(win + WIN_LAGGED).unwrap(),
                RING + RING_BYTES,
                "no lag: the lagged end is the end"
            );
        }
    }

    #[test]
    fn one_segment_publishes_a_null_second_buffer_whatever_the_frame_held() {
        let mut g = guest();
        window_stream(&mut g, false);
        g.set_u32(RECORD + BUFFER1, 0x1234).unwrap();
        build_window(&mut g, OBJECT as u64, 0, CONSUMED as u64, RECORD as u64, SP).unwrap();
        assert_eq!(g.u32(RECORD + BUFFER1).unwrap(), 0);
        let segs = SP - WINDOW_FRAME + SEGMENTS_OFFSET;
        assert_eq!(
            g.u32(segs + SEG_STRIDE + SEG_OUT).unwrap(),
            0xFFFF_FFFF,
            "that slot was never written"
        );
    }

    #[test]
    fn the_window_divides_the_reach_by_the_span_and_keeps_the_remainder() {
        // position 300 + consumed 28 = 328 words; mod 256 is 72, plus a lag of 8 is 80 words into the
        // second block, which starts 256 words past the ring base.
        let mut g = guest();
        window_stream(&mut g, false);
        g.set_u32(OBJECT + OBJ_POSITION, 300).unwrap();
        g.set_u32(OBJECT + OBJ_SPAN_LOW, 8).unwrap();
        build_window(&mut g, OBJECT as u64, 1, CONSUMED as u64, RECORD as u64, SP).unwrap();
        let win = SP - WINDOW_FRAME + WINDOW_OFFSET;
        assert_eq!(g.u32(win + RING_BASE).unwrap(), RING + 1024);
        assert_eq!(g.u32(win + RING_END).unwrap(), RING + 2048);
        assert_eq!(g.u32(win + RING_CURSOR).unwrap(), RING + 1024 + 80 * 4);
        assert_eq!(g.u32(win + WIN_LAGGED).unwrap(), RING + 2048 - 32);
        let segs = SP - WINDOW_FRAME + SEGMENTS_OFFSET;
        assert_eq!(g.u32(segs + SEG_CAP).unwrap(), 255, "block 0 + 255");
    }

    #[test]
    fn a_zero_span_divides_to_zero_before_either_kernel_runs() {
        // The window is built before the first call, so it can be read whatever the kernels then do
        // with an empty ring -- which here is a copy run long enough to leave the test's mapping.
        let mut g = guest();
        window_stream(&mut g, false);
        g.set_u32(OBJECT + OBJ_SPAN_HIGH, 0).unwrap();
        let _ = build_window(&mut g, OBJECT as u64, 3, CONSUMED as u64, RECORD as u64, SP);
        let win = SP - WINDOW_FRAME + WINDOW_OFFSET;
        assert_eq!(
            g.u32(win + RING_BASE).unwrap(),
            RING,
            "a zero span makes every block start at the base"
        );
        assert_eq!(
            g.u32(win + RING_CURSOR).unwrap(),
            RING + 128 * 4,
            "the whole reach is the remainder"
        );
    }
}
