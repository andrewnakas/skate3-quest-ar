//! `sub_82B1E290` — the evaluator's tick and interpreter.
//!
//! **Unverified, and new work rather than a transcription of a verified body.** The function fails
//! the port screen's gate 1 (its `bctrl` dispatches a data word) and gate 2 (its write set is
//! whatever the program bytes address), so its C++ body in `recomp/src/audio_ports/` carries
//! `STATUS: gate-1` and was never compared. This follows the lifted code line by line and is unit
//! tested; the evidence that it is right on real programs is `skate-audio-formats`'
//! `verify_patch_programs`, which walks all 385 bank programs with the same record grammar and ends
//! every one exactly at its operand area's last byte. Nothing has replayed it against the game.
//!
//! Called once per audio frame with the frame delta. When the delta differs from the cached one it
//! recounts how many deltas fit in one update period and republishes the tick-scale global the
//! timer, ramp and delay-ring ops read. It then counts a countdown down, and on the frame it reaches
//! zero reloads it and walks the node list, running each node's program:
//!
//! ```text
//! node    { +0 next, +8 program, +12 operand block }
//! record  { u8 opcode, u8 pairs, u16 unread, pairs x { i32 src, i32 dst }, i32 block_advance }
//! ```
//!
//! Per record the op in `opcode`'s slot runs over the block. For each pair whose `src` is -1 the
//! low word of its result is stored at `block + dst`, and any other `src` copies
//! `block[src]` to `block[dst]`. Then the block pointer moves by `block_advance`, and opcode 255
//! ends the stream. How nodes get onto the list: `docs/audio-banks.md`, "From a message to the
//! evaluator".
//!
//! Two places are reproduced rather than guarded, as the C++ does, with one exception. The recount
//! loop never ends when the delta cannot advance its accumulator: the original spins there forever,
//! and here that state is detected exactly (the accumulator did not change, and nothing else feeds
//! the loop test) and returned as an error instead of hanging. A countdown of 0 in memory decrements
//! to `0xFFFF_FFFF` and does not tick.

use super::dispatch;
use crate::fp::{fcfid, frsp, load_single, store_single};
use crate::{Error, Guest, Result};

/// `lis -31993` + 30168: f32, `float(float(count) * delta) * SCALE_UNIT`. Read by the timer ops.
pub const SCALE_GLOBAL: u32 = 0x8307_75D8;
/// `+30172`: f32, the delta the current count was made for.
pub const DELTA_CACHE: u32 = 0x8307_75DC;
/// `+30176`: u32, deltas per period, the countdown's reload value.
pub const FRAME_COUNT: u32 = 0x8307_75E0;
/// `+30180`: u32, frames left until the next walk.
pub const COUNTDOWN: u32 = 0x8307_75E4;
/// `lis -32206` + -22460: the period's numerator, 1.0 in the image dump.
pub const PERIOD_NUMER: u32 = 0x8231_A844;
/// `lis -32003` + 13812: the period's denominator. 41.6 in the boot-time image dump, but game init
/// (`sub_826D4C30`) overwrites it with 30.0 from `0x820D4924` and zeroes [`DELTA_CACHE`]. So in play
/// one period is 1/30 s.
pub const PERIOD_DENOM: u32 = 0x82FD_35F4;
/// `lis -32234` + 23056: 0.0, the accumulator's start.
pub const ZERO_SINGLE: u32 = 0x8216_5A10;
/// `lis -32219` + 28648: the scale unit, 1000.0 in the image dump. So the scale global is the
/// period's length in milliseconds as the frames actually tile it: `count * delta * 1000`.
pub const SCALE_UNIT: u32 = 0x8225_6FE8;
/// `lis -31997` + 28492: head of the node list.
pub const LIST_HEAD: u32 = 0x8303_6F4C;
/// The opcode that ends a program.
pub const END_OPCODE: u8 = 255;

const NODE_NEXT: u32 = 0;
const NODE_PROGRAM: u32 = 8;
const NODE_BLOCK: u32 = 12;
const RECORD_PAIRS: u32 = 1;
const RECORD_BODY: u32 = 4;
const PAIR_BYTES: u32 = 8;
const SRC_IS_RESULT: i32 = -1;

