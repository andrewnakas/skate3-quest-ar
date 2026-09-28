//! Append a resident SndPlayer1 request (`sub_82B32DC8`).
//!
//! This is the short, source-kind-zero branch of the recovered play command. It claims an empty
//! 48-byte slot, seeds its matching 80-byte record, asks the host to prepare/decode the source,
//! applies the retail admission check, and commits the write index. Source kinds 1 and 2 require
//! the unported voice-pool, allocation, and scheduled-callback lifecycle, so this module rejects
//! them explicitly rather than treating them as resident PCM.

use crate::{Error, Guest, Result};

/// Request record fields, relative to the command record received by `sub_82B32DC8`.
pub const REQUEST_STREAM: u32 = 4;
pub const REQUEST_START: u32 = 8;
pub const REQUEST_TIME: u32 = 16;
pub const REQUEST_SPAN: u32 = 24;
pub const REQUEST_OFFSET: u32 = 32;
pub const REQUEST_DETAIL: u32 = 36;
pub const REQUEST_RESULT: u32 = 44;
pub const REQUEST_FLAGS: u32 = 46;
pub const REQUEST_LEVEL: u32 = 48;

/// Stream fields.
pub const STREAM_RECORDS: u32 = 96;
pub const STREAM_PENDING: u32 = 444;
pub const STREAM_LEVEL_ENTRY: u32 = 448;
pub const STREAM_LEVEL_COMMITTED: u32 = 452;
pub const STREAM_SLOT_TABLE: u32 = 464;
pub const STREAM_WRITE_INDEX: u32 = 467;
pub const STREAM_RING_SIZE: u32 = 470;

/// A 48-byte stream slot.
pub const SLOT_START: u32 = 0;
pub const SLOT_COUNTER: u32 = 8;
pub const SLOT_LEVEL: u32 = 12;
pub const SLOT_RATE: u32 = 16;
pub const SLOT_QUEUED: u32 = 20;
pub const SLOT_CURSOR: u32 = 24;
pub const SLOT_STATE: u32 = 46;
pub const SLOT_MODE: u32 = 47;
pub const SLOT_BYTES: u32 = 48;

/// An 80-byte record parallel to the slot ring.
pub const RECORD_TIME: u32 = 0;
pub const RECORD_POSITION: u32 = 40;
pub const RECORD_NAME_BUFFER: u32 = 28;
pub const RECORD_POOL: u32 = 32;
pub const RECORD_VOICE: u32 = 36;
pub const RECORD_HANDLE: u32 = 44;
pub const RECORD_KIND: u32 = 73;
pub const RECORD_FLAGS: u32 = 75;
pub const RECORD_READY: u32 = 76;
pub const RECORD_BYTES: u32 = 80;

/// The decoder-specific parts of the original play command.
///
/// `prepare` corresponds to `sub_82B335A8`; it initializes the selected slot from the requested
/// archive offset. `decode` corresponds to `sub_82B33780`; it makes source frames available for
/// that slot. Both return `false` when the source cannot be prepared. The host owns decoded PCM
/// and may attach a cursor to its stream object, but it must not modify the write index or claim a
/// different slot.
pub trait PlayHost {
    fn prepare(&mut self, g: &mut Guest, stream: u32, index: u8, offset: u32) -> Result<bool>;
    fn decode(&mut self, g: &mut Guest, stream: u32, index: u8, detail: u32) -> Result<bool>;
}

/// A host that makes missing decoder work visible.
#[derive(Default)]
pub struct NoPlayHost;
impl PlayHost for NoPlayHost {
    fn prepare(&mut self, _g: &mut Guest, stream: u32, _index: u8, _offset: u32) -> Result<bool> {
        Err(Error::new(
            stream,
            "resident stream preparation is not implemented",
        ))
    }

    fn decode(&mut self, _g: &mut Guest, stream: u32, _index: u8, _detail: u32) -> Result<bool> {
        Err(Error::new(
            stream,
            "resident stream decode is not implemented",
        ))
    }
}

/// Address of a slot whose index is a byte, using the original's `3 * index * 16` arithmetic.
pub fn slot_address(g: &Guest, stream: u32, index: u8) -> Result<u32> {
    let table = u32::from(g.u16(stream + STREAM_SLOT_TABLE)?);
    Ok(stream
        .wrapping_add(table)
        .wrapping_add(u32::from(index) * SLOT_BYTES))
}

/// Address of the parallel 80-byte record.
pub fn record_address(g: &Guest, stream: u32, index: u8) -> Result<u32> {
    let records = g.u32(stream + STREAM_RECORDS)?;
    Ok(records.wrapping_add(u32::from(index) * RECORD_BYTES))
}

