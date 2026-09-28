//! The command phase of `rw_system` (`sub_82B48530`).
//!
//! The original's fourth phase snapshots `system + 204`, invokes the handler stored in each
//! record's first word, advances by the size returned by that handler, records high-water usage,
//! and advances the drain count. The surrounding scheduler/list passes remain the responsibility
//! of the block driver; keeping this phase separate makes the queue's ownership and failure mode
//! testable now.
//!
//! Host execution deliberately differs in one concurrency detail. The retail ring resets its
//! write offset after dispatch. A handler that appends more records during a drain therefore has
//! no durable next-frame representation. Here those post-snapshot records are moved to the start
//! of the frame-scoped ring and deferred to the next drain. That prevents a host-side handler from
//! silently losing work while preserving the retail snapshot boundary: records appended during a
//! drain are never executed by that same drain.

use crate::device;
use crate::leaves;
use crate::modules;
use crate::patch::Heap;
use crate::play;
use crate::voices;
use crate::{Error, Guest, Result, mem};

/// `rw_system` fields used by phase four of `sub_82B48530`.
pub const COMMAND_BUFFER: u32 = 48;
pub const COMMAND_WRITE_OFFSET: u32 = 204;
pub const COMMAND_HIGH_WATER: u32 = 208;
pub const DRAIN_COUNT: u32 = 256;

/// A successful command phase summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drain {
    /// Number of records executed from the snapshot.
    pub records: u32,
    /// Bytes in the snapshot taken at the beginning of this phase.
    pub consumed_bytes: u32,
    /// Bytes appended by handlers and deferred to the next phase.
    pub deferred_bytes: u32,
}

/// Handlers whose recovered implementations need higher-level stream/host services.
///
/// Implementations must return the record's exact byte size. Returning zero or a size beyond the
/// snapshotted record area is rejected by [`drain`]. This makes a partially ported handler unable
/// to desynchronise the rest of the frame silently.
pub trait CommandHost {
    /// `sub_82B49238`: stop an SndPlayer1 record and let the host detach its graph resources.
    ///
    /// The retail handler calls `sub_82B48F28(player, 0)`. Its list removal and allocator free
    /// belong to the concrete graph owner, so this boundary preserves the exact command record
    /// without pretending that a guest-only drain can safely free host PCM state.
    fn stop_player(&mut self, _g: &mut Guest, _heap: &mut dyn Heap, record: u32) -> Result<u32> {
        Err(Error::new(
            record,
            "SndPlayer1 stop command is not implemented",
        ))
    }

    /// `sub_82B32DC8`: append a stream request to SndPlayer1's segment ring.
    fn play(&mut self, _g: &mut Guest, record: u32) -> Result<u32> {
        Err(Error::new(
            record,
            "SndPlayer1 play command is not implemented",
        ))
    }

    /// `sub_82B31720`: configure a send by its authored name.
    fn send_name(&mut self, _g: &mut Guest, record: u32) -> Result<u32> {
        Err(Error::new(record, "named Send command is not implemented"))
    }

    /// `sub_82B315E0`: the second Send command form.
    fn send_2(&mut self, _g: &mut Guest, record: u32) -> Result<u32> {
        Err(Error::new(
            record,
            "secondary Send command is not implemented",
        ))
    }
}

/// A host that exposes every missing high-level handler as an error.
#[derive(Default)]
pub struct UnsupportedCommandHost;
impl CommandHost for UnsupportedCommandHost {}

/// Adapt a resident [`play::PlayHost`] into a command-ring host while leaving unrelated missing
/// handlers explicit.
pub struct ResidentCommandHost<P> {
    pub playback: P,
}

impl<P: play::PlayHost> CommandHost for ResidentCommandHost<P> {
    fn play(&mut self, g: &mut Guest, record: u32) -> Result<u32> {
        play::append_resident(g, &mut self.playback, record)
    }
}

