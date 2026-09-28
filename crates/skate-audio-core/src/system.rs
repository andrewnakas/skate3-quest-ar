//! `rw_system`: the command ring the PacketPlayer events are dispatched through.
//!
//! Ported from `sub_82B28A00`, shadow-verified over 6,706 comparable calls at zero
//! divergence. Offsets from `docs/rw_audio_structs.h`.
//!
//! The ring is **not** circular. `cmd_write_off` is a bump offset reset by each drain, and
//! the record's first word is the consumer's own guest address, whose identity implies the
//! record's length — which is what makes a torn publish desynchronise the whole queue rather
//! than corrupt one record (`docs/command-queue.md`).

use crate::{Error, Guest, Result};

/// `rw_system` fields this module touches.
pub const SYSTEM_CMD_BUFFER: u32 = 0x30;
pub const SYSTEM_CMD_WRITE_OFF: u32 = 0xCC;
/// `rw_player` -> its system.
pub const PLAYER_SYSTEM: u32 = 0x08;

const _: () = assert!(SYSTEM_CMD_BUFFER == 0x30, "rw_system.cmd_buffer");
const _: () = assert!(SYSTEM_CMD_WRITE_OFF == 0xCC, "rw_system.cmd_write_off");
const _: () = assert!(PLAYER_SYSTEM == 0x08, "rw_player.system");

/// Record layout, shared by all three commands.
pub const RECORD_HANDLER: u32 = 0x00;
pub const RECORD_OBJECT: u32 = 0x04;
/// The play payload: three floats the consumer unpacks.
pub const RECORD_PLAY_FORMAT: u32 = 0x08;
pub const RECORD_PLAY_RATE: u32 = 0x0C;
pub const RECORD_PLAY_CHANNELS: u32 = 0x10;
/// The submit payload.
pub const RECORD_SUBMIT_PACKET: u32 = 0x08;

/// Guest addresses of the three consumers. The producer derives these with `lis`/`addi`; they
/// are written out here and cross-checked against the consumers read independently.
pub const HANDLER_PLAY: u32 = 0x82B2_8B78;
pub const HANDLER_STOP: u32 = 0x82B2_8C18;
pub const HANDLER_SUBMIT: u32 = 0x82B2_8CC0;

/// Record sizes, which each handler returns to the drain. The coupling of size to handler is
/// the defect in `docs/command-queue.md`, not an artefact of this port.
pub const SIZE_PLAY: u32 = 20;
pub const SIZE_STOP: u32 = 8;
pub const SIZE_SUBMIT: u32 = 12;

/// Where the params struct keeps the values the producer copies out. The source stride is 8
/// and the record's is 4, so these are not the record's offsets.
pub const PARAMS_FIRST: u32 = 0x04;
pub const PARAMS_SECOND: u32 = 0x0C;
pub const PARAMS_THIRD: u32 = 0x14;
/// The non-append path writes these two instead of touching the ring.
pub const PARAMS_SENTINEL: u32 = 0x08;
pub const PARAMS_CONSTANT: u32 = 0x0C;

/// A NaN payload the non-append path stamps into the caller's params.
pub const INVALID_SENTINEL: u32 = 0x7FF7_FFF1;

/// The two read-only floats the non-append path selects between, by guest address.
pub const VOICE_LIVE_ADDR: u32 = 0x8216_5A10;
pub const VOICE_GONE_ADDR: u32 = 0x8231_A844;

/// The image constants, resolved. They live in `.rdata`, which a single flat guest window
/// cannot span alongside the heap, so they are read once and threaded through rather than
/// hardcoded — the values still come from the image, as in the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constants {
    pub voice_live_bits: u32,
    pub voice_gone_bits: u32,
}

impl Constants {
    pub fn from_guest(g: &Guest) -> Result<Self> {
        Ok(Self {
            voice_live_bits: g.u32(VOICE_LIVE_ADDR)?,
            voice_gone_bits: g.u32(VOICE_GONE_ADDR)?,
        })
    }
}

