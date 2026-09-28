//! Four small verified leaves, each reached from a different mechanism.
//!
//! They share a file because none of them belongs to a larger one that is ported yet, and a
//! module per two-instruction function would hide them rather than organise them. Each is
//! ported from its `recomp/src/audio_ports/sub_*.inc`, all four **STATUS: verified** —
//! compared against the original body call for call under the shadow harness.
//!
//! | function | guest | lifted lines | calls/boot | calls/play | reached from |
//! |---|---|---|---|---|---|
//! | [`stamp_slot`] | `sub_82B463A8` | 31 | 761,054 | 1,137,330 | a command-ring record's handler slot |
//! | [`set_field_460`] | `sub_82B34268` | 11 | 202,510 | 304,676 | a direct `bl` |
//! | [`fourth_argument`] | `sub_82B2C8E8` | 7 | 123,881 | 225,599 | a pointer slot only |
//! | [`set_field_364`] | `sub_82B29278` | 11 | 1,684 | 1,685 | a direct `bl` |
//! | [`five_point_ramp`] | `sub_82B2FE00` | 113 | 8 | 8 | a direct `bl` — **thin** |
//! | [`stream_remaining`] | `sub_82B23C10` | 47 | 122,997 | 214,853 | five direct callers |
//! | [`pair_record_size`] | `sub_82B4F8A8` | 17 | 1,930 | 2,166 | a direct `bl` |
//! | [`publish_float`] | `sub_82B49268` | 17 | 745 | 981 | a command-ring record |
//! | [`zero_two_fields`] | `sub_82B3D578` | 18 | 10 | 11 | a direct `bl` — **thin** |
//! | [`copy_and_mark_filled`] | `sub_82B1D840` | 40 | 247 | 343 | a direct `bl` |
//! | [`push_node`] | `sub_82B34B10` | 36 | 284 | 317 | a command-ring record |
//! | [`publish_command`] | `sub_82B23828` | 77 | 1,910 | 1,495 | a command-ring record |
//!
//! **Replayed against the game, 2026-09-13: 6,661 recorded vectors, 0 disagreements**
//! (3,232 + 230 + 230 + 2,969, sessions `leaves` and `leaves2`). The four were recorded on purpose
//! — no earlier capture contained them — by naming them in `AUDIO_VECTORS_ONLY`, and the same
//! sessions compared them live against the original 672,366 / 161,603 / 99,945 / 97,943 times with
//! zero divergence. One limit: the recording keeps only the low word of `r3`, so
//! [`stream_remaining`]'s borrow into the upper word is checked by the tests below and not by any
//! vector.
//!
//! The six added later — [`pair_record_size`] through [`publish_command`] — replay **6,596 recorded
//! calls, 0 disagreements** (session `leaves3`, 2,619 / 1,404 / 11 / 924 / 397 / 1,241); the thin
//! [`zero_two_fields`] carries only eleven, which is all a session produces.
//!
//! Two of the four objects are unnamed. `docs/rw_audio_structs.h` has no entry for the slot
//! array at `+0x10`, the `u16` at `+460`, or the stream table [`stream_remaining`] walks, so
//! every offset below is a plain constant with the instruction that produced it beside it.

use crate::{Guest, Result, fp};

/// A command-ring record: `{+0 handler, +4 object, +8 index, +12 value}`, 16 bytes.
///
/// The producer at `skate3_recomp.11.cpp:59366` stores [`stamp_slot`]'s own address at `+0` and
/// advances the ring by 16; the drain advances by whatever the handler returns, which is why
/// [`stamp_slot`] returns [`RECORD_SIZE`] rather than a status.
pub const RECORD_OBJECT: u32 = 4;
/// Slot index, `u32`. Scaled by 8 in 32 bits — see [`stamp_slot`].
pub const RECORD_INDEX: u32 = 8;
/// The value to stamp, stored as a single.
pub const RECORD_VALUE: u32 = 12;
/// `li r3,16` — the record size the drain advances by.
pub const RECORD_SIZE: u32 = 16;

/// `object + 0x10` holds the **base of** an array of 8-byte slots `{marker u32, value f32}`.
pub const OBJECT_SLOTS: u32 = 16;
/// One slot: `{+0 marker u32, +4 value f32}`.
pub const SLOT_STRIDE: u32 = 8;

/// `lis r10,32759 ; ori r8,r10,65521` — computed as `((imm & 0xFFFF) << 16) | imm`, never read
/// off. The same NaN payload `docs/command-queue.md` sees stamped by the query path of
/// `sub_82B28A00`; here it sits beside an arbitrary float rather than a 0.0/1.0 constant, so what
/// it *means* is not settled. Reproduced, not interpreted.
pub const SLOT_MARKER: u32 = ((32759u32) << 16) | 65521u32;

const _: () = assert!(SLOT_MARKER == 0x7FF7_FFF1, "lis 32759 ; ori 65521");

/// `sub_82B34268`'s only store. Nothing names `+460` (`0x1CC`); `sub_82B29278` is the same shape
/// at `+364`.
pub const FIELD_460: u32 = 460;

/// Stamp `{`[`SLOT_MARKER`]`, value}` into slot `index` of the record's object, and return the
/// record size (`sub_82B463A8`).
///
/// The hottest command handler seen on `RwAudioCore Dac`: 1,137,330 calls in a played session. It
/// is never the target of a `bl` anywhere in the corpus — its address only ever appears as a
/// record's `+0`, so it runs through the ring's dispatch.
///
/// Three details are load-bearing, and each is the reason a plausible rewrite would be wrong:
///
/// - **The value round-trips through a double.** `lfs f0,12(r3)` widens the single, `stfs f0,4(r11)`
///   narrows it back. That is value-preserving for every finite single, and *not* for a signalling
///   NaN, which comes back quiet, and not for a denormal, which is **flushed to zero**. The guest
///   emits `disableFlushModeUnconditional` at the load, and that does not mean what it says:
///   both of RexGlue's modes carry `FZ|DAZ` (see [`crate::vmx::Fpscr`]), so `DAZ` reads the
///   denormal single as zero on the way in. That is exactly why this function is x86-only here and
///   holds an [`Fpscr`](crate::vmx::Fpscr) — a build without one would run under Rust's default
///   MXCSR and *preserve* a denormal the recomp destroys.
/// - **`index << 3` is 32-bit.** The `rlwinm r10,r9,3,0,28` drops the top three bits of the index,
///   so an index at or above 2^29 wraps — identically in both bodies.
/// - **Store order is value then marker.** A concurrent reader can see the slot half-written, and
///   which half it sees is observable. Preserved, not tidied.
///
/// An object of 0, or a slot base of 0, returns `Err` here: the store would go through a
/// null-based address. The harness declines those calls too (`Windows()` returns false), so no
/// recorded vector can contain one, and this is the one input where the two bodies part company.
#[cfg(target_arch = "x86_64")]
pub fn stamp_slot(g: &mut Guest, record: u32) -> Result<u64> {
    let object = g.u32(record + RECORD_OBJECT)?; // lwz r11,4(r3)
    let index = g.u32(record + RECORD_INDEX)?; // lwz r9,8(r3)

    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f0,12(r3)
    let value = fp::single_from_bits(g.u32(record + RECORD_VALUE)?); // lfs f0,12(r3)

    let slots = g.u32(object + OBJECT_SLOTS)?; // lwz r11,16(r11)
    let slot = slots.wrapping_add(index << 3); // add r11,r11,r10

    fp::store_single(g, slot + 4, value)?; // stfs f0,4(r11)
    g.set_u32(slot, SLOT_MARKER)?; // stw r8,0(r11)
    drop(fpscr);
    Ok(u64::from(RECORD_SIZE)) // li r3,16
}

/// Store `value` as a `u16` at `object + 460`, and return 0 (`sub_82B34268`).
///
/// Eleven lifted lines, two of which do anything: `sth r6,460(r11)` and `li r3,0`. The value is
/// the **fourth** argument in the guest — `r4` and `r5` are untouched — which is the kind of
/// detail a rewrite from the role line alone gets wrong.
pub fn set_field_460(g: &mut Guest, object: u32, value: u16) -> Result<u64> {
    g.set_u16(object + FIELD_460, value)?; // sth r6,460(r11)
    Ok(0) // li r3,0
}

/// `sub_82B29278`'s only store. The same shape as [`set_field_460`], at `+364` (`0x16C`); nothing
/// names either offset.
pub const FIELD_364: u32 = 364;

/// Store `value` as a `u16` at `object + 364`, and return 0 (`sub_82B29278`) — [`set_field_460`]'s
/// twin: `sth r6,364(r11)` and `li r3,0`, with the value in the guest's fourth argument.
pub fn set_field_364(g: &mut Guest, object: u32, value: u16) -> Result<u64> {
    g.set_u16(object + FIELD_364, value)?; // sth r6,364(r11)
    Ok(0) // li r3,0
}

/// Return the fourth argument unchanged (`sub_82B2C8E8`).
///
/// `mr r3,r6 ; blr`, and nothing else: no loads, no stores, no branches, no condition register.
/// It is here because it is not *nothing* — 225,599 calls in a played session, reached only
/// through a pointer slot, and the move is **64-bit**, so whatever a caller left in the upper half
/// of `r6` is carried into `r3` verbatim. A `u32` signature would be a divergence the first time a
/// caller left junk above bit 31, which is why this takes and returns `u64`.
///
/// What it is *for* is unknown. Its neighbours look like slots of one small class — `82B2C8C0` is
/// `li r3,44`, a size — but nothing here verifies that, so the description says only what the two
/// instructions say.
pub fn fourth_argument(r6: u64) -> u64 {
    r6 // mr r3,r6
}

/// `+28` — the live cursor of whichever stream is currently active.
pub const ACTIVE_CURSOR: u32 = 28;
/// `+36` — a byte offset **from the object**, not a pointer, of the element array.
pub const STREAM_TABLE: u32 = 36;
/// `+49` — the index of the currently active stream, one byte.
pub const ACTIVE_INDEX: u32 = 49;
/// The element array's stride.
pub const ELEMENT_STRIDE: u32 = 24;
/// Per element: `+8` that stream's saved cursor, stale while it is the active one.
pub const ELEMENT_CURSOR: u32 = 8;
/// Per element: `+12` its limit. Zero means the element is unbound.
pub const ELEMENT_LIMIT: u32 = 12;

/// Address of element `index`, from entry state alone.
///
/// `rlwinm r11,r4,1,23,30 ; add r11,r10,r11 ; rlwinm r11,r11,3,0,28` is exactly `24 * index` for
/// an index of 255 or less, which the `clrlwi` guarantees. The lifted adds are 64-bit and the load
/// truncates to 32, and addition modulo 2^32 is associative, so `u32` arithmetic here is the same
/// effective address rather than an approximation of one.
pub fn element_address(g: &Guest, object: u32, index: u8) -> Result<u32> {
    let table = g.u32(object + STREAM_TABLE)?; // lwz r9,36(r3)
    Ok(object
        .wrapping_add(table)
        .wrapping_add(u32::from(index) * ELEMENT_STRIDE))
}

/// How much of stream `index` is unconsumed: its limit minus its cursor (`sub_82B23C10`).
///
/// Reached as `Player + 0x150` (`rw_player.decoder`) from `sub_82B29288`, with the index byte
/// coming out of the Player's `+0x54` table. Only the low byte of the guest's `r4` is used
/// (`clrlwi r10,r4,24`), which is why this takes a `u8`.
///
/// Three behaviours worth stating, because each is a branch a rewrite can flatten:
///
/// - **An unbound element short-circuits.** Limit 0 returns 0 without reading the cursor at all —
///   before the active-index test, so a zero limit wins even for the active stream.
/// - **The active stream's saved cursor is stale.** When `index` equals the byte at `+49`, the
///   cursor comes from the object's `+28`, not from the element. The comparison is unsigned on two
///   zero-extended bytes.
/// - **The subtraction is 64-bit, and it can borrow.** `subf r3,r10,r11` on two zero-extended
///   words, so a cursor past the limit leaves `0xFFFF_FFFF_xxxx_xxxx` in `r3` — which the callers
///   read back as a negative `i32`. Returning `u32` here would erase that, so this returns `u64`.
pub fn stream_remaining(g: &Guest, object: u32, index: u8) -> Result<u64> {
    let element = element_address(g, object, index)?;

    let limit = u64::from(g.u32(element + ELEMENT_LIMIT)?); // lwz r11,12(r9)
    if limit == 0 {
        return Ok(0); // cmpwi cr6,r11,0 ; li r3,0
    }

    let active = g.u8(object + ACTIVE_INDEX)?; // lbz r8,49(r3)
    let cursor = if index == active {
        // cmplw cr6,r10,r8 -- the live cursor, the element's copy being stale
        u64::from(g.u32(object + ACTIVE_CURSOR)?) // lwz r10,28(r3)
    } else {
        u64::from(g.u32(element + ELEMENT_CURSOR)?) // lwz r10,8(r9)
    };

    Ok(limit.wrapping_sub(cursor)) // subf r3,r10,r11
}

// --------------------------------------------------------------- more small verified leaves