/// Dispatch one record and return its handler-owned size.
///
/// `stack` is the guest stack top available to handlers such as `sub_82B31680`; callers should
/// reserve its frame below this address before invoking the block driver.
pub fn dispatch<H: Heap, C: CommandHost + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    host: &mut C,
    record: u32,
    stack: u32,
) -> Result<u32> {
    let handler = g.u32(record)?;
    let size = match handler {
        modules::INSTALL_COMMAND => modules::install_command(g, heap, record)?,
        modules::SUBMIX_INSTALL_COMMAND => modules::install_submix(g, record)?,
        device::COMMAND_PLAYER_STOP => host.stop_player(g, heap, record)?,
        device::COMMAND_PLAYER_FLOAT => leaves::publish_float(g, record)? as u32,
        device::COMMAND_STAMP => leaves::stamp_slot(g, record)? as u32,
        device::COMMAND_SEND_BUS => voices::repoint_link(g, record, stack)? as u32,
        device::COMMAND_FADER => leaves::publish_command(g, record)? as u32,
        device::COMMAND_PLAY => host.play(g, record)?,
        device::COMMAND_SEND_NAME => host.send_name(g, record)?,
        device::COMMAND_SEND_2 => host.send_2(g, record)?,
        other => {
            return Err(Error::new(
                other,
                format!("unimplemented audio command handler {other:#010x}"),
            ));
        }
    };
    if size == 0 {
        return Err(Error::new(
            record,
            format!("audio command handler {handler:#010x} returned zero bytes"),
        ));
    }
    Ok(size)
}