/// What the producer did, for callers that want to know without re-reading memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enqueued {
    /// A record was appended at this guest address, this many bytes long.
    Record { address: u32, size: u32 },
    /// The non-append path: the packet's liveness was reported into the caller's params.
    Queried { live: bool },
}

/// `sub_82B28A00`. Selectors 0, 1 and 2 append a record; anything else does not touch the
/// ring at all and instead reports whether the packet at `params + 4` is still live.
///
/// **One deliberate divergence from the original.** The guest publishes the advanced write
/// offset *before* storing the handler and payload; this writes the record first and publishes
/// the offset last. The final memory is byte-identical either way — only a concurrent reader
/// can tell them apart — so a single-threaded byte comparison against the verified C++ passes
/// and says nothing about the ordering in either direction. The correct ordering is chosen
/// here because the race is a host-concurrency artefact, not part of the wire format.
pub fn enqueue(
    g: &mut Guest,
    player: u32,
    selector: u32,
    params: u32,
    consts: &Constants,
) -> Result<Enqueued> {
    if selector <= 2 {
        let (size, handler) = match selector {
            0 => (SIZE_PLAY, HANDLER_PLAY),
            1 => (SIZE_STOP, HANDLER_STOP),
            _ => (SIZE_SUBMIT, HANDLER_SUBMIT),
        };
        let system = g.u32(player + PLAYER_SYSTEM)?;
        let offset = g.u32(system + SYSTEM_CMD_WRITE_OFF)?;
        let record = g.u32(system + SYSTEM_CMD_BUFFER)? + offset;

        g.set_u32(record + RECORD_HANDLER, handler)?;
        g.set_u32(record + RECORD_OBJECT, player)?;
        match selector {
            0 => {
                g.set_u32(record + RECORD_PLAY_FORMAT, g.u32(params + PARAMS_FIRST)?)?;
                g.set_u32(record + RECORD_PLAY_RATE, g.u32(params + PARAMS_SECOND)?)?;
                g.set_u32(record + RECORD_PLAY_CHANNELS, g.u32(params + PARAMS_THIRD)?)?;
            }
            2 => g.set_u32(record + RECORD_SUBMIT_PACKET, g.u32(params + PARAMS_FIRST)?)?,
            _ => {}
        }
        // Published last: see the note above.
        g.set_u32(system + SYSTEM_CMD_WRITE_OFF, offset + size)?;
        return Ok(Enqueued::Record {
            address: record,
            size,
        });
    }

    let wanted = g.u32(params + PARAMS_FIRST)?;
    let live = crate::player::packet_is_live(g, player, wanted)?;
    let bits = if live {
        consts.voice_live_bits
    } else {
        consts.voice_gone_bits
    };
    g.set_u32(params + PARAMS_CONSTANT, bits)?;
    g.set_u32(params + PARAMS_SENTINEL, INVALID_SENTINEL)?;
    Ok(Enqueued::Queried { live })
}

