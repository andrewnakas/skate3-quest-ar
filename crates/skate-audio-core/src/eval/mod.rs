//! The RwAudio expression evaluator: the 40-slot opcode table at guest `0x82FD3600`.
//!
//! The interpreter `sub_82B1E290` walks a list of nodes, each `{ +0 next, +8 program, +12 operand
//! block }`. A program is a stream of records
//!
//! ```text
//! { u8 opcode, u8 pairs, 2 unread, { u32 src, u32 dst } pair[pairs], u32 block_advance }
//! ```
//!
//! and for each record it calls `TABLE[opcode]` with the operand block in `r3`. The returned `r3`
//! is then **consumed**: for every pair whose `src` is `-1` the interpreter stores the result's low
//! word into `block[dst]`, and any other `src` is a block-to-block copy. Opcode 255 ends the
//! stream. That is what makes every op here `s32 op(block*)` with no direct call site anywhere in
//! the lifted tree, and why each one's whole result is the value it returns.
//!
//! **This module is the table's slots; the interpreter is [`interp`], unverified new work (2026-09-14).** `sub_82B1E290` itself fails the port
//! screen's gate 1 — its `bctrl` dispatches a data word, so its callee set is whatever the program
//! bytes name — and gate 2, since its write set is the union over an unbounded node list of an
//! unbounded record stream whose store addresses the dispatched ops may rewrite mid-walk. It has no
//! verified C++ body to translate, so there is none here. [`dispatch`] provides the *table lookup*
//! the interpreter would perform, which is testable on its own; a Rust interpreter would still need
//! the node walk, the period arithmetic and the pair machinery, and would be new work rather than a
//! transcription.
//!
//! ## Where the table came from
//!
//! Read out of the validated guest image dump at `probe/harness/out/image/g_82E0.bin`, offset
//! `0x1D3600`. The table is exactly 40 entries: slot 39 is the last word that decodes as a text
//! address, and slot 40 onward is unrelated data (`0xF22D0E56`, then what looks like a fixed-point
//! table). Three of the notes in `probe/ports/notes/` had independently identified slots 23, 24,
//! 25, 29, 30, 31 and 36 from the same dump; reading the whole range agrees with all seven and
//! fixes the remaining thirty-three.
//!
//! ## Coverage
//!
//! **All 31 slots that have a verified C++ body are ported, plus slots 1 and 2** (2026-09-14): those
//! two are single-instruction accessors outside the audio corpus (`lwz r3,20(r3); blr` and
//! `lwz r3,24(r3); blr`), not shadow-verified, and needed by every player bank's program. Of the 9
//! slots without a verified body:
//!
//! - **Three fail the port screen's gate 1** and have no verified reference in either language:
//!   slots 4, 27 and 39 (`sub_82B1C150`, `sub_82B1D240`, `sub_82B1C450`), each because it makes an
//!   indirect call — a vtable release, a handler-list notify, a closure that leaves the audio
//!   corpus entirely.
//! - **One is host-bound**: slot 5 `sub_82B1C210` clamps values and calls the table-1 broadcast
//!   dispatcher. [`crate::patch::PatchHost`] implements it together with the recovered
//!   `sub_828E29C0` handle checks and the known player-bank callback. The pure table dispatcher
//!   continues to refuse it because it has no callback host.
//! - **Five are outside the 216 audio-thread functions** (1 and 2 now ported as above): slots 1, 2, 19, 20 and 38 have no `.inc`
//!   at all, so nothing has been screened for them. Slots 1 and 2 are not even in the audio corpus.
//!
//! The C++ side moves: this coverage reflects `recomp/src/audio_ports/` as of 2026-09-12, and
//! `python3 probe/ports/queue.py report` is the live answer. A slot whose `absent` text no longer
//! matches its `.inc` header is a signal that a port became translatable, not a bug in the table.
//!
//! ## Verification status of what *is* here
//!
//! Every ported op is a transcription of a `STATUS: verified` C++ body — compared call-for-call
//! against the original recompiled code under the shadow harness, on real inputs the running game
//! generated. **The Rust is not itself replayed against recorded vectors.** That distinction is the
//! one the crate README draws and it holds here: `system.rs`, `player.rs` and `buffers.rs` have
//! numbers attached (8,607 comparisons), and these ops have a proven *reference* but only unit
//! tests of their own. A divergence in one of them is a transcription error, which is a far smaller
//! search space than a misreading of the engine — but it is not zero.