/// `li r9,16` — the alignment [`pair_record_size`] reports through `r4`.
pub const PAIR_RECORD_ALIGNMENT: u32 = 16;
/// `addi r3,r11,88` — the record's fixed header.
pub const PAIR_RECORD_HEADER: u64 = 88;
/// `mulli r11,r10,28` — bytes per pair of entries.
pub const PAIR_RECORD_STRIDE: u64 = 28;

/// The size of a record holding `count` entries two to a slot, after an 88-byte header
/// (`sub_82B4F8A8`). Stores the alignment, 16, through `out`, and returns `28 * ceil(count/2) + 88`.
///
/// The halving is `addi` then a 32-bit logical shift right, so a count of `0xFFFF_FFFF` wraps to zero
/// slots rather than to 2^31.
pub fn pair_record_size(g: &mut Guest, count: u32, out: u32) -> Result<u64> {
    let pairs = count.wrapping_add(1) >> 1; // addi r11,r3,1 ; rlwinm r10,r11,31,1,31
    g.set_u32(out, PAIR_RECORD_ALIGNMENT)?; // stw r9,0(r4)
    Ok(u64::from(pairs) * PAIR_RECORD_STRIDE + PAIR_RECORD_HEADER) // mulli ; addi
}

/// `lis -32234 ; lfs 23056` — the image's zero single, the cell [`crate::mix::ZERO_SINGLE`] names.
pub const ZERO_CELL: u32 = (((-32234i32 as u32) & 0xFFFF) << 16).wrapping_add(23056);
const _: () = assert!(ZERO_CELL == 0x8216_5A10, "lis -32234 ; lfs 23056");

/// A publishing command record: `{+0 handler, +4 target, +8 value …}`.
pub const COMMAND_TARGET: u32 = 4;

/// Publish the single at `record + 8` onto `target + 56`, and return the record size, 12
/// (`sub_82B49268`). A command-ring handler, like [`stamp_slot`].
#[cfg(target_arch = "x86_64")]
pub fn publish_float(g: &mut Guest, record: u32) -> Result<u64> {
    let target = g.u32(record + COMMAND_TARGET)?; // lwz r11,4(r3)
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted before the lfs
    let value = fp::load_single(g, record + 8)?; // lfs f0,8(r3)
    fp::store_single(g, target.wrapping_add(56), value)?; // stfs f0,56(r11)
    drop(fpscr);
    Ok(12) // li r3,12
}

/// Store the image's zero into the two singles at `+16` and `+20` (`sub_82B3D578`, **thin**: ten
/// calls a session). The cell is read live rather than assumed to be `0.0`.
#[cfg(target_arch = "x86_64")]
pub fn zero_two_fields(g: &mut Guest, object: u32) -> Result<()> {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let zero = fp::load_single(g, ZERO_CELL)?; // lfs f0,23056(r11)
    fp::store_single(g, object.wrapping_add(16), zero)?; // stfs f0,16(r3)
    fp::store_single(g, object.wrapping_add(20), zero) // stfs f0,20(r3)
}

/// `lbz r10,24(r4)` — the destination's element count, re-read after every copied word.
pub const FILL_COUNT: u32 = 24;
/// `stb r11,25(r4)` — set to 1 on the way out, whether or not anything was copied.
pub const FILL_FLAG: u32 = 25;
/// Where the first copied word lands: `stwu 4(r9)` with `r9 = dest + 24`.
pub const FILL_FIRST_WORD: u32 = 28;

/// Copy `count` words from `source` into the destination block and mark it filled (`sub_82B1D840`).
///
/// A do-while: with a non-zero count the first word is copied before the count is tested, and the
/// count byte is **re-read after every word**. No store here can reach it — the copy ascends from
/// `+28` — so the reload is unobservable in practice, and it is kept because the original has it.
pub fn copy_and_mark_filled(g: &mut Guest, source: u32, dest: u32) -> Result<()> {
    if g.u8(dest + FILL_COUNT)? != 0 {
        let (mut from, mut to) = (source, dest.wrapping_add(FILL_FIRST_WORD));
        let mut copied = 0u32;
        loop {
            let word = g.u32(from)?; // lwzu r8,4(r10)
            g.set_u32(to, word)?; // stwu r8,4(r9)
            copied += 1;
            let count = g.u8(dest + FILL_COUNT)?; // lbz r8,24(r4) -- reloaded
            if copied as i32 >= i32::from(count) {
                break;
            }
            from = from.wrapping_add(4);
            to = to.wrapping_add(4);
        }
    }
    g.set_u8(dest + FILL_FLAG, 1) // li r11,1 ; stb r11,25(r4)
}

/// `lis -31988 ; lwz r9,-8520(r10)` — the head of a global singly-linked list.
pub const LIST_HEAD: u32 = (((-31988i32 as u32) & 0xFFFF) << 16).wrapping_sub(8520);
const _: () = assert!(LIST_HEAD == 0x830B_DEB8, "lis -31988 ; -8520");
/// The node on the object: `+44` next, `+48` a second word cleared on push.
pub const NODE_OFFSET: u32 = 44;
/// `stb r9,164(r8)` — set once the object is on the list.
pub const NODE_LINKED: u32 = 164;

/// Push the record's object onto the global list and return the record size, 8 (`sub_82B34B10`).
///
/// The head is loaded, the node's two words are written, and then the head is **loaded again** before
/// the previous head's back-link is set — the compiler could not prove the two stores miss the head
/// cell. Reproduced; the C++ window builder refuses the one layout where it matters.
pub fn push_node(g: &mut Guest, record: u32) -> Result<u64> {
    let object = g.u32(record + COMMAND_TARGET)?; // lwz r8,4(r3)
    let node = object.wrapping_add(NODE_OFFSET); // addi r11,r8,44
    let head = g.u32(LIST_HEAD)?; // lwz r9,-8520(r10)
    g.set_u32(object.wrapping_add(48), 0)?; // stw r7,48(r8)
    g.set_u32(node, head)?; // stw r9,44(r8)
    let head_again = g.u32(LIST_HEAD)?; // lwz r9,-8520(r10) -- reloaded
    if head_again != 0 {
        g.set_u32(head_again.wrapping_add(4), node)?; // stw r11,4(r9)
    }
    g.set_u32(LIST_HEAD, node)?; // stw r11,-8520(r10)
    g.set_u8(object.wrapping_add(NODE_LINKED), 1)?; // stb r9,164(r8)
    Ok(8) // li r3,8
}

/// `lis -32208 ; addi r8,r9,-31232 ; lfd f13,256(r8)` — the double a command's value is tested
/// against to choose the "clear" form.
pub const SENTINEL_DOUBLE: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232) + 256;
const _: () = assert!(
    SENTINEL_DOUBLE == 0x822F_8700,
    "lis -32208 ; addi -31232 ; lfd 256"
);

/// Publish a command's value onto its target in one of two forms, returning the record size, 32
/// (`sub_82B23828`).
///
/// The record is `{+4 target, +8 double, +16 single A, +20 single B, +24 word}`. When the double
/// equals the image's sentinel **and** single A equals the image's zero, the "clear" form stores B at
/// `+108` and zeroes the flag bytes at `+113` and `+112`. Otherwise the "set" form stores the double
/// at `+56`, A at `+64`, B at `+68`, the word at `+72`, and sets `+112`. The double comparison is
/// unordered, so a NaN takes the set form; and single A is only read when the double matched.
#[cfg(target_arch = "x86_64")]
pub fn publish_command(g: &mut Guest, record: u32) -> Result<u64> {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted before the lfd
    let bits = g.u64(record + 8)?; // lfd f0,8(r3)
    let target = g.u32(record + COMMAND_TARGET)?; // lwz r11,4(r3)
    let sentinel = fp::load_double(g, SENTINEL_DOUBLE)?; // lfd f13,256(r8)

    if f64::from_bits(bits) == sentinel
        && fp::load_single(g, record + 16)? == fp::load_single(g, ZERO_CELL)?
    {
        let b = fp::load_single(g, record + 20)?;
        fp::store_single(g, target.wrapping_add(108), b)?; // stfs f0,108(r11)
        g.set_u8(target.wrapping_add(113), 0)?; // stb r9,113(r11)
        g.set_u8(target.wrapping_add(112), 0)?; // stb r9,112(r11)
        return Ok(32);
    }
    g.set_u64(target.wrapping_add(56), bits)?; // stfd f0,56(r11)
    let a = fp::load_single(g, record + 16)?;
    fp::store_single(g, target.wrapping_add(64), a)?; // stfs f0,64(r11)
    let b = fp::load_single(g, record + 20)?;
    fp::store_single(g, target.wrapping_add(68), b)?; // stfs f13,68(r11)
    let word = g.u32(record + 24)?;
    g.set_u32(target.wrapping_add(72), word)?; // stw r8,72(r11)
    g.set_u8(target.wrapping_add(112), 1)?; // stb r9,112(r11)
    Ok(32) // li r3,32
}

// ------------------------------------------------------------------ the five-point ramp (thin)

/// `lis -32208 ; addi -31232` — the pool three of the ramp's constants sit in.
const RAMP_POOL: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232);
/// `lfs f13,468(r11)` — the rate's upper limit.
pub const RAMP_UPPER_LIMIT: u32 = RAMP_POOL + 468;
/// `lfs f0,472(r11)` — where the ramp starts when the span hits the ceiling.
pub const RAMP_BASE: u32 = RAMP_POOL + 472;
/// `lfs f12,476(r11)` — the rate written back when the span hits the ceiling.
pub const RAMP_START: u32 = RAMP_POOL + 476;
/// `lis -32250 ; lfs 3152` — the rate's lower limit.
pub const RAMP_LOWER_LIMIT: u32 = (((-32250i32 as u32) & 0xFFFF) << 16) + 3152;
/// `lis -32247 ; lfs -32180` — the rate-to-first-point scale.
pub const RAMP_SCALE_A: u32 = (((-32247i32 as u32) & 0xFFFF) << 16).wrapping_sub(32180);
/// `lis -32222 ; lfs 18868` — the first-point-to-span scale. **0x49B4, not 0x4BB4**: reading this
/// immediate by eye produced the project's first shadow divergence, which is why it is computed.
pub const RAMP_SCALE_B: u32 = (((-32222i32 as u32) & 0xFFFF) << 16) + 18868;
/// `lis -32241 ; lfs -10884` — the span's ceiling (the clipper's 100.0 cell).
pub const RAMP_CEILING: u32 = (((-32241i32 as u32) & 0xFFFF) << 16).wrapping_sub(10884);
/// `lis -32246 ; lfs -28032` — the step scale.
pub const RAMP_STEP_SCALE: u32 = (((-32246i32 as u32) & 0xFFFF) << 16).wrapping_sub(28032);
const _: () = assert!(RAMP_POOL == 0x822F_8600 && RAMP_UPPER_LIMIT == 0x822F_87D4);
const _: () = assert!(RAMP_SCALE_A == 0x8208_824C && RAMP_SCALE_B == 0x8222_49B4);
const _: () = assert!(RAMP_CEILING == 0x820E_D57C && RAMP_STEP_SCALE == 0x8209_9280);
const _: () = assert!(RAMP_LOWER_LIMIT == 0x8206_0C50);

/// Clamp a rate into range, then fill a five-point ramp plus its span (`sub_82B2FE00`, **thin**:
/// eight calls a session). `rate_field` is `r3`, `out` is `r4`; returns 1.
///
/// The rate is clamped in place — above the upper limit to it, and below the lower limit **or NaN**
/// to the lower limit, since the second compare is `bge` and an unordered rate fails it. Then the rate
/// is **re-read** from memory, scaled into a first point and a span, and if the span passes the
/// ceiling the ramp restarts from the pool's own base and the rate field is overwritten a second
/// time. The five points are an arithmetic progression from `first`, each `fadds` single-rounded from
/// the one before rather than computed as `first + k*step`, and the span is stored at `+20` before the
/// points are.
#[cfg(target_arch = "x86_64")]
pub fn five_point_ramp(g: &mut Guest, rate_field: u32, out: u32) -> Result<u64> {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let rate = fp::load_single(g, rate_field)?; // lfs f0,0(r3)
    let mut limit = fp::load_single(g, RAMP_UPPER_LIMIT)?; // lfs f13,468(r11)
    let mut clamp = rate > limit; // fcmpu ; bgt
    if !clamp {
        limit = fp::load_single(g, RAMP_LOWER_LIMIT)?; // lfs f13,3152(r10)
        clamp = !(rate >= limit); // bge -- so a NaN clamps, to the lower limit
    }
    if clamp {
        fp::store_single(g, rate_field, limit)?; // stfs f13,0(r3)
    }
    let clamped = fp::load_single(g, rate_field)?; // lfs f13,0(r3) -- reloaded
    let mut first = fp::mul_single(clamped, fp::load_single(g, RAMP_SCALE_A)?); // fmuls f0,f13,f0
    let mut span = fp::mul_single(first, fp::load_single(g, RAMP_SCALE_B)?); // fmuls f13,f0,f13
    let ceiling = fp::load_single(g, RAMP_CEILING)?; // lfs f12,-10884(r8)
    if span > ceiling {
        span = ceiling; // fmr f13,f12
        let start = fp::load_single(g, RAMP_START)?; // lfs f12,476(r11)
        first = fp::load_single(g, RAMP_BASE)?; // lfs f0,472(r11)
        fp::store_single(g, rate_field, start)?; // stfs f12,0(r3)
    }
    let width = fp::sub_single(span, first); // fsubs f12,f13,f0
    fp::store_single(g, out.wrapping_add(20), span)?; // stfs f13,20(r4)
    fp::store_single(g, out, first)?; // stfs f0,0(r4)
    let step = fp::mul_single(width, fp::load_single(g, RAMP_STEP_SCALE)?); // fmuls f11,f12,f13
    let mut point = first;
    for k in 1..5u32 {
        point = fp::add_single(
            if k == 1 { step } else { point },
            if k == 1 { first } else { step },
        );
        fp::store_single(g, out.wrapping_add(4 * k), point)?; // fadds ; stfs +4, +8, +12, +16
    }
    Ok(1) // li r3,1
}

