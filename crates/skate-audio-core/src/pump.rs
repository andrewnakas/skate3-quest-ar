//! Pump a `SndPlayer1` segment ring (`sub_82B31EE0`).
//!
//! The retail pump owns the ring traversal, state transitions, retirement order, and published
//! time bounds. Its platform stream calls take a decoder lock, so they remain on [`PumpHost`].
//! This keeps the recovered guest-state contract in core while making an actual PCM backend supply
//! only the five source-specific operations.

use crate::play::{SLOT_STATE, record_address, slot_address};
use crate::{Error, Guest, Result};

pub const CLOCK: u32 = 8;
pub const OWNER: u32 = 12;
pub const SENTINEL: u32 = 48;
pub const CURRENT_RATE: u32 = 52;
pub const START_TIME: u32 = 56;
pub const END_TIME: u32 = 64;
pub const RECORDS: u32 = 96;
pub const RATE: u32 = 424;
pub const DIVISOR: u32 = 428;
pub const START_SAMPLES: u32 = 432;
pub const END_SAMPLES: u32 = 436;
pub const DEFAULT_SOURCE: u32 = 440;
pub const REFRESH_GATE: u32 = 444;
pub const RETIRE_INDEX: u32 = 468;
pub const FIRST_ACTIVE: u32 = 469;
pub const RING_SIZE: u32 = 470;
pub const SELECTOR: u32 = 473;

pub const OWNER_STATE: u32 = 71;
pub const OWNER_RATE: u32 = 56;
pub const SLOT_WORK: u32 = 20;
pub const SLOT_END: u32 = 24;
pub const RECORD_CURSOR: u32 = 20;
pub const RECORD_SINK: u32 = 36;
pub const RECORD_KIND: u32 = 72;
pub const SELECTOR_FLAG: u32 = 113;
pub const IDLE_SENTINEL: u32 = 0x7FF7_FFF1;

/// The two image constants the original loads during a pump.
#[derive(Clone, Copy, Debug)]
pub struct PumpConstants {
    pub far_future: f64,
    pub time_bump: f64,
}

/// Platform-specific stream work called by the pump.
///
/// Every `bool` matches the low byte returned by the corresponding retail helper. Returning
/// `false` stops this pass without moving to another ring slot.
pub trait PumpHost {
    fn preflight(&mut self, g: &mut Guest, stream: u32) -> Result<()>;
    fn retire(&mut self, g: &mut Guest, stream: u32, index: u8) -> Result<()>;
    fn prepare(&mut self, g: &mut Guest, stream: u32, index: u8, counter: &mut u32)
    -> Result<bool>;
    fn at_end(&mut self, g: &mut Guest, stream: u32, index: u8, counter: &mut u32) -> Result<bool>;
    /// Returns whether the source wrapped. A wrapped slot becomes state 3.
    fn at_work(
        &mut self,
        g: &mut Guest,
        stream: u32,
        index: u8,
        counter: &mut u32,
    ) -> Result<(bool, bool)>;
    fn submit(&mut self, g: &mut Guest, stream: u32, index: u8, counter: &mut u32) -> Result<bool>;
}

fn next_index(index: u8, limit: u8) -> Result<u8> {
    if limit == 0 {
        return Err(Error::new(0, "SndPlayer1 ring size is zero"));
    }
    let next = index.wrapping_add(1);
    Ok(if next == limit { 0 } else { next })
}

fn active(state: u8) -> bool {
    state != 0 && state != 4
}

fn selector_blocked(g: &Guest, stream: u32) -> Result<bool> {
    let selector = u32::from(g.u8(stream + SELECTOR)?);
    Ok(g.u8(stream.wrapping_add(selector * 16 + SELECTOR_FLAG))? != 0)
}

fn publish_idle(g: &mut Guest, stream: u32, constants: PumpConstants) -> Result<()> {
    let gate = g.u32(stream + REFRESH_GATE)?;
    if gate == 0 {
        return Err(Error::new(
            stream + REFRESH_GATE,
            "SndPlayer1 refresh gate is null",
        ));
    }
    if g.u32(gate)? == 0 {
        let source = g.u32(stream + DEFAULT_SOURCE)?;
        if source == 0 {
            return Err(Error::new(
                stream + DEFAULT_SOURCE,
                "SndPlayer1 default rate is null",
            ));
        }
        g.set_u32(stream + RATE, g.f32(source)?.to_bits())?;
    }
    g.set_u32(stream + CURRENT_RATE, g.f32(stream + RATE)?.to_bits())?;
    g.set_u32(stream + SENTINEL, IDLE_SENTINEL)?;
    g.set_u64(stream + START_TIME, constants.far_future.to_bits())?;
    g.set_u64(stream + END_TIME, constants.far_future.to_bits())
}

