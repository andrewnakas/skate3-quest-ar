//! The mix accumulator: flushing it, folding the pending deltas into it, and clearing ahead of it.
//!
//! Three verified bodies that between them own the block boundary. [`flush_accumulator`] runs once
//! per block on the mixer-side object: it swaps the row-owner pair, copies or clears each channel's
//! row, folds any pending deltas through [`fold_deltas`], and then zeroes the whole accumulator.
//! [`advance_and_clear`] is the other half of the same idea one level out — move a fill position
//! forward and clear that many frames in every channel of a buffer plan.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`flush_accumulator`] | `sub_82B34E08` | verified | 176 | 722,892 | 845,754 |
//! | [`fold_deltas`] | `sub_82B3C668` | verified | 292 | 4,266 | 4,560 |
//! | [`advance_and_clear`] | `sub_82B443F8` | verified | 122 | 500,817 | 541,975 |
//! | [`refold_rows`] | `sub_82B2C8F0` | verified | 143 | 48,658 | 147,982 |
//!
//! [`refold_rows`] is replayed against **800 recorded calls, 0 disagreements**, alongside 36,457 live
//! comparisons in the same session.
//!
//! [`fold_deltas`] is here because [`flush_accumulator`] calls it and nothing else in the corpus
//! does. Porting it separately would have left the flush with a hole on exactly the path that makes
//! it interesting — and `sub_82B3C668`'s own row arithmetic is the same `mullw`/`rlwinm` pair the
//! flush uses, written twice in the C++ and once here ([`row_offset`]).
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green: each C++
//! body was compared call-for-call against the original under the shadow harness at zero
//! divergence, on the real inputs those call counts come from. The Rust has no recorded vectors of
//! its own.
//!
//! Nothing in `docs/rw_audio_structs.h` names any of these structures. Every offset below is the
//! raw offset plus the use the lifted body makes of it.
//!
//! ## Reloads reproduced rather than hoisted
//!
//! All three bodies re-read their loop control out of memory on every pass — the channel count, the
//! row base, the row stride, the plan's buffer. The C++ `Windows()` builders treat a write span
//! that covers any of them as **not comparable** and decline the call, so a caller whose rows
//! overlap its own header is input the harness has never seen. The reloads are kept anyway, because
//! the originals have them; what that establishes is that they survived the transcription, not that
//! the guest agrees with the answer on those layouts.
//!
//! ## What is deliberately not claimed
//!
//! `sub_82B31838` — the neighbouring "mix one source's channels into the bus blocks" body — is
//! **not** here, and it is verified, so the reason is worth stating. It builds two pointer arrays
//! **on its own guest stack frame** (`r1 - 208 + 80` and `+ 112`) and hands them to its mixers, so a
//! faithful port needs the guest stack pointer as an argument and writes into a region no window
//! declares and no recorded vector contains. It also calls `sub_82B46810`, which has a verified C++
//! body that is not translated yet. Both are solvable; neither is solved here, and a version that
//! papered over either would be the kind of green this crate exists to avoid.

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp, mem};

// ---------------------------------------------------------------- sub_82B34E08: the mixer object

/// `u8` channel count. **Reloaded at every use** — five times in one call.
pub const CHANNELS: u32 = 42;
/// `rw_ptr` accumulator, `CHANNELS * 1024` bytes.
pub const ACCUM: u32 = 60;
/// `float[]` — the delta array [`fold_deltas`] consumes, at `self + 64`.
pub const DELTAS: u32 = 64;
/// `u16` byte offset, into the accumulator, of a flag word.
pub const PENDING_OFFSET: u32 = 96;
/// `u8` — non-zero: fold the deltas, then clear this.
pub const DELTA_FLAG: u32 = 98;
/// `u8` — `0xFF` forces the flush with nothing else pending.
pub const FORCE_BYTE: u32 = 165;

/// The other owner of the pair at `r4`, written with the front one.
pub const PAIR_BACK: u32 = 28;
/// The owner **this** call uses, read before the swap. `rw_ptr`.
pub const PAIR_FRONT: u32 = 32;

/// `rw_ptr` row base, on the row owner. The same field [`fold_deltas`] reads.
pub const ROW_BASE: u32 = 4;
/// `u16` row pitch in **singles**; the byte pitch is four times this.
pub const ROW_STRIDE: u32 = 14;
/// `li r5,1024` — the memcpy/memset length per channel.
pub const ROW_BYTES: u32 = 1024;

// ----------------------------------------------------------------- sub_82B3C668: the delta fold