// ---------------------------------------------------------------- sub_82B2FEA8: the table ramp

const _: () = assert!(RAMP_POOL == 0x822F_8600, "lis -32208 ; addi r6,r10,-31232");
/// `lfs f8,456(r6)` — scales every ramp point.
pub const TABLE_INPUT_SCALE: u32 = RAMP_POOL + 456;
/// `lfs f12,460(r6)` — above this the input is clamped to it and the gain rises.
pub const TABLE_THRESHOLD: u32 = RAMP_POOL + 460;
/// `lfs f9,464(r6)` — the gain's scale above the threshold.
pub const TABLE_HIGH_SCALE: u32 = RAMP_POOL + 464;
/// `lis -32206 ; lfs f10,-22460(r9)` — the baseline gain, and the bar a gain must clear to rescale.
pub const TABLE_BASELINE: u32 = (((-32206i32 as u32) & 0xFFFF) << 16).wrapping_sub(22460);
const _: () = assert!(TABLE_BASELINE == 0x8231_A844);
/// `lwz r9,1096(r3)` — the ascending table, reloaded every point.
pub const TABLE_POINTER: u32 = 1096;
/// `cmpwi cr6,r11,1652`.
pub const TABLE_ENTRIES: i32 = 1652;
/// `li r10,6 ; mtctr r10`.
pub const TABLE_POINTS: u32 = 6;

/// Map a six-point ramp through a 1,652-entry ascending table into six integers (`sub_82B2FEA8`).
/// Returns 1.
///
/// `object` is `r3`, `ramp` the six floats in `r4` (the block [`five_point_ramp`]'s family fills),
/// `out` the six words in `r5` and `input` `f1`. Only the **last** output word is cleared on entry.
///
/// Each point's target is `min(input, threshold) * ramp[k] * scale`, and the point stores the first
/// table entry above it, truncated to an integer. **The table cursor persists across the points**, so
/// the whole call is one forward sweep: a later point can never find an entry before an earlier one's,
/// and once the sweep runs off the end, a point stores nothing. Above the threshold every point is then
/// rescaled by `input * high_scale` — **read back out of memory**, so a point that stored nothing
/// rescales whatever its word already held.
pub fn table_ramp(g: &mut Guest, object: u32, ramp: u32, out: u32, input: f64) -> Result<u64> {
    g.set_u32(out.wrapping_add(20), 0)?; // stw r11,20(r5)
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let baseline = crate::fp::load_single(g, TABLE_BASELINE)?;
    let high_scale = crate::fp::load_single(g, TABLE_HIGH_SCALE)?;
    let threshold = crate::fp::load_single(g, TABLE_THRESHOLD)?;
    let input_scale = crate::fp::load_single(g, TABLE_INPUT_SCALE)?;
    let mut index: i32 = 0; // li r11,0 -- across all six points
    for point in 0..TABLE_POINTS {
        let slot = out.wrapping_add(4 * point);
        fpscr.disable_flush_mode_unconditional();
        let value = crate::fp::load_single(g, ramp.wrapping_add(4 * point))?; // lfsx f0,r7,r8
        let scaled = crate::fp::mul_single(value, input_scale); // fmuls f11,f0,f8
        // fcmpu cr6,f1,f12 ; ble -- a NaN input takes the else.
        let (selected, gain) = if input > threshold {
            (threshold, crate::fp::mul_single(input, high_scale)) // fmr f0,f12 ; fmuls f13,f1,f9
        } else {
            (input, baseline) // fmr f0,f1 ; fmr f13,f10
        };
        let table = g.u32(object.wrapping_add(TABLE_POINTER))?; // lwz r9,1096(r3)
        let target = crate::fp::mul_single(selected, scaled); // fmuls f0,f0,f11
        if index < TABLE_ENTRIES {
            let mut cursor = table.wrapping_add((index as u32) << 2);
            loop {
                let entry = crate::fp::load_single(g, cursor)?; // lfs f11,0(r10)
                if entry > target {
                    // The found entry is reloaded through the index rather than reused from f11.
                    let found = table.wrapping_add((index as u32) << 2);
                    index += 1;
                    let found_value = crate::fp::load_single(g, found)?; // lfsx f0,r10,r9
                    g.set_u32(slot, crate::fp::fctiwz_low_word(found_value))?; // fctiwz ; stfiwx
                    break;
                }
                index += 1;
                cursor = cursor.wrapping_add(4);
                if index >= TABLE_ENTRIES {
                    break;
                }
            }
        }
        // loc_82B2FF4C, on both paths.
        if gain > baseline {
            let stored = g.u32(slot)? as i32; // lwz r10,0(r8) ; extsw
            let widened = f64::from(f64::from(stored) as f32); // fcfid f11,f0 ; frsp f7,f11
            let rescaled = crate::fp::mul_single(widened, gain); // fmuls f6,f7,f13
            g.set_u32(slot, crate::fp::fctiwz_low_word(rescaled))?; // fctiwz f5,f6 ; stfiwx
        }
    }
    Ok(1) // li r3,1
}

// ------------------------------------------------------------ sub_82B2F798: settling the levels

/// `lwz r10,12(r31)` — a second object whose level this call accumulates into.
pub const LEVEL_LINKED: u32 = 12;
/// `lfs f9,40(r10)` — that object's level.
pub const LEVEL_LINKED_LEVEL: u32 = 40;
/// `stfs f6,32(r31)` — this object's level, overwritten with the new sum.
pub const LEVEL: u32 = 32;
/// `lwz r11,408(r3)` — the first term's count.
pub const LEVEL_FIRST_COUNT: u32 = 408;
/// `lfs f1,460(r3)` — the first term's `log10` argument.
pub const LEVEL_FIRST_VALUE: u32 = 460;
/// `addi r11,r31,1060` — the candidates, max-reduced from 0.0.
pub const LEVEL_CANDIDATES: u32 = 1060;
/// `addi r9,r31,1072` — the counts, max-reduced from 0.
pub const LEVEL_COUNTS: u32 = 1072;
/// `lbz r9,1089(r31)` — the trip count of both reductions.
pub const LEVEL_CANDIDATE_COUNT: u32 = 1089;
/// `lis -32231 ; lfs 25572` — 10.0.
pub const LEVEL_TEN: u32 = (((-32231i32 as u32) & 0xFFFF) << 16) + 25572;
const _: () = assert!(LEVEL_TEN == 0x8219_63E4);

/// `extsw ; std ; lfd ; fcfid ; frsp` — a signed word to a single.
fn word_to_single(word: u32) -> f64 {
    f64::from(f64::from(word as i32) as f32)
}

/// Settle two level terms and publish them (`sub_82B2F798`). `object` is `r3`.
///
/// Each term is `n - 10n / log10(x)`, single-rounded throughout: the first from the object's own
/// count and value, the second from the largest count and the largest candidate over up to 255 of
/// each (floors 0 and 0.0, NaN candidates never winning). Their sum becomes the object's level, and the
/// change from the old level is added into the linked object's.
///
/// The linked pointer is loaded only after both `log10` calls, and the linked level is read before
/// either store, so an object linked to itself sees the original's order: the linked store first, then
/// this one. The original's 128-byte frame and its conversion scratch are not reproduced; nothing
/// outside the call reads them.
pub fn settle_levels(g: &mut Guest, object: u32) -> Result<()> {
    use crate::fp::{add_single, div_single, load_single, mul_single, store_single, sub_single};
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let first_count = g.u32(object.wrapping_add(LEVEL_FIRST_COUNT))?; // lwz r11,408(r3)
    let first_value = load_single(g, object.wrapping_add(LEVEL_FIRST_VALUE))?; // lfs f1,460(r3)
    let n1 = word_to_single(first_count);
    let log1 = f64::from(crate::mathlib::log10(g, first_value)? as f32); // bl 0x82f55068 ; frsp
    fpscr.disable_flush_mode_unconditional();
    let count = g.u8(object.wrapping_add(LEVEL_CANDIDATE_COUNT))?; // lbz r9,1089(r31)
    let ten = load_single(g, LEVEL_TEN)?;
    let n1_ten = mul_single(n1, ten); // fmuls f11,f30,f31
    let mut largest = load_single(g, ZERO_CELL)?; // lfs f1,23056(r7)
    let term1 = sub_single(n1, div_single(n1_ten, log1)); // fdivs f10 ; fsubs f30
    if count != 0 {
        for k in 0..u32::from(count) {
            let candidate = load_single(g, object.wrapping_add(LEVEL_CANDIDATES + 4 * k))?;
            if candidate > largest {
                largest = candidate; // fcmpu ; ble ; fmr f1,f0
            }
        }
    }
    let mut most = 0i32; // li r10,0
    if count != 0 {
        // lbz r11,1089(r31) ; mtctr -- reloaded; nothing has been stored since, so it is `count`.
        let trips = u32::from(g.u8(object.wrapping_add(LEVEL_CANDIDATE_COUNT))?);
        for k in 0..trips {
            let value = g.u32(object.wrapping_add(LEVEL_COUNTS + 4 * k))? as i32;
            if value > most {
                most = value; // cmpw ; ble ; mr r10,r11
            }
        }
    }
    let n2 = word_to_single(most as u32);
    let log2 = f64::from(crate::mathlib::log10(g, largest)? as f32); // bl 0x82f55068 ; frsp
    fpscr.disable_flush_mode_unconditional();
    let level = load_single(g, object.wrapping_add(LEVEL))?; // lfs f11,32(r31)
    let n2_ten = mul_single(n2, ten); // fmuls f10,f29,f31
    let linked = g.u32(object.wrapping_add(LEVEL_LINKED))?; // lwz r10,12(r31)
    let linked_level = load_single(g, linked.wrapping_add(LEVEL_LINKED_LEVEL))?; // lfs f9,40(r10)
    let term2 = sub_single(n2, div_single(n2_ten, log2)); // fdivs f8 ; fsubs f7
    let sum = add_single(term2, term1); // fadds f6,f7,f30
    let delta = sub_single(sum, level); // fsubs f5,f6,f11
    store_single(
        g,
        linked.wrapping_add(LEVEL_LINKED_LEVEL),
        add_single(delta, linked_level),
    )?;
    store_single(g, object.wrapping_add(LEVEL), sum)?; // stfs f6,32(r31)
    Ok(())
}

// ---------------------------------------------------------- sub_82B2F2C8: resampling the bands

const BAND_TABLE: u32 = ((-32239i32 as u32) & 0xFFFF) << 16;
/// `lis -32239 ; addi 24992` — three segment edges: `+0` low, `+4` mid, `+8` high.
pub const BAND_EDGES: u32 = BAND_TABLE + 24992;
/// `addi 25008` — nine knot abscissae, then the rows the two segments blend between.
pub const BAND_KNOT_X: u32 = BAND_TABLE + 25008;
/// The row cursor, `BAND_KNOT_X + 104`, read at `-68` and `+4` and moved 72 bytes a segment.
pub const BAND_KNOT_ROWS: u32 = BAND_KNOT_X + 104;
const _: () = assert!(BAND_EDGES == 0x8211_61A0 && BAND_KNOT_X == 0x8211_61B0);
/// `lfs f0,1120(r9)` / `lfs f0,1124(r9)` — the lower and upper segments' rates.
pub const BAND_RATE_LOWER: u32 = RAMP_POOL + 1120;
/// The upper segment's rate.
pub const BAND_RATE_UPPER: u32 = RAMP_POOL + 1124;
/// `lis -32250 ; lfs 14920` — added to the last band before normalising.
pub const BAND_HEADROOM: u32 = (((-32250i32 as u32) & 0xFFFF) << 16) + 14920;
const _: () = assert!(BAND_HEADROOM == 0x8206_3A48);
/// `lfs f13,68(r3)` / `lfs f0,360(r3)` — equal levels end the call after the resample.
pub const BAND_CURRENT_LEVEL: u32 = 68;
/// The level the current one is compared with.
pub const BAND_TARGET_LEVEL: u32 = 360;
/// `addi r8,r3,392` — the pending sizes, a stride-12 cursor read at `-4`, `+0` and `+4`.
pub const BAND_PENDING_SIZES: u32 = 392;
/// `addi r10,r3,480` — the envelope slots, pre-incremented 36 bytes a store.
pub const BAND_ENVELOPES: u32 = 480;
/// `addi r11,r3,720` — two queues of three 60-byte reservation records, 180 bytes a queue.
pub const BAND_QUEUES: u32 = 720;
/// The abscissae's red-zone copy, `r1 - 80`; the blended ordinates follow at `r1 - 44`.
pub const BAND_RED_ZONE: u32 = 80;