fn next_index(index: u8, limit: u8) -> u8 {
    let next = index.wrapping_add(1);
    if next == limit { 0 } else { next }
}

fn abandon(g: &mut Guest, slot: u32) -> Result<()> {
    g.set_u8(slot + SLOT_STATE, 0)?;
    g.set_u32(slot + SLOT_QUEUED, 0)
}

/// Execute the resident branch of `sub_82B32DC8` and return `u16[request + 44]`.
///
/// The original decrements the pending counter and publishes `+448` before discovering that its
/// selected slot is busy. Those writes are retained. A host refusal abandons the claimed slot but
/// likewise retains the earlier observable stores.
pub fn append_resident<H: PlayHost + ?Sized>(
    g: &mut Guest,
    host: &mut H,
    request: u32,
) -> Result<u32> {
    let stream = g.u32(request + REQUEST_STREAM)?;
    let pending = g.u32(stream + STREAM_PENDING)?;
    g.set_u32(pending, g.u32(pending)?.wrapping_sub(1))?;

    let level = g.f32(request + REQUEST_LEVEL)?;
    let index = g.u8(stream + STREAM_WRITE_INDEX)?;
    g.set_u32(stream + STREAM_LEVEL_ENTRY, level.to_bits())?;
    let slot = slot_address(g, stream, index)?;
    if g.u8(slot + SLOT_STATE)? != 0 {
        return Ok(u32::from(g.u16(request + REQUEST_RESULT)?));
    }
    let record = record_address(g, stream, index)?;

    // The seed order follows the lifted command. The record kind is initialized by preparation;
    // source kinds 1/2 intentionally stop below, before the pool/callback branch.
    g.set_u32(slot + SLOT_LEVEL, level.to_bits())?;
    g.set_u32(slot + SLOT_COUNTER, 0)?;
    g.set_u32(record + RECORD_POSITION, 0)?;
    g.set_u64(slot + SLOT_START, g.u64(request + REQUEST_START)?)?;
    g.set_u64(record + RECORD_TIME, g.u64(request + REQUEST_TIME)?)?;
    g.set_u8(record + RECORD_FLAGS, g.u8(request + REQUEST_FLAGS)?)?;
    g.set_u8(slot + SLOT_STATE, 1)?;
    for offset in [20, 24, RECORD_VOICE, RECORD_HANDLE, RECORD_NAME_BUFFER] {
        g.set_u32(record + offset, 0)?;
    }

    if !host.prepare(g, stream, index, g.u32(request + REQUEST_OFFSET)?)? {
        abandon(g, slot)?;
        return Ok(u32::from(g.u16(request + REQUEST_RESULT)?));
    }

    let kind = g.u8(record + RECORD_KIND)?;
    if kind == 1 || kind == 2 {
        abandon(g, slot)?;
        return Err(Error::new(
            record + RECORD_KIND,
            format!("SndPlayer1 source kind {kind} needs the long voice-pool path"),
        ));
    }

    // The admission check is signed and uses a double product truncated to a signed 32-bit word.
    // Resident source kind zero always compares against zero because it does not take the kind-1/2
    // cursor branch.
    let span = f64::from_bits(g.u64(request + REQUEST_SPAN)?);
    let product = f64::from(g.f32(slot + SLOT_RATE)?) * span;
    let truncated = if product.is_nan() {
        i32::MIN
    } else if product > f64::from(i32::MAX) {
        i32::MAX
    } else if product < f64::from(i32::MIN) {
        i32::MIN
    } else {
        product.trunc() as i32
    };
    let limit = if truncated > 0 && kind != 2 && (g.u32(slot + SLOT_CURSOR)? as i32) < 0 {
        truncated
    } else {
        0
    };
    if (g.u32(slot + SLOT_QUEUED)? as i32) <= limit {
        abandon(g, slot)?;
        return Ok(u32::from(g.u16(request + REQUEST_RESULT)?));
    }
    if !host.decode(g, stream, index, g.u32(request + REQUEST_DETAIL)?)? {
        abandon(g, slot)?;
        return Ok(u32::from(g.u16(request + REQUEST_RESULT)?));
    }

    // Record kind zero has no pool/voice/name/callback branch. A ready record with an empty name
    // has an additional offset calculation in retail, but that is not needed for a resident bank
    // stream to enter the pump and will be added with the stream-pump contract.
    g.set_u8(slot + SLOT_STATE, 1)?;
    let ring_size = g.u8(stream + STREAM_RING_SIZE)?;
    g.set_u8(stream + STREAM_WRITE_INDEX, next_index(index, ring_size))?;
    g.set_u32(stream + STREAM_LEVEL_COMMITTED, level.to_bits())?;
    Ok(u32::from(g.u16(request + REQUEST_RESULT)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const STREAM: u32 = BASE;
    const REQUEST: u32 = BASE + 0x1000;
    const RECORDS: u32 = BASE + 0x1800;
    const PENDING: u32 = BASE + 0x1F00;

    struct ReadyHost {
        prepared: u32,
        decoded: u32,
        kind: u8,
    }

    impl PlayHost for ReadyHost {
        fn prepare(&mut self, g: &mut Guest, stream: u32, index: u8, _offset: u32) -> Result<bool> {
            self.prepared += 1;
            let slot = slot_address(g, stream, index)?;
            let record = record_address(g, stream, index)?;
            g.set_u32(slot + SLOT_RATE, 48_000f32.to_bits())?;
            g.set_u32(slot + SLOT_QUEUED, 256)?;
            g.set_u8(record + RECORD_KIND, self.kind)?;
            Ok(true)
        }

        fn decode(
            &mut self,
            _g: &mut Guest,
            _stream: u32,
            _index: u8,
            _detail: u32,
        ) -> Result<bool> {
            self.decoded += 1;
            Ok(true)
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x3000);
        g.set_u32(STREAM + STREAM_RECORDS, RECORDS).unwrap();
        g.set_u32(STREAM + STREAM_PENDING, PENDING).unwrap();
        g.set_u32(PENDING, 1).unwrap();
        g.set_u16(STREAM + STREAM_SLOT_TABLE, 0x200).unwrap();
        g.set_u8(STREAM + STREAM_RING_SIZE, 2).unwrap();
        g.set_u16(REQUEST + REQUEST_RESULT, 64).unwrap();
        g.set_u32(REQUEST + REQUEST_STREAM, STREAM).unwrap();
        g.set_u64(REQUEST + REQUEST_START, 123.0f64.to_bits())
            .unwrap();
        g.set_u64(REQUEST + REQUEST_TIME, 456.0f64.to_bits())
            .unwrap();
        g.set_u64(REQUEST + REQUEST_SPAN, 1.0f64.to_bits()).unwrap();
        g.set_u32(REQUEST + REQUEST_OFFSET, 17).unwrap();
        g.set_u32(REQUEST + REQUEST_DETAIL, 99).unwrap();
        g.set_u8(REQUEST + REQUEST_FLAGS, 7).unwrap();
        g.set_u32(REQUEST + REQUEST_LEVEL, 0.25f32.to_bits())
            .unwrap();
        g
    }

    #[test]
    fn resident_request_claims_and_commits_a_slot() {
        let mut g = guest();
        let mut host = ReadyHost {
            prepared: 0,
            decoded: 0,
            kind: 0,
        };
        assert_eq!(append_resident(&mut g, &mut host, REQUEST).unwrap(), 64);
        let slot = slot_address(&g, STREAM, 0).unwrap();
        assert_eq!(g.u8(slot + SLOT_STATE).unwrap(), 1);
        assert_eq!(g.u64(slot + SLOT_START).unwrap(), 123.0f64.to_bits());
        assert_eq!(g.u8(STREAM + STREAM_WRITE_INDEX).unwrap(), 1);
        assert_eq!(g.f32(STREAM + STREAM_LEVEL_COMMITTED).unwrap(), 0.25);
        assert_eq!(g.u32(PENDING).unwrap(), 0);
        assert_eq!((host.prepared, host.decoded), (1, 1));
    }

    #[test]
    fn a_busy_slot_consumes_pending_but_does_not_prepare_or_advance() {
        let mut g = guest();
        let slot = slot_address(&g, STREAM, 0).unwrap();
        g.set_u8(slot + SLOT_STATE, 1).unwrap();
        let mut host = ReadyHost {
            prepared: 0,
            decoded: 0,
            kind: 0,
        };
        assert_eq!(append_resident(&mut g, &mut host, REQUEST).unwrap(), 64);
        assert_eq!(g.u32(PENDING).unwrap(), 0);
        assert_eq!(g.u8(STREAM + STREAM_WRITE_INDEX).unwrap(), 0);
        assert_eq!((host.prepared, host.decoded), (0, 0));
    }

    #[test]
    fn long_source_kinds_are_rejected_after_their_kind_is_known() {
        let mut g = guest();
        let mut host = ReadyHost {
            prepared: 0,
            decoded: 0,
            kind: 1,
        };
        let error = append_resident(&mut g, &mut host, REQUEST).unwrap_err();
        assert!(error.message.contains("long voice-pool"));
        let slot = slot_address(&g, STREAM, 0).unwrap();
        assert_eq!(g.u8(slot + SLOT_STATE).unwrap(), 0);
        assert_eq!(host.decoded, 0);
    }
}