pub mod accessors;
pub mod arith;
pub mod interp;
pub mod state;
pub mod wave;

use crate::{Error, Guest, Result};

/// The opcode table's guest address.
pub const TABLE_BASE: u32 = 0x82FD_3600;
/// Entries in the table. Slot 40 onward is not code.
pub const TABLE_SLOTS: usize = 40;

/// Guest `.rdata` and data cells the ops load **live**, exactly as the originals do.
///
/// They are not folded into constants even where the image dump shows a stable value, for two
/// reasons: the ops read them through the guest map, so a caller with a patched image gets the
/// patched value; and two of the five are not constants at all.
///
/// | address | dump value | what it is |
/// |---|---|---|
/// | `0x82165A10` | `0.0f` | the compare-against-zero cell, shared with the command queue |
/// | `0x8209975C` | `0.5f` | the round-half-away-from-zero offset |
/// | `0x8216DEE0` | `-1.0f` | the timer's parked value |
/// | `0x822F890C` | `1/4096` | the ramp's rate unit |
/// | `0x830775D8` | `0.0f` | **not** a constant: the scheduler's tick scale, written at run time |
///
/// `0x830775D8` read 0.0 in a dump taken at the second guest function of boot, before
/// `sub_82B1E290` had stored anything into it. Three ops divide or multiply by it
/// ([`state::op_timer`], [`state::op_ramp`], [`state::op_delay_ring`]), so a test that leaves it
/// zero is testing a division by zero, not the engine.
pub const ZERO_SINGLE: u32 = 0x8216_5A10;
/// See [`ZERO_SINGLE`].
pub const HALF_SINGLE: u32 = 0x8209_975C;
/// See [`ZERO_SINGLE`].
pub const MINUS_ONE_SINGLE: u32 = 0x8216_DEE0;
/// See [`ZERO_SINGLE`].
pub const RATE_UNIT_SINGLE: u32 = 0x822F_890C;
/// See [`ZERO_SINGLE`]. Written by the interpreter's tick, not a constant.
pub const TICK_SCALE_GLOBAL: u32 = 0x8307_75D8;

/// The oscillator's cells, and the quarter-sine table they index. Dump values, all read live.
///
/// | address | dump value | what it is |
/// |---|---|---|
/// | `0x8231A844` | `1.0f` | the oscillator's phase wrap point and the curve's nearest-mode test |
/// | `0x822F8EA4` | `1024.0f` | phase to table units: 1024 units make one cycle |
/// | `0x82098D0C` | `1/65536` | the sample normalisation after a table lookup |
/// | `0x82060C50` | `2.0f` | the triangle's slope gain |
/// | `0x82FD36B8` | 257 halfwords | the quarter-sine table; see [`wave::op_oscillator`] |
///
/// `0x8231A844` is the same cell `system::VOICE_GONE_ADDR` names and the same one `sub_82B1E290`
/// uses as its period numerator, so four unrelated readings all resolve to one `1.0f`. It is named
/// for its value here rather than for any of those roles: a shared constant pool is not a shared
/// meaning, and naming it after the first use found would have made the others look wrong.
pub const ONE_SINGLE: u32 = 0x8231_A844;
/// See [`ONE_SINGLE`].
pub const PHASE_TO_UNITS: u32 = 0x822F_8EA4;
/// See [`ONE_SINGLE`].
pub const SINE_NORM: u32 = 0x8209_8D0C;
/// See [`ONE_SINGLE`].
pub const TRIANGLE_GAIN: u32 = 0x8206_0C50;
/// See [`ONE_SINGLE`].
pub const SINE_TABLE: u32 = 0x82FD_36B8;
/// The table's used extent: 257 halfwords, not 256, because quadrants 1 and 3 read backwards from
/// the mirror at `+512` and so reach the halfword *at* it.
pub const SINE_TABLE_BYTES: usize = 514;