/// `addi r7,r9,35 ; rlwinm r7,r7,0,0,26 ; add r10,r7,r10` — the size plus a four-byte header,
/// rounded up to 32, added to the write position.
fn reservation_needed(size: u32, pos: u32) -> u32 {
    (size.wrapping_add(35) & 0xFFFF_FFE0).wrapping_add(pos)
}

/// `subfc ; eqv ; rlwinm ; addze ; clrlwi ; cmplwi ; bne` — the carry plus sign-equality idiom,
/// kept in its raw form. It works out to a signed `limit >= needed`.
fn reservation_fits(limit: u32, needed: u32) -> bool {
    let carry = u32::from(limit >= needed);
    let signs_equal = !(needed ^ limit) >> 31;
    (signs_equal + carry) & 1 == 0
}

/// Resample six band values through a blended knot curve, normalise them, and re-arm the ring
/// reservations (`sub_82B2F2C8`). Returns 1.
///
/// `object` is `r3`, `out` the six singles in `r4`, `input` the six in `r6`, `band` is `f1` and `sp`
/// is `r1`. Where `band` falls between the three edges picks a segment and a blend weight; the nine
/// knot ordinates are the blend of two table rows, written with the abscissae into the red zone
/// below `sp`. Each input is then located by an **unbounded** walk up the abscissae — an input above
/// every knot walks on into the ordinates and past them, as in the original — and interpolated
/// linearly with one fused `fmadds`.
///
/// When the object's two levels differ, the bands are normalised by the larger of the current level
/// and the last band plus the headroom, six envelope slots are cleared, and each of the six
/// reservation records whose pending size fits is re-armed. Record 0 of each queue stores its five
/// fields in a different order from the other two; both orders are kept.
pub fn resample_bands(
    g: &mut Guest,
    object: u32,
    out: u32,
    input: u32,
    band: f64,
    sp: u32,
) -> Result<u64> {
    use crate::fp::{
        add_single, div_single, fmadd_single, load_single, mul_single, store_single, sub_single,
    };
    let xs = sp.wrapping_sub(BAND_RED_ZONE); // addi r8,r1,-80
    let ys = xs.wrapping_add(36); // addi r7,r1,-44
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let high_edge = load_single(g, BAND_EDGES + 8)?; // lfs f12,8(r11)
    let (mut span, mut rate, mut segment) = (0.0, 0.0, 0u32);
    let mut upper = !(band < high_edge); // fcmpu cr6,f1,f12 ; blt -- a NaN band takes the upper path
    let mut anchor = high_edge;
    if !upper {
        let low_edge = load_single(g, BAND_EDGES)?; // lfs f0,0(r11)
        let mid_edge = load_single(g, BAND_EDGES + 4)?; // lfs f13,4(r11)
        anchor = low_edge;
        if band > low_edge {
            anchor = band; // fmr f0,f1
            upper = band > mid_edge; // fcmpu cr6,f1,f13 ; bgt
        }
        if !upper {
            span = sub_single(mid_edge, anchor); // fsubs f13,f13,f0
            rate = load_single(g, BAND_RATE_LOWER)?; // lfs f0,1120(r9)
        }
    }
    if upper {
        span = sub_single(high_edge, anchor); // fsubs f13,f12,f0
        segment = 1;
        rate = load_single(g, BAND_RATE_UPPER)?; // lfs f0,1124(r9)
    }
    let weight = mul_single(span, rate); // fmuls f0,f13,f0
    let one = load_single(g, crate::routing::UNITY_GAIN)?; // lfs f11,0(r10)
    let nine = segment.wrapping_add((segment << 3) & 0xFFFF_FFF8); // rlwinm ; add r11,r11,r9
    let row = ((nine << 3) & 0xFFFF_FFF8).wrapping_add(BAND_KNOT_ROWS); // rlwinm r7 ; add r10
    let rest = sub_single(one, weight); // fsubs f13,f11,f0
    let mut walk = row;
    for k in 0..9u32 {
        let row_a = load_single(g, walk.wrapping_sub(68))?; // lfs f10,-68(r10)
        walk = walk.wrapping_add(4); // lfsu f12,4(r10)
        let row_b = load_single(g, walk)?;
        let scaled_b = mul_single(row_b, rest); // fmuls f9,f12,f13
        let knot_x = load_single(g, BAND_KNOT_X + 4 * k)?; // lfsx f8,r11,r9
        store_single(g, xs.wrapping_add(4 * k), knot_x)?; // stfsx f8,r11,r8
        store_single(
            g,
            ys.wrapping_add(4 * k),
            fmadd_single(row_a, weight, scaled_b),
        )?; // fmadds ; stfsx
    }
    let x1 = load_single(g, xs.wrapping_add(4))?; // lfs f0,-76(r1)
    let x0 = load_single(g, xs)?; // lfs f13,-80(r1)
    let inv_step = div_single(one, sub_single(x1, x0)); // fsubs f12 ; fdivs f12,f11,f12
    let mut cursor = out;
    for i in 0..6u32 {
        let value = load_single(g, input.wrapping_add(4 * i))?; // lfsx f13,r8,r9
        let (mut index, mut probe) = (0u32, xs);
        loop {
            probe = probe.wrapping_add(4); // lfsu f0,4(r10)
            let edge = load_single(g, probe)?;
            index = index.wrapping_add(1);
            if !(value > edge) {
                break; // fcmpu cr6,f13,f0 ; bgt -- unbounded, as the original
            }
        }
        let slot = (index << 2) & 0xFFFF_FFFC;
        let behind = sub_single(load_single(g, xs.wrapping_add(slot))?, value); // fsubs f13,f0,f13
        let y_below = load_single(g, ys.wrapping_sub(4).wrapping_add(slot))?; // lfsx f10 (r1-48)
        let y_at = load_single(g, ys.wrapping_add(slot))?; // lfsx f9 (r1-44)
        let frac = mul_single(behind, inv_step); // fmuls f8,f13,f12
        let ahead = sub_single(one, frac); // fsubs f7,f11,f8
        let lower = mul_single(y_below, frac); // fmuls f6,f10,f8
        store_single(g, cursor, fmadd_single(y_at, ahead, lower))?; // fmadds f5 ; stfs f5,0(r9)
        cursor = cursor.wrapping_add(4);
    }

    let current = load_single(g, object.wrapping_add(BAND_CURRENT_LEVEL))?;
    let reference = load_single(g, object.wrapping_add(BAND_TARGET_LEVEL))?;
    if current == reference {
        return Ok(1); // beq cr6,0x82b2f54c
    }
    let last = load_single(g, out.wrapping_add(20))?; // lfs f12,20(r4) -- reloaded
    let mut ceiling = add_single(last, load_single(g, BAND_HEADROOM)?); // fadds f0,f12,f0
    if current > ceiling {
        ceiling = current; // fmr f0,f13
    }
    let norm = div_single(one, ceiling); // fdivs f0,f11,f0
    let mut src = out.wrapping_sub(4); // addi r11,r4,-4
    let mut envelope = object.wrapping_add(BAND_ENVELOPES); // addi r10,r3,480
    let zero = load_single(g, ZERO_CELL)?; // lfs f13,23056(r9)
    for _ in 0..6 {
        envelope = envelope.wrapping_add(36); // stfsu f13,36(r10)
        store_single(g, envelope, zero)?;
        let raw = load_single(g, src.wrapping_add(4))?; // lfs f12,4(r11)
        src = src.wrapping_add(4); // stfsu f11,4(r11)
        store_single(g, src, mul_single(raw, norm))?;
    }

    let mut sizes = object.wrapping_add(BAND_PENDING_SIZES); // addi r8,r3,392
    let mut queue = object.wrapping_add(BAND_QUEUES); // addi r11,r3,720
    for _ in 0..2 {
        // Record 0: the size word BEFORE the cursor, and its own store order.
        let size = g.u32(sizes.wrapping_sub(4))?; // lwz r9,-4(r8)
        let pos = g.u32(queue.wrapping_add(4))?; // lwz r10,4(r11)
        let limit = g.u32(queue)?; // lwz r6,0(r11)
        if reservation_fits(limit, reservation_needed(size, pos)) {
            let limit_again = g.u32(queue)?; // lwz r7,0(r11)
            let pos_again = g.u32(queue.wrapping_add(4))?; // lwz r6,4(r11)
            g.set_u32(queue.wrapping_add(20), size.wrapping_add(1))?; // stw r10,20(r11)
            g.set_u32(queue.wrapping_add(16), 0)?; // stw r5,16(r11)
            g.set_u8(queue.wrapping_add(36), 0)?; // stb r5,36(r11)
            g.set_u32(queue.wrapping_add(12), limit_again)?; // stw r7,12(r11)
            g.set_u32(queue.wrapping_add(32), pos_again)?; // stw r6,32(r11)
        }
        // Records 1 and 2, sixty bytes apart, sizes at +0 and +4.
        for (rec, size_at) in [
            (queue.wrapping_add(60), sizes),
            (queue.wrapping_add(120), sizes.wrapping_add(4)),
        ] {
            let size = g.u32(size_at)?;
            let pos = g.u32(rec.wrapping_add(4))?;
            let limit = g.u32(rec)?;
            if reservation_fits(limit, reservation_needed(size, pos)) {
                g.set_u32(rec.wrapping_add(16), 0)?; // stw r5,76(r11)
                let limit_again = g.u32(rec)?;
                g.set_u32(rec.wrapping_add(12), limit_again)?; // stw r9,72(r11)
                let pos_again = g.u32(rec.wrapping_add(4))?;
                g.set_u32(rec.wrapping_add(32), pos_again)?; // stw r7,92(r11)
                g.set_u32(rec.wrapping_add(20), size.wrapping_add(1))?; // stw r10,80(r11)
                g.set_u8(rec.wrapping_add(36), 0)?; // stb r5,96(r11)
            }
        }
        sizes = sizes.wrapping_add(12); // addi r8,r8,12
        queue = queue.wrapping_add(180); // addi r11,r11,180
    }
    Ok(1) // li r3,1
}

// ------------------------------------------------------------ sub_82B2FF88: rearming the levels

/// `lis -32239 ; lfs 25224` — the one-pair layout's rate, and the three-pair layout's middle one.
pub const LEVEL_RATE_ONE: u32 = BAND_TABLE + 25224;
/// `lis -32239 ; lfs 25004` — the candidate that goes with it.
pub const LEVEL_CANDIDATE_ONE: u32 = BAND_TABLE + 25004;
/// `lfs 1104` on the ramp pool — the first pair's rate.
pub const LEVEL_RATE_FIRST: u32 = RAMP_POOL + 1104;
/// `lfs 1108` — the last pair's candidate.
pub const LEVEL_CANDIDATE_LAST: u32 = RAMP_POOL + 1108;
/// `lfs 1112` — the last pair's rate.
pub const LEVEL_RATE_LAST: u32 = RAMP_POOL + 1112;
/// `lis -32244 ; lfs -22064` — the first pair's candidate.
pub const LEVEL_CANDIDATE_FIRST: u32 = (((-32244i32 as u32) & 0xFFFF) << 16).wrapping_sub(22064);
/// `lis -32246 ; lfs -26788` — 0.5, the rounding half.
pub const LEVEL_HALF: u32 = (((-32246i32 as u32) & 0xFFFF) << 16).wrapping_sub(26788);
const _: () = assert!(LEVEL_RATE_ONE == 0x8211_6288 && LEVEL_CANDIDATE_ONE == 0x8211_61AC);
const _: () = assert!(LEVEL_RATE_FIRST == 0x822F_8A50 && LEVEL_CANDIDATE_FIRST == 0x820B_A9D0);
const _: () = assert!(LEVEL_HALF == 0x8209_975C);
/// `lfs f13,1100(r3)` — the source every count is scaled from.
pub const LEVEL_SOURCE: u32 = 1100;
/// `lbz r11,42(r3)` — the layout byte, [`crate::layout::LAYOUT`] on the same object.
pub const LEVEL_LAYOUT: u32 = 42;

/// `fcmpu` against the zero cell ; `blt` — below zero takes the half off, anything else (a NaN
/// included) adds it; then `fctiwz` and the low word.
fn round_away(value: f64, zero: f64, half: f64) -> u32 {
    let adjusted = if value < zero {
        crate::fp::sub_single(value, half)
    } else {
        crate::fp::add_single(value, half)
    };
    crate::fp::fctiwz_low_word(adjusted)
}

