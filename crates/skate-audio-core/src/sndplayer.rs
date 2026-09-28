//! `SndPlayer1`'s process: render one block of a stream (`sub_82B34278`), and the fade-out path it
//! hands the whole call to (`sub_82B34108`).
//!
//! **Unverified.** `sub_82B34278`'s C++ body is gate 1 (through [`deliver_frames`], whose fill is an
//! indirect call); `sub_82B34108` has no port at all. Both are transcribed: the first from its C++
//! body, the second from the lifted instructions. Only unit tests stand behind them.
//!
//! This is the voice graph's source (`docs/audio-banks.md`, the class table). Frames come from
//! [`deliver_frames`], whose [`StreamFill`] is where the engine's own decoded PCM plugs in. The
//! object layout, the 48-byte record and the descriptor pair are named in the constants below, all
//! as read from the C++ body's own names.

use crate::cursors::advance_ring_cursor;
use crate::fp::{fcfid, fctidz, frsp, load_single, store_single};
use crate::leaves::stream_remaining;
use crate::mem::memset;
use crate::stream::{StreamFill, deliver_frames};
use crate::{Guest, Result};

/// `0x822F8600 + 256`: the pool's zero double, compared against a record's start time and stored back.
pub const ZERO_DOUBLE: u32 = 0x822F_8700;
/// `0x822F8600 + 1096`: the "too far ahead to schedule" bound.
pub const DELAY_LIMIT: u32 = 0x822F_8A48;
/// `lis -32206` + -22460: 1.0, the fade's numerator.
pub const ONE_SINGLE: u32 = 0x8231_A844;

fn word(value: u64) -> i32 {
    value as u32 as i32
}

/// `object + [object+464] + 48*[object+469]`.
fn current_record(g: &Guest, object: u32) -> Result<u32> {
    let cursor = g.u8(object + 469)? as u32;
    let table = g.u16(object + 464)? as u32;
    Ok((48 * cursor).wrapping_add(table).wrapping_add(object))
}

/// Record states 0 and 4 are "not running".
fn running(g: &Guest, record: u32) -> Result<bool> {
    let state = g.u8(record + 46)?;
    Ok(state != 4 && state != 0)
}

/// The consumer slot: `object + 16*[object+474]`.
fn consumer_slot(g: &Guest, object: u32) -> Result<u32> {
    Ok(((g.u8(object + 474)? as u32) << 4).wrapping_add(object))
}

/// Take `bytes` rounded up to 128 off the holder's scratch pointer. Returns (entry value, new value).
fn take_scratch(g: &mut Guest, object: u32, bytes: u64) -> Result<(u32, u32)> {
    let owner = g.u32(object + 8)?;
    let holder = g.u32(owner)?;
    let rounded = ((bytes as u32).wrapping_add(127)) & !127;
    let saved = g.u32(holder + 32)?;
    let now = saved.wrapping_sub(rounded);
    g.set_u32(holder + 32, now)?;
    Ok((saved, now))
}

/// `loc_82B34974`: the record's format no longer matches; publish it and render nothing.
fn publish_format(g: &mut Guest, object: u32, pair: u32, record: u32) -> Result<()> {
    g.set_u32(pair + 48, 0)?;
    g.set_u8(pair + 60, g.u8(record + 47)?)?;
    let rate = load_single(g, record + 16)?;
    store_single(g, pair + 52, rate)?;
    let rate = load_single(g, record + 16)?;
    store_single(g, object + 456, rate)?;
    g.set_u8(object + 42, g.u8(record + 47)?)
}