const _: () = assert!(ZERO_SINGLE == 0x8216_0000 + 23056, "lis -32234 ; lfs 23056");
const _: () = assert!(
    HALF_SINGLE == 0x820A_0000 - 26788,
    "lis -32246 ; lfs -26788"
);
const _: () = assert!(
    MINUS_ONE_SINGLE == 0x8216_0000 + 0xDEE0,
    "lis -32233 ; lfs -8480"
);
const _: () = assert!(
    TICK_SCALE_GLOBAL == 0x8307_0000 + 30168,
    "lis -31993 ; lfs 30168"
);
const _: () = assert!(
    TICK_SCALE_GLOBAL + 24 == crate::counter::COUNTER,
    "the tick block precedes the counter"
);
// Each of these is the ((lis_imm & 0xFFFF) << 16) + offset the lifted body computes, written out so
// a transposed digit fails the build. One misread constant in sub_82B2FE00 caused this project's
// first shadow divergence, which is why the C++ notes insist on computing them rather than reading
// them off a dump by eye.
const _: () = assert!(ONE_SINGLE == 0x8232_0000 - 22460, "lis -32206 ; lfs -22460");
const _: () = assert!(
    PHASE_TO_UNITS == 0x8230_0000 - 31232 + 2212,
    "lis -32208 ; addi -31232 ; lfs 2212"
);
const _: () = assert!(SINE_NORM == 0x820A_0000 - 29428, "lis -32246 ; lfs -29428");
const _: () = assert!(TRIANGLE_GAIN == 0x8206_0000 + 3152, "lis -32250 ; lfs 3152");
const _: () = assert!(SINE_TABLE == 0x82FD_0000 + 14008, "lis -32003 ; addi 14008");
const _: () = assert!(
    RATE_UNIT_SINGLE == 0x8230_0000 - 31232 + 780,
    "lis -32208 ; addi -31232 ; lfs 780"
);

/// One table slot, called with the operand block in `r3` and returning the guest's full 64-bit
/// `r3`.
///
/// Uniformly `&mut Guest` even for the ops that write nothing, because that is the guest's own
/// signature for all 40 slots and it is what makes the table dispatchable. Each op's doc comment
/// states its write set; the signature does not.
pub type Op = fn(&mut Guest, u32) -> Result<u64>;

/// A row of the guest's opcode table.
pub struct Slot {
    /// The function address the image holds in this slot.
    pub guest: u32,
    /// The `sub_` name, for reports.
    pub name: &'static str,
    /// The Rust port, when there is one.
    pub port: Option<Op>,
    /// Why there is not, when there is not.
    pub absent: &'static str,
}

const fn ported(guest: u32, name: &'static str, port: Op) -> Slot {
    Slot {
        guest,
        name,
        port: Some(port),
        absent: "",
    }
}

const fn missing(guest: u32, name: &'static str, absent: &'static str) -> Slot {
    Slot {
        guest,
        name,
        port: None,
        absent,
    }
}