/// Rearm the level tables [`settle_levels`] reads (`sub_82B2FF88`). `object` is `r3`.
///
/// The layout byte picks one pair (layout 1), two (layouts 2 and 4) or three (anything else). Each
/// pair is a candidate single copied from rodata to `+1060..` and a count at `+1072..` that is the
/// object's `+1100` times a rodata rate, rounded half away from zero; the pair count goes to
/// `+1089`. The three-pair layout's middle pair is the one-pair layout's.
pub fn rearm_level_table(g: &mut Guest, object: u32) -> Result<()> {
    use crate::fp::{load_single, mul_single, store_single};
    let at = |offset: u32| object.wrapping_add(offset);
    let layout = g.u8(at(LEVEL_LAYOUT))?; // lbz r11,42(r3)
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let source = load_single(g, at(LEVEL_SOURCE))?; // lfs f13,1100(r3)
    if layout == 1 {
        let scaled = mul_single(source, load_single(g, LEVEL_RATE_ONE)?); // fmuls f0,f13,f0
        let zero = load_single(g, ZERO_CELL)?;
        let candidate = load_single(g, LEVEL_CANDIDATE_ONE)?;
        g.set_u8(at(LEVEL_CANDIDATE_COUNT), 1)?; // stb r8,1089(r3)
        store_single(g, at(LEVEL_CANDIDATES), candidate)?; // stfs f13,1060(r3)
        let half = load_single(g, LEVEL_HALF)?;
        g.set_u32(at(LEVEL_COUNTS), round_away(scaled, zero, half))?; // stw r11,1072(r3)
        return Ok(());
    }
    let zero = load_single(g, ZERO_CELL)?;
    let scaled_first = mul_single(source, load_single(g, LEVEL_RATE_FIRST)?); // lfs 1104 ; fmuls
    if layout == 2 || layout == 4 {
        // loc_82B3010C: the first and the last pair.
        g.set_u8(at(LEVEL_CANDIDATE_COUNT), 2)?; // stb r7,1089(r3)
        let candidate_first = load_single(g, LEVEL_CANDIDATE_FIRST)?;
        store_single(g, at(LEVEL_CANDIDATES), candidate_first)?; // stfs f11,1060(r3)
        let half = load_single(g, LEVEL_HALF)?;
        let first = round_away(scaled_first, zero, half);
        let rate_last = load_single(g, LEVEL_RATE_LAST)?; // lfs f0,1112(r10)
        g.set_u32(at(LEVEL_COUNTS), first)?; // stw r11,1072(r3)
        let scaled_last = mul_single(source, rate_last);
        let candidate_last = load_single(g, LEVEL_CANDIDATE_LAST)?;
        store_single(g, at(LEVEL_CANDIDATES + 4), candidate_last)?; // stfs f13,1064(r3)
        g.set_u32(at(LEVEL_COUNTS + 4), round_away(scaled_last, zero, half))?; // stw r11,1076(r3)
        return Ok(());
    }
    // Three pairs: the first, the one-pair layout's, and the last.
    g.set_u8(at(LEVEL_CANDIDATE_COUNT), 3)?; // stb r7,1089(r3)
    let candidate_first = load_single(g, LEVEL_CANDIDATE_FIRST)?; // lfs f11,-22064(r8)
    let half = load_single(g, LEVEL_HALF)?; // lfs f10,0(r11)
    store_single(g, at(LEVEL_CANDIDATES), candidate_first)?;
    g.set_u32(at(LEVEL_COUNTS), round_away(scaled_first, zero, half))?; // stw r11,1072(r3)
    let scaled_middle = mul_single(source, load_single(g, LEVEL_RATE_ONE)?);
    let candidate_one = load_single(g, LEVEL_CANDIDATE_ONE)?;
    store_single(g, at(LEVEL_CANDIDATES + 4), candidate_one)?; // stfs f11,1064(r3)
    let middle = round_away(scaled_middle, zero, half);
    let rate_last = load_single(g, LEVEL_RATE_LAST)?; // lfs f0,1112(r10)
    g.set_u32(at(LEVEL_COUNTS + 4), middle)?; // stw r11,1076(r3)
    let scaled_last = mul_single(source, rate_last);
    let candidate_last = load_single(g, LEVEL_CANDIDATE_LAST)?;
    store_single(g, at(LEVEL_CANDIDATES + 8), candidate_last)?; // stfs f13,1068(r3)
    g.set_u32(at(LEVEL_COUNTS + 8), round_away(scaled_last, zero, half))?; // stw r11,1080(r3)
    Ok(())
}

// ------------------------------------------------------------ sub_82B2F590: recommitting the filter

/// `lfs f0,52(r31)` — floored against [`RECOMMIT_FLOOR`] for the weights.
pub const RECOMMIT_PARAM_52: u32 = 52;
/// `stw r10,56(r31)` — poisoned with [`RECOMMIT_GUARD_WORD`] on the rate path.
pub const RECOMMIT_GUARD: u32 = 56;
/// `lfs f0,60(r3)` — the rate [`five_point_ramp`] clamps.
pub const RECOMMIT_RATE: u32 = 60;
/// `lfs f0,68(r31)` — [`resample_bands`]' current level.
pub const RECOMMIT_LEVEL: u32 = 68;
/// The four published copies: `+348` of `+1100`, `+352` of `+52`, `+356` of `+60`, `+360` of `+68`.
pub const RECOMMIT_PUBLISHED_BLEND: u32 = 348;
/// The published `+52`.
pub const RECOMMIT_PUBLISHED_52: u32 = 352;
/// The published rate.
pub const RECOMMIT_PUBLISHED_RATE: u32 = 356;
/// The published level.
pub const RECOMMIT_PUBLISHED_LEVEL: u32 = 360;
/// Six ramp singles, [`five_point_ramp`]'s output and [`table_ramp`]'s input.
pub const RECOMMIT_RAMP: u32 = 364;
/// Six table points, [`table_ramp`]'s output.
pub const RECOMMIT_POINTS: u32 = 388;
/// Six bands, [`resample_bands`]' output.
pub const RECOMMIT_BANDS: u32 = 436;
/// Six weights, `(1 - band) * (1 - floor / max(+52, floor))`.
pub const RECOMMIT_WEIGHTS: u32 = 460;
/// `lbz r10,1088(r31)` — zero selects [`rearm_level_table`] over the record loop.
pub const RECOMMIT_READY: u32 = 1088;
/// `lwz r11,1092(r3)` — 1 runs the function; it is left 2.
pub const RECOMMIT_STATE: u32 = 1092;
/// The per-voice records the loop rewrites, 60 bytes each from `+140`.
pub const RECOMMIT_RECORDS: u32 = 140;
/// `addi r9,r31,1068` — the word array, pre-incremented, so `+1072` is the first read.
pub const RECOMMIT_WORDS: u32 = 1068;
/// `stwu r1,-128(r1)`. Real: the rate is clamped in place at `r1 + 80`.
pub const RECOMMIT_FRAME: u32 = 128;
/// `lis r11,32759 ; ori r10,r11,65521`.
pub const RECOMMIT_GUARD_WORD: u32 = 0x7FF7_FFF1;
/// `lfs f13,1116(r10)` on the ramp pool — the floor.
pub const RECOMMIT_FLOOR: u32 = RAMP_POOL + 1116;
const _: () = assert!(RECOMMIT_FLOOR == 0x822F_8A5C);
/// The deepest stack the call reaches: its frame and [`resample_bands`]' red zone below it.
pub const RECOMMIT_STACK_DEPTH: u32 = RECOMMIT_FRAME + BAND_RED_ZONE;
const RECOMMIT_SCRATCH: u32 = 80;