fn publish_active(g: &mut Guest, stream: u32) -> Result<()> {
    let divisor = f64::from(g.f32(stream + DIVISOR)?);
    let start = g.u32(stream + START_SAMPLES)? as i32;
    let end = g.u32(stream + END_SAMPLES)? as i32;
    g.set_u32(stream + CURRENT_RATE, g.f32(stream + RATE)?.to_bits())?;
    g.set_u32(stream + SENTINEL, IDLE_SENTINEL)?;
    g.set_u64(stream + START_TIME, (f64::from(start) / divisor).to_bits())?;
    g.set_u64(stream + END_TIME, (f64::from(end) / divisor).to_bits())
}

/// Run one stream pump pass. The caller owns the audio-thread synchronization required by its
/// decoder and may call this once per audio block.
pub fn pump<H: PumpHost + ?Sized>(
    g: &mut Guest,
    host: &mut H,
    stream: u32,
    constants: PumpConstants,
) -> Result<()> {
    let owner = g.u32(stream + OWNER)?;
    if owner == 0 {
        return Err(Error::new(stream + OWNER, "SndPlayer1 owner is null"));
    }
    if g.u8(owner + OWNER_STATE)? == 2 {
        return Ok(());
    }
    host.preflight(g, stream)?;

    loop {
        let index = g.u8(stream + RETIRE_INDEX)?;
        let slot = slot_address(g, stream, index)?;
        if g.u8(slot + SLOT_STATE)? != 4 {
            break;
        }
        host.retire(g, stream, index)?;
        let next = next_index(g.u8(stream + RETIRE_INDEX)?, g.u8(stream + RING_SIZE)?)?;
        g.set_u8(stream + RETIRE_INDEX, next)?;
    }

    let mut index = g.u8(stream + FIRST_ACTIVE)?;
    let mut slot = slot_address(g, stream, index)?;
    if !active(g.u8(slot + SLOT_STATE)?) {
        return publish_idle(g, stream, constants);
    }
    publish_active(g, stream)?;

    if g.u32(slot + SLOT_WORK)? == 0 {
        let first = index;
        loop {
            index = next_index(index, g.u8(stream + RING_SIZE)?)?;
            slot = slot_address(g, stream, index)?;
            if index == first || !active(g.u8(slot + SLOT_STATE)?) {
                return Ok(());
            }
            if g.u32(slot + SLOT_WORK)? != 0 {
                break;
            }
        }
    }

    let mut counter = 0u32;
    loop {
        if !active(g.u8(slot + SLOT_STATE)?) || selector_blocked(g, stream)? {
            return Ok(());
        }
        let record = record_address(g, stream, index)?;
        let sink = g.u32(record + RECORD_SINK)?;
        if sink != 0 {
            g.set_u32(sink + 8, g.f32(owner + OWNER_RATE)?.to_bits())?;
        }

        if g.u8(slot + SLOT_STATE)? == 1 {
            if !host.prepare(g, stream, index, &mut counter)? {
                return Ok(());
            }
            g.set_u8(slot + SLOT_STATE, 2)?;
            if g.u8(record + RECORD_KIND)? == 3
                && g.u64(slot)? == constants.far_future.to_bits()
                && index == g.u8(stream + FIRST_ACTIVE)?
            {
                let clock = g.u32(stream + CLOCK)?;
                if clock == 0 {
                    return Err(Error::new(stream + CLOCK, "SndPlayer1 clock is null"));
                }
                g.set_u64(
                    slot,
                    (f64::from_bits(g.u64(clock + 8)?) + constants.time_bump).to_bits(),
                )?;
            }
        }

        let mut advance = true;
        if g.u8(slot + SLOT_STATE)? == 2 && !selector_blocked(g, stream)? {
            let cursor = g.u32(record + RECORD_CURSOR)? as i32;
            if cursor == g.u32(slot + SLOT_END)? as i32 {
                if !host.at_end(g, stream, index, &mut counter)? {
                    return Ok(());
                }
                advance = false;
            } else if cursor == g.u32(slot + SLOT_WORK)? as i32 {
                let (continued, wrapped) = host.at_work(g, stream, index, &mut counter)?;
                if !continued {
                    return Ok(());
                }
                if wrapped {
                    g.set_u8(slot + SLOT_STATE, 3)?;
                } else {
                    advance = false;
                }
            } else if !host.submit(g, stream, index, &mut counter)? {
                return Ok(());
            } else {
                advance = false;
            }
        }

        if advance {
            index = next_index(index, g.u8(stream + RING_SIZE)?)?;
            if index == g.u8(stream + FIRST_ACTIVE)? {
                return Ok(());
            }
            slot = slot_address(g, stream, index)?;
        }
        if counter > 8192 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::{STREAM_RECORDS, STREAM_SLOT_TABLE};

    const BASE: u32 = 0x4000_0000;
    const STREAM: u32 = BASE;
    const OWNER_ADDR: u32 = BASE + 0x800;
    const RECORDS_ADDR: u32 = BASE + 0x1000;
    const RATE_SOURCE: u32 = BASE + 0x1800;
    const GATE: u32 = BASE + 0x1810;

    #[derive(Default)]
    struct Host {
        prepares: u32,
        retires: u32,
        submits: u32,
    }
    impl PumpHost for Host {
        fn preflight(&mut self, _g: &mut Guest, _stream: u32) -> Result<()> {
            Ok(())
        }
        fn retire(&mut self, _g: &mut Guest, _stream: u32, _index: u8) -> Result<()> {
            self.retires += 1;
            Ok(())
        }
        fn prepare(
            &mut self,
            _g: &mut Guest,
            _stream: u32,
            _index: u8,
            _counter: &mut u32,
        ) -> Result<bool> {
            self.prepares += 1;
            Ok(true)
        }
        fn at_end(
            &mut self,
            _g: &mut Guest,
            _stream: u32,
            _index: u8,
            _counter: &mut u32,
        ) -> Result<bool> {
            Ok(true)
        }
        fn at_work(
            &mut self,
            _g: &mut Guest,
            _stream: u32,
            _index: u8,
            _counter: &mut u32,
        ) -> Result<(bool, bool)> {
            Ok((true, true))
        }
        fn submit(
            &mut self,
            _g: &mut Guest,
            _stream: u32,
            _index: u8,
            _counter: &mut u32,
        ) -> Result<bool> {
            self.submits += 1;
            Ok(true)
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x3000);
        g.set_u32(STREAM + OWNER, OWNER_ADDR).unwrap();
        g.set_u32(STREAM + STREAM_RECORDS, RECORDS_ADDR).unwrap();
        g.set_u16(STREAM + STREAM_SLOT_TABLE, 0x200).unwrap();
        g.set_u8(STREAM + RING_SIZE, 2).unwrap();
        g.set_u32(STREAM + REFRESH_GATE, GATE).unwrap();
        g.set_u32(STREAM + DEFAULT_SOURCE, RATE_SOURCE).unwrap();
        g.set_u32(RATE_SOURCE, 24_000f32.to_bits()).unwrap();
        g.set_u32(STREAM + RATE, 48_000f32.to_bits()).unwrap();
        g.set_u32(STREAM + DIVISOR, 48_000f32.to_bits()).unwrap();
        g.set_u32(STREAM + START_SAMPLES, 48_000).unwrap();
        g.set_u32(STREAM + END_SAMPLES, 96_000).unwrap();
        g
    }

    fn constants() -> PumpConstants {
        PumpConstants {
            far_future: 1.0e30,
            time_bump: 0.25,
        }
    }

    #[test]
    fn retires_finished_slots_then_publishes_idle_bounds() {
        let mut g = guest();
        let slot = slot_address(&g, STREAM, 0).unwrap();
        g.set_u8(slot + SLOT_STATE, 4).unwrap();
        let mut host = Host::default();
        pump(&mut g, &mut host, STREAM, constants()).unwrap();
        assert_eq!(host.retires, 1);
        assert_eq!(g.u8(STREAM + RETIRE_INDEX).unwrap(), 1);
        assert_eq!(g.f32(STREAM + CURRENT_RATE).unwrap(), 24_000.0);
        assert_eq!(
            f64::from_bits(g.u64(STREAM + START_TIME).unwrap()),
            constants().far_future
        );
        assert_eq!(
            f64::from_bits(g.u64(STREAM + END_TIME).unwrap()),
            constants().far_future
        );
    }

    #[test]
    fn prepares_then_wraps_an_active_slot() {
        let mut g = guest();
        let slot = slot_address(&g, STREAM, 0).unwrap();
        g.set_u8(slot + SLOT_STATE, 1).unwrap();
        g.set_u32(slot + SLOT_WORK, 10).unwrap();
        g.set_u32(slot + SLOT_END, 20).unwrap();
        let record = RECORDS_ADDR;
        g.set_u32(record + RECORD_CURSOR, 10).unwrap();
        let mut host = Host::default();
        pump(&mut g, &mut host, STREAM, constants()).unwrap();
        assert_eq!(host.prepares, 1);
        assert_eq!(g.u8(slot + SLOT_STATE).unwrap(), 3);
        assert_eq!(f64::from_bits(g.u64(STREAM + START_TIME).unwrap()), 1.0);
        assert_eq!(f64::from_bits(g.u64(STREAM + END_TIME).unwrap()), 2.0);
    }

    #[test]
    fn selector_flag_parks_the_active_walk() {
        let mut g = guest();
        let slot = slot_address(&g, STREAM, 0).unwrap();
        g.set_u8(slot + SLOT_STATE, 2).unwrap();
        g.set_u32(slot + SLOT_WORK, 10).unwrap();
        g.set_u8(STREAM + SELECTOR, 1).unwrap();
        g.set_u8(STREAM + 16 + SELECTOR_FLAG, 1).unwrap();
        let mut host = Host::default();
        pump(&mut g, &mut host, STREAM, constants()).unwrap();
        assert_eq!(host.submits, 0);
        assert_eq!(g.u8(slot + SLOT_STATE).unwrap(), 2);
    }
}