/// `sub_82B34108`: with a fade pending (`+472` frames) after something was rendered, ramp each
/// channel's last sample (the table at `+462`) toward zero for up to one block, and publish that.
/// Returns 1.
pub fn fade_block(g: &mut Guest, object: u32, pair: u32) -> Result<u64> {
    let table = g.u16(object + 462)? as u32;
    let fade = g.u8(object + 472)? as u32;
    let block = g.u16(object + 460)? as u32;
    let mut tail = table.wrapping_add(object);
    let desc = g.u32(pair + 32)?;
    let frames = if fade < block { fade } else { block }; // cmplw: unsigned
    if g.u8(object + 42)? != 0 {
        let stride = g.u16(desc + 14)? as u32;
        let mut data = g.u32(desc + 4)?;
        let one = load_single(g, ONE_SINGLE)?;
        let mut channels = g.u8(object + 42)? as u32; // loaded once, counted down by addic.
        let inverse = frsp(one / frsp(fcfid(fade as i64))); // fdivs f12,f0,f12
        loop {
            let mut value = load_single(g, tail)?;
            let step = frsp(value * inverse); // fmuls f0,f13,f12
            // The original unrolls this by four and stores the running value back after each of the
            // two loops; the subtractions are sequential either way, so one loop is the same values
            // in the same order, and the last store is the one that remains.
            for i in 0..frames {
                value = frsp(value - step);
                store_single(g, data.wrapping_add(4 * i), value)?;
            }
            if frames > 0 {
                store_single(g, tail, value)?;
            }
            channels = channels.wrapping_sub(1);
            data = data.wrapping_add((stride << 2) & 0xFFFF_FFFF);
            tail = tail.wrapping_add(4);
            if channels == 0 {
                break;
            }
        }
    }
    let left = fade.wrapping_sub(frames);
    g.set_u8(object + 472, left as u8)?;
    let first = g.u32(pair + 28)?;
    let second = g.u32(pair + 32)?;
    g.set_u32(pair + 32, first)?;
    g.set_u32(pair + 28, second)?;
    g.set_u8(pair + 60, g.u8(object + 42)?)?;
    let rate = load_single(g, object + 456)?;
    store_single(g, pair + 52, rate)?;
    g.set_u32(pair + 48, frames)?;
    if g.u8(object + 472)? == 0 {
        g.set_u8(object + 471, 0)?;
    }
    Ok(1)
}