/// Recommit a filter object whose state word says 1 (`sub_82B2F590`).
///
/// `object` is `r3`, `sp` is `r1`. Which of three parameters moved picks the work: a moved rate is
/// clamped in the frame by [`five_point_ramp`] and rebuilds the ramp, then remaps it through
/// [`table_ramp`] and resamples through [`resample_bands`]; a moved blend does the last two and
/// clears the ready byte; a moved level only resamples. Every compare is unordered-is-unequal. Then
/// the six weights are computed, every load before every store; then either
/// [`rearm_level_table`] (ready byte zero) or, only after the rate path, the per-voice record loop
/// whose bound is re-read every iteration and whose last store lands on the next record's first
/// byte. The four parameters are published last, the state word stored between two of their loads.
pub fn recommit_filter(g: &mut Guest, object: u32, sp: u32) -> Result<()> {
    use crate::fp::{div_single, load_single, mul_single, store_single, sub_single};
    let at = |offset: u32| object.wrapping_add(offset);
    let frame = sp.wrapping_sub(RECOMMIT_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-128(r1)
    if g.u32(at(RECOMMIT_STATE))? as i32 != 1 {
        return Ok(()); // bne cr6,0x82b2f790
    }
    let mut retuned = false;
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let rate = load_single(g, at(RECOMMIT_RATE))?;
    let rate_published = load_single(g, at(RECOMMIT_PUBLISHED_RATE))?;
    if rate != rate_published {
        let scratch = frame.wrapping_add(RECOMMIT_SCRATCH);
        store_single(g, scratch, rate)?; // stfs f0,80(r1)
        five_point_ramp(g, scratch, at(RECOMMIT_RAMP))?; // bl 0x82b2fe00
        fpscr.disable_flush_mode_unconditional();
        let clamped = load_single(g, scratch)?; // lfs f0,80(r1)
        store_single(g, at(RECOMMIT_RATE), clamped)?; // stfs f0,60(r31)
        g.set_u32(at(RECOMMIT_GUARD), RECOMMIT_GUARD_WORD)?; // stw r10,56(r31)
        let blend = load_single(g, at(LEVEL_SOURCE))?; // lfs f1,1100(r31)
        table_ramp(g, object, at(RECOMMIT_RAMP), at(RECOMMIT_POINTS), blend)?; // bl 0x82b2fea8
        fpscr.disable_flush_mode_unconditional();
        let blend = load_single(g, at(LEVEL_SOURCE))?; // reloaded
        resample_bands(
            g,
            object,
            at(RECOMMIT_BANDS),
            at(RECOMMIT_RAMP),
            blend,
            frame,
        )?; // bl 0x82b2f2c8
        retuned = true; // li r29,1
    } else {
        let blend_published = load_single(g, at(RECOMMIT_PUBLISHED_BLEND))?; // lfs f0,348(r31)
        let blend = load_single(g, at(LEVEL_SOURCE))?; // lfs f1,1100(r31)
        if blend != blend_published {
            table_ramp(g, object, at(RECOMMIT_RAMP), at(RECOMMIT_POINTS), blend)?;
            fpscr.disable_flush_mode_unconditional();
            let blend = load_single(g, at(LEVEL_SOURCE))?;
            resample_bands(
                g,
                object,
                at(RECOMMIT_BANDS),
                at(RECOMMIT_RAMP),
                blend,
                frame,
            )?;
            g.set_u8(at(RECOMMIT_READY), 0)?; // stb r30,1088(r31)
        } else {
            let level = load_single(g, at(RECOMMIT_LEVEL))?;
            let level_published = load_single(g, at(RECOMMIT_PUBLISHED_LEVEL))?;
            if level != level_published {
                // f1 is not reloaded here: it still holds the blend from the compare above.
                resample_bands(
                    g,
                    object,
                    at(RECOMMIT_BANDS),
                    at(RECOMMIT_RAMP),
                    blend,
                    frame,
                )?;
            }
        }
    }

    // loc_82B2F670: the six weights, every load before every store.
    fpscr.disable_flush_mode_unconditional();
    let mut param = load_single(g, at(RECOMMIT_PARAM_52))?;
    let floor = load_single(g, RECOMMIT_FLOOR)?;
    if !(param > floor) {
        param = floor; // bgt ; fmr f0,f13 -- a NaN is replaced
    }
    let ratio = div_single(floor, param); // fdivs f13,f13,f0
    let mut band = [0.0f64; 6];
    for (k, value) in band.iter_mut().take(4).enumerate() {
        *value = load_single(g, at(RECOMMIT_BANDS + 4 * k as u32))?;
    }
    let one = load_single(g, crate::routing::UNITY_GAIN)?; // lfs f0,-22460(r11)
    band[4] = load_single(g, at(RECOMMIT_BANDS + 16))?;
    band[5] = load_single(g, at(RECOMMIT_BANDS + 20))?;
    let rest = band.map(|b| sub_single(one, b));
    let depth = sub_single(one, ratio); // fsubs f2,f0,f13
    for (k, r) in rest.iter().enumerate() {
        store_single(
            g,
            at(RECOMMIT_WEIGHTS + 4 * k as u32),
            mul_single(*r, depth),
        )?;
    }

    if g.u8(at(RECOMMIT_READY))? == 0 {
        rearm_level_table(g, object)?; // bl 0x82b2ff88
    } else if retuned && g.u8(at(LEVEL_CANDIDATE_COUNT))? != 0 {
        let mut words = at(RECOMMIT_WORDS); // addi r9,r31,1068
        let mut record = at(RECOMMIT_RECORDS); // addi r11,r31,140
        let mut index = 0i32;
        loop {
            let from24 = g.u32(record.wrapping_add(24))?; // lwz r7,24(r11)
            index += 1;
            words = words.wrapping_add(4); // lwzu r8,4(r9)
            let word = g.u32(words)?;
            let from28 = g.u32(record.wrapping_add(28))?; // lwz r6,28(r11)
            g.set_u32(record.wrapping_add(40), 0)?; // stw r30,40(r11)
            g.set_u32(record.wrapping_add(36), from24)?; // stw r7,36(r11)
            g.set_u32(record.wrapping_add(44), word)?; // stw r8,44(r11)
            g.set_u32(record.wrapping_add(56), from28)?; // stw r6,56(r11)
            record = record.wrapping_add(60); // stbu r30,60(r11) -- the next record's +0
            g.set_u8(record, 0)?;
            if index >= i32::from(g.u8(at(LEVEL_CANDIDATE_COUNT))?) {
                break; // lbz r5,1089(r31) -- re-read ; cmpw ; blt
            }
        }
    }

    // loc_82B2F768: publish, the state word stored between the loads.
    fpscr.disable_flush_mode_unconditional();
    let final52 = load_single(g, at(RECOMMIT_PARAM_52))?;
    let final_rate = load_single(g, at(RECOMMIT_RATE))?;
    let final_level = load_single(g, at(RECOMMIT_LEVEL))?;
    g.set_u32(at(RECOMMIT_STATE), 2)?; // li r11,2 ; stw r11,1092(r31)
    let final_blend = load_single(g, at(LEVEL_SOURCE))?;
    store_single(g, at(RECOMMIT_PUBLISHED_52), final52)?;
    store_single(g, at(RECOMMIT_PUBLISHED_RATE), final_rate)?;
    store_single(g, at(RECOMMIT_PUBLISHED_LEVEL), final_level)?;
    store_single(g, at(RECOMMIT_PUBLISHED_BLEND), final_blend)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE + 0x100;
    const SLOTS: u32 = BASE + 0x400;
    const RECORD: u32 = BASE + 0x40;
    const POISON: u32 = 0xDEAD_BEEF;

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        for i in 0..0x400 / 4 {
            g.set_u32(SLOTS + i * 4, POISON).unwrap();
        }
        g.set_u32(OBJECT + OBJECT_SLOTS, SLOTS).unwrap();
        g
    }

    fn record(g: &mut Guest, index: u32, value_bits: u32) {
        g.set_u32(RECORD, 0x82B4_63A8).unwrap(); // the handler pointer the producer writes
        g.set_u32(RECORD + RECORD_OBJECT, OBJECT).unwrap();
        g.set_u32(RECORD + RECORD_INDEX, index).unwrap();
        g.set_u32(RECORD + RECORD_VALUE, value_bits).unwrap();
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_marker_and_the_value_land_in_the_indexed_slot() {
        for index in [0u32, 1, 7, 63] {
            let mut g = guest();
            record(&mut g, index, 0.375f32.to_bits());

            assert_eq!(stamp_slot(&mut g, RECORD).unwrap(), 16);

            let slot = SLOTS + index * SLOT_STRIDE;
            assert_eq!(g.u32(slot).unwrap(), 0x7FF7_FFF1, "index {index}: marker");
            assert_eq!(g.f32(slot + 4).unwrap(), 0.375, "index {index}: value");
            // Nothing outside the eight bytes, which is the whole declared window.
            if index > 0 {
                assert_eq!(g.u32(slot - 4).unwrap(), POISON, "index {index}: below");
            }
            assert_eq!(
                g.u32(slot + SLOT_STRIDE).unwrap(),
                POISON,
                "index {index}: above"
            );
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_index_is_scaled_by_eight_in_thirty_two_bits() {
        // The `rlwinm ...,3,0,28` drops the top three bits, so 2^29 aliases index 0 exactly. A
        // 64-bit scale would put this store 4 GB away and the test would fail on the mapping.
        let mut g = guest();
        record(&mut g, 1 << 29, 1.0f32.to_bits());
        stamp_slot(&mut g, RECORD).unwrap();
        assert_eq!(g.u32(SLOTS).unwrap(), 0x7FF7_FFF1);
        assert_eq!(g.f32(SLOTS + 4).unwrap(), 1.0);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_value_round_trips_through_a_double() {
        // Every finite single survives `lfs` then `stfs` exactly, which is what this asserts and all
        // it asserts.
        let mut g = guest();
        for bits in [
            0.375f32.to_bits(),
            1.0f32.to_bits(),
            (-2.5e30f32).to_bits(),
            0x0000_0000,
        ] {
            record(&mut g, 2, bits);
            stamp_slot(&mut g, RECORD).unwrap();
            assert_eq!(g.u32(SLOTS + 2 * SLOT_STRIDE + 4).unwrap(), bits);
        }

        // **What a denormal or a signalling NaN does here is a property of the build, not of this
        // port, and it is not settled.** Measured 2026-09-13 with `examples/flush_probe`: LLVM folds
        // `fptrunc(fpext(x))` into a no-op at `opt-level >= 1`, so the two conversions are simply
        // deleted and neither `DAZ` nor the hardware's sNaN quieting happens. At `opt-level = 0`
        // both execute, a denormal becomes `+0` and `0x7FA00000` comes back `0x7FE00000`. The recomp
        // is built `-O3` and its lifted body has the same `double(f32)`/`float(f64)` pair, so it is
        // very likely folded there too — but that was not measured, and no recorded vector for this
        // function carries a denormal or an sNaN, so the harness never decided it either. Asserting
        // either answer would be asserting this crate's profile flags.
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_null_object_is_refused_before_anything_is_written() {
        let mut g = guest();
        record(&mut g, 0, 1.0f32.to_bits());
        g.set_u32(RECORD + RECORD_OBJECT, 0).unwrap();
        assert!(stamp_slot(&mut g, RECORD).is_err());
        assert_eq!(g.u32(SLOTS).unwrap(), POISON, "nothing written");
    }

    #[test]
    fn the_field_at_460_is_two_bytes_big_endian_and_the_result_is_zero() {
        let mut g = Guest::single(BASE, 0x400);
        g.set_u32(BASE + 456, POISON).unwrap();
        g.set_u32(BASE + FIELD_460, POISON).unwrap();
        g.set_u32(BASE + 464, POISON).unwrap();

        assert_eq!(set_field_460(&mut g, BASE, 0x1234).unwrap(), 0);

        assert_eq!(g.u16(BASE + FIELD_460).unwrap(), 0x1234);
        // Two bytes, not four: the low half of that word is untouched.
        assert_eq!(g.u16(BASE + FIELD_460 + 2).unwrap(), 0xBEEF);
        assert_eq!(g.u32(BASE + 456).unwrap(), POISON);
        assert_eq!(g.u32(BASE + 464).unwrap(), POISON);
    }

    #[test]
    fn the_fourth_argument_comes_back_with_its_upper_half() {
        assert_eq!(
            fourth_argument(0x1234_5678_9ABC_DEF0),
            0x1234_5678_9ABC_DEF0
        );
        // The case a u32 port gets wrong: junk above bit 31 is part of the result.
        assert_eq!(
            fourth_argument(0xFFFF_FFFF_0000_0001),
            0xFFFF_FFFF_0000_0001
        );
        assert_eq!(fourth_argument(0), 0);
    }

    const DECODER: u32 = BASE + 0x200;
    const TABLE_OFFSET: u32 = 0x80; // +36 holds an offset from the object, not a pointer

    fn decoder(active: u8, live_cursor: u32) -> Guest {
        let mut g = Guest::single(BASE, 0x2000);
        g.set_u32(DECODER + STREAM_TABLE, TABLE_OFFSET).unwrap();
        g.set_u8(DECODER + ACTIVE_INDEX, active).unwrap();
        g.set_u32(DECODER + ACTIVE_CURSOR, live_cursor).unwrap();
        g
    }

    fn element(g: &mut Guest, index: u8, saved_cursor: u32, limit: u32) {
        let e = DECODER + TABLE_OFFSET + u32::from(index) * ELEMENT_STRIDE;
        g.set_u32(e + ELEMENT_CURSOR, saved_cursor).unwrap();
        g.set_u32(e + ELEMENT_LIMIT, limit).unwrap();
    }

    #[test]
    fn an_unbound_element_is_zero_even_when_it_is_the_active_stream() {
        let mut g = decoder(2, 4096);
        element(&mut g, 2, 7, 0);
        // The limit test comes first, so the live cursor is never read. If the order were the
        // other way round this would return a huge borrowed value instead.
        assert_eq!(stream_remaining(&g, DECODER, 2).unwrap(), 0);
    }

    #[test]
    fn the_active_stream_uses_the_live_cursor_and_the_others_their_saved_one() {
        let mut g = decoder(1, 1000);
        element(&mut g, 1, 7, 4096); // saved cursor 7 is stale for the active stream
        element(&mut g, 2, 512, 4096);

        assert_eq!(stream_remaining(&g, DECODER, 1).unwrap(), 4096 - 1000);
        assert_eq!(stream_remaining(&g, DECODER, 2).unwrap(), 4096 - 512);
        // The distinguishing part: reading the element's cursor for the active stream would give
        // 4089 here, which is a plausible answer and the wrong one.
        assert_ne!(stream_remaining(&g, DECODER, 1).unwrap(), 4096 - 7);
    }

    #[test]
    fn a_cursor_past_the_limit_borrows_into_the_upper_word() {
        let mut g = decoder(0, 0);
        element(&mut g, 5, 5000, 4096);
        // subf on two zero-extended words: 4096 - 5000 as a 64-bit subtract, so the borrow fills
        // the upper word. The callers read the low word back as a negative i32.
        let r = stream_remaining(&g, DECODER, 5).unwrap();
        assert_eq!(r, 0xFFFF_FFFF_FFFF_FC78);
        assert_eq!(r as u32 as i32, -904);
    }

    #[test]
    fn the_element_stride_is_twenty_four_and_the_index_reaches_255() {
        let mut g = Guest::single(BASE, 0x2000);
        g.set_u32(DECODER + STREAM_TABLE, TABLE_OFFSET).unwrap();
        g.set_u8(DECODER + ACTIVE_INDEX, 0).unwrap();
        for index in [0u8, 1, 2, 255] {
            // A stride of 16 or 32 would read a neighbour's words, and every limit here differs.
            element(&mut g, index, 0, 100 + u32::from(index));
        }
        for index in [1u8, 2, 255] {
            assert_eq!(
                stream_remaining(&g, DECODER, index).unwrap(),
                u64::from(100 + u32::from(index)),
                "index {index}"
            );
        }
        assert_eq!(
            element_address(&g, DECODER, 255).unwrap(),
            DECODER + TABLE_OFFSET + 255 * 24
        );
    }

    // ------------------------------------------------------------ more small verified leaves

    const L2: u32 = 0x5000_0000;

    fn small() -> Guest {
        let mut g = Guest::single(L2, 0x1000);
        g.put(ZERO_CELL, 0.0f32.to_bits().to_be_bytes().to_vec());
        g.put(LIST_HEAD, vec![0u8; 4]);
        g.put(SENTINEL_DOUBLE, (-1.0f64).to_bits().to_be_bytes().to_vec());
        g
    }

    #[test]
    fn the_pair_record_size_rounds_the_count_up_to_pairs() {
        let mut g = small();
        for (count, want) in [(0u32, 88u64), (1, 116), (2, 116), (3, 144), (10, 228)] {
            assert_eq!(
                pair_record_size(&mut g, count, L2).unwrap(),
                want,
                "count {count}"
            );
            assert_eq!(g.u32(L2).unwrap(), 16, "the alignment, stored through r4");
        }
        // addi then a 32-bit shift: 0xFFFFFFFF + 1 wraps to zero pairs.
        assert_eq!(pair_record_size(&mut g, u32::MAX, L2).unwrap(), 88);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn publish_float_lands_on_the_target_and_returns_twelve() {
        let mut g = small();
        let (record, target) = (L2, L2 + 0x100);
        g.set_u32(record + COMMAND_TARGET, target).unwrap();
        g.set_u32(record + 8, 0.625f32.to_bits()).unwrap();
        assert_eq!(publish_float(&mut g, record).unwrap(), 12);
        assert_eq!(g.f32(target + 56).unwrap(), 0.625);
        assert_eq!(g.u32(target + 60).unwrap(), 0, "one word, not two");
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_two_fields_take_the_image_zero_read_live() {
        let mut g = small();
        g.set_u32(ZERO_CELL, 2.5f32.to_bits()).unwrap(); // patch the cell: it is read, not assumed
        g.set_u32(L2 + 12, 0xDEAD_BEEF).unwrap();
        g.set_u32(L2 + 24, 0xDEAD_BEEF).unwrap();
        zero_two_fields(&mut g, L2).unwrap();
        assert_eq!(
            (g.f32(L2 + 16).unwrap(), g.f32(L2 + 20).unwrap()),
            (2.5, 2.5)
        );
        assert_eq!(
            (g.u32(L2 + 12).unwrap(), g.u32(L2 + 24).unwrap()),
            (0xDEAD_BEEF, 0xDEAD_BEEF)
        );
    }

    #[test]
    fn copy_and_mark_filled_copies_count_words_then_sets_the_flag() {
        let mut g = small();
        let (source, dest) = (L2, L2 + 0x100);
        for i in 0..8u32 {
            g.set_u32(source + 4 * i, 0x1000 + i).unwrap();
            g.set_u32(dest + FILL_FIRST_WORD + 4 * i, 0xDEAD_BEEF)
                .unwrap();
        }
        g.set_u8(dest + FILL_COUNT, 3).unwrap();
        copy_and_mark_filled(&mut g, source, dest).unwrap();
        for i in 0..3u32 {
            assert_eq!(g.u32(dest + FILL_FIRST_WORD + 4 * i).unwrap(), 0x1000 + i);
        }
        assert_eq!(
            g.u32(dest + FILL_FIRST_WORD + 12).unwrap(),
            0xDEAD_BEEF,
            "three words"
        );
        assert_eq!(g.u8(dest + FILL_FLAG).unwrap(), 1);

        // A zero count copies nothing but still marks the block.
        let mut g = small();
        g.set_u32(dest + FILL_FIRST_WORD, 0xDEAD_BEEF).unwrap();
        copy_and_mark_filled(&mut g, source, dest).unwrap();
        assert_eq!(g.u32(dest + FILL_FIRST_WORD).unwrap(), 0xDEAD_BEEF);
        assert_eq!(g.u8(dest + FILL_FLAG).unwrap(), 1);
    }

    #[test]
    fn push_node_onto_an_empty_then_a_non_empty_list() {
        let mut g = small();
        let (record, first, second) = (L2, L2 + 0x200, L2 + 0x400);
        g.set_u32(record + COMMAND_TARGET, first).unwrap();
        g.set_u32(first + 48, 0xDEAD_BEEF).unwrap();
        assert_eq!(push_node(&mut g, record).unwrap(), 8);
        assert_eq!(
            g.u32(LIST_HEAD).unwrap(),
            first + NODE_OFFSET,
            "the head is the node, not the object"
        );
        assert_eq!(
            g.u32(first + NODE_OFFSET).unwrap(),
            0,
            "an empty list's next is null"
        );
        assert_eq!(
            g.u32(first + 48).unwrap(),
            0,
            "and the second word is cleared"
        );
        assert_eq!(g.u8(first + NODE_LINKED).unwrap(), 1);

        g.set_u32(record + COMMAND_TARGET, second).unwrap();
        push_node(&mut g, record).unwrap();
        assert_eq!(g.u32(LIST_HEAD).unwrap(), second + NODE_OFFSET);
        assert_eq!(
            g.u32(second + NODE_OFFSET).unwrap(),
            first + NODE_OFFSET,
            "next is the old head"
        );
        assert_eq!(
            g.u32(first + NODE_OFFSET + 4).unwrap(),
            second + NODE_OFFSET,
            "the old head's back-link"
        );
    }

    #[cfg(target_arch = "x86_64")]
    fn command(g: &mut Guest, value: f64, a: f32, b: f32, word: u32) -> (u32, u32) {
        let (record, target) = (L2, L2 + 0x100);
        g.set_u32(record + COMMAND_TARGET, target).unwrap();
        g.set_u64(record + 8, value.to_bits()).unwrap();
        g.set_u32(record + 16, a.to_bits()).unwrap();
        g.set_u32(record + 20, b.to_bits()).unwrap();
        g.set_u32(record + 24, word).unwrap();
        for off in (56..116).step_by(4) {
            g.set_u32(target + off, 0xDEAD_BEEF).unwrap();
        }
        (record, target)
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_command_publishes_the_set_form_unless_it_is_the_clear_one() {
        // Not the sentinel: the set form.
        let mut g = small();
        let (record, target) = command(&mut g, 3.5, 1.25, 2.25, 77);
        assert_eq!(publish_command(&mut g, record).unwrap(), 32);
        assert_eq!(g.u64(target + 56).unwrap(), 3.5f64.to_bits());
        assert_eq!(
            (g.f32(target + 64).unwrap(), g.f32(target + 68).unwrap()),
            (1.25, 2.25)
        );
        assert_eq!(g.u32(target + 72).unwrap(), 77);
        assert_eq!(g.u8(target + 112).unwrap(), 1);
        assert_eq!(
            g.u32(target + 108).unwrap(),
            0xDEAD_BEEF,
            "+108 is the clear form's field"
        );

        // The sentinel and a zero A: the clear form.
        let mut g = small();
        let (record, target) = command(&mut g, -1.0, 0.0, 2.25, 77);
        assert_eq!(publish_command(&mut g, record).unwrap(), 32);
        assert_eq!(g.f32(target + 108).unwrap(), 2.25);
        assert_eq!(
            (g.u8(target + 112).unwrap(), g.u8(target + 113).unwrap()),
            (0, 0)
        );
        assert_eq!(
            g.u32(target + 56).unwrap(),
            0xDEAD_BEEF,
            "the set form's fields are untouched"
        );

        // The sentinel but a non-zero A: still the set form — both halves of the test are needed.
        let mut g = small();
        let (record, target) = command(&mut g, -1.0, 0.5, 2.25, 77);
        publish_command(&mut g, record).unwrap();
        assert_eq!(g.u8(target + 112).unwrap(), 1);

        // A NaN value is unordered, never equal to the sentinel: the set form.
        let mut g = small();
        let (record, target) = command(&mut g, f64::NAN, 0.0, 2.25, 77);
        publish_command(&mut g, record).unwrap();
        assert_eq!(g.u8(target + 112).unwrap(), 1);
    }

    #[test]
    fn the_new_leaf_addresses_come_from_the_lis_immediates() {
        assert_eq!(LIST_HEAD, 0x830C_0000 - 8520);
        assert_eq!(SENTINEL_DOUBLE, 0x8230_0000 - 31232 + 256);
        assert_eq!(
            ZERO_CELL,
            crate::mix::ZERO_SINGLE,
            "the image's zero, reached again"
        );
    }

    #[test]
    fn the_field_at_364_is_two_bytes_and_the_result_is_zero() {
        let mut g = Guest::single(BASE, 0x400);
        g.set_u32(BASE + FIELD_364, POISON).unwrap();
        assert_eq!(set_field_364(&mut g, BASE, 0xABCD).unwrap(), 0);
        assert_eq!(g.u16(BASE + FIELD_364).unwrap(), 0xABCD);
        assert_eq!(
            g.u16(BASE + FIELD_364 + 2).unwrap(),
            0xBEEF,
            "two bytes, not four"
        );
    }

    #[cfg(target_arch = "x86_64")]
    fn ramp_guest(rate: f32) -> Guest {
        let mut g = Guest::single(BASE, 0x400);
        for (addr, v) in [
            (RAMP_UPPER_LIMIT, 48000.0f32),
            (RAMP_BASE, 3.0),
            (RAMP_START, 44100.0),
            (RAMP_LOWER_LIMIT, 2.0),
            (RAMP_SCALE_A, 0.001),
            (RAMP_SCALE_B, 4.0),
            (RAMP_CEILING, 100.0),
            (RAMP_STEP_SCALE, 0.25),
        ] {
            g.put(addr, v.to_bits().to_be_bytes().to_vec());
        }
        g.set_u32(BASE, rate.to_bits()).unwrap();
        g
    }

    #[cfg(target_arch = "x86_64")]
    fn ramp_points(g: &Guest) -> Vec<f32> {
        (0..6u32)
            .map(|k| g.f32(BASE + 0x40 + 4 * k).unwrap())
            .collect()
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_ramp_is_an_arithmetic_progression_from_the_scaled_rate() {
        let mut g = ramp_guest(10_000.0);
        assert_eq!(five_point_ramp(&mut g, BASE, BASE + 0x40).unwrap(), 1);
        let first = 10_000.0f32 * 0.001; // 10
        let span = first * 4.0; // 40, under the ceiling
        let step = (span - first) * 0.25; // 7.5
        assert_eq!(
            ramp_points(&g),
            vec![
                first,
                first + step,
                first + 2.0 * step,
                first + 3.0 * step,
                first + 4.0 * step,
                span
            ]
        );
        assert_eq!(
            g.f32(BASE).unwrap(),
            10_000.0,
            "an in-range rate is left alone"
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_rate_clamps_both_ways_and_a_nan_takes_the_lower_limit() {
        for (rate, want) in [(90_000.0f32, 48_000.0f32), (1.0, 2.0), (f32::NAN, 2.0)] {
            let mut g = ramp_guest(rate);
            five_point_ramp(&mut g, BASE, BASE + 0x40).unwrap();
            // 48000 * 0.001 * 4 = 192 is past the ceiling, so the upper clamp is then overwritten by
            // the pool's start value; the other two stay at the lower limit.
            let stored = g.f32(BASE).unwrap();
            if rate > 48_000.0 {
                assert_eq!(
                    stored, 44_100.0,
                    "rate {rate}: the ceiling path rewrites it"
                );
            } else {
                assert_eq!(stored, want, "rate {rate}");
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_span_past_the_ceiling_restarts_from_the_pool_base() {
        let mut g = ramp_guest(40_000.0); // first 40, span 160 > 100
        five_point_ramp(&mut g, BASE, BASE + 0x40).unwrap();
        let pts = ramp_points(&g);
        assert_eq!(pts[0], 3.0, "the ramp starts from the pool's base");
        assert_eq!(pts[5], 100.0, "and spans to the ceiling");
        assert_eq!(pts[1], 3.0 + (100.0 - 3.0) * 0.25);
        assert_eq!(
            g.f32(BASE).unwrap(),
            44_100.0,
            "the rate field is overwritten a second time"
        );
    }
}

#[cfg(test)]
mod table_ramp_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const RAMP: u32 = BASE + 0x100;
    const OUT: u32 = BASE + 0x200;
    const TABLE: u32 = BASE + 0x1000;

    /// Scale 1, threshold 0.5, high scale 4, baseline 1; the table holds `i` at index `i`, and the
    /// output words are poisoned with -1.
    fn guest(ramp: [f32; 6]) -> Guest {
        let mut g = Guest::single(BASE, 0x3000);
        g.put(
            TABLE_INPUT_SCALE,
            [1.0f32, 0.5, 4.0]
                .iter()
                .flat_map(|v| v.to_bits().to_be_bytes())
                .collect(),
        );
        g.put(TABLE_BASELINE, 1.0f32.to_bits().to_be_bytes().to_vec());
        g.set_u32(OBJECT + TABLE_POINTER, TABLE).unwrap();
        for i in 0..TABLE_ENTRIES as u32 {
            g.set_u32(TABLE + 4 * i, (i as f32).to_bits()).unwrap();
        }
        for (k, v) in ramp.iter().enumerate() {
            g.set_u32(RAMP + 4 * k as u32, v.to_bits()).unwrap();
            g.set_u32(OUT + 4 * k as u32, u32::MAX).unwrap();
        }
        g
    }

    fn out(g: &Guest) -> Vec<i32> {
        (0..6).map(|k| g.u32(OUT + 4 * k).unwrap() as i32).collect()
    }

    #[test]
    fn each_point_takes_the_first_entry_above_its_target() {
        let mut g = guest([4.0, 8.0, 12.0, 16.0, 20.0, 24.0]);
        assert_eq!(table_ramp(&mut g, OBJECT, RAMP, OUT, 0.25).unwrap(), 1);
        assert_eq!(
            out(&g),
            [2, 3, 4, 5, 6, 7],
            "targets 1..6, each answered by the next entry up"
        );
    }

    #[test]
    fn the_sweep_never_goes_back() {
        let mut g = guest([20.0, 4.0, 4.0, 4.0, 4.0, 4.0]);
        table_ramp(&mut g, OBJECT, RAMP, OUT, 0.25).unwrap();
        assert_eq!(
            out(&g),
            [6, 7, 8, 9, 10, 11],
            "target 1 after target 5 still answers 7, not 2"
        );
    }

    #[test]
    fn above_the_threshold_the_gain_rescales_the_stored_word() {
        let mut g = guest([4.0; 6]);
        table_ramp(&mut g, OBJECT, RAMP, OUT, 1.0).unwrap();
        // The input clamps to 0.5, so each target is 2; the gain is 1.0 * 4.
        assert_eq!(out(&g), [12, 16, 20, 24, 28, 32]);
    }

    #[test]
    fn a_sweep_off_the_end_stores_nothing_but_still_rescales_what_was_there() {
        let mut g = guest([4000.0; 6]);
        table_ramp(&mut g, OBJECT, RAMP, OUT, 1.0).unwrap();
        assert_eq!(
            out(&g),
            [-4, -4, -4, -4, -4, 0],
            "the poison rescaled; only the last word was cleared"
        );
    }
}

#[cfg(test)]
mod settle_levels_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const LINKED: u32 = BASE + 0x800;

    fn guest(count: u8) -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        crate::mathlib::tests::with_log_pool(&mut g);
        g.put(LEVEL_TEN, 10.0f32.to_bits().to_be_bytes().to_vec());
        g.put(ZERO_CELL, 0.0f32.to_bits().to_be_bytes().to_vec());
        g.set_u32(OBJECT + LEVEL_LINKED, LINKED).unwrap();
        g.set_u32(OBJECT + LEVEL, 1.0f32.to_bits()).unwrap();
        g.set_u32(LINKED + LEVEL_LINKED_LEVEL, 0.5f32.to_bits())
            .unwrap();
        g.set_u32(OBJECT + LEVEL_FIRST_COUNT, 3).unwrap();
        g.set_u32(OBJECT + LEVEL_FIRST_VALUE, 100.0f32.to_bits())
            .unwrap();
        g.set_u8(OBJECT + LEVEL_CANDIDATE_COUNT, count).unwrap();
        for (k, v) in [10.0f32, 1000.0, 5.0].iter().enumerate() {
            g.set_u32(OBJECT + LEVEL_CANDIDATES + 4 * k as u32, v.to_bits())
                .unwrap();
        }
        for (k, v) in [2i32, 7, -1].iter().enumerate() {
            g.set_u32(OBJECT + LEVEL_COUNTS + 4 * k as u32, *v as u32)
                .unwrap();
        }
        g
    }

    fn log(g: &Guest, x: f32) -> f32 {
        crate::mathlib::log10(g, f64::from(x)).unwrap() as f32
    }

    #[test]
    fn both_terms_are_summed_and_the_change_accumulated_into_the_linked_level() {
        let mut g = guest(3);
        settle_levels(&mut g, OBJECT).unwrap();
        // Single arithmetic on single operands is what the fdivs/fsubs chain computes.
        let term1 = 3.0f32 - (3.0f32 * 10.0) / log(&g, 100.0);
        let term2 = 7.0f32 - (7.0f32 * 10.0) / log(&g, 1000.0);
        let sum = term2 + term1;
        assert_eq!(g.f32(OBJECT + LEVEL).unwrap(), sum);
        assert_eq!(
            g.f32(LINKED + LEVEL_LINKED_LEVEL).unwrap(),
            (sum - 1.0) + 0.5
        );
    }

    #[test]
    fn with_no_candidates_the_second_term_is_of_zero_and_vanishes() {
        // largest stays 0.0, log10 of it is -inf, and 0 - 0/-inf is +0.
        let mut g = guest(0);
        settle_levels(&mut g, OBJECT).unwrap();
        let term1 = 3.0f32 - (3.0f32 * 10.0) / log(&g, 100.0);
        assert_eq!(g.f32(OBJECT + LEVEL).unwrap(), term1);
    }
}

#[cfg(test)]
mod resample_bands_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const OUT: u32 = BASE + 0x800;
    const INPUT: u32 = BASE + 0x840;
    const SP: u32 = BASE + 0x1000;

    fn cell(g: &mut Guest, at: u32, v: f32) {
        if g.set_u32(at, v.to_bits()).is_err() {
            g.put(at, v.to_bits().to_be_bytes().to_vec());
        }
    }

    /// Edges 0, 1, 2; rates 1; knots at 0..8; every table row holding `k` at knot `k`, so whatever the
    /// blend weight the curve is the identity and each output equals its input.
    fn guest(inputs: [f32; 6]) -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        g.put(
            BAND_EDGES,
            [0.0f32, 1.0, 2.0]
                .iter()
                .flat_map(|v| v.to_bits().to_be_bytes())
                .collect(),
        );
        g.put(BAND_KNOT_X, vec![0u8; 216]);
        for k in 0..9u32 {
            cell(&mut g, BAND_KNOT_X + 4 * k, k as f32);
            for row in 1..6u32 {
                cell(&mut g, BAND_KNOT_X + 36 * row + 4 * k, k as f32);
            }
        }
        for (at, v) in [
            (BAND_RATE_LOWER, 1.0f32),
            (BAND_RATE_UPPER, 1.0),
            (crate::routing::UNITY_GAIN, 1.0),
            (BAND_HEADROOM, 0.5),
            (ZERO_CELL, 0.0),
        ] {
            cell(&mut g, at, v);
        }
        for (i, v) in inputs.iter().enumerate() {
            g.set_u32(INPUT + 4 * i as u32, v.to_bits()).unwrap();
        }
        g
    }

    fn out(g: &Guest) -> Vec<f32> {
        (0..6).map(|i| g.f32(OUT + 4 * i).unwrap()).collect()
    }

    #[test]
    fn an_identity_curve_interpolates_each_input_to_itself() {
        let inputs = [0.5f32, 2.25, 3.0, 4.75, 6.5, 7.125];
        let mut g = guest(inputs);
        assert_eq!(
            resample_bands(&mut g, OBJECT, OUT, INPUT, 0.25, SP).unwrap(),
            1
        );
        assert_eq!(out(&g), inputs);
        assert_eq!(
            g.f32(SP - BAND_RED_ZONE + 4 * 8).unwrap(),
            8.0,
            "the abscissae copied into the red zone"
        );
    }

    #[test]
    fn the_segment_weight_blends_two_rows() {
        // Band 0.25 is in the lower segment: weight (1 - 0.25) * 1, so each ordinate is
        // 0.75 * row0 + 0.25 * row2. With row0 = 1 and row2 = 3 everywhere, every output is 1.5.
        let mut g = guest([0.5; 6]);
        for k in 0..9u32 {
            cell(&mut g, BAND_KNOT_X + 36 + 4 * k, 1.0);
            cell(&mut g, BAND_KNOT_X + 108 + 4 * k, 3.0);
        }
        resample_bands(&mut g, OBJECT, OUT, INPUT, 0.25, SP).unwrap();
        assert_eq!(out(&g), [1.5; 6]);
    }

    #[test]
    fn equal_levels_stop_after_the_resample() {
        let mut g = guest([1.0; 6]);
        g.set_u32(OBJECT + BAND_ENVELOPES + 36, 0x7777_7777)
            .unwrap();
        resample_bands(&mut g, OBJECT, OUT, INPUT, 0.25, SP).unwrap();
        assert_eq!(g.u32(OBJECT + BAND_ENVELOPES + 36).unwrap(), 0x7777_7777);
    }

    #[test]
    fn differing_levels_normalise_clear_the_envelopes_and_rearm_what_fits() {
        let mut g = guest([1.0, 1.0, 1.0, 1.0, 1.0, 1.5]);
        g.set_u32(OBJECT + BAND_CURRENT_LEVEL, 1.0f32.to_bits())
            .unwrap();
        g.set_u32(OBJECT + BAND_ENVELOPES + 36, 0x7777_7777)
            .unwrap();
        // Queue 0, record 0: size 10 at +388, needing 32 bytes from position 0 in a limit of 100.
        g.set_u32(OBJECT + BAND_PENDING_SIZES - 4, 10).unwrap();
        g.set_u32(OBJECT + BAND_QUEUES, 100).unwrap();
        // Queue 0, record 1: size 100 needs (100 + 35) & !31 = 128, over its limit of 100.
        g.set_u32(OBJECT + BAND_PENDING_SIZES, 100).unwrap();
        g.set_u32(OBJECT + BAND_QUEUES + 60, 100).unwrap();
        g.set_u32(OBJECT + BAND_QUEUES + 60 + 20, 0x5555).unwrap();
        resample_bands(&mut g, OBJECT, OUT, INPUT, 0.25, SP).unwrap();
        // The ceiling is the last band plus the headroom, 2.0, above the current level of 1.0.
        assert_eq!(out(&g), [0.5, 0.5, 0.5, 0.5, 0.5, 0.75]);
        assert_eq!(g.u32(OBJECT + BAND_ENVELOPES + 36).unwrap(), 0);
        let rec = OBJECT + BAND_QUEUES;
        assert_eq!(g.u32(rec + 20).unwrap(), 11, "size + 1");
        assert_eq!(g.u32(rec + 12).unwrap(), 100, "the limit copied");
        assert_eq!(
            g.u32(rec + 60 + 20).unwrap(),
            0x5555,
            "the record that does not fit is left"
        );
    }

    #[test]
    fn the_fit_test_is_a_signed_compare() {
        assert!(reservation_fits(100, 32));
        assert!(!reservation_fits(100, 128));
        assert!(
            !reservation_fits(0x8000_0000, 32),
            "a negative limit fits nothing"
        );
        assert!(
            reservation_fits(32, 0x8000_0000),
            "anything fits above a negative need"
        );
    }
}

#[cfg(test)]
mod level_table_tests {
    use super::*;
    use crate::Segment;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const SP: u32 = BASE + 0x8000;

    fn cell(g: &mut Guest, at: u32, v: f32) {
        g.set_u32(at, v.to_bits()).unwrap();
    }

    /// Rates 1.5 (one), 2.0 (first), 3.0 (last); candidates 10, 20, 30; the source 2.0.
    fn guest(layout: u8, source: f32) -> Guest {
        let mut g = Guest::from_segments(vec![
            Segment {
                base: BASE,
                bytes: vec![0u8; 0x10000],
            },
            Segment {
                base: 0x8209_9000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x820B_A000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8211_6000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8216_5000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x822F_8000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8231_A000,
                bytes: vec![0u8; 0x1000],
            },
        ]);
        for (at, v) in [
            (LEVEL_RATE_ONE, 1.5f32),
            (LEVEL_CANDIDATE_ONE, 10.0),
            (LEVEL_RATE_FIRST, 2.0),
            (LEVEL_CANDIDATE_FIRST, 20.0),
            (LEVEL_RATE_LAST, 3.0),
            (LEVEL_CANDIDATE_LAST, 30.0),
            (LEVEL_HALF, 0.5),
            (ZERO_CELL, 0.0),
            (RECOMMIT_FLOOR, 1.0),
            (crate::routing::UNITY_GAIN, 1.0),
        ] {
            cell(&mut g, at, v);
        }
        g.set_u8(OBJECT + LEVEL_LAYOUT, layout).unwrap();
        cell(&mut g, OBJECT + LEVEL_SOURCE, source);
        g
    }

    fn pairs(g: &Guest, n: u32) -> (u8, Vec<f32>, Vec<i32>) {
        (
            g.u8(OBJECT + LEVEL_CANDIDATE_COUNT).unwrap(),
            (0..n)
                .map(|k| g.f32(OBJECT + LEVEL_CANDIDATES + 4 * k).unwrap())
                .collect(),
            (0..n)
                .map(|k| g.u32(OBJECT + LEVEL_COUNTS + 4 * k).unwrap() as i32)
                .collect(),
        )
    }

    #[test]
    fn each_layout_rearms_its_pairs() {
        let mut g = guest(1, 2.0);
        rearm_level_table(&mut g, OBJECT).unwrap();
        assert_eq!(
            pairs(&g, 1),
            (1, vec![10.0], vec![3]),
            "2 * 1.5 = 3, plus the half, truncated"
        );
        let mut g = guest(4, 2.0);
        rearm_level_table(&mut g, OBJECT).unwrap();
        assert_eq!(pairs(&g, 2), (2, vec![20.0, 30.0], vec![4, 6]));
        let mut g = guest(6, 2.0);
        rearm_level_table(&mut g, OBJECT).unwrap();
        assert_eq!(
            pairs(&g, 3),
            (3, vec![20.0, 10.0, 30.0], vec![4, 3, 6]),
            "the middle pair is layout 1's"
        );
    }

    #[test]
    fn a_negative_count_rounds_away_from_zero() {
        let mut g = guest(1, -1.0);
        rearm_level_table(&mut g, OBJECT).unwrap();
        assert_eq!(pairs(&g, 1).2, vec![-2], "-1.5 less the half is -2.0");
    }

    /// A filter object whose three parameters all equal their published copies, bands of 0.5, and
    /// +52 of 2.0 against a floor of 1.0: the weights are (1 - 0.5) * (1 - 0.5).
    fn unchanged(g: &mut Guest, state: u32, ready: u8) {
        g.set_u32(OBJECT + RECOMMIT_STATE, state).unwrap();
        g.set_u8(OBJECT + RECOMMIT_READY, ready).unwrap();
        cell(g, OBJECT + RECOMMIT_PARAM_52, 2.0);
        for (a, b, v) in [
            (RECOMMIT_RATE, RECOMMIT_PUBLISHED_RATE, 7.0f32),
            (LEVEL_SOURCE, RECOMMIT_PUBLISHED_BLEND, 2.0),
            (RECOMMIT_LEVEL, RECOMMIT_PUBLISHED_LEVEL, 0.25),
        ] {
            cell(g, OBJECT + a, v);
            cell(g, OBJECT + b, v);
        }
        for k in 0..6u32 {
            cell(g, OBJECT + RECOMMIT_BANDS + 4 * k, 0.5);
            g.set_u32(OBJECT + RECOMMIT_WEIGHTS + 4 * k, 0xDEAD_BEEF)
                .unwrap();
        }
    }

    #[test]
    fn a_state_other_than_one_writes_nothing() {
        let mut g = guest(1, 2.0);
        unchanged(&mut g, 2, 0);
        recommit_filter(&mut g, OBJECT, SP).unwrap();
        assert_eq!(g.u32(OBJECT + RECOMMIT_WEIGHTS).unwrap(), 0xDEAD_BEEF);
        assert_eq!(g.u8(OBJECT + LEVEL_CANDIDATE_COUNT).unwrap(), 0);
    }

    #[test]
    fn with_nothing_moved_the_weights_are_recomputed_and_the_table_rearmed() {
        let mut g = guest(1, 2.0);
        unchanged(&mut g, 1, 0);
        recommit_filter(&mut g, OBJECT, SP).unwrap();
        for k in 0..6u32 {
            assert_eq!(
                g.f32(OBJECT + RECOMMIT_WEIGHTS + 4 * k).unwrap(),
                0.25,
                "weight {k}"
            );
        }
        assert_eq!(
            pairs(&g, 1),
            (1, vec![10.0], vec![3]),
            "the ready byte is 0"
        );
        assert_eq!(g.u32(OBJECT + RECOMMIT_STATE).unwrap(), 2);
        assert_eq!(g.f32(OBJECT + RECOMMIT_PUBLISHED_52).unwrap(), 2.0);
    }

    #[test]
    fn a_ready_object_off_the_rate_path_neither_rearms_nor_rewrites_records() {
        let mut g = guest(1, 2.0);
        unchanged(&mut g, 1, 1);
        g.set_u8(OBJECT + LEVEL_CANDIDATE_COUNT, 2).unwrap();
        g.set_u32(OBJECT + RECOMMIT_RECORDS + 36, 0x5555).unwrap();
        recommit_filter(&mut g, OBJECT, SP).unwrap();
        assert_eq!(g.u8(OBJECT + LEVEL_CANDIDATE_COUNT).unwrap(), 2);
        assert_eq!(g.u32(OBJECT + RECOMMIT_RECORDS + 36).unwrap(), 0x5555);
    }
}