const LIS_82300000: u32 = ((-32208i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82300000 == 0x8230_0000, "lis r11,-32208");

/// `lis -32208 ; addi r7,r11,-31232` — the audio pool, the same cell three other ports name.
pub const POOL: u32 = LIS_82300000.wrapping_add(-31232i32 as u32);
const _: () = assert!(POOL == 0x822F_8600, "lis -32208 ; addi -31232");

/// Sixteen consecutive singles at `pool + 1028 … + 1088`, the curve each delta is spread over.
///
/// The lifted body loads `f30` from `+1028` and `f0` from `+1088` and applies them in **ascending**
/// order, so tap `k` is `+1028 + 4k`. Addresses computed as `((imm & 0xFFFF) << 16) + offset` and
/// asserted, never read off by eye. No value is assumed: they are read live through the guest map.
pub const TAP_TABLE: u32 = POOL + 1028;
const _: () = assert!(TAP_TABLE == 0x822F_8A04, "lfs f30,1028(r7)");
/// Sixteen taps, and therefore the leading 64 bytes of each row.
pub const TAP_COUNT: usize = 16;
const _: () = assert!(
    TAP_TABLE + 4 * (TAP_COUNT as u32 - 1) == 0x822F_8A40,
    "lfs f0,1088(r7)"
);

const LIS_82160000: u32 = ((-32234i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82160000 == 0x8216_0000, "lis r8,-32234");
/// `lfs f29,23056(r8)` — the `0.0f` cell, stored over each delta once it has been consumed.
pub const ZERO_SINGLE: u32 = LIS_82160000 + 23056;
const _: () = assert!(
    ZERO_SINGLE == crate::eval::ZERO_SINGLE,
    "the same pool cell eval names"
);

// ------------------------------------------------------------------ sub_82B443F8: the fill state

/// `float` — the limit at [`LIMIT`] is clamped **up** to this.
pub const LIMIT_FLOOR: u32 = 40;
/// `float` — frames accounted for so far.
pub const POSITION: u32 = 44;
/// `float`.
pub const LIMIT: u32 = 48;
/// `u8` — zeroed on the "nothing to do" return.
pub const IDLE_FLAG: u32 = 69;
/// `u8` channel count on the layout in `r5`, re-read every pass of the loop.
pub const LAYOUT_CHANNELS: u32 = 42;
/// `rw_ptr` to the buffer plan, on the owner in `r3`.
pub const PLAN: u32 = 28;
/// `u32` — the frame count this call last cleared, published on the owner.
pub const FRAMES_CLEARED: u32 = 48;
/// `float*` first channel buffer, on the plan.
pub const PLAN_BUFFER: u32 = 4;
/// `u16` frame stride between channels, on the plan.
pub const PLAN_STRIDE: u32 = 14;

const _: () = assert!(LIMIT == POSITION + 4, "lfs f0,44(r4) ; lfs f13,48(r4)");
const _: () = assert!(
    LAYOUT_CHANNELS == CHANNELS,
    "both structures put the count at +42"
);

// ------------------------------------------------------------------------------- shared idioms

/// `mullw r8,r11,r30 ; rlwinm r11,r8,2,0,29` — the byte offset of row `index`.
///
/// The **64-bit** product of the two sign-extended words, then its **low word** shifted left two
/// with the bottom two bits cleared. Keeping the product 64-bit matters even though only the low
/// word survives: `mullw`'s result is what a later `add` carries into bit 32 elsewhere, and
/// truncating early is the mistake `CLAUDE.md` warns leaves memory byte-identical and a register
/// wrong.
///
/// `sub_82B34E08` and `sub_82B3C668` compute this identically, which is what makes the fold's rows
/// a subset of the flush's declared spans.
fn row_offset(stride: u32, index: u32) -> u32 {
    let product = (stride as i32 as i64).wrapping_mul(index as i32 as i64);
    ((product as u64 as u32) << 2) & 0xFFFF_FFFC
}

/// `rlwinm rN,rX,2,0,29` — the low word scaled by four, zero-extended back to 64 bits.
fn words_to_bytes(value: u64) -> u64 {
    ((value as u32 as u64) << 2) & 0xFFFF_FFFC
}

// --------------------------------------------------------------------------------- sub_82B3C668

/// `sub_82B3C668` — add each pending delta, times a 16-entry pool curve, into the head of its row,
/// then clear the delta.
///
/// Arguments, by register: `object` is `r3` (the row owner, `+4` base and `+14` stride), `deltas`
/// is `r4` (an array of `count` singles) and `count` is `r5`. Only low words are used. No return
/// value.
///
/// **Writes** the leading 64 bytes of each of `count` rows, and the whole delta array — each single
/// is overwritten with the `0.0f` at [`ZERO_SINGLE`] once consumed. Reads the taps at
/// [`TAP_TABLE`], `object + 4`, `object + 14`, and the deltas.
///
/// **The delta is reloaded from memory before every one of the sixteen `fmadds`.** The taps and the
/// zero are hoisted out of the loop by the original and are hoisted here; the delta is not, because
/// the original does not — a row that overlapped the delta array would feed the changed value
/// forward into the remaining taps. Reproduced, not tidied.
///
/// The operand order `fmadds(delta, tap, accumulator)` is every one of the sixteen lifted lines,
/// and is kept: a fused multiply-add is only commutative in its product for non-NaN operands
/// (`docs/vmx128-exactness.md` rule 4).
pub fn fold_deltas(g: &mut Guest, object: u32, deltas: u32, count: u32) -> Result<()> {
    if count == 0 {
        return Ok(()); // cmplwi cr6,r5,0 ; beq — not one store
    }

    let mut fpscr = Fpscr::capture();
    // The taps and the fill are hoisted out of the loop by the original, so a store that aliased
    // the pool would not be seen by later iterations. Hoisted here too.
    fpscr.disable_flush_mode_unconditional(); // lfs f29,23056(r8)
    let fill = fp::load_single(g, ZERO_SINGLE)?;
    let mut tap = [0f64; TAP_COUNT];
    for (k, slot) in tap.iter_mut().enumerate() {
        *slot = fp::load_single(g, TAP_TABLE + 4 * k as u32)?;
    }

    let mut cursor = deltas.wrapping_sub(4); // addi r10,r4,-4: the stfsu pre-increments
    for i in 0..count {
        let stride = g.u16(object.wrapping_add(ROW_STRIDE))? as u32; // lhz r8,14(r3), every pass
        fpscr.disable_flush_mode_unconditional();
        let row_base = g.u32(object.wrapping_add(ROW_BASE))?; // lwz r11,4(r3), every pass
        // add r11,r8,r11 on two zero-extended words: the sum is 64-bit and the stores keep the low
        // word, so the truncation is explicit here.
        let row = ((row_offset(stride, i) as u64) + (row_base as u64)) as u32;
        for (k, tk) in tap.iter().enumerate() {
            let at = row.wrapping_add(4 * k as u32);
            let delta = fp::load_single(g, cursor.wrapping_add(4))?; // reloaded, every tap
            let acc = fp::load_single(g, at)?;
            fp::store_single(g, at, fp::fmadd_single(delta, *tk, acc))?;
        }
        let ea = cursor.wrapping_add(4); // stfsu f29,4(r10): store at r10+4, then r10 = r10+4
        fp::store_single(g, ea, fill)?;
        cursor = ea;
    }
    Ok(())
}

// --------------------------------------------------------------------------------- sub_82B34E08

/// `sub_82B34E08` — flush the mix accumulator. Returns 1 when it did anything and 0 when it
/// returned early.
///
/// Arguments, by register: `object` is `r3` (the mixer-side object) and `pair` is `r4` (the
/// two-entry row-owner pair). Both are addressed through their low words only. The return is the
/// guest's `r3`.
///
/// The order is: decide whether to run at all; swap the pair; **copy** each channel's row when the
/// pending word is set, or **clear** each row when it is not; clear the pending word; fold the
/// deltas if the flag says so; zero the whole accumulator.
///
/// **Writes** `pair + 28` and `pair + 32` (the swap), the pending word inside the accumulator, the
/// delta flag, one 1 KB row per channel, [`fold_deltas`]'s write set, and `1024 * channels` bytes at
/// the accumulator. **The early return writes nothing at all** — every input to it is entry state:
/// the pending word, the delta flag and `+165`.
///
/// **The owner used for the rows is the one read *before* the swap**, `pair + 32`. The swap then
/// moves it to `pair + 28`. Reading the pair after the swap would address the other owner's rows,
/// which is the single easiest thing to get wrong here.
///
/// **The pending word is read twice**: once before the swap, to decide the early return, and again
/// after it, to choose between the copy and the clear loop. They can differ if the swap's two
/// stores land on it, and the original reloads, so this does.
pub fn flush_accumulator(g: &mut Guest, object: u32, pair: u32) -> Result<u64> {
    let pending_off = g.u16(object.wrapping_add(PENDING_OFFSET))? as u32; // lhz r27,96(r3)
    let accum = g.u32(object.wrapping_add(ACCUM))?; // lwz r26,60(r3)
    let pending_addr = pending_off.wrapping_add(accum); // the lwzx/stwx address

    // lwzx r11,r27,r26 ; bne — nothing pending, no deltas, and +165 short of 0xFF: return 0 without
    // a single store.
    if g.u32(pending_addr)? == 0
        && g.u8(object.wrapping_add(DELTA_FLAG))? == 0
        && g.u8(object.wrapping_add(FORCE_BYTE))? < 255
    {
        return Ok(0); // li r3,0
    }

    // loc_82B34E50 — swap the pair.
    let back = g.u32(pair.wrapping_add(PAIR_BACK))?; // lwz r11,28(r4)
    let owner = g.u32(pair.wrapping_add(PAIR_FRONT))?; // lwz r29,32(r4)
    g.set_u32(pair.wrapping_add(PAIR_FRONT), back)?; // stw r11,32(r4)
    g.set_u32(pair.wrapping_add(PAIR_BACK), owner)?; // stw r29,28(r4)

    // Both reloaded after the swap, in this order, before the branch.
    let pending = g.u32(pending_addr)?; // lwzx r10,r27,r26
    let channels = g.u8(object.wrapping_add(CHANNELS))?; // lbz r11,42(r31)

    let mut channel: u32 = 0; // li r30,0 — one counter, shared by the two exclusive loops
    if pending != 0 {
        if channels != 0 {
            let mut source: u32 = 0; // li r28,0
            loop {
                // loc_82B34E80
                let stride = g.u16(owner.wrapping_add(ROW_STRIDE))? as u32; // lhz r11,14(r29)
                let accum_now = g.u32(object.wrapping_add(ACCUM))?; // lwz r9,60(r31)
                let row_base = g.u32(owner.wrapping_add(ROW_BASE))?; // lwz r10,4(r29)
                // add r4,r9,r28 and add r3,r11,r10 are 64-bit; the callee addresses with the low
                // words. bl 0x82edf460 — memcpy, 1024 bytes.
                mem::memcpy(
                    g,
                    ((row_offset(stride, channel) as u64) + (row_base as u64)) as u32,
                    ((accum_now as u64) + (source as u64)) as u32,
                    ROW_BYTES as u64,
                )?;
                channel += 1; // addi r30,r30,1
                source = source.wrapping_add(ROW_BYTES); // addi r28,r28,1024
                // lbz r7,42(r31) ; cmplw cr6,r30,r7 ; blt — the count is reloaded every iteration
                if !(channel < g.u8(object.wrapping_add(CHANNELS))? as u32) {
                    break;
                }
            }
        }
        g.set_u32(pending_addr, 0)?; // loc_82B34EB8: li r11,0 ; stwx r11,r27,r26
    } else if channels != 0 {
        // loc_82B34EC4
        loop {
            // loc_82B34ECC
            let stride = g.u16(owner.wrapping_add(ROW_STRIDE))? as u32;
            let row_base = g.u32(owner.wrapping_add(ROW_BASE))?;
            // bl 0x82ee5e80 — memset, 1024 bytes of zero. Note there is no accumulator read here.
            mem::memset(
                g,
                ((row_offset(stride, channel) as u64) + (row_base as u64)) as u32,
                0,
                ROW_BYTES as u64,
            )?;
            channel += 1;
            if !(channel < g.u8(object.wrapping_add(CHANNELS))? as u32) {
                break;
            }
        }
    }

    // loc_82B34EFC — the flag is reloaded here, after both loops.
    if g.u8(object.wrapping_add(DELTA_FLAG))? != 0 {
        // mr r3,r29 ; addi r4,r31,64 ; lbz r5,42(r31)
        let n = g.u8(object.wrapping_add(CHANNELS))? as u32;
        fold_deltas(g, owner, object.wrapping_add(DELTAS), n)?;
        g.set_u8(object.wrapping_add(DELTA_FLAG), 0)?; // li r11,0 ; stb r11,98(r31)
    }

    // loc_82B34F20 — clear the whole accumulator. `rotlwi r5,r11,10` on a byte is `r11 * 1024`:
    // the byte cannot rotate a bit past 32, so the rotate and the shift agree. Both fields reloaded.
    let channels_now = g.u8(object.wrapping_add(CHANNELS))? as u32; // lbz r11,42(r31)
    let accum_now = g.u32(object.wrapping_add(ACCUM))?; // lwz r3,60(r31)
    mem::memset(g, accum_now, 0, channels_now.rotate_left(10) as u64)?;
    Ok(1) // li r3,1
}

// --------------------------------------------------------------------------------- sub_82B443F8

/// The clear target for channel `index`.
///
/// `rlwinm`/`mullw`/`rlwinm` then `add r3,r11,r10`, a 64-bit add of two zero-extended words — a
/// carry into bit 32 survives into the callee's `r3`, so the 64-bit value is what gets passed and
/// only the store addresses truncate.
fn channel_target(stride: u32, index: u32, buffer: u32) -> u64 {
    // mullw r9,r11,r31 keeps the full 64-bit product of the sign-extended halves; the rlwinm that
    // follows then takes its LOW word and scales that by four.
    let frames = (stride as i32 as i64).wrapping_mul(index as i32 as i64) as u64;
    words_to_bytes(frames).wrapping_add(buffer as u64)
}

/// `sub_82B443F8` — advance a fill position by `count` frames and clear `count` frames in every
/// channel of a buffer plan. Returns 1 when it cleared and 0 when there was nothing to do.
///
/// Arguments, by register: `owner` is `r3` (the plan pointer at `+28`, the published count at
/// `+48`), `state` is `r4` (three singles and a byte), `layout` is `r5` (the channel count at
/// `+42`) and `count` is `r6`. All four are used through their low words only. The return is the
/// guest's `r3`.
///
/// **Writes** `state + 48` (only when the limit is below its floor), `state + 44` and
/// `owner + 48` on the clearing path, `state + 69` on the idle path, and `4 * count` bytes in each
/// of `channels` channels. Reads `state + 40`, `state + 44`, `state + 48`, `layout + 42`,
/// `owner + 28`, and the plan's `+4` and `+14`.
///
/// **Both float compares are unordered-aware and reproduced as written.** `fcmpu cr6,f13,f0 ; bge`
/// takes the branch on a NaN, so a NaN limit skips the clamp store — which `limit < floor` gives
/// for free. `fcmpu cr6,f0,f13 ; blt` likewise falls out with zero on a NaN, which is why the
/// second test is written `!(position < limit)` and not `position >= limit`.
///
/// **The limit is reloaded after the clamp**, so the compare that decides the early return sees the
/// stored value rather than the register — which differs if the store aliased something.
///
/// **Zero channels still publishes the count.** The loop is skipped, `owner + 48` is written, and
/// the call returns 1.
pub fn advance_and_clear(
    g: &mut Guest,
    owner: u32,
    state: u32,
    layout: u32,
    count: u32,
) -> Result<u64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let floor_limit = fp::load_single(g, state.wrapping_add(LIMIT_FLOOR))?; // lfs f0,40(r4)
    let limit = fp::load_single(g, state.wrapping_add(LIMIT))?; // lfs f13,48(r4)
    if limit < floor_limit {
        fp::store_single(g, state.wrapping_add(LIMIT), floor_limit)?; // stfs f0,48(r4)
    }

    fpscr.disable_flush_mode_unconditional();
    let limit_now = fp::load_single(g, state.wrapping_add(LIMIT))?; // lfs f13,48(r4) — reloaded
    let position = fp::load_single(g, state.wrapping_add(POSITION))?; // lfs f0,44(r4)
    if !(position < limit_now) {
        g.set_u8(state.wrapping_add(IDLE_FLAG), 0)?; // li r11,0 ; stb r11,69(r4)
        return Ok(0); // li r3,0
    }

    fpscr.disable_flush_mode_unconditional();
    // fadds f10,f11,f0 ; stfs f10,44(r4) — the position moves before anything is cleared, and the
    // converted count is the *first* operand of the add, as lifted.
    let moved = fp::add_single(fp::word_to_single(count), position);
    fp::store_single(g, state.wrapping_add(POSITION), moved)?;

    let channels = g.u8(layout.wrapping_add(LAYOUT_CHANNELS))?; // lbz r10,42(r29)
    let plan = g.u32(owner.wrapping_add(PLAN))?; // lwz r30,28(r26)
    // cmplwi cr6,r10,0 ; beq cr6 — zero channels clears nothing but still publishes the count.
    if channels != 0 {
        let bytes = words_to_bytes(count as u64); // rlwinm r28,r27,2,0,29
        let mut index: u32 = 0; // li r31,0
        loop {
            // Both reloaded every pass, as the original does: a clear that lands on the plan
            // changes the next pass's target.
            let stride = g.u16(plan.wrapping_add(PLAN_STRIDE))? as u32; // lhz r11,14(r30)
            let buffer = g.u32(plan.wrapping_add(PLAN_BUFFER))?; // lwz r10,4(r30)
            // bl 0x82ee5e80 — memset(target, 0, bytes); li r4,0 supplies the fill byte.
            mem::memset(g, channel_target(stride, index, buffer) as u32, 0, bytes)?;
            index += 1; // addi r31,r31,1
            // lbz r8,42(r29) ; cmplw cr6,r31,r8 ; blt — the count is re-read, unsigned compare.
            if !(index < g.u8(layout.wrapping_add(LAYOUT_CHANNELS))? as u32) {
                break;
            }
        }
    }
    g.set_u32(owner.wrapping_add(FRAMES_CLEARED), count)?; // stw r27,48(r26)
    Ok(1) // li r3,1
}

// ------------------------------------------------------- re-folding the rows for a new channel count

/// `stb r6,41(r3)` — the mixer's mirror of the count the pair was last folded for, written only when
/// the pair has no row length yet.
pub const MIRRORED_COUNT: u32 = 41;
/// `lwz r7,48(r4)` — the pair's row length **in floats**. Zero means there are no rows to remap.
pub const PAIR_ROW_LENGTH: u32 = 48;
/// `lbz r6,60(r4)` — the channel count the pair was last folded for.
pub const PAIR_FOLD_COUNT: u32 = 60;
/// `stwu r1,-176(r1)` — the frame the two pointer arrays live in.
pub const REFOLD_FRAME_BYTES: u32 = 176;
/// `addi r4,r1,80` — eight source row pointers.
pub const REFOLD_SOURCE_ARRAY: u32 = 80;
/// `addi r3,r1,112` — eight destination row pointers.
pub const REFOLD_DEST_ARRAY: u32 = 112;
/// Both arrays hold eight words, which is what clamps the source list.
pub const REFOLD_ARRAY_WORDS: u32 = 8;

/// Re-fold the mix rows for a new channel count (`sub_82B2C8F0`).
///
/// `mixer` is `r3`, `pair` is `r4`, and `sp` is the guest `r1` on entry. Returns the constant 1 the
/// original leaves in `r3`.
///
/// When the mixer's channel count differs from the count the owner pair was last folded for, this
/// builds both owners' row pointers into frame scratch, hands the remap to
/// [`crate::routing::gather_bank`], then exchanges the two owners and records the new count. When the
/// counts agree it does nothing at all — except that a pair with **no row length** still mirrors its
/// count into the mixer's `+41`, which happens before the comparison and on every call.
///
/// Four details that are the original's rather than a tidy reading:
///
/// - **The channel count is read three times** — once for the comparison, once as the destination
///   loop's trip count, once for the byte finally recorded — and a store between them would be
///   visible. The reads happen where the original has them.
/// - **The source list is clamped to eight, unsigned** — and that matters more than it looks, because
///   the two frame arrays abut: eight source words at `+80`, the destination words from `+112`. A
///   ninth source pointer would be written straight onto the first *destination* pointer. The C++
///   window builder declines calls with a count above eight outright.
/// - **A zero count skips its loop entirely**, because `mtctr` with zero would run 2^32 times.
/// - **The row pitch is a `rotlwi ...,2` of a `u16`** — the stride is in singles and the pitch in
///   bytes — and the row walk is 64-bit with only the store truncating.
///
/// The frame is reproduced here, unlike [`crate::routing::scatter_mix`]'s: the two pointer arrays are
/// real guest memory that [`crate::routing::gather_bank`] reads back, so they have to be written
/// where the original writes them.
pub fn refold_rows(g: &mut Guest, mixer: u32, pair: u32, sp: u32) -> Result<u64> {
    // stwu r1,-176(r1) -- the back chain, inside this function's own frame.
    let frame = sp.wrapping_sub(REFOLD_FRAME_BYTES);
    g.set_u32(frame, sp)?;

    let length = g.u32(pair + PAIR_ROW_LENGTH)?; // lwz r7,48(r4)
    let old_count = u32::from(g.u8(pair + PAIR_FOLD_COUNT)?); // lbz r6,60(r4)

    if length == 0 {
        // stb r6,41(r3) -- before the comparison, so this happens even when nothing else does.
        g.set_u8(mixer + MIRRORED_COUNT, old_count as u8)?;
    }

    // loc_82B2C920
    let new_count = u32::from(g.u8(mixer + CHANNELS)?); // lbz r5,42(r30)
    if old_count != new_count {
        if length != 0 {
            let back = g.u32(pair + PAIR_BACK)?; // lwz r11,28(r31)
            let front = g.u32(pair + PAIR_FRONT)?; // lwz r4,32(r31)
            // cmplwi cr6,r6,8 ; ble ; li r10,8 -- unsigned, so the source list stops at eight.
            let sources = old_count.min(REFOLD_ARRAY_WORDS);

            if sources != 0 {
                let pitch = u64::from(u32::from(g.u16(back + ROW_STRIDE)?).rotate_left(2));
                let mut row = u64::from(g.u32(back + ROW_BASE)?); // lwz r11,4(r11)
                for i in 0..sources {
                    g.set_u32(frame + REFOLD_SOURCE_ARRAY + 4 * i, row as u32)?; // stwu r11,4(r10)
                    row += pitch; // add r11,r9,r11 -- 64-bit; only the store truncates
                }
            }

            if new_count != 0 {
                let trips = u32::from(g.u8(mixer + CHANNELS)?); // lbz r8,42(r30) -- reloaded
                let pitch = u64::from(u32::from(g.u16(front + ROW_STRIDE)?).rotate_left(2));
                let mut row = u64::from(g.u32(front + ROW_BASE)?); // lwz r11,4(r4)
                for i in 0..trips {
                    g.set_u32(frame + REFOLD_DEST_ARRAY + 4 * i, row as u32)?;
                    row += pitch;
                }
            }

            // loc_82B2C9A4: the destination array is r3 and the source array r4.
            crate::routing::gather_bank(
                g,
                frame + REFOLD_DEST_ARRAY,
                frame + REFOLD_SOURCE_ARRAY,
                new_count,
                old_count,
                u64::from(length),
            )?;
        }

        // loc_82B2C9B0 -- both owners reloaded, then exchanged.
        let front_now = g.u32(pair + PAIR_FRONT)?; // lwz r11,32(r31)
        let back_now = g.u32(pair + PAIR_BACK)?; // lwz r10,28(r31)
        g.set_u32(pair + PAIR_BACK, front_now)?; // stw r11,28(r31)
        g.set_u32(pair + PAIR_FRONT, back_now)?; // stw r10,32(r31)
        let recorded = g.u8(mixer + CHANNELS)?; // lbz r9,42(r30) -- the third read
        g.set_u8(pair + PAIR_FOLD_COUNT, recorded)?; // stb r9,60(r31)
    }

    Ok(1) // li r3,1
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const PAIR: u32 = BASE + 0x100;
    const OWNER_A: u32 = BASE + 0x140;
    const OWNER_B: u32 = BASE + 0x180;
    const ROWS: u32 = BASE + 0x2000;
    const ACCUM_AT: u32 = BASE + 0x6000;

    /// A guest with both rodata pools mapped.
    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0xA000);
        g.put(TAP_TABLE, vec![0u8; 4 * TAP_COUNT]);
        g.put(ZERO_SINGLE, vec![0u8; 4]);
        g
    }

    // -------------------------------------------------------------------- the row re-fold

    const MIXER: u32 = BASE + 0x300;
    const SP: u32 = BASE + 0x9000;
    const SRC_ROWS: u32 = BASE + 0x3000;
    const DST_ROWS: u32 = BASE + 0x4000;
    /// Row pitch in singles, deliberately not the row length, so the pitch is visible.
    const PITCH_SINGLES: u16 = 64;
    /// Row length in floats.
    const ROW_FLOATS: u32 = 8;
    const REFOLD_POISON: u32 = 0xFEED_FACE;

    /// Owner A holds the source rows, owner B the destination rows; the frame is seeded the way the
    /// replay seeds it, so a word the loops never wrote is visible.
    fn refold_guest(old_count: u8, new_count: u8, length: u32) -> Guest {
        let mut g = Guest::single(BASE, 0xA000);
        g.put(
            crate::routing::UNITY_GAIN,
            1.0f32.to_bits().to_be_bytes().to_vec(),
        );
        g.set_u8(MIXER + CHANNELS, new_count).unwrap();
        g.set_u8(MIXER + MIRRORED_COUNT, 0xFF).unwrap();
        g.set_u32(PAIR + PAIR_BACK, OWNER_A).unwrap();
        g.set_u32(PAIR + PAIR_FRONT, OWNER_B).unwrap();
        g.set_u32(PAIR + PAIR_ROW_LENGTH, length).unwrap();
        g.set_u8(PAIR + PAIR_FOLD_COUNT, old_count).unwrap();
        g.set_u32(OWNER_A + ROW_BASE, SRC_ROWS).unwrap();
        g.set_u16(OWNER_A + ROW_STRIDE, PITCH_SINGLES).unwrap();
        g.set_u32(OWNER_B + ROW_BASE, DST_ROWS).unwrap();
        g.set_u16(OWNER_B + ROW_STRIDE, PITCH_SINGLES).unwrap();
        for row in 0..8u32 {
            for i in 0..ROW_FLOATS {
                let src = SRC_ROWS + row * 4 * u32::from(PITCH_SINGLES) + i * 4;
                let dst = DST_ROWS + row * 4 * u32::from(PITCH_SINGLES) + i * 4;
                g.set_u32(src, ((row + 1) as f32 * 10.0 + i as f32).to_bits())
                    .unwrap();
                g.set_u32(dst, REFOLD_POISON).unwrap();
            }
        }
        // The frame, seeded: 176 bytes below SP.
        for w in 0..(REFOLD_FRAME_BYTES / 4) {
            g.set_u32(SP - REFOLD_FRAME_BYTES + w * 4, REFOLD_POISON)
                .unwrap();
        }
        g
    }

    fn src_row(row: u32) -> Vec<f32> {
        (0..ROW_FLOATS)
            .map(|i| (row + 1) as f32 * 10.0 + i as f32)
            .collect()
    }
    fn dst_row(g: &Guest, row: u32) -> Vec<f32> {
        (0..ROW_FLOATS)
            .map(|i| {
                g.f32(DST_ROWS + row * 4 * u32::from(PITCH_SINGLES) + i * 4)
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn a_pair_with_no_rows_mirrors_its_count_into_the_mixer() {
        // The `stb r6,41(r3)` happens before the comparison, so it lands even when the counts differ
        // and the remap is skipped.
        let mut g = refold_guest(2, 4, 0);
        assert_eq!(refold_rows(&mut g, MIXER, PAIR, SP).unwrap(), 1);
        assert_eq!(
            g.u8(MIXER + MIRRORED_COUNT).unwrap(),
            2,
            "the old count was mirrored"
        );
        // No rows were touched, but the owners still swapped and the new count was recorded.
        assert_eq!(
            dst_row(&g, 0),
            vec![f32::from_bits(REFOLD_POISON); ROW_FLOATS as usize]
        );
        assert_eq!(
            g.u32(PAIR + PAIR_BACK).unwrap(),
            OWNER_B,
            "owners exchanged"
        );
        assert_eq!(g.u32(PAIR + PAIR_FRONT).unwrap(), OWNER_A);
        assert_eq!(g.u8(PAIR + PAIR_FOLD_COUNT).unwrap(), 4);
    }

    #[test]
    fn equal_counts_leave_everything_alone() {
        let mut g = refold_guest(4, 4, ROW_FLOATS);
        assert_eq!(refold_rows(&mut g, MIXER, PAIR, SP).unwrap(), 1);
        assert_eq!(
            g.u8(MIXER + MIRRORED_COUNT).unwrap(),
            0xFF,
            "a pair with rows mirrors nothing"
        );
        assert_eq!(g.u32(PAIR + PAIR_BACK).unwrap(), OWNER_A, "no swap");
        assert_eq!(
            g.u8(PAIR + PAIR_FOLD_COUNT).unwrap(),
            4,
            "and no count write"
        );
        assert_eq!(
            dst_row(&g, 0),
            vec![f32::from_bits(REFOLD_POISON); ROW_FLOATS as usize]
        );
    }

    #[test]
    fn it_remaps_the_rows_then_swaps_the_owners_and_records_the_count() {
        // 3 -> 5 is not a standard-layout pair, so the gather copies row by row at unity gain and
        // zeroes the destinations the sources ran out for.
        let mut g = refold_guest(3, 5, ROW_FLOATS);
        assert_eq!(refold_rows(&mut g, MIXER, PAIR, SP).unwrap(), 1);

        for row in 0..3u32 {
            assert_eq!(dst_row(&g, row), src_row(row), "row {row} copied");
        }
        for row in 3..5u32 {
            assert_eq!(
                dst_row(&g, row),
                vec![0.0f32; ROW_FLOATS as usize],
                "row {row} zeroed"
            );
        }
        // The sixth row is outside the new count and keeps its poison.
        assert_eq!(
            dst_row(&g, 5),
            vec![f32::from_bits(REFOLD_POISON); ROW_FLOATS as usize]
        );
        assert_eq!(g.u32(PAIR + PAIR_BACK).unwrap(), OWNER_B);
        assert_eq!(g.u32(PAIR + PAIR_FRONT).unwrap(), OWNER_A);
        assert_eq!(g.u8(PAIR + PAIR_FOLD_COUNT).unwrap(), 5);
    }

    #[test]
    fn the_row_pointers_are_built_with_the_stride_as_a_byte_pitch() {
        // The stride is in singles and the pitch is `rotlwi ...,2` of it, so row n sits
        // 4*stride bytes on. The frame words the loops wrote say so directly.
        let mut g = refold_guest(3, 5, ROW_FLOATS);
        refold_rows(&mut g, MIXER, PAIR, SP).unwrap();
        let frame = SP - REFOLD_FRAME_BYTES;
        for i in 0..3u32 {
            let want = SRC_ROWS + i * 4 * u32::from(PITCH_SINGLES);
            assert_eq!(
                g.u32(frame + REFOLD_SOURCE_ARRAY + i * 4).unwrap(),
                want,
                "source {i}"
            );
        }
        for i in 0..5u32 {
            let want = DST_ROWS + i * 4 * u32::from(PITCH_SINGLES);
            assert_eq!(
                g.u32(frame + REFOLD_DEST_ARRAY + i * 4).unwrap(),
                want,
                "dest {i}"
            );
        }
        // The fourth source word was never written: only `old_count` of them are.
        assert_eq!(
            g.u32(frame + REFOLD_SOURCE_ARRAY + 3 * 4).unwrap(),
            REFOLD_POISON
        );
        // And the back chain is in the frame's first word.
        assert_eq!(g.u32(frame).unwrap(), SP);
    }

    #[test]
    fn the_source_list_stops_at_eight_which_is_where_the_destination_array_starts() {
        // The two arrays **abut**: eight source words at +80, then the destination words at +112. So
        // a ninth source pointer would land on the first *destination* pointer, and the unsigned
        // clamp is what stops it. Observed with a new count of zero, because then the destination
        // loop does not run and cannot cover the evidence.
        let mut g = refold_guest(10, 0, ROW_FLOATS);
        refold_rows(&mut g, MIXER, PAIR, SP).unwrap();
        let frame = SP - REFOLD_FRAME_BYTES;
        for i in 0..8u32 {
            let want = SRC_ROWS + i * 4 * u32::from(PITCH_SINGLES);
            assert_eq!(
                g.u32(frame + REFOLD_SOURCE_ARRAY + i * 4).unwrap(),
                want,
                "source {i}"
            );
        }
        assert_eq!(
            REFOLD_SOURCE_ARRAY + 4 * REFOLD_ARRAY_WORDS,
            REFOLD_DEST_ARRAY,
            "they abut"
        );
        assert_eq!(
            g.u32(frame + REFOLD_DEST_ARRAY).unwrap(),
            REFOLD_POISON,
            "an unclamped ninth source would have written the first destination pointer here"
        );
    }

    #[test]
    fn ten_sources_for_two_destinations_feeds_both_and_zeroes_nothing() {
        let mut g = refold_guest(10, 2, ROW_FLOATS);
        refold_rows(&mut g, MIXER, PAIR, SP).unwrap();
        assert_eq!(dst_row(&g, 0), src_row(0));
        assert_eq!(dst_row(&g, 1), src_row(1));
    }

    #[test]
    fn a_new_count_of_zero_writes_no_destination_pointers() {
        let mut g = refold_guest(2, 0, ROW_FLOATS);
        refold_rows(&mut g, MIXER, PAIR, SP).unwrap();
        let frame = SP - REFOLD_FRAME_BYTES;
        assert_eq!(
            g.u32(frame + REFOLD_DEST_ARRAY).unwrap(),
            REFOLD_POISON,
            "loop skipped"
        );
        assert_eq!(
            dst_row(&g, 0),
            vec![f32::from_bits(REFOLD_POISON); ROW_FLOATS as usize]
        );
        // The swap and the count still happen.
        assert_eq!(g.u32(PAIR + PAIR_BACK).unwrap(), OWNER_B);
        assert_eq!(g.u8(PAIR + PAIR_FOLD_COUNT).unwrap(), 0);
    }

    fn taps(g: &mut Guest, values: &[f32; TAP_COUNT]) {
        for (k, v) in values.iter().enumerate() {
            g.set_u32(TAP_TABLE + 4 * k as u32, v.to_bits()).unwrap();
        }
    }

    fn owner(g: &mut Guest, at: u32, base: u32, stride_singles: u16) {
        g.set_u32(at + ROW_BASE, base).unwrap();
        g.set_u16(at + ROW_STRIDE, stride_singles).unwrap();
    }

    // --------------------------------------------------------------------- sub_82B3C668

    #[test]
    fn each_delta_is_spread_over_sixteen_taps_of_its_own_row() {
        let mut g = guest();
        owner(&mut g, OWNER_A, ROWS, 256); // 1024-byte rows
        taps(
            &mut g,
            &[
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
                16.0,
            ],
        );
        let deltas = BASE + 0x300;
        g.set_u32(deltas, 0.5f32.to_bits()).unwrap();
        g.set_u32(deltas + 4, (-2.0f32).to_bits()).unwrap();
        // Row 1 starts pre-loaded, so the add is visible as an add rather than as a store.
        for k in 0..TAP_COUNT as u32 {
            g.set_u32(ROWS + 1024 + 4 * k, 100.0f32.to_bits()).unwrap();
        }

        fold_deltas(&mut g, OWNER_A, deltas, 2).unwrap();

        for k in 0..TAP_COUNT {
            let tap = (k + 1) as f32;
            assert_eq!(
                g.f32(ROWS + 4 * k as u32).unwrap(),
                0.5 * tap,
                "row 0 tap {k}"
            );
            assert_eq!(
                g.f32(ROWS + 1024 + 4 * k as u32).unwrap(),
                100.0 + -2.0 * tap,
                "row 1 tap {k}"
            );
        }
        // The 17th single of each row is untouched: exactly sixteen taps, not a whole row.
        assert_eq!(g.u32(ROWS + 64).unwrap(), 0);
        // And both deltas are consumed.
        assert_eq!(g.f32(deltas).unwrap(), 0.0);
        assert_eq!(g.f32(deltas + 4).unwrap(), 0.0);
    }

    #[test]
    fn the_taps_are_applied_in_ascending_address_order() {
        // tap[k] is pool + 1028 + 4k, applied to row word k. A body that walked the table
        // backwards — which the lifted register order invites, f30 at +1028 and f0 at +1088 —
        // would put tap 15 on word 0.
        let mut g = guest();
        owner(&mut g, OWNER_A, ROWS, 256);
        let mut t = [0.0f32; TAP_COUNT];
        t[0] = 1.0;
        taps(&mut g, &t);
        let deltas = BASE + 0x300;
        g.set_u32(deltas, 4.0f32.to_bits()).unwrap();

        fold_deltas(&mut g, OWNER_A, deltas, 1).unwrap();

        assert_eq!(
            g.f32(ROWS).unwrap(),
            4.0,
            "the first tap lands on the first word"
        );
        assert_eq!(g.f32(ROWS + 60).unwrap(), 0.0, "and not on the sixteenth");
    }

    #[test]
    fn a_zero_count_writes_nothing_at_all() {
        let mut g = guest();
        owner(&mut g, OWNER_A, ROWS, 256);
        taps(&mut g, &[1.0; TAP_COUNT]);
        let deltas = BASE + 0x300;
        g.set_u32(deltas, 3.0f32.to_bits()).unwrap();
        g.set_u32(ROWS, 7.0f32.to_bits()).unwrap();

        fold_deltas(&mut g, OWNER_A, deltas, 0).unwrap();

        assert_eq!(g.f32(ROWS).unwrap(), 7.0);
        assert_eq!(
            g.f32(deltas).unwrap(),
            3.0,
            "the delta is not cleared either"
        );
    }

    #[test]
    fn the_fold_is_a_fused_multiply_add_into_the_accumulator() {
        // Same distinguishing input the scale kernels use: a product needing 48 bits, with the
        // accumulator holding the negation of its f32 rounding. Fused leaves the discarded bits;
        // a separate multiply and add leaves zero.
        let a = 1.0f32 + f32::EPSILON;
        let b = 1.0f32 - f32::EPSILON;
        let rounded = a * b;
        let fused = ((a as f64) * (b as f64) - rounded as f64) as f32;
        assert_ne!(fused, 0.0);

        let mut g = guest();
        owner(&mut g, OWNER_A, ROWS, 256);
        let mut t = [0.0f32; TAP_COUNT];
        t[0] = b;
        taps(&mut g, &t);
        let deltas = BASE + 0x300;
        g.set_u32(deltas, a.to_bits()).unwrap();
        g.set_u32(ROWS, (-rounded).to_bits()).unwrap();

        fold_deltas(&mut g, OWNER_A, deltas, 1).unwrap();

        assert_eq!(
            g.f32(ROWS).unwrap(),
            fused,
            "the multiply-add must round once, not twice"
        );
    }

    #[test]
    fn the_row_stride_is_in_singles_and_a_zero_stride_stacks_every_row() {
        // The pitch is `stride * 4` bytes. With stride 16 the rows abut at 64 bytes, so row 1's
        // taps start exactly where row 0's ended.
        let mut g = guest();
        owner(&mut g, OWNER_A, ROWS, 16);
        let mut t = [0.0f32; TAP_COUNT];
        t[0] = 1.0;
        taps(&mut g, &t);
        let deltas = BASE + 0x300;
        g.set_u32(deltas, 1.0f32.to_bits()).unwrap();
        g.set_u32(deltas + 4, 2.0f32.to_bits()).unwrap();

        fold_deltas(&mut g, OWNER_A, deltas, 2).unwrap();

        assert_eq!(g.f32(ROWS).unwrap(), 1.0);
        assert_eq!(g.f32(ROWS + 64).unwrap(), 2.0, "64 bytes on, not 1024");

        // Stride zero puts every row on the same 64 bytes, and the second delta accumulates on top.
        let mut h = guest();
        owner(&mut h, OWNER_A, ROWS, 0);
        taps(&mut h, &t);
        h.set_u32(deltas, 1.0f32.to_bits()).unwrap();
        h.set_u32(deltas + 4, 2.0f32.to_bits()).unwrap();
        fold_deltas(&mut h, OWNER_A, deltas, 2).unwrap();
        assert_eq!(h.f32(ROWS).unwrap(), 3.0, "1 then 2 into the same word");
    }

    // --------------------------------------------------------------------- sub_82B34E08

    fn mixer(g: &mut Guest, channels: u8, pending_off: u16) {
        g.set_u8(OBJECT + CHANNELS, channels).unwrap();
        g.set_u32(OBJECT + ACCUM, ACCUM_AT).unwrap();
        g.set_u16(OBJECT + PENDING_OFFSET, pending_off).unwrap();
        g.set_u8(OBJECT + DELTA_FLAG, 0).unwrap();
        g.set_u8(OBJECT + FORCE_BYTE, 0).unwrap();
        g.set_u32(PAIR + PAIR_BACK, OWNER_B).unwrap();
        g.set_u32(PAIR + PAIR_FRONT, OWNER_A).unwrap();
        owner(g, OWNER_A, ROWS, 256);
        owner(g, OWNER_B, ROWS + 0x2000, 256);
    }

    #[test]
    fn nothing_pending_nothing_flagged_and_no_force_returns_zero_without_a_store() {
        let mut g = guest();
        mixer(&mut g, 2, 0x800);
        g.set_u32(ACCUM_AT + 0x800, 0).unwrap();

        assert_eq!(flush_accumulator(&mut g, OBJECT, PAIR).unwrap(), 0);

        assert_eq!(
            g.u32(PAIR + PAIR_FRONT).unwrap(),
            OWNER_A,
            "the pair is not swapped"
        );
        assert_eq!(g.u32(PAIR + PAIR_BACK).unwrap(), OWNER_B);
    }

    #[test]
    fn the_force_byte_alone_is_enough_to_run_the_flush() {
        // Three independent triggers, and each one has to work on its own. The force byte is
        // `>= 255`, i.e. exactly 0xFF; 0xFE does not fire.
        for (force, expect) in [(0u8, 0u64), (254, 0), (255, 1)] {
            let mut g = guest();
            mixer(&mut g, 1, 0x800);
            g.set_u8(OBJECT + FORCE_BYTE, force).unwrap();
            assert_eq!(
                flush_accumulator(&mut g, OBJECT, PAIR).unwrap(),
                expect,
                "force {force}"
            );
        }
        // And the delta flag on its own.
        let mut g = guest();
        mixer(&mut g, 1, 0x800);
        g.set_u8(OBJECT + DELTA_FLAG, 1).unwrap();
        taps(&mut g, &[0.0; TAP_COUNT]);
        assert_eq!(flush_accumulator(&mut g, OBJECT, PAIR).unwrap(), 1);
        assert_eq!(
            g.u8(OBJECT + DELTA_FLAG).unwrap(),
            0,
            "and it is cleared after the fold"
        );
    }

    #[test]
    fn a_pending_word_copies_the_accumulator_into_the_rows_and_then_clears_both() {
        let mut g = guest();
        mixer(&mut g, 2, 0x800);
        g.set_u32(ACCUM_AT + 0x800, 1).unwrap(); // pending
        // Channel 0's 1 KB of accumulator, and channel 1's.
        g.set_u32(ACCUM_AT, 0x1111_1111).unwrap();
        g.set_u32(ACCUM_AT + 1024, 0x2222_2222).unwrap();

        assert_eq!(flush_accumulator(&mut g, OBJECT, PAIR).unwrap(), 1);

        // The rows came from the accumulator, each channel from its own 1 KB slice.
        assert_eq!(g.u32(ROWS).unwrap(), 0x1111_1111, "row 0");
        assert_eq!(g.u32(ROWS + 1024).unwrap(), 0x2222_2222, "row 1");
        // The accumulator is zeroed afterwards, all 2 KB of it.
        assert_eq!(g.u32(ACCUM_AT).unwrap(), 0);
        assert_eq!(g.u32(ACCUM_AT + 1024).unwrap(), 0);
        assert_eq!(
            g.u32(ACCUM_AT + 0x800).unwrap(),
            0,
            "and so is the pending word"
        );
        // The pair is swapped.
        assert_eq!(g.u32(PAIR + PAIR_FRONT).unwrap(), OWNER_B);
        assert_eq!(g.u32(PAIR + PAIR_BACK).unwrap(), OWNER_A);
    }

    #[test]
    fn no_pending_word_clears_the_rows_instead_of_copying_them() {
        // The two loops are mutually exclusive, and the clearing one never reads the accumulator.
        let mut g = guest();
        mixer(&mut g, 2, 0x800);
        g.set_u32(ACCUM_AT + 0x800, 0).unwrap();
        g.set_u8(OBJECT + FORCE_BYTE, 255).unwrap(); // get past the early return
        g.set_u32(ACCUM_AT, 0x1111_1111).unwrap();
        g.set_u32(ROWS, 0x9999_9999).unwrap();
        g.set_u32(ROWS + 1024, 0x9999_9999).unwrap();

        assert_eq!(flush_accumulator(&mut g, OBJECT, PAIR).unwrap(), 1);

        assert_eq!(g.u32(ROWS).unwrap(), 0, "row 0 cleared, not copied");
        assert_eq!(g.u32(ROWS + 1024).unwrap(), 0, "row 1 cleared");
    }

    #[test]
    fn the_rows_belong_to_the_owner_read_before_the_swap() {
        // The single easiest thing to get wrong: read `pair + 32` after the swap and every row
        // lands on the other owner's buffers.
        let mut g = guest();
        mixer(&mut g, 1, 0x800);
        g.set_u32(ACCUM_AT + 0x800, 1).unwrap();
        g.set_u32(ACCUM_AT, 0xABCD_1234).unwrap();

        flush_accumulator(&mut g, OBJECT, PAIR).unwrap();

        assert_eq!(
            g.u32(ROWS).unwrap(),
            0xABCD_1234,
            "owner A's rows, the pre-swap front"
        );
        assert_eq!(g.u32(ROWS + 0x2000).unwrap(), 0, "owner B's are untouched");
    }

    #[test]
    fn zero_channels_still_clears_the_pending_word_and_the_accumulator() {
        let mut g = guest();
        mixer(&mut g, 0, 0x800);
        g.set_u32(ACCUM_AT + 0x800, 1).unwrap();
        g.set_u32(ROWS, 0x9999_9999).unwrap();

        assert_eq!(flush_accumulator(&mut g, OBJECT, PAIR).unwrap(), 1);

        assert_eq!(g.u32(ROWS).unwrap(), 0x9999_9999, "no row loop ran");
        assert_eq!(
            g.u32(ACCUM_AT + 0x800).unwrap(),
            0,
            "but the pending word was cleared"
        );
        // `channels * 1024` is zero, so the closing memset writes nothing — which is why the
        // pending word above survives as a zero rather than being re-cleared by it.
        assert_eq!(g.u32(ACCUM_AT).unwrap(), 0);
    }

    #[test]
    fn the_deltas_are_folded_into_the_rows_after_the_copy_not_before() {
        // Ordering: the copy overwrites the row, then the fold adds into it. Folding first would be
        // erased by the copy and this test would see the accumulator's value alone.
        let mut g = guest();
        mixer(&mut g, 1, 0x800);
        g.set_u32(ACCUM_AT + 0x800, 1).unwrap();
        g.set_u32(ACCUM_AT, 10.0f32.to_bits()).unwrap();
        g.set_u8(OBJECT + DELTA_FLAG, 1).unwrap();
        g.set_u32(OBJECT + DELTAS, 1.0f32.to_bits()).unwrap();
        let mut t = [0.0f32; TAP_COUNT];
        t[0] = 5.0;
        taps(&mut g, &t);

        flush_accumulator(&mut g, OBJECT, PAIR).unwrap();

        assert_eq!(
            g.f32(ROWS).unwrap(),
            15.0,
            "10 copied in, then 1 * 5 folded on top"
        );
        assert_eq!(g.u8(OBJECT + DELTA_FLAG).unwrap(), 0);
        assert_eq!(
            g.f32(OBJECT + DELTAS).unwrap(),
            0.0,
            "the delta is consumed"
        );
    }

    #[test]
    fn the_closing_clear_covers_one_kilobyte_per_channel() {
        // `rotlwi r5,r11,10` on the channel byte. Three channels is 3 KB, so the byte just past
        // that has to survive.
        let mut g = guest();
        mixer(&mut g, 3, 0x900);
        g.set_u32(ACCUM_AT + 0x900, 0).unwrap();
        g.set_u8(OBJECT + FORCE_BYTE, 255).unwrap();
        g.set_u32(ACCUM_AT + 3 * 1024 - 4, 0x5555_5555).unwrap();
        g.set_u32(ACCUM_AT + 3 * 1024, 0x7777_7777).unwrap();

        flush_accumulator(&mut g, OBJECT, PAIR).unwrap();

        assert_eq!(
            g.u32(ACCUM_AT + 3 * 1024 - 4).unwrap(),
            0,
            "the last word of 3 KB"
        );
        assert_eq!(
            g.u32(ACCUM_AT + 3 * 1024).unwrap(),
            0x7777_7777,
            "and not one byte more"
        );
    }

    // --------------------------------------------------------------------- sub_82B443F8

    const STATE: u32 = BASE + 0x400;
    const LAYOUT: u32 = BASE + 0x480;
    const PLAN_AT: u32 = BASE + 0x4C0;
    const CHANBUF: u32 = BASE + 0x7000;

    fn fill_state(g: &mut Guest, floor: f32, position: f32, limit: f32) {
        g.set_u32(STATE + LIMIT_FLOOR, floor.to_bits()).unwrap();
        g.set_u32(STATE + POSITION, position.to_bits()).unwrap();
        g.set_u32(STATE + LIMIT, limit.to_bits()).unwrap();
        g.set_u8(STATE + IDLE_FLAG, 0xFF).unwrap();
    }

    fn plan(g: &mut Guest, channels: u8, stride_frames: u16) {
        g.set_u8(LAYOUT + LAYOUT_CHANNELS, channels).unwrap();
        g.set_u32(OBJECT + PLAN, PLAN_AT).unwrap();
        g.set_u32(PLAN_AT + PLAN_BUFFER, CHANBUF).unwrap();
        g.set_u16(PLAN_AT + PLAN_STRIDE, stride_frames).unwrap();
    }

    #[test]
    fn a_position_at_or_past_the_limit_zeroes_the_idle_byte_and_returns_zero() {
        let mut g = guest();
        fill_state(&mut g, 0.0, 5.0, 5.0);
        plan(&mut g, 2, 64);
        g.set_u32(CHANBUF, 0x1234_5678).unwrap();
        g.set_u32(OBJECT + FRAMES_CLEARED, 0xDEAD).unwrap();

        assert_eq!(
            advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 8).unwrap(),
            0
        );

        assert_eq!(g.u8(STATE + IDLE_FLAG).unwrap(), 0);
        assert_eq!(
            g.f32(STATE + POSITION).unwrap(),
            5.0,
            "the position did not move"
        );
        assert_eq!(g.u32(CHANBUF).unwrap(), 0x1234_5678, "nothing was cleared");
        assert_eq!(
            g.u32(OBJECT + FRAMES_CLEARED).unwrap(),
            0xDEAD,
            "and nothing published"
        );
    }

    #[test]
    fn the_limit_is_clamped_up_to_its_floor_before_the_compare() {
        // A limit below the floor is raised, which can turn a "nothing to do" into work. That is
        // why the limit is reloaded after the clamp rather than kept in a register.
        let mut g = guest();
        fill_state(&mut g, 100.0, 5.0, 1.0); // limit 1 < position 5, but floor 100 > 5
        plan(&mut g, 1, 64);

        assert_eq!(
            advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 8).unwrap(),
            1
        );

        assert_eq!(
            g.f32(STATE + LIMIT).unwrap(),
            100.0,
            "the limit was raised and stored"
        );
        assert_eq!(g.f32(STATE + POSITION).unwrap(), 13.0, "5 + 8");
        assert_eq!(
            g.u8(STATE + IDLE_FLAG).unwrap(),
            0xFF,
            "the idle byte is not touched here"
        );
    }

    #[test]
    fn a_limit_already_above_its_floor_is_left_alone() {
        let mut g = guest();
        fill_state(&mut g, 1.0, 0.0, 50.0);
        plan(&mut g, 1, 64);
        advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 4).unwrap();
        assert_eq!(g.f32(STATE + LIMIT).unwrap(), 50.0);
    }

    #[test]
    fn a_nan_on_either_compare_takes_the_branch_the_unordered_form_takes() {
        // `fcmpu ; bge` and `fcmpu ; blt` both fall through to the *skip* on an unordered result.
        // A NaN limit therefore skips the clamp; a NaN position returns zero.
        let mut g = guest();
        fill_state(&mut g, 1.0, 0.0, f32::NAN);
        plan(&mut g, 1, 64);
        assert_eq!(
            advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 4).unwrap(),
            0
        );
        assert!(
            g.f32(STATE + LIMIT).unwrap().is_nan(),
            "the clamp store was skipped"
        );
        assert_eq!(
            g.u8(STATE + IDLE_FLAG).unwrap(),
            0,
            "and it took the idle exit"
        );

        let mut h = guest();
        fill_state(&mut h, 1.0, f32::NAN, 50.0);
        plan(&mut h, 1, 64);
        assert_eq!(
            advance_and_clear(&mut h, OBJECT, STATE, LAYOUT, 4).unwrap(),
            0
        );
        assert!(
            h.f32(STATE + POSITION).unwrap().is_nan(),
            "and the position did not move"
        );
    }

    #[test]
    fn every_channel_is_cleared_at_its_own_stride() {
        let mut g = guest();
        fill_state(&mut g, 0.0, 0.0, 1000.0);
        plan(&mut g, 3, 64); // 64 frames apart, i.e. 256 bytes
        for i in 0..3u32 {
            for w in 0..8u32 {
                g.set_u32(CHANBUF + 256 * i + 4 * w, 0x9999_9999).unwrap();
            }
        }

        assert_eq!(
            advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 4).unwrap(),
            1
        );

        for i in 0..3u32 {
            for w in 0..4u32 {
                assert_eq!(
                    g.u32(CHANBUF + 256 * i + 4 * w).unwrap(),
                    0,
                    "channel {i} word {w}"
                );
            }
            assert_eq!(
                g.u32(CHANBUF + 256 * i + 16).unwrap(),
                0x9999_9999,
                "channel {i}: only `count` frames"
            );
        }
        assert_eq!(g.u32(OBJECT + FRAMES_CLEARED).unwrap(), 4);
        assert_eq!(g.f32(STATE + POSITION).unwrap(), 4.0);
    }

    #[test]
    fn zero_channels_clears_nothing_but_still_publishes_the_count() {
        let mut g = guest();
        fill_state(&mut g, 0.0, 0.0, 1000.0);
        plan(&mut g, 0, 64);
        g.set_u32(CHANBUF, 0x9999_9999).unwrap();

        assert_eq!(
            advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 4).unwrap(),
            1
        );

        assert_eq!(g.u32(CHANBUF).unwrap(), 0x9999_9999);
        assert_eq!(
            g.u32(OBJECT + FRAMES_CLEARED).unwrap(),
            4,
            "published anyway"
        );
        assert_eq!(
            g.f32(STATE + POSITION).unwrap(),
            4.0,
            "and the position still moved"
        );
    }

    #[test]
    fn the_count_reaches_the_position_through_a_single_rounding() {
        // `extsw ; fcfid ; frsp` — the count is sign-extended, converted exactly, then **narrowed
        // to a single**. Past 2^24 that narrowing is visible: 16_777_217 rounds to even.
        let mut g = guest();
        fill_state(&mut g, 0.0, 0.0, f32::INFINITY);
        plan(&mut g, 0, 64);
        advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 16_777_217).unwrap();
        assert_eq!(g.f32(STATE + POSITION).unwrap(), 16_777_216.0);

        // And it is signed: a count with bit 31 set moves the position backwards.
        let mut h = guest();
        fill_state(&mut h, 0.0, 0.0, f32::INFINITY);
        plan(&mut h, 0, 64);
        advance_and_clear(&mut h, OBJECT, STATE, LAYOUT, 0xFFFF_FFFF).unwrap();
        assert_eq!(h.f32(STATE + POSITION).unwrap(), -1.0);
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest();
        fill_state(&mut g, 0.0, 0.0, 1000.0);
        plan(&mut g, 2, 64);
        let before = crate::vmx::get_mxcsr();
        advance_and_clear(&mut g, OBJECT, STATE, LAYOUT, 4).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);

        let mut h = guest();
        mixer(&mut h, 1, 0x800);
        h.set_u32(ACCUM_AT + 0x800, 1).unwrap();
        flush_accumulator(&mut h, OBJECT, PAIR).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }

    // --------------------------------------------------------------------- the shared idioms

    #[test]
    fn the_row_offset_keeps_a_sixty_four_bit_product_and_then_takes_its_low_word() {
        assert_eq!(row_offset(256, 0), 0);
        assert_eq!(row_offset(256, 1), 1024);
        assert_eq!(row_offset(256, 3), 3072);
        // A stride large enough to overflow the word: the product is formed in 64 bits and only
        // its low word is scaled, so this wraps rather than saturating.
        assert_eq!(row_offset(0x4000_0000, 2), 0);
        // And the scale clears the low two bits after the shift, not before.
        assert_eq!(row_offset(1, 0x3FFF_FFFF), 0xFFFF_FFFC);
    }
}