/// Drain the command-ring snapshot for one audio block.
///
/// No scheduler or DSP work occurs here. A successful call always increments `system + 256` once.
/// On an error the queue remains intact, which gives callers an actionable record address and
/// avoids dropping configuration commands just because a newer handler is not ported yet.
pub fn drain<H: Heap, C: CommandHost + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    host: &mut C,
    system_addr: u32,
    stack: u32,
) -> Result<Drain> {
    let ring = g.u32(system_addr + COMMAND_BUFFER)?;
    if ring == 0 {
        return Err(Error::new(
            system_addr + COMMAND_BUFFER,
            "audio command ring has no buffer",
        ));
    }
    let used = g.u32(system_addr + COMMAND_WRITE_OFFSET)?;
    // Validate the whole initial snapshot before executing its first handler. A bad producer must
    // not turn into a partial drain followed by a misleading high-water/reset update.
    g.span(ring, used as usize)?;

    let mut cursor = 0u32;
    let mut records = 0u32;
    while cursor < used {
        let record = ring
            .checked_add(cursor)
            .ok_or_else(|| Error::new(ring, "audio command record address overflow"))?;
        let size = dispatch(g, heap, host, record, stack)?;
        let remaining = used - cursor;
        if size > remaining {
            return Err(Error::new(
                record,
                format!(
                    "audio command handler returned {size} bytes with only {remaining} in the snapshot"
                ),
            ));
        }
        cursor += size;
        records += 1;
    }

    // Re-read after dispatch, as the original does for high-water accounting. The command area
    // must still be addressable if a handler appended deferred work.
    let after = g.u32(system_addr + COMMAND_WRITE_OFFSET)?;
    if after < used {
        return Err(Error::new(
            system_addr + COMMAND_WRITE_OFFSET,
            "audio command write offset moved backwards during drain",
        ));
    }
    g.span(ring, after as usize)?;
    let previous_high_water = g.u32(system_addr + COMMAND_HIGH_WATER)?;
    if after > previous_high_water {
        g.set_u32(system_addr + COMMAND_HIGH_WATER, after)?;
    }

    let deferred = after - used;
    if deferred != 0 {
        mem::memmove(g, ring, ring + used, deferred as u64)?;
    }
    g.set_u32(system_addr + COMMAND_WRITE_OFFSET, deferred)?;
    let count = g.u32(system_addr + DRAIN_COUNT)?;
    g.set_u32(system_addr + DRAIN_COUNT, count.wrapping_add(1))?;
    Ok(Drain {
        records,
        consumed_bytes: used,
        deferred_bytes: deferred,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::BumpHeap;
    use crate::play;

    const BASE: u32 = 0x4000_0000;
    const SYSTEM: u32 = BASE;
    const RING: u32 = BASE + 0x400;
    const TARGET: u32 = BASE + 0x900;
    const STACK: u32 = BASE + 0x1F00;

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x3000);
        g.set_u32(SYSTEM + COMMAND_BUFFER, RING).unwrap();
        g
    }

    fn heap() -> BumpHeap {
        BumpHeap {
            next: BASE + 0x2000,
            end: BASE + 0x2F00,
        }
    }

    #[test]
    fn executes_known_handlers_updates_high_water_and_resets_queue() {
        let mut g = guest();
        g.set_u32(RING, device::COMMAND_PLAYER_FLOAT).unwrap();
        g.set_u32(RING + 4, TARGET).unwrap();
        g.set_u32(RING + 8, 0.75f32.to_bits()).unwrap();
        g.set_u32(SYSTEM + COMMAND_WRITE_OFFSET, 12).unwrap();
        let result = drain(
            &mut g,
            &mut heap(),
            &mut UnsupportedCommandHost,
            SYSTEM,
            STACK,
        )
        .unwrap();
        assert_eq!(
            result,
            Drain {
                records: 1,
                consumed_bytes: 12,
                deferred_bytes: 0
            }
        );
        assert_eq!(g.f32(TARGET + 56).unwrap(), 0.75);
        assert_eq!(g.u32(SYSTEM + COMMAND_HIGH_WATER).unwrap(), 12);
        assert_eq!(g.u32(SYSTEM + COMMAND_WRITE_OFFSET).unwrap(), 0);
        assert_eq!(g.u32(SYSTEM + DRAIN_COUNT).unwrap(), 1);
    }

    #[test]
    fn unimplemented_handler_leaves_snapshot_for_diagnosis() {
        let mut g = guest();
        g.set_u32(RING, 0x8200_0000).unwrap();
        g.set_u32(SYSTEM + COMMAND_WRITE_OFFSET, 4).unwrap();
        let error = drain(
            &mut g,
            &mut heap(),
            &mut UnsupportedCommandHost,
            SYSTEM,
            STACK,
        )
        .unwrap_err();
        assert!(error.message.contains("0x82000000"), "{}", error.message);
        assert_eq!(g.u32(SYSTEM + COMMAND_WRITE_OFFSET).unwrap(), 4);
        assert_eq!(g.u32(SYSTEM + DRAIN_COUNT).unwrap(), 0);
    }

    struct StopHost {
        stopped: Option<u32>,
    }
    impl CommandHost for StopHost {
        fn stop_player(&mut self, g: &mut Guest, _heap: &mut dyn Heap, record: u32) -> Result<u32> {
            self.stopped = Some(g.u32(record + 4)?);
            Ok(8)
        }
    }

    #[test]
    fn player_stop_command_reaches_the_graph_owner() {
        let mut g = guest();
        g.set_u32(RING, device::COMMAND_PLAYER_STOP).unwrap();
        g.set_u32(RING + 4, TARGET).unwrap();
        g.set_u32(SYSTEM + COMMAND_WRITE_OFFSET, 8).unwrap();
        let mut host = StopHost { stopped: None };
        let result = drain(&mut g, &mut heap(), &mut host, SYSTEM, STACK).unwrap();
        assert_eq!(result.records, 1);
        assert_eq!(host.stopped, Some(TARGET));
        assert_eq!(g.u32(SYSTEM + COMMAND_WRITE_OFFSET).unwrap(), 0);
        assert_eq!(g.u32(SYSTEM + DRAIN_COUNT).unwrap(), 1);
    }

    struct DeferringHost;
    impl CommandHost for DeferringHost {
        fn play(&mut self, g: &mut Guest, record: u32) -> Result<u32> {
            let system = g.u32(record + 4)?;
            let ring = g.u32(system + COMMAND_BUFFER)?;
            let offset = g.u32(system + COMMAND_WRITE_OFFSET)?;
            g.set_u32(ring + offset, device::COMMAND_PLAYER_FLOAT)?;
            g.set_u32(ring + offset + 4, TARGET)?;
            g.set_u32(ring + offset + 8, 1.0f32.to_bits())?;
            g.set_u32(system + COMMAND_WRITE_OFFSET, offset + 12)?;
            Ok(8)
        }
    }

    #[test]
    fn commands_appended_by_a_handler_are_deferred_without_loss() {
        let mut g = guest();
        g.set_u32(RING, device::COMMAND_PLAY).unwrap();
        g.set_u32(RING + 4, SYSTEM).unwrap();
        g.set_u32(SYSTEM + COMMAND_WRITE_OFFSET, 8).unwrap();
        let first = drain(&mut g, &mut heap(), &mut DeferringHost, SYSTEM, STACK).unwrap();
        assert_eq!(
            first,
            Drain {
                records: 1,
                consumed_bytes: 8,
                deferred_bytes: 12
            }
        );
        assert_eq!(g.u32(RING).unwrap(), device::COMMAND_PLAYER_FLOAT);
        assert_eq!(g.u32(SYSTEM + COMMAND_WRITE_OFFSET).unwrap(), 12);
        let second = drain(
            &mut g,
            &mut heap(),
            &mut UnsupportedCommandHost,
            SYSTEM,
            STACK,
        )
        .unwrap();
        assert_eq!(second.records, 1);
        assert_eq!(g.f32(TARGET + 56).unwrap(), 1.0);
        assert_eq!(g.u32(SYSTEM + COMMAND_WRITE_OFFSET).unwrap(), 0);
    }

    struct ResidentHost;
    impl play::PlayHost for ResidentHost {
        fn prepare(&mut self, g: &mut Guest, stream: u32, index: u8, _offset: u32) -> Result<bool> {
            let slot = play::slot_address(g, stream, index)?;
            let record = play::record_address(g, stream, index)?;
            g.set_u32(slot + play::SLOT_RATE, 48_000f32.to_bits())?;
            g.set_u32(slot + play::SLOT_QUEUED, 256)?;
            g.set_u8(record + play::RECORD_KIND, 0)?;
            Ok(true)
        }

        fn decode(
            &mut self,
            _g: &mut Guest,
            _stream: u32,
            _index: u8,
            _detail: u32,
        ) -> Result<bool> {
            Ok(true)
        }
    }

    #[test]
    fn play_command_reaches_a_committed_resident_slot() {
        const STREAM: u32 = BASE + 0x1000;
        const RECORDS: u32 = BASE + 0x1800;
        const PENDING: u32 = BASE + 0x1E00;

        let mut g = guest();
        g.set_u32(RING, device::COMMAND_PLAY).unwrap();
        g.set_u32(RING + play::REQUEST_STREAM, STREAM).unwrap();
        g.set_u64(RING + play::REQUEST_SPAN, 1.0f64.to_bits())
            .unwrap();
        g.set_u16(RING + play::REQUEST_RESULT, 64).unwrap();
        g.set_u32(RING + play::REQUEST_LEVEL, 0.5f32.to_bits())
            .unwrap();
        g.set_u32(SYSTEM + COMMAND_WRITE_OFFSET, 64).unwrap();

        g.set_u32(STREAM + play::STREAM_RECORDS, RECORDS).unwrap();
        g.set_u32(STREAM + play::STREAM_PENDING, PENDING).unwrap();
        g.set_u32(PENDING, 1).unwrap();
        g.set_u16(STREAM + play::STREAM_SLOT_TABLE, 0x200).unwrap();
        g.set_u8(STREAM + play::STREAM_RING_SIZE, 2).unwrap();

        let mut host = ResidentCommandHost {
            playback: ResidentHost,
        };
        let result = drain(&mut g, &mut heap(), &mut host, SYSTEM, STACK).unwrap();
        let slot = play::slot_address(&g, STREAM, 0).unwrap();
        assert_eq!(result.records, 1);
        assert_eq!(g.u8(slot + play::SLOT_STATE).unwrap(), 1);
        assert_eq!(g.u8(STREAM + play::STREAM_WRITE_INDEX).unwrap(), 1);
        assert_eq!(g.u32(PENDING).unwrap(), 0);
        assert_eq!(g.u32(SYSTEM + COMMAND_WRITE_OFFSET).unwrap(), 0);
    }
}