/// The table as the guest image holds it, slot for slot.
pub static TABLE: [Slot; TABLE_SLOTS] = [
    ported(0x82B1_BF68, "sub_82B1BF68", accessors::op_take_word_16),
    ported(0x8283_2BA8, "sub_82832BA8", accessors::op_word_20),
    ported(0x82C8_CDC8, "sub_82C8CDC8", accessors::op_word_24),
    ported(0x82B1_BF80, "sub_82B1BF80", accessors::op_take_word_0),
    missing(
        0x82B1_C150,
        "sub_82B1C150",
        "gate 1: sub_82B1BF98's closure leaves the audio corpus",
    ),
    missing(
        0x82B1_C210,
        "sub_82B1C210",
        "host-bound broadcast; use patch::PatchHost",
    ),
    ported(0x82B1_C4B8, "sub_82B1C4B8", state::op_stepping_cursor),
    ported(0x82B1_C528, "sub_82B1C528", state::op_random_in_range),
    ported(0x82B1_C598, "sub_82B1C598", state::op_shuffle_bag),
    ported(0x82B1_C6C8, "sub_82B1C6C8", state::op_weighted_cursor),
    ported(0x82B1_C778, "sub_82B1C778", state::op_window_latch),
    ported(0x82B1_C7E8, "sub_82B1C7E8", state::op_timer),
    ported(0x82B1_C878, "sub_82B1C878", accessors::op_select_first_flag),
    ported(0x82B1_C8D0, "sub_82B1C8D0", accessors::op_any_nonzero),
    ported(0x82B1_C910, "sub_82B1C910", wave::op_envelope),
    ported(0x82B1_CAD8, "sub_82B1CAD8", wave::op_curve),
    ported(0x82B1_CD28, "sub_82B1CD28", state::op_delay_ring),
    ported(0x82B1_CE18, "sub_82B1CE18", accessors::op_stack_top),
    ported(0x82B1_CE48, "sub_82B1CE48", accessors::op_stack_push),
    missing(
        0x82B1_CEA0,
        "sub_82B1CEA0",
        "no .inc: outside the 216 audio-thread functions",
    ),
    missing(
        0x82B1_CEF8,
        "sub_82B1CEF8",
        "no .inc: outside the 216 audio-thread functions",
    ),
    ported(0x82B1_CF50, "sub_82B1CF50", arith::op_round_product),
    ported(0x82B1_D118, "sub_82B1D118", arith::op_sum),
    ported(0x82B1_D1A8, "sub_82B1D1A8", arith::op_sub),
    ported(0x82B1_D1B8, "sub_82B1D1B8", arith::op_mul),
    ported(0x82B1_D1C8, "sub_82B1D1C8", arith::op_div),
    ported(0x82B1_D200, "sub_82B1D200", arith::op_rem),
    missing(
        0x82B1_D240,
        "sub_82B1D240",
        "gate 1: three vtable bctrls, one of them a release",
    ),
    ported(0x82B1_D3D0, "sub_82B1D3D0", wave::op_oscillator),
    ported(0x82B1_D5D0, "sub_82B1D5D0", state::op_ramp),
    ported(0x82B1_D700, "sub_82B1D700", arith::op_sum_capped),
    ported(0x82B1_D790, "sub_82B1D790", arith::op_sub_floor),
    ported(0x82B1_D7B0, "sub_82B1D7B0", arith::op_mul_cap),
    ported(0x82B1_CEE0, "sub_82B1CEE0", arith::op_min),
    ported(0x82B1_CF38, "sub_82B1CF38", arith::op_max),
    ported(0x82B1_D098, "sub_82B1D098", arith::op_round_scaled),
    ported(0x82B1_D198, "sub_82B1D198", arith::op_add),
    ported(0x82B1_D7D0, "sub_82B1D7D0", accessors::op_take_flag_25),
    missing(
        0x82B1_C2B8,
        "sub_82B1C2B8",
        "no .inc: outside the 216 audio-thread functions",
    ),
    missing(
        0x82B1_C450,
        "sub_82B1C450",
        "gate 1: notifies through a handler list by ctr",
    ),
];

/// Run the op in `opcode`'s slot over `block`, the way the interpreter's `bctrl` would.
///
/// An unported or out-of-range opcode is an error naming the table entry, never a silent no-op:
/// the interpreter would store *something* into the block, and inventing a value would turn a gap
/// in coverage into a wrong answer.
pub fn dispatch(g: &mut Guest, opcode: u8, block: u32) -> Result<u64> {
    match TABLE.get(opcode as usize) {
        Some(slot) => match slot.port {
            Some(op) => op(g, block),
            None => Err(Error::new(
                slot.guest,
                format!(
                    "opcode {opcode} ({}) is not ported: {}",
                    slot.name, slot.absent
                ),
            )),
        },
        None => Err(Error::new(
            TABLE_BASE + 4 * opcode as u32,
            format!("opcode {opcode} is past the {TABLE_SLOTS}-entry table"),
        )),
    }
}