/// Opcodes the pure table cannot run because they touch the runtime: allocation, other threads'
/// objects, voices. The host gets the first say on any slot without a pure port.
pub trait Host {
    /// Run `opcode` over `block` if this host implements it; `None` leaves it unported.
    fn op(&mut self, g: &mut Guest, opcode: u8, block: u32) -> Option<Result<u64>>;
}

/// A host that implements nothing, for callers that only need the pure table.
pub struct NoHost;

impl Host for NoHost {
    fn op(&mut self, _g: &mut Guest, _opcode: u8, _block: u32) -> Option<Result<u64>> {
        None
    }
}

/// What one call did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// Whether the delta differed from the cached one, so the period was recounted.
    pub recounted: bool,
    /// Whether the countdown reached zero, so the node list was walked.
    pub walked: bool,
    /// Nodes visited and ops dispatched on the walk.
    pub nodes: usize,
    pub ops: usize,
}

/// One call of `sub_82B1E290` with `delta` in `f1`, running only the pure table.
pub fn tick(g: &mut Guest, delta: f64) -> Result<Tick> {
    tick_with(g, delta, &mut NoHost)
}

/// One call of `sub_82B1E290`, with `host` running the slots the pure table does not port.
pub fn tick_with(g: &mut Guest, delta: f64, host: &mut dyn Host) -> Result<Tick> {
    let mut out = Tick {
        recounted: false,
        walked: false,
        nodes: 0,
        ops: 0,
    };
    let cached = load_single(g, DELTA_CACHE)?; // lfs f0,30172(r11)

    let (countdown, period): (u64, u64);
    // fcmpu cr6,f1,f0 ; beq -- exact equality; a NaN on either side takes the recount.
    if delta != cached {
        out.recounted = true;
        store_single(g, DELTA_CACHE, delta)?; // stfs f1,30172(r11)
        let numer = load_single(g, PERIOD_NUMER)?;
        let denom = load_single(g, PERIOD_DENOM)?;
        let limit = frsp(numer / denom); // fdivs f13,f13,f12
        let mut accumulated = load_single(g, ZERO_SINGLE)?; // lfs f0,23056(r5)
        let mut count: u64 = 0; // li r11,0
        loop {
            let before = accumulated;
            accumulated = frsp(accumulated + delta); // fadds f0,f0,f1
            count += 1; // addi r11,r11,1
            let next = frsp(accumulated + delta); // fadds f12,f0,f1
            // fcmpu cr6,f12,f13 ; blt -- a NaN clears lt and leaves.
            if !(next < limit) {
                break;
            }
            if accumulated.to_bits() == before.to_bits() {
                return Err(Error::new(
                    0x82B1_E2DC,
                    format!(
                        "delta {delta} cannot advance the period count; the original never returns"
                    ),
                ));
            }
        }
        g.set_u32(FRAME_COUNT, count as u32)?; // stw r11,30176(r9)
        // clrldi r10,r11,32 ; std r10,80(r1) ; lfd f0,80(r1) ; fcfid f13,f0 ; frsp f12,f13. The
        // spill through the frame is value-transparent: the doubleword stored is the masked count.
        let as_single = frsp(fcfid((count & 0xFFFF_FFFF) as i64));
        let scaled = frsp(as_single * delta); // fmuls f11,f12,f1
        let unit = load_single(g, SCALE_UNIT)?; // lfs f0,28648(r9)
        store_single(g, SCALE_GLOBAL, frsp(scaled * unit))?; // fmuls f0,f11,f0 ; stfs 30168(r7)
        countdown = count; // mr r10,r11
        period = count;
    } else {
        countdown = g.u32(COUNTDOWN)? as u64; // lwz r10,30180(r8)
        period = g.u32(FRAME_COUNT)? as u64; // lwz r11,30176(r9)
    }

    // addic. r10,r10,-1 ; stw r10,30180(r8) ; bne -- cr0 tests the low word.
    let countdown = countdown.wrapping_sub(1);
    g.set_u32(COUNTDOWN, countdown as u32)?;
    if countdown as u32 != 0 {
        return Ok(out);
    }
    out.walked = true;
    g.set_u32(COUNTDOWN, period as u32)?; // stw r11,30180(r8)

    let mut node = g.u32(LIST_HEAD)?; // lwz r11,28492(r10)
    while node != 0 {
        out.nodes += 1;
        let mut record = g.u32(node + NODE_PROGRAM)? as u64; // lwz r30,8(r11)
        let next_node = g.u32(node + NODE_NEXT)?; // lwz r28,0(r11)
        // lwz r31,12(r11). Kept 64-bit: `add r31,r11,r31` carries into bit 32 on a negative
        // advance, and every access truncates to the low word.
        let mut block = g.u32(node + NODE_BLOCK)? as u64;
        let mut opcode = g.u8(record as u32)?; // lbz r11,0(r30)
        while opcode != END_OPCODE {
            // lwzx r11,r11,r29 ; mr r3,r31 ; bctrl. `Op` takes the low word of r3.
            let pure = super::TABLE.get(opcode as usize).and_then(|slot| slot.port);
            let evaluated = match pure {
                Some(op) => op(g, block as u32),
                None => match host.op(g, opcode, block as u32) {
                    Some(result) => result,
                    None => dispatch(g, opcode, block as u32), // the error naming the slot
                },
            };
            let result = evaluated.map_err(|error| {
                Error::new(
                    error.address,
                    format!(
                        "evaluator opcode {opcode} at record {:#010x}, block {:#010x}: {}",
                        record as u32, block as u32, error.message
                    ),
                )
            })? as u32;
            out.ops += 1;
            let mut pair = record + RECORD_BODY as u64; // addi r11,r30,4
            let mut index: i32 = 0; // li r8,0
            // lbz r10,1(r30) is reloaded on every trip, so an op that rewrote its own record
            // changes the count mid-loop.
            while index < g.u8(record as u32 + RECORD_PAIRS)? as i32 {
                let src = g.u32(pair as u32)?; // lwz r10,0(r11)
                let dst = g.u32(pair as u32 + 4)?; // lwz r9,4(r11)
                let target = dst.wrapping_add(block as u32);
                if src as i32 == SRC_IS_RESULT {
                    g.set_u32(target, result)?; // stwx r3,r9,r31
                } else {
                    let word = g.u32(src.wrapping_add(block as u32))?; // lwzx r10,r10,r31
                    g.set_u32(target, word)?; // stwx r10,r9,r31
                }
                index += 1;
                pair += PAIR_BYTES as u64;
            }
            // loc_82B1E3C8: addi r30,r11,4 ; lwz r11,0(r11) ; add r31,r11,r31
            record = pair + 4;
            block = (g.u32(pair as u32)? as u64).wrapping_add(block);
            opcode = g.u8(record as u32)?;
        }
        node = next_node;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x4000_0000;
    const NODE_A: u32 = MEM;
    const NODE_B: u32 = MEM + 0x20;
    const PROGRAM_A: u32 = MEM + 0x100;
    const PROGRAM_B: u32 = MEM + 0x200;
    const BLOCK_A: u32 = MEM + 0x400;

    fn be(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// The globals and constants, each in its own segment so no `put` lands inside another.
    fn guest(numer: f32, denom: f32, unit: f32) -> Guest {
        let mut g = Guest::single(MEM, 0x800);
        g.put(SCALE_GLOBAL, vec![0; 16]);
        g.put(PERIOD_NUMER, numer.to_bits().to_be_bytes().to_vec());
        g.put(PERIOD_DENOM, denom.to_bits().to_be_bytes().to_vec());
        g.put(ZERO_SINGLE, vec![0; 4]);
        g.put(SCALE_UNIT, unit.to_bits().to_be_bytes().to_vec());
        g.put(LIST_HEAD, vec![0; 4]);
        g
    }

    fn put_words(g: &mut Guest, at: u32, words: &[u32]) {
        g.set_span(at, &be(words)).unwrap();
    }

    /// Two nodes. A: op 3 (take word 0) with a result pair and a copy pair, advance 16, then op 0
    /// (take word 16) over the moved block, then the end. B: an empty program.
    fn wire_programs(g: &mut Guest) {
        g.set_u32(LIST_HEAD, NODE_A).unwrap();
        put_words(g, NODE_A, &[NODE_B, 0, PROGRAM_A, BLOCK_A]);
        put_words(g, NODE_B, &[0, 0, PROGRAM_B, BLOCK_A]);
        put_words(
            g,
            PROGRAM_A,
            &[
                0x0302_0000,
                0xFFFF_FFFF,
                8,
                8,
                12,
                16,
                0x0000_0000,
                0,
                0xFF00_0000,
            ],
        );
        g.set_u8(PROGRAM_B, END_OPCODE).unwrap();
        put_words(g, BLOCK_A, &[0xAAAA]);
        g.set_u32(BLOCK_A + 32, 0xBBBB).unwrap();
    }

    #[test]
    fn a_new_delta_recounts_the_period_and_publishes_the_scale() {
        // limit = 50: 49 deltas of 1.0 fit before the next would reach it.
        let mut g = guest(1.0, 0.02, 0.5);
        let t = tick(&mut g, 1.0).unwrap();
        assert!(t.recounted && !t.walked);
        assert_eq!(g.u32(FRAME_COUNT).unwrap(), 49);
        assert_eq!(g.u32(COUNTDOWN).unwrap(), 48);
        assert_eq!(g.f32(SCALE_GLOBAL).unwrap(), 24.5);
        assert_eq!(g.f32(DELTA_CACHE).unwrap(), 1.0);
    }

    #[test]
    fn the_walk_stores_results_copies_and_moves_the_block() {
        let mut g = guest(1.0, 1.0, 1.0);
        wire_programs(&mut g);
        let t = tick(&mut g, 1.0).unwrap();
        assert_eq!(
            t,
            Tick {
                recounted: true,
                walked: true,
                nodes: 2,
                ops: 2
            }
        );
        assert_eq!(g.u32(BLOCK_A).unwrap(), 0, "op 3 takes the word");
        assert_eq!(g.u32(BLOCK_A + 8).unwrap(), 0xAAAA, "the result pair");
        assert_eq!(
            g.u32(BLOCK_A + 12).unwrap(),
            0xAAAA,
            "the copy pair runs after it"
        );
        assert_eq!(
            g.u32(BLOCK_A + 32).unwrap(),
            0,
            "op 0 ran 16 bytes further on"
        );
        assert_eq!(g.u32(COUNTDOWN).unwrap(), 1, "reloaded from the period");
    }

    #[test]
    fn the_same_delta_uses_the_stored_countdown() {
        let mut g = guest(1.0, 1.0, 1.0);
        wire_programs(&mut g);
        tick(&mut g, 1.0).unwrap();
        let t = tick(&mut g, 1.0).unwrap();
        assert!(!t.recounted && t.walked);
    }

    #[test]
    fn a_zero_countdown_wraps_and_does_not_walk() {
        let mut g = guest(1.0, 1.0, 1.0);
        wire_programs(&mut g);
        tick(&mut g, 1.0).unwrap();
        g.set_u32(COUNTDOWN, 0).unwrap();
        let t = tick(&mut g, 1.0).unwrap();
        assert!(!t.walked);
        assert_eq!(g.u32(COUNTDOWN).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn an_unported_opcode_is_an_error_not_a_value() {
        let mut g = guest(1.0, 1.0, 1.0);
        wire_programs(&mut g);
        g.set_u8(PROGRAM_A, 27).unwrap();
        let err = tick(&mut g, 1.0).unwrap_err();
        assert!(err.message.contains("sub_82B1D240"), "{}", err.message);
    }

    #[test]
    fn a_delta_that_cannot_advance_is_reported_instead_of_hanging() {
        let mut g = guest(1.0, 1.0, 1.0);
        // The cache must differ, or a zero delta equals the zeroed cache and never recounts.
        g.set_u32(DELTA_CACHE, 1.0f32.to_bits()).unwrap();
        let err = tick(&mut g, 0.0).unwrap_err();
        assert!(err.message.contains("never returns"), "{}", err.message);
    }

    #[test]
    fn a_nan_delta_counts_once_and_recounts_every_call() {
        let mut g = guest(1.0, 0.02, 1.0);
        let t = tick(&mut g, f64::NAN).unwrap();
        assert!(
            t.recounted && t.walked,
            "count 1, so the countdown hits zero at once"
        );
        assert_eq!(g.u32(FRAME_COUNT).unwrap(), 1);
        assert!(
            tick(&mut g, f64::NAN).unwrap().recounted,
            "NaN never equals the cache"
        );
    }
}