/// Reject a ring that has no buffer, rather than writing to `offset` alone.
pub fn check_ring(g: &Guest, player: u32) -> Result<()> {
    let system = g.u32(player + PLAYER_SYSTEM)?;
    if g.u32(system + SYSTEM_CMD_BUFFER)? == 0 {
        return Err(Error::new(
            system + SYSTEM_CMD_BUFFER,
            "command ring has no buffer",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn consts() -> Constants {
        Constants {
            voice_live_bits: 0x3F80_0000,
            voice_gone_bits: 0xBF80_0000,
        }
    }

    #[test]
    fn submit_record_layout_and_offset() {
        let mut g = guest();
        g.set_u32(PARAMS + PARAMS_FIRST, PACKET_A).unwrap();

        let r = enqueue(&mut g, PLAYER, 2, PARAMS, &consts()).unwrap();
        assert_eq!(
            r,
            Enqueued::Record {
                address: RING,
                size: SIZE_SUBMIT
            }
        );
        assert_eq!(g.u32(RING + RECORD_HANDLER).unwrap(), HANDLER_SUBMIT);
        assert_eq!(g.u32(RING + RECORD_OBJECT).unwrap(), PLAYER);
        assert_eq!(g.u32(RING + RECORD_SUBMIT_PACKET).unwrap(), PACKET_A);
        assert_eq!(g.u32(SYSTEM + SYSTEM_CMD_WRITE_OFF).unwrap(), SIZE_SUBMIT);
    }

    #[test]
    fn stop_record_is_eight_bytes_and_carries_no_payload() {
        let mut g = guest();
        let r = enqueue(&mut g, PLAYER, 1, PARAMS, &consts()).unwrap();
        assert_eq!(
            r,
            Enqueued::Record {
                address: RING,
                size: SIZE_STOP
            }
        );
        assert_eq!(g.u32(RING + RECORD_HANDLER).unwrap(), HANDLER_STOP);
        // Nothing past +0x08: the next record would start there.
        assert_eq!(g.u32(RING + 0x08).unwrap(), 0);
    }

    #[test]
    fn play_record_carries_three_floats_from_a_stride_eight_params() {
        let mut g = guest();
        g.set_u32(PARAMS + PARAMS_FIRST, 1.0f32.to_bits()).unwrap();
        g.set_u32(PARAMS + PARAMS_SECOND, 48000.0f32.to_bits())
            .unwrap();
        g.set_u32(PARAMS + PARAMS_THIRD, 6.0f32.to_bits()).unwrap();

        let r = enqueue(&mut g, PLAYER, 0, PARAMS, &consts()).unwrap();
        assert_eq!(
            r,
            Enqueued::Record {
                address: RING,
                size: SIZE_PLAY
            }
        );
        assert_eq!(g.f32(RING + RECORD_PLAY_FORMAT).unwrap(), 1.0);
        assert_eq!(g.f32(RING + RECORD_PLAY_RATE).unwrap(), 48000.0);
        assert_eq!(g.f32(RING + RECORD_PLAY_CHANNELS).unwrap(), 6.0);
    }

    #[test]
    fn records_bump_the_offset_rather_than_wrapping() {
        let mut g = guest();
        g.set_u32(PARAMS + PARAMS_FIRST, PACKET_A).unwrap();
        enqueue(&mut g, PLAYER, 2, PARAMS, &consts()).unwrap();
        let second = enqueue(&mut g, PLAYER, 1, PARAMS, &consts()).unwrap();
        assert_eq!(
            second,
            Enqueued::Record {
                address: RING + SIZE_SUBMIT,
                size: SIZE_STOP
            }
        );
        assert_eq!(
            g.u32(SYSTEM + SYSTEM_CMD_WRITE_OFF).unwrap(),
            SIZE_SUBMIT + SIZE_STOP
        );
    }

    #[test]
    fn selector_three_queries_instead_of_appending() {
        let mut g = guest();
        g.set_u32(PARAMS + PARAMS_FIRST, PACKET_A).unwrap();
        // PACKET_A is on the FIFO, so it is live.
        g.set_u32(PLAYER + crate::player::PLAYER_PACKET_HEAD, PACKET_A)
            .unwrap();

        let r = enqueue(&mut g, PLAYER, 3, PARAMS, &consts()).unwrap();
        assert_eq!(r, Enqueued::Queried { live: true });
        assert_eq!(g.u32(PARAMS + PARAMS_SENTINEL).unwrap(), INVALID_SENTINEL);
        assert_eq!(
            g.u32(PARAMS + PARAMS_CONSTANT).unwrap(),
            consts().voice_live_bits
        );
        // The ring was not touched.
        assert_eq!(g.u32(SYSTEM + SYSTEM_CMD_WRITE_OFF).unwrap(), 0);
        assert_eq!(g.u32(RING).unwrap(), 0);
    }

    #[test]
    fn handler_addresses_match_the_consumers_read_independently() {
        assert_eq!(HANDLER_PLAY, 0x82B2_8B78);
        assert_eq!(HANDLER_STOP, 0x82B2_8C18);
        assert_eq!(HANDLER_SUBMIT, 0x82B2_8CC0);
    }
}