/// `sub_82B34278`: render one block from the current record into the descriptor pair.
/// Returns 1, or 0 when nothing at all was rendered or discarded and the block size is non-zero.
pub fn render_block(
    g: &mut Guest,
    fill: &mut dyn StreamFill,
    object: u32,
    pair: u32,
) -> Result<u64> {
    if g.u8(object + 472)? != 0 && g.u8(object + 471)? != 0 {
        return fade_block(g, object, pair);
    }
    let mut skipped: u64 = 0; // r23
    let mut produced: u64 = 0; // r28
    let mut scratch_saved: u32 = 0; // r21
    let mut scratch_now: u32 = 0; // r20

    g.set_u8(object + 472, 0)?;
    g.set_u32(pair + 48, 0)?;
    g.set_u32(object + 420, 0)?;
    let mut record = current_record(g, object)?;

    'tail: {
        if !running(g, record)? {
            break 'tail;
        }
        if word(g.u32(record + 20)? as u64) == 0 {
            loop {
                g.set_u8(record + 46, 4)?;
                advance_ring_cursor(g, object)?; // sub_82B349A8
                record = current_record(g, object)?;
                if !running(g, record)? {
                    break 'tail;
                }
                if word(g.u32(record + 20)? as u64) != 0 {
                    break;
                }
            }
        }
        let state = g.u8(record + 46)?;
        if state != 2 && state != 3 {
            break 'tail;
        }

        // The format tests; a NaN rate takes the change path.
        let rate = load_single(g, record + 16)?;
        if rate != load_single(g, object + 456)? || g.u8(record + 47)? != g.u8(object + 42)? {
            publish_format(g, object, pair, record)?;
            return Ok(1);
        }

        // The consumer slot must be ready; an empty one steps the cursor toward the producer.
        let slot = consumer_slot(g, object)?;
        if g.u8(slot + 113)? == 0 {
            let producer = g.u8(object + 473)?;
            loop {
                let at = g.u8(object + 474)?;
                if at == producer {
                    break;
                }
                let next = at.wrapping_add(1);
                let wrapped = if next == 20 { 0 } else { next };
                g.set_u8(object + 474, wrapped)?;
                let stepped = ((wrapped as u32) << 4).wrapping_add(object);
                if g.u8(stepped + 113)? != 0 {
                    break;
                }
            }
        }
        let ready = consumer_slot(g, object)?;
        if g.u8(ready + 113)? != 1 {
            break 'tail;
        }

        // The start timestamp: silence until it arrives.
        let start = f64::from_bits(g.u64(record)?);
        let pool_bits = g.u64(ZERO_DOUBLE)?;
        let pool = f64::from_bits(pool_bits);
        if start != pool {
            let now = f64::from_bits(g.u64(pair + 16)?);
            let ahead = start - now; // fsub: double
            let (delay, have) = if ahead > pool {
                let format = g.u32(pair + 40)?;
                let limit = load_single(g, DELAY_LIMIT)?;
                let source = load_single(g, format + 12)?;
                let scaled = frsp(source * ahead);
                if !(scaled < limit) {
                    // The original loads a stale frame word here; it is dead, `have` stays false.
                    (0u64, false)
                } else {
                    let per_second = load_single(g, pair + 56)?;
                    let frames = frsp(per_second * scaled);
                    ((fctidz(frames) as u64) & 0xFFFF_FFFF, true) // lwz r26,84(r1): the low word
                }
            } else {
                (0u64, true)
            };
            if !have {
                g.set_u32(object + 432, 0)?;
                break 'tail;
            }
            if delay != 0 {
                let block = g.u16(object + 460)? as u64;
                let delay = if (delay as u32) < (block as u32) {
                    delay
                } else {
                    block
                };
                let bytes = ((delay as u32) << 2) as u64;
                let desc = g.u32(pair + 32)?;
                if g.u8(record + 47)? != 0 {
                    let mut lane: u64 = 0;
                    loop {
                        let stride = g.u16(desc + 14)? as u64;
                        let data = g.u32(desc + 4)?;
                        let product = word(stride) as i64 * word(lane) as i64;
                        let at = ((product as u32) << 2).wrapping_add(data);
                        memset(g, at, 0, bytes)?; // sub_82EE5E80
                        lane += 1;
                        if !((lane as u32) < g.u8(record + 47)? as u32) {
                            break;
                        }
                    }
                }
                let was_first = g.u32(pair + 28)?;
                let was_second = g.u32(pair + 32)?;
                g.set_u32(pair + 48, delay as u32)?;
                g.set_u32(pair + 32, was_first)?;
                g.set_u32(pair + 28, was_second)?;
                g.set_u8(pair + 60, g.u8(record + 47)?)?;
                let rate = load_single(g, record + 16)?;
                store_single(g, pair + 52, rate)?;
                g.set_u32(object + 432, 0)?;
                return Ok(1);
            }
            g.set_u64(record, pool_bits)?; // the delay is spent
        }

        // Render: discard the pre-roll 256 frames at a time, then one block.
        let want = g.u16(record + 44)? as u64;
        (scratch_saved, scratch_now) = take_scratch(g, object, want)?;
        let slot = consumer_slot(g, object)?;
        let stream = g.u32(record + 8)?;
        g.set_u32(object + 420, stream)?;
        let id = g.u8(slot + 112)?;
        let available = stream_remaining(g, stream, id)?; // sub_82B23C10
        produced = g.u32(record + 28)? as u64;
        if word(available) < word(produced) {
            produced = available;
        }
        let block = g.u16(object + 460)? as u64;
        let rest = available.wrapping_sub(produced);
        let render = if word(block) < word(rest) {
            block
        } else {
            rest
        };
        let desc = g.u32(pair + 32)?;
        if word(produced) != 0 {
            loop {
                let take = if word(produced) < 256 { produced } else { 256 };
                let from = g.u32(object + 420)?;
                produced = produced.wrapping_sub(take);
                let got = deliver_frames(g, fill, from, desc, take)?;
                skipped = got.wrapping_add(skipped);
                if word(produced) == 0 {
                    break;
                }
            }
        }
        let from = g.u32(object + 420)?;
        produced = deliver_frames(g, fill, from, desc, render)?;

        if word(produced) > 0 {
            g.set_u8(object + 471, 1)?;
            let cap = g.u8(object + 466)? as u32;
            let have = g.u8(record + 47)? as u32;
            let lanes = (if have < cap { have } else { cap }) & 0xFF;
            let table = (g.u16(object + 462)? as u32).wrapping_add(object);
            for lane in 0..lanes {
                let stride = g.u16(desc + 14)? as u64;
                let data = g.u32(desc + 4)?;
                let product = word(stride) as i64 * lane as i64;
                let at = (((product as u64).wrapping_add(produced) as u32) << 2).wrapping_add(data);
                let last = load_single(g, at.wrapping_sub(4))?;
                store_single(g, table.wrapping_add(4 * lane), last)?;
            }
        }

        // Publish the block and swap the pair.
        let was_second = g.u32(pair + 32)?;
        let was_first = g.u32(pair + 28)?;
        g.set_u32(pair + 48, produced as u32)?;
        g.set_u32(pair + 28, was_second)?;
        g.set_u32(pair + 32, was_first)?;
        g.set_u8(pair + 60, g.u8(record + 47)?)?;
        let rate = load_single(g, record + 16)?;
        store_single(g, pair + 52, rate)?;
        let param = load_single(g, record + 12)?;
        let position = g.u32(object + 432)?;
        store_single(g, object + 424, param)?;
        if word(position as u64) == 0 {
            let span_b = g.u32(record + 36)?;
            let span_a = g.u32(record + 32)?;
            g.set_u32(object + 432, span_b.wrapping_add(span_a))?;
        }

        let at_position = g.u32(object + 432)? as u64;
        let delivered = produced.wrapping_add(skipped);
        let cursor = g.u8(object + 474)? as u32;
        let unused = available.wrapping_sub(produced);
        g.set_u32(object + 432, delivered.wrapping_add(at_position) as u32)?;
        let mut leftover = unused.wrapping_sub(skipped); // r6 at loc_82B34894
        let live = (cursor << 4).wrapping_add(object);
        let rate = load_single(g, record + 16)?;
        store_single(g, object + 428, rate)?;
        g.set_u32(object + 436, g.u32(record + 20)?)?;
        let counted = (g.u32(live + 108)? as u64).wrapping_add(produced);
        g.set_u32(live + 108, counted.wrapping_add(skipped) as u32)?;

        let total = g.u32(record + 20)?;
        if word(g.u32(object + 432)? as u64) == word(total as u64) {
            let loop_point = g.u32(record + 24)?;
            if (loop_point as i32) >= 0 {
                g.set_u32(object + 432, loop_point)?;
            } else {
                g.set_u8(record + 46, 4)?;
                if g.u32(object + 420)? != 0 {
                    let owner = g.u32(object + 8)?;
                    g.set_u32(object + 420, 0)?;
                    let holder = g.u32(owner)?;
                    g.set_u32(holder + 32, scratch_saved)?;
                }
                advance_ring_cursor(g, object)?;
                let next = current_record(g, object)?;
                if running(g, next)? && g.u32(next + 8)? != 0 {
                    let want_next = g.u16(next + 44)? as u64;
                    (scratch_saved, scratch_now) = take_scratch(g, object, want_next)?;
                    g.set_u32(object + 420, g.u32(next + 8)?)?;
                }
            }
        }

        // loc_82B34894: anything left means the slot is not finished.
        if word(leftover) != 0 {
            break 'tail;
        }
        loop {
            // loc_82B3489C: retire the consumer slot, step, and ask about the next one.
            let slot = consumer_slot(g, object)?;
            if g.u8(slot + 113)? != 1 {
                break 'tail;
            }
            g.set_u8(slot + 113, 2)?;
            let stream = g.u32(object + 420)?;
            let at = g.u8(object + 474)?;
            let next = at.wrapping_add(1);
            let wrapped = if next == 20 { 0 } else { next };
            g.set_u8(object + 474, wrapped)?;
            if stream != 0 {
                let stepped = (((wrapped as u32) << 4) & 0xFF0).wrapping_add(object);
                if g.u8(stepped + 113)? == 1 {
                    let id = g.u8((16 * (wrapped as u32 + 7)).wrapping_add(object))?;
                    leftover = stream_remaining(g, stream, id)?;
                }
            }
            // loc_82B34910
            if word(leftover) != 0 {
                break 'tail;
            }
        }
    }

    // loc_82B34918
    if g.u32(object + 420)? != 0 {
        let restore = scratch_now != 0;
        g.set_u32(object + 420, 0)?;
        if restore {
            let owner = g.u32(object + 8)?;
            let holder = g.u32(owner)?;
            g.set_u32(holder + 32, scratch_saved)?;
        }
    }
    g.set_u8(pair + 60, g.u8(object + 42)?)?;
    let rate = load_single(g, object + 456)?;
    store_single(g, pair + 52, rate)?;
    if word(produced) == 0 && word(skipped) == 0 && g.u16(object + 460)? != 0 {
        Ok(0)
    } else {
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x5000_0000;
    const OBJ: u32 = MEM;
    const PAIR: u32 = MEM + 0x400;
    const DESC_A: u32 = MEM + 0x480;
    const DESC_B: u32 = MEM + 0x4C0;
    const DATA: u32 = MEM + 0x800;
    const RING: u16 = 0x200; // records at OBJ+0x200
    const TAIL: u16 = 0x1E0;

    struct NoFill;
    impl StreamFill for NoFill {
        fn fill(&mut self, _g: &mut Guest, _s: u32, _d: u32, _f: u64) -> Result<u64> {
            panic!("no stream should be read on this path")
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x1000);
        g.put(0x822F_8700, vec![0; 8]); // ZERO_DOUBLE
        g.put(0x822F_8A48, 1_000_000.0f32.to_bits().to_be_bytes().to_vec());
        g.put(0x8231_A844, 1.0f32.to_bits().to_be_bytes().to_vec());
        g.set_u16(OBJ + 460, 4).unwrap(); // block frames
        g.set_u16(OBJ + 462, TAIL).unwrap();
        g.set_u16(OBJ + 464, RING).unwrap();
        g.set_u8(OBJ + 42, 1).unwrap(); // one channel
        g.set_u32(OBJ + 456, 48000.0f32.to_bits()).unwrap();
        g.set_u32(PAIR + 28, DESC_A).unwrap();
        g.set_u32(PAIR + 32, DESC_B).unwrap();
        g.set_u32(DESC_B + 4, DATA).unwrap();
        g.set_u16(DESC_B + 14, 16).unwrap();
        let rec = OBJ + RING as u32;
        g.set_u8(rec + 46, 2).unwrap(); // running
        g.set_u32(rec + 20, 1000).unwrap(); // total
        g.set_u32(rec + 16, 48000.0f32.to_bits()).unwrap();
        g.set_u8(rec + 47, 1).unwrap();
        g
    }

    const STREAM: u32 = MEM + 0xA00;

    /// Writes `n + 1.0` as floats straight into the descriptor, counting across calls.
    struct FloatRamp {
        next: f32,
    }
    impl StreamFill for FloatRamp {
        fn fill(
            &mut self,
            g: &mut Guest,
            _stream: u32,
            descriptor: u32,
            frames: u64,
        ) -> Result<u64> {
            let data = g.u32(descriptor + 4)?;
            for i in 0..frames as u32 {
                self.next += 1.0;
                g.set_u32(data + 4 * i, self.next.to_bits())?;
            }
            Ok(frames)
        }
    }

    /// A running record on a real stream object: one ready slot, no scratch, a stream of `limit`.
    fn with_stream(limit: u32, loop_point: i32) -> Guest {
        let mut g = guest();
        let rec = OBJ + RING as u32;
        g.set_u32(rec + 8, STREAM).unwrap();
        g.set_u32(rec + 20, limit).unwrap();
        g.set_u32(rec + 24, loop_point as u32).unwrap();
        g.set_u8(OBJ + 108 + 5, 1).unwrap(); // slot 0 ready, stream id 0
        g.set_u8(OBJ + 466, 1).unwrap(); // tail table holds one channel
        g.set_u32(OBJ + 8, MEM + 0x700).unwrap(); // owner -> holder -> scratch pointer
        g.set_u32(MEM + 0x700, MEM + 0x720).unwrap();
        g.set_u32(MEM + 0x720 + 32, MEM + 0x900).unwrap();
        g.set_u32(STREAM + 36, 64).unwrap(); // entry table
        g.set_u32(STREAM + 64 + 12, limit).unwrap();
        g.set_u8(STREAM + 46, 1).unwrap();
        g.set_u8(STREAM + 50, 1).unwrap();
        g
    }

    #[test]
    fn a_block_is_rendered_its_last_sample_latched_and_the_position_advanced() {
        let mut g = with_stream(1000, -1);
        let mut fill = FloatRamp { next: 0.0 };
        assert_eq!(render_block(&mut g, &mut fill, OBJ, PAIR).unwrap(), 1);
        let block: Vec<f32> = (0..4).map(|i| g.f32(DATA + 4 * i).unwrap()).collect();
        assert_eq!(block, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(g.u32(PAIR + 48).unwrap(), 4);
        assert_eq!(
            g.f32(OBJ + TAIL as u32).unwrap(),
            4.0,
            "the last sample, for a later fade"
        );
        assert_eq!(g.u8(OBJ + 471).unwrap(), 1);
        assert_eq!(g.u32(OBJ + 432).unwrap(), 4);
        assert_eq!(g.u32(OBJ + 108).unwrap(), 4, "the slot's delivered count");
        assert_eq!(g.u32(STREAM + 28).unwrap(), 4, "the stream's own position");
        assert_eq!(
            (g.u32(PAIR + 28).unwrap(), g.u32(PAIR + 32).unwrap()),
            (DESC_B, DESC_A)
        );
        assert_eq!(
            g.u32(OBJ + 420).unwrap(),
            0,
            "the live stream is cleared on the way out"
        );
        assert_eq!(
            g.u32(MEM + 0x720 + 32).unwrap(),
            MEM + 0x900,
            "the scratch pointer handed back"
        );
    }

    #[test]
    fn a_record_that_reaches_its_end_loops_and_its_slot_retires() {
        let mut g = with_stream(4, 0);
        let mut fill = FloatRamp { next: 0.0 };
        assert_eq!(render_block(&mut g, &mut fill, OBJ, PAIR).unwrap(), 1);
        assert_eq!(g.u32(OBJ + 432).unwrap(), 0, "looped to the loop point");
        assert_eq!(
            g.u8(OBJ + 108 + 5).unwrap(),
            2,
            "the slot that ran dry is retired"
        );
        assert_eq!(
            g.u8(OBJ + 474).unwrap(),
            1,
            "and the consumer stepped past it"
        );
        assert_eq!(
            g.u8(OBJ + RING as u32 + 46).unwrap(),
            2,
            "a looping record keeps running"
        );
    }

    #[test]
    fn a_stopped_record_renders_nothing_and_republishes_the_format() {
        let mut g = guest();
        g.set_u8(OBJ + RING as u32 + 46, 0).unwrap();
        assert_eq!(render_block(&mut g, &mut NoFill, OBJ, PAIR).unwrap(), 0);
        assert_eq!(g.u8(PAIR + 60).unwrap(), 1);
        assert_eq!(g.f32(PAIR + 52).unwrap(), 48000.0);
    }

    #[test]
    fn a_rate_change_publishes_the_record_format_and_stops() {
        let mut g = guest();
        g.set_u32(OBJ + RING as u32 + 16, 44100.0f32.to_bits())
            .unwrap();
        assert_eq!(render_block(&mut g, &mut NoFill, OBJ, PAIR).unwrap(), 1);
        assert_eq!(g.f32(OBJ + 456).unwrap(), 44100.0);
        assert_eq!(g.u32(PAIR + 48).unwrap(), 0);
    }

    #[test]
    fn a_future_start_time_emits_a_block_of_silence() {
        let mut g = guest();
        let rec = OBJ + RING as u32;
        g.set_u8(OBJ + 108 + 5, 1).unwrap(); // slot 0 ready
        g.set_u64(rec, 1.0f64.to_bits()).unwrap(); // starts at t = 1 s
        g.set_u64(PAIR + 16, 0.0f64.to_bits()).unwrap(); // now = 0
        g.set_u32(PAIR + 40, MEM + 0x600).unwrap(); // format object
        g.set_u32(MEM + 0x600 + 12, 1.0f32.to_bits()).unwrap();
        g.set_u32(PAIR + 56, 48000.0f32.to_bits()).unwrap(); // seconds to frames
        g.set_u32(DATA, 0xDEAD_BEEF).unwrap();
        assert_eq!(render_block(&mut g, &mut NoFill, OBJ, PAIR).unwrap(), 1);
        assert_eq!(g.u32(PAIR + 48).unwrap(), 4, "clamped to one block");
        assert_eq!(g.u32(DATA).unwrap(), 0, "silence");
        assert_eq!(
            (g.u32(PAIR + 28).unwrap(), g.u32(PAIR + 32).unwrap()),
            (DESC_B, DESC_A),
            "swapped"
        );
    }

    #[test]
    fn the_fade_path_ramps_the_last_sample_to_zero_over_the_pending_frames() {
        let mut g = guest();
        g.set_u8(OBJ + 471, 1).unwrap();
        g.set_u8(OBJ + 472, 8).unwrap();
        g.set_u32(OBJ + TAIL as u32, 1.0f32.to_bits()).unwrap();
        assert_eq!(render_block(&mut g, &mut NoFill, OBJ, PAIR).unwrap(), 1);
        let first: Vec<f32> = (0..4).map(|i| g.f32(DATA + 4 * i).unwrap()).collect();
        assert_eq!(first, vec![0.875, 0.75, 0.625, 0.5]);
        assert_eq!(g.u8(OBJ + 472).unwrap(), 4);
        assert_eq!(g.f32(OBJ + TAIL as u32).unwrap(), 0.5);
        // The pair swapped, so the next block lands in the other descriptor.
        g.set_u32(DESC_A + 4, DATA + 0x100).unwrap();
        g.set_u16(DESC_A + 14, 16).unwrap();
        render_block(&mut g, &mut NoFill, OBJ, PAIR).unwrap();
        let second: Vec<f32> = (0..4)
            .map(|i| g.f32(DATA + 0x100 + 4 * i).unwrap())
            .collect();
        assert_eq!(second, vec![0.375, 0.25, 0.125, 0.0]);
        assert_eq!(
            (g.u8(OBJ + 472).unwrap(), g.u8(OBJ + 471).unwrap()),
            (0, 0),
            "fade done clears +471"
        );
    }
}