/// How many slots have a Rust port.
pub fn ported_slots() -> usize {
    TABLE.iter().filter(|s| s.port.is_some()).count()
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// The operand block every op test uses. Far from the `.rdata` cells, as in a real session.
    pub const BLOCK: u32 = 0x4000_0000;
    /// The block span, generous enough for the flag tables and ring slots the tests reach.
    pub const BLOCK_BYTES: usize = 0x200;

    pub fn block_guest() -> Guest {
        Guest::single(BLOCK, BLOCK_BYTES)
    }

    /// Write consecutive big-endian words from the block's base.
    pub fn put_words(g: &mut Guest, words: &[u32]) {
        for (i, w) in words.iter().enumerate() {
            g.set_u32(BLOCK + 4 * i as u32, *w).unwrap();
        }
    }

    /// A `u8` count field at `+0` occupies the **top** byte of the big-endian word there, so a
    /// test that writes the count as a word would store zero. This makes that explicit.
    pub fn count_word(count: u8) -> u32 {
        (count as u32) << 24
    }

    /// Install the five guest cells the float ops load, plus the counter global, at their real
    /// addresses. The values are the image dump's, except the tick scale, which the dump caught
    /// before it was written — tests that need it set it themselves.
    pub fn put_rodata(g: &mut Guest) {
        g.put(ZERO_SINGLE, 0.0f32.to_bits().to_be_bytes().to_vec());
        g.put(HALF_SINGLE, 0.5f32.to_bits().to_be_bytes().to_vec());
        g.put(MINUS_ONE_SINGLE, (-1.0f32).to_bits().to_be_bytes().to_vec());
        g.put(
            RATE_UNIT_SINGLE,
            (1.0f32 / 4096.0).to_bits().to_be_bytes().to_vec(),
        );
        // One span covering the tick block and the counter that follows it.
        g.put(TICK_SCALE_GLOBAL, vec![0u8; 48]);
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn the_table_is_the_image_dump_read_back() {
        // Spot-checks against the seven slots the C++ notes had identified independently, which
        // is the only cross-check available for a table read out of one dump.
        assert_eq!(TABLE[23].guest, 0x82B1_D1A8, "note: slot 23 is +0 minus +4");
        assert_eq!(TABLE[24].guest, 0x82B1_D1B8, "note: slot 24 is the product");
        assert_eq!(TABLE[25].guest, 0x82B1_D1C8, "note: slot 25 at table+0x64");
        assert_eq!(TABLE[29].guest, 0x82B1_D5D0, "note: slot 29 at table+0x74");
        assert_eq!(TABLE[30].guest, 0x82B1_D700, "note: slot 30 at table+120");
        assert_eq!(TABLE[31].guest, 0x82B1_D790, "note: slot 31 at table+0x7C");
        assert_eq!(TABLE[36].guest, 0x82B1_D198, "note: slot 36 is the sum");

        // Every row names itself consistently, and every gap says why.
        for (i, slot) in TABLE.iter().enumerate() {
            assert_eq!(
                slot.name,
                format!("sub_{:08X}", slot.guest),
                "slot {i} name disagrees with its address"
            );
            assert_eq!(
                slot.port.is_none(),
                !slot.absent.is_empty(),
                "slot {i} must have a port or a reason, never both or neither"
            );
        }
    }

    #[test]
    fn thirty_three_of_forty_slots_are_ported() {
        assert_eq!(ported_slots(), 33);
        assert_eq!(TABLE.len(), TABLE_SLOTS);
    }

    #[test]
    fn dispatch_routes_by_opcode_and_refuses_the_gaps() {
        let mut g = block_guest();
        put_words(&mut g, &[7, 5]);
        // Slot 36 is the sum, slot 23 the difference, slot 33 the minimum.
        assert_eq!(dispatch(&mut g, 36, BLOCK).unwrap(), 12);
        assert_eq!(dispatch(&mut g, 23, BLOCK).unwrap(), 2);
        assert_eq!(dispatch(&mut g, 33, BLOCK).unwrap(), 5);

        // An unported slot is an error naming the function, not a zero.
        let err = dispatch(&mut g, 4, BLOCK).unwrap_err();
        assert_eq!(err.address, 0x82B1_C150);
        assert!(err.message.contains("sub_82B1C150"), "{}", err.message);
        assert!(err.message.contains("gate 1"), "{}", err.message);

        // Past the table, including the interpreter's own end-of-stream marker.
        let err = dispatch(&mut g, 255, BLOCK).unwrap_err();
        assert_eq!(err.address, TABLE_BASE + 4 * 255);
        assert!(
            err.message.contains("past the 40-entry table"),
            "{}",
            err.message
        );
    }

    #[test]
    fn the_rodata_addresses_match_the_lis_addi_pairs_the_bodies_form() {
        // The compile-time asserts above cover the arithmetic; these pin the values the dump
        // shows, so a future edit that moves a cell has to move the expectation too.
        let mut g = block_guest();
        put_rodata(&mut g);
        assert_eq!(g.f32(ZERO_SINGLE).unwrap(), 0.0);
        assert_eq!(g.f32(HALF_SINGLE).unwrap(), 0.5);
        assert_eq!(g.f32(MINUS_ONE_SINGLE).unwrap(), -1.0);
        assert_eq!(g.f32(RATE_UNIT_SINGLE).unwrap(), 1.0 / 4096.0);
        // The tick scale is a runtime global and starts life at zero, which is a division by
        // zero for three of the ops. Recorded here so nobody reads the dump's 0.0 as a constant.
        assert_eq!(g.f32(TICK_SCALE_GLOBAL).unwrap(), 0.0);
    }
}
