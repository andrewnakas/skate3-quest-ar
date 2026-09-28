//! `rw_player`: the PacketPlayer instance and its three command consumers.
//!
//! Ported from `sub_82B28B78` (`EVENT_PLAY`), `sub_82B28C18` (`EVENT_STOP`) and
//! `sub_82B28CC0` (`EVENT_SUBMIT`), as reproduced in `skate3_audio_native.cpp`. Verification
//! status differs per function and is recorded on each one: a reader should not assume the
//! three are equally well checked, because they are not.

use crate::patch::Heap;
use crate::system::{
    RECORD_OBJECT, RECORD_PLAY_CHANNELS, RECORD_PLAY_FORMAT, RECORD_PLAY_RATE,
    RECORD_SUBMIT_PACKET, SIZE_PLAY, SIZE_STOP, SIZE_SUBMIT,
};
use crate::voices;
use crate::{Guest, Result};

pub const PLAYER_SOURCE: u32 = 0x50;
/// The producer's non-append path walks from +0x48 with `stwu`, so the first word it touches
/// is at +0x54. The header documents the table as starting there.
pub const PLAYER_TABLE_WALK: u32 = 0x48;
pub const PLAYER_TABLE: u32 = 0x54;
pub const TABLE_ENTRIES: u32 = 20;
pub const TABLE_STRIDE: u32 = 12;
pub const TABLE_DISCRIMINATOR: u32 = 9;
pub const PLAYER_PACKET_HEAD: u32 = 0x148;
pub const PLAYER_PACKET_TAIL: u32 = 0x14C;
pub const PLAYER_DECODER: u32 = 0x150;
pub const PLAYER_SAMPLE_RATE: u32 = 0x154;
pub const PLAYER_TORN_DOWN: u32 = 0x158;
pub const PLAYER_STATE: u32 = 0x15E;
pub const PLAYER_CHANNEL_COUNT: u32 = 0x15F;
pub const PLAYER_FORMAT_INDEX: u32 = 0x160;
/// Undocumented until 2026-09-11: EVENT_STOP writes these three, 16 to the first.
pub const PLAYER_STOP_F172: u32 = 0x172;
pub const PLAYER_STOP_F173: u32 = 0x173;
pub const PLAYER_STOP_F174: u32 = 0x174;
/// Bytes EVENT_STOP clears from +0x150 onward, which overwrites the 0xFF it just wrote.
pub const STOP_WIPE: u32 = 0x14;
pub const PACKET_NEXT: u32 = 0x0C;

/// Fields used by `sub_82B48F28`, the graph-object destructor behind the deferred
/// `SndPlayer1` stop command. These are deliberately kept separate from the packet-player
/// fields above: both structures are called "player" by the original, but their layouts differ.
pub const GRAPH_OWNER: u32 = 16;
pub const GRAPH_NODE: u32 = 28;
pub const GRAPH_CHILD_COUNT: u32 = 68;
pub const GRAPH_TYPE: u32 = 71;
pub const GRAPH_CHILDREN: u32 = 80;

/// The graph owner's five intrusive-list heads, selected by [`GRAPH_TYPE`].
pub const GRAPH_HEAD_TYPE_2: u32 = 16;
pub const GRAPH_HEAD_TYPE_1: u32 = 20;
pub const GRAPH_HEAD_TYPE_4: u32 = 24;
pub const GRAPH_HEAD_TYPE_3: u32 = 28;
pub const GRAPH_HEAD_TYPE_5: u32 = 32;

/// The two child vtable entries and the owner's allocator release entry used by the destructor.
pub const GRAPH_VTABLE_ACQUIRE: u32 = 0;
pub const GRAPH_VTABLE_RELEASE: u32 = 12;

/// Host side of the three indirect calls in `sub_82B48F28`.
///
/// The original reads the vtable entry at each call site. Passing that recovered entry address to
/// the host preserves the guest dispatch boundary without assuming every module uses the same
/// teardown routine. `free_graph` receives the owner's allocator object and its vtable entry;
/// an ordinary runtime can dispatch it to its guest heap while a bounded test heap can release
/// the graph allocation directly.
pub trait GraphReleaseHost {
    fn acquire_child(&mut self, g: &mut Guest, child: u32, entry: u32) -> Result<()>;
    fn release_child(&mut self, g: &mut Guest, child: u32, entry: u32, flag: u32) -> Result<()>;
    fn free_graph<H: Heap + ?Sized>(
        &mut self,
        g: &mut Guest,
        heap: &mut H,
        allocator: u32,
        entry: u32,
        graph: u32,
    ) -> Result<()>;
}

fn detach_node(g: &mut Guest, node: u32) -> Result<()> {
    // `sub_82B48F28` tests both links as signed words. Addresses in supported guest regions are
    // below 0x8000_0000, but retain the exact comparison for malformed/rebased test images.
    let next = g.u32(node + 4)?;
    if (next as i32) != 0 {
        g.set_u32(next, g.u32(node)?)?;
    }
    let previous = g.u32(node)?;
    if (previous as i32) != 0 {
        g.set_u32(previous + 4, g.u32(node + 4)?)?;
    }
    Ok(())
}

fn detach_graph_head(g: &mut Guest, graph: u32, head_offset: u32) -> Result<()> {
    let owner = g.u32(graph + GRAPH_OWNER)?;
    let node = graph + GRAPH_NODE;
    if g.u32(owner + head_offset)? == node {
        // The original advances the head through node + 0 (the predecessor link), then performs
        // the two neighbour repairs below even when this object was not the head.
        g.set_u32(owner + head_offset, g.u32(node)?)?;
    }
    detach_node(g, node)
}

/// Tear down a built graph (`sub_82B48F28`).
///
/// Every non-null child is acquired and released through the exact vtable entries the guest
/// loads. With a clear low flag, the graph node is unlinked from the type-selected owner list;
/// types 1 and 3 are spliced before [`voices::remove_handle`], while types 2, 4, and 5 are
/// spliced only when that handle removal returns zero. Finally the owner's allocator receives
/// the graph. The `GraphReleaseHost` boundary is intentional: these three calls are genuinely
/// dynamic and cannot be replaced with a generic guest-memory free.
pub fn release_graph<H: Heap + ?Sized, R: GraphReleaseHost + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    host: &mut R,
    graph: u32,
    flag: u32,
) -> Result<()> {
    let mut child_slot = graph + GRAPH_CHILDREN;
    let mut child_index = 0i32;
    if g.u8(graph + GRAPH_CHILD_COUNT)? != 0 {
        loop {
            let child = g.u32(child_slot)?;
            if child != 0 {
                let vtable = g.u32(child)?;
                host.acquire_child(g, child, g.u32(vtable + GRAPH_VTABLE_ACQUIRE)?)?;

                // Reload the child and vtable after the acquire call. Both loads are observable
                // when a child destructor aliases its containing graph.
                let child = g.u32(child_slot)?;
                let vtable = g.u32(child)?;
                host.release_child(g, child, g.u32(vtable + GRAPH_VTABLE_RELEASE)?, 0)?;
            }
            child_index += 1;
            child_slot = child_slot.wrapping_add(4);
            if child_index >= i32::from(g.u8(graph + GRAPH_CHILD_COUNT)?) {
                break;
            }
        }
    }

    if flag & 0xFF == 0 {
        match g.u8(graph + GRAPH_TYPE)? {
            1 => detach_graph_head(g, graph, GRAPH_HEAD_TYPE_1)?,
            3 => detach_graph_head(g, graph, GRAPH_HEAD_TYPE_3)?,
            _ => {}
        }

        // `remove_handle` returns one when it found and removed this graph. The later type cases
        // run only for a zero low byte, exactly as the lifted branch does.
        if (voices::remove_handle(g, graph)? as u8) == 0 {
            match g.u8(graph + GRAPH_TYPE)? {
                2 => detach_graph_head(g, graph, GRAPH_HEAD_TYPE_2)?,
                4 => detach_graph_head(g, graph, GRAPH_HEAD_TYPE_4)?,
                5 => detach_graph_head(g, graph, GRAPH_HEAD_TYPE_5)?,
                _ => {}
            }
        }
    }

    let owner = g.u32(graph + GRAPH_OWNER)?;
    let allocator = g.u32(owner + 36)?;
    let vtable = g.u32(allocator)?;
    host.free_graph(
        g,
        heap,
        allocator,
        g.u32(vtable + GRAPH_VTABLE_RELEASE)?,
        graph,
    )
}

pub const STATE_PLAYING: u8 = 1;
pub const STATE_STOPPED: u8 = 4;

const _: () = assert!(PLAYER_SOURCE == 0x50, "rw_player.source");
const _: () = assert!(PLAYER_PACKET_HEAD == 0x148, "rw_player.packet_head");
const _: () = assert!(PLAYER_PACKET_TAIL == 0x14C, "rw_player.packet_tail");
const _: () = assert!(PLAYER_DECODER == 0x150, "rw_player.decoder");
const _: () = assert!(PLAYER_SAMPLE_RATE == 0x154, "rw_player.sample_rate");
const _: () = assert!(PLAYER_STATE == 0x15E, "rw_player.state");
const _: () = assert!(PLAYER_CHANNEL_COUNT == 0x15F, "rw_player.channel_count");
const _: () = assert!(PLAYER_FORMAT_INDEX == 0x160, "rw_player.format_index");
const _: () = assert!(PACKET_NEXT == 0x0C, "rw_packet.next");
const _: () = assert!(
    PLAYER_TABLE == PLAYER_TABLE_WALK + TABLE_STRIDE,
    "table walk base"
);

/// The guest's `fctidz`, then `stfd` and `lbz +7`: truncate toward zero into a 64-bit integer,
/// spill big-endian, read the least significant byte.
///
/// Written out branch for branch rather than as `value as i64`, because Rust's float-to-int
/// cast **saturates** and the guest path does not agree with saturation at the edges: NaN
/// spills `i64::MIN` (low byte 0), anything above `2^63` saturates to `i64::MAX` (low byte
/// 0xFF), and exactly `2^63` takes CVTTSD2SI's "integer indefinite" instead (low byte 0),
/// because the original's test is `>` and not `>=`.
pub fn truncated_low_byte(value: f32) -> u8 {
    const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
    let widened = value as f64;
    let spilled: u64 = if widened.is_nan() {
        0x8000_0000_0000_0000
    } else if widened > TWO_POW_63 {
        i64::MAX as u64
    } else if widened >= TWO_POW_63 || widened < -TWO_POW_63 {
        0x8000_0000_0000_0000
    } else {
        (widened as i64) as u64
    };
    (spilled & 0xFF) as u8
}

/// `lfs` then `stfs`: a float -> double -> float round trip, lossless except that it quiets a
/// signalling NaN. Reproduced rather than copied as a word, for that one case.
fn round_trip_f32_bits(bits: u32) -> u32 {
    ((f32::from_bits(bits) as f64) as f32).to_bits()
}

/// `EVENT_PLAY` (`sub_82B28B78`). Publishes the stream's format onto the player and marks it
/// playing. Returns the record size the drain advances by.
///
/// **Verification: one comparable call, at one input point** (48 kHz, six channels, format 1).
/// It fires once per stream start and a boot plays one frontend movie, so a session yields
/// exactly one comparison. Treat this as the least-checked of the three.
///
/// `restart` stands in for `sub_82B29018`, which the original calls only if the state byte
/// reads 4 or 0 *after* being set to 1 — reachable only if the two writes through `source`
/// overlap that byte. It never fired in any observed session. The callee takes a critical
/// section and makes two indirect calls, so it is not portable; the caller supplies it.
pub fn event_play(g: &mut Guest, record: u32, restart: Option<&mut dyn FnMut(u32)>) -> Result<u32> {
    let player = g.u32(record + RECORD_OBJECT)?;

    // Cleared, not torn down: EVENT_PLAY runs before a decoder exists.
    g.set_u32(player + PLAYER_DECODER, 0)?;

    let format = truncated_low_byte(g.f32(record + RECORD_PLAY_FORMAT)?);
    g.set_u8(player + PLAYER_FORMAT_INDEX, format)?;

    let rate_bits = round_trip_f32_bits(g.u32(record + RECORD_PLAY_RATE)?);
    let source = g.u32(player + PLAYER_SOURCE)?;
    g.set_u32(player + PLAYER_SAMPLE_RATE, rate_bits)?;
    g.set_u8(player + PLAYER_STATE, STATE_PLAYING)?;
    g.set_u8(
        player + PLAYER_CHANNEL_COUNT,
        truncated_low_byte(g.f32(record + RECORD_PLAY_CHANNELS)?),
    )?;

    g.set_u32(source, 0)?;
    g.set_u8(source + 4, g.u8(player + PLAYER_FORMAT_INDEX)?)?;

    let state = g.u8(player + PLAYER_STATE)?;
    if state == STATE_STOPPED || state == 0 {
        if let Some(f) = restart {
            f(player);
        }
    }
    Ok(SIZE_PLAY)
}

/// `EVENT_STOP` (`sub_82B28C18`). Stops playback and drops every submitted packet.
///
/// **Verification: none.** Zero comparable calls across every session: it arrives with a live
/// decoder, whose release the shadow harness cannot rewind, so the harness ran the original
/// every time and counted the skip. This body is translated from a C++ body that was itself
/// never checked against the original. Treat it as unverified in both languages.
///
/// `teardown` stands in for `sub_82B3C930`, which releases the voice through four indirect
/// calls.
pub fn event_stop(g: &mut Guest, record: u32, mut teardown: impl FnMut(u32)) -> Result<u32> {
    let player = g.u32(record + RECORD_OBJECT)?;

    let decoder = g.u32(player + PLAYER_DECODER)?;
    if decoder != 0 {
        teardown(decoder);
        g.set_u32(player + PLAYER_DECODER, 0)?;
        g.set_u32(player + PLAYER_TORN_DOWN, 255)?;
    }
    // The 0xFF above is immediately overwritten by this wipe. Reproduced anyway: the job is to
    // match the original, not to tidy it.
    g.fill(player + PLAYER_DECODER, 0, STOP_WIPE)?;
    g.set_u8(player + PLAYER_STATE, STATE_STOPPED)?;

    loop {
        let packet = g.u32(player + PLAYER_PACKET_HEAD)?;
        if packet == 0 {
            break;
        }
        let next = g.u32(packet + PACKET_NEXT)?;
        g.set_u32(player + PLAYER_PACKET_HEAD, next)?;
        if next == 0 {
            g.set_u32(player + PLAYER_PACKET_TAIL, 0)?;
        }
        g.set_u32(packet + PACKET_NEXT, 0)?;
    }

    // Keeps the original's `stwu` shape: the byte at entry+21, then advance, then the word at
    // the new entry. Reordering it would change which slot the last iteration clears.
    let mut entry = player + PLAYER_TABLE_WALK;
    for _ in 0..TABLE_ENTRIES {
        g.set_u8(entry + 21, 0)?;
        entry += TABLE_STRIDE;
        g.set_u32(entry, 0)?;
    }

    g.set_u8(player + PLAYER_STOP_F173, 0)?;
    g.set_u8(player + PLAYER_STOP_F174, 0)?;
    g.set_u8(player + PLAYER_STOP_F172, 16)?;
    Ok(SIZE_STOP)
}

/// `EVENT_SUBMIT` (`sub_82B28CC0`). Appends a packet to the player's FIFO.
///
/// **Verification: 1,678 comparable calls at zero divergence, and promoted** — it runs
/// natively in the recomp. The best-checked of the three.
pub fn event_submit(g: &mut Guest, record: u32) -> Result<u32> {
    let player = g.u32(record + RECORD_OBJECT)?;
    let packet = g.u32(record + RECORD_SUBMIT_PACKET)?;

    if g.u32(player + PLAYER_PACKET_HEAD)? == 0 {
        g.set_u32(player + PLAYER_PACKET_HEAD, packet)?;
    } else {
        let tail = g.u32(player + PLAYER_PACKET_TAIL)?;
        g.set_u32(tail + PACKET_NEXT, packet)?;
    }
    g.set_u32(player + PLAYER_PACKET_TAIL, packet)?;
    g.set_u32(packet + PACKET_NEXT, 0)?;
    Ok(SIZE_SUBMIT)
}

/// Whether `wanted` is still live on this player: on the submitted-packet FIFO, or in the
/// 20-entry table with a discriminator that is not 2.
///
/// This is the producer's non-append path (`sub_82B28A00`, selector 3 and up), which supplied
/// 4,602 of the 6,706 comparable calls that body saw — about three liveness queries per
/// submitted packet, so it is the best-exercised path in the queue.
pub fn packet_is_live(g: &Guest, player: u32, wanted: u32) -> Result<bool> {
    let mut node = g.u32(player + PLAYER_PACKET_HEAD)?;
    while node != 0 {
        if node == wanted {
            return Ok(true);
        }
        node = g.u32(node + PACKET_NEXT)?;
    }
    for i in 0..TABLE_ENTRIES {
        let entry = player + PLAYER_TABLE + i * TABLE_STRIDE;
        if g.u32(entry)? == wanted {
            return Ok(g.u8(entry + TABLE_DISCRIMINATOR)? != 2);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::BumpHeap;
    use crate::testutil::*;

    const GRAPH: u32 = 0x4000_1000;
    const OWNER: u32 = 0x4000_1800;
    const ALLOCATOR: u32 = 0x4000_1C00;
    const ALLOCATOR_VTABLE: u32 = 0x4000_1D00;
    const CHILD_A: u32 = 0x4000_2000;
    const CHILD_B: u32 = 0x4000_2200;
    const CHILD_A_VTABLE: u32 = 0x4000_2400;
    const CHILD_B_VTABLE: u32 = 0x4000_2500;

    #[derive(Default)]
    struct ReleaseRecorder {
        calls: Vec<(char, u32, u32, u32)>,
    }

    impl GraphReleaseHost for ReleaseRecorder {
        fn acquire_child(&mut self, _g: &mut Guest, child: u32, entry: u32) -> Result<()> {
            self.calls.push(('a', child, entry, 0));
            Ok(())
        }

        fn release_child(
            &mut self,
            _g: &mut Guest,
            child: u32,
            entry: u32,
            flag: u32,
        ) -> Result<()> {
            self.calls.push(('r', child, entry, flag));
            Ok(())
        }

        fn free_graph<H: Heap + ?Sized>(
            &mut self,
            _g: &mut Guest,
            _heap: &mut H,
            allocator: u32,
            entry: u32,
            graph: u32,
        ) -> Result<()> {
            self.calls.push(('f', allocator, entry, graph));
            Ok(())
        }
    }

    fn graph_guest(kind: u8) -> Guest {
        let mut g = Guest::single(0x4000_0000, 0x4000);
        g.set_u32(GRAPH + GRAPH_OWNER, OWNER).unwrap();
        g.set_u8(GRAPH + GRAPH_CHILD_COUNT, 2).unwrap();
        g.set_u8(GRAPH + GRAPH_TYPE, kind).unwrap();
        g.set_u32(GRAPH + GRAPH_CHILDREN, CHILD_A).unwrap();
        g.set_u32(GRAPH + GRAPH_CHILDREN + 4, CHILD_B).unwrap();
        g.set_u32(CHILD_A, CHILD_A_VTABLE).unwrap();
        g.set_u32(CHILD_B, CHILD_B_VTABLE).unwrap();
        g.set_u32(CHILD_A_VTABLE + GRAPH_VTABLE_ACQUIRE, 0xA001)
            .unwrap();
        g.set_u32(CHILD_A_VTABLE + GRAPH_VTABLE_RELEASE, 0xA00C)
            .unwrap();
        g.set_u32(CHILD_B_VTABLE + GRAPH_VTABLE_ACQUIRE, 0xB001)
            .unwrap();
        g.set_u32(CHILD_B_VTABLE + GRAPH_VTABLE_RELEASE, 0xB00C)
            .unwrap();
        g.set_u32(OWNER + 36, ALLOCATOR).unwrap();
        g.set_u32(ALLOCATOR, ALLOCATOR_VTABLE).unwrap();
        g.set_u32(ALLOCATOR_VTABLE + GRAPH_VTABLE_RELEASE, 0xF00C)
            .unwrap();
        // No handle entry means `remove_handle` returns zero and makes types 2/4/5 eligible for
        // their post-removal list splice.
        g.set_u16(OWNER + voices::HANDLE_COUNT, 0).unwrap();
        g
    }

    #[test]
    fn graph_release_runs_child_pairs_then_unlinks_type_two_and_frees() {
        let mut g = graph_guest(2);
        let node = GRAPH + GRAPH_NODE;
        let previous = 0x4000_2800;
        let next = 0x4000_2900;
        g.set_u32(OWNER + GRAPH_HEAD_TYPE_2, node).unwrap();
        g.set_u32(node, previous).unwrap();
        g.set_u32(node + 4, next).unwrap();
        g.set_u32(previous + 4, node).unwrap();
        g.set_u32(next, node).unwrap();
        let mut host = ReleaseRecorder::default();
        let mut heap = BumpHeap {
            next: 0x4000_3000,
            end: 0x4000_3F00,
        };

        release_graph(&mut g, &mut heap, &mut host, GRAPH, 0).unwrap();

        assert_eq!(
            host.calls,
            [
                ('a', CHILD_A, 0xA001, 0),
                ('r', CHILD_A, 0xA00C, 0),
                ('a', CHILD_B, 0xB001, 0),
                ('r', CHILD_B, 0xB00C, 0),
                ('f', ALLOCATOR, 0xF00C, GRAPH),
            ]
        );
        assert_eq!(g.u32(OWNER + GRAPH_HEAD_TYPE_2).unwrap(), previous);
        assert_eq!(g.u32(previous + 4).unwrap(), next);
        assert_eq!(g.u32(next).unwrap(), previous);
    }

    #[test]
    fn graph_release_skips_intrusive_lists_when_its_low_flag_is_set() {
        let mut g = graph_guest(1);
        let node = GRAPH + GRAPH_NODE;
        g.set_u32(OWNER + GRAPH_HEAD_TYPE_1, node).unwrap();
        g.set_u32(node, 0x4000_2800).unwrap();
        g.set_u32(node + 4, 0x4000_2900).unwrap();
        let mut host = ReleaseRecorder::default();
        let mut heap = BumpHeap {
            next: 0x4000_3000,
            end: 0x4000_3F00,
        };

        release_graph(&mut g, &mut heap, &mut host, GRAPH, 1).unwrap();

        assert_eq!(g.u32(OWNER + GRAPH_HEAD_TYPE_1).unwrap(), node);
        assert_eq!(g.u32(node).unwrap(), 0x4000_2800);
        assert_eq!(g.u32(node + 4).unwrap(), 0x4000_2900);
        assert_eq!(host.calls.last(), Some(&('f', ALLOCATOR, 0xF00C, GRAPH)));
    }

    #[test]
    fn fctidz_low_byte_disagrees_with_a_saturating_cast_at_exactly_two_pow_63() {
        // The whole reason truncated_low_byte is written branch for branch. At 2^63 the guest
        // takes CVTTSD2SI's indefinite value (low byte 0x00) because its test is `>` and not
        // `>=`, while Rust's saturating `as i64` would give i64::MAX (low byte 0xFF).
        let two_pow_63 = 9_223_372_036_854_775_808.0f32;
        assert_eq!(truncated_low_byte(two_pow_63), 0x00);
        assert_eq!(
            ((two_pow_63 as f64) as i64 as u64 & 0xFF) as u8,
            0xFF,
            "the naive cast"
        );

        // Cases where the two happen to agree, asserted so a future simplification that breaks
        // one of them is caught rather than assumed safe.
        assert_eq!(truncated_low_byte(f32::NAN), 0x00);
        assert_eq!(truncated_low_byte(1e30), 0xFF);
        assert_eq!(truncated_low_byte(-1e30), 0x00);

        assert_eq!(truncated_low_byte(1.0), 1);
        assert_eq!(truncated_low_byte(6.0), 6);
        assert_eq!(truncated_low_byte(255.9), 255);
        assert_eq!(truncated_low_byte(256.0), 0);
        assert_eq!(truncated_low_byte(-1.0), 0xFF);
        assert_eq!(truncated_low_byte(-0.5), 0);
    }

    #[test]
    fn submit_appends_then_links_the_tail() {
        let mut g = guest();
        g.set_u32(PLAYER + crate::system::PLAYER_SYSTEM, SYSTEM)
            .unwrap();

        // First submit: empty FIFO, so head and tail both become the packet.
        g.set_u32(RING + crate::system::RECORD_OBJECT, PLAYER)
            .unwrap();
        g.set_u32(RING + crate::system::RECORD_SUBMIT_PACKET, PACKET_A)
            .unwrap();
        assert_eq!(event_submit(&mut g, RING).unwrap(), 12);
        assert_eq!(g.u32(PLAYER + PLAYER_PACKET_HEAD).unwrap(), PACKET_A);
        assert_eq!(g.u32(PLAYER + PLAYER_PACKET_TAIL).unwrap(), PACKET_A);
        assert_eq!(g.u32(PACKET_A + PACKET_NEXT).unwrap(), 0);

        // Second: the old tail's next points at it, head is unchanged.
        g.set_u32(RING + crate::system::RECORD_SUBMIT_PACKET, PACKET_B)
            .unwrap();
        event_submit(&mut g, RING).unwrap();
        assert_eq!(g.u32(PLAYER + PLAYER_PACKET_HEAD).unwrap(), PACKET_A);
        assert_eq!(g.u32(PACKET_A + PACKET_NEXT).unwrap(), PACKET_B);
        assert_eq!(g.u32(PLAYER + PLAYER_PACKET_TAIL).unwrap(), PACKET_B);
    }

    #[test]
    fn stop_unlinks_everything_and_writes_the_undocumented_bytes() {
        let mut g = guest();
        g.set_u32(RING + crate::system::RECORD_OBJECT, PLAYER)
            .unwrap();
        g.set_u32(PLAYER + PLAYER_PACKET_HEAD, PACKET_A).unwrap();
        g.set_u32(PACKET_A + PACKET_NEXT, PACKET_B).unwrap();
        g.set_u32(PLAYER + PLAYER_PACKET_TAIL, PACKET_B).unwrap();
        g.set_u32(PLAYER + PLAYER_DECODER, 0xDEAD_BEEF).unwrap();

        let mut torn = Vec::new();
        assert_eq!(event_stop(&mut g, RING, |d| torn.push(d)).unwrap(), 8);

        assert_eq!(
            torn,
            vec![0xDEAD_BEEF],
            "the live decoder is torn down exactly once"
        );
        assert_eq!(g.u32(PLAYER + PLAYER_PACKET_HEAD).unwrap(), 0);
        assert_eq!(g.u32(PLAYER + PLAYER_PACKET_TAIL).unwrap(), 0);
        assert_eq!(g.u32(PACKET_A + PACKET_NEXT).unwrap(), 0);
        assert_eq!(g.u8(PLAYER + PLAYER_STATE).unwrap(), STATE_STOPPED);
        assert_eq!(g.u32(PLAYER + PLAYER_DECODER).unwrap(), 0);
        // The 0xFF written to +0x158 is overwritten by the wipe that follows it.
        assert_eq!(g.u32(PLAYER + PLAYER_TORN_DOWN).unwrap(), 0);
        assert_eq!(g.u8(PLAYER + PLAYER_STOP_F172).unwrap(), 16);
        assert_eq!(g.u8(PLAYER + PLAYER_STOP_F173).unwrap(), 0);
        assert_eq!(g.u8(PLAYER + PLAYER_STOP_F174).unwrap(), 0);
    }

    #[test]
    fn play_unpacks_three_floats_and_publishes_through_source() {
        let mut g = guest();
        g.set_u32(RING + crate::system::RECORD_OBJECT, PLAYER)
            .unwrap();
        g.set_u32(RING + crate::system::RECORD_PLAY_FORMAT, 1.0f32.to_bits())
            .unwrap();
        g.set_u32(RING + crate::system::RECORD_PLAY_RATE, 48000.0f32.to_bits())
            .unwrap();
        g.set_u32(RING + crate::system::RECORD_PLAY_CHANNELS, 6.0f32.to_bits())
            .unwrap();
        g.set_u32(PLAYER + PLAYER_DECODER, 0x1234).unwrap();

        let mut restarted = 0;
        assert_eq!(
            event_play(&mut g, RING, Some(&mut |_| restarted += 1)).unwrap(),
            20
        );
        // The input point the one verified call covered: 48 kHz, six channels, format 1.
        assert_eq!(g.u8(PLAYER + PLAYER_FORMAT_INDEX).unwrap(), 1);
        assert_eq!(g.f32(PLAYER + PLAYER_SAMPLE_RATE).unwrap(), 48000.0);
        assert_eq!(g.u8(PLAYER + PLAYER_CHANNEL_COUNT).unwrap(), 6);
        assert_eq!(g.u8(PLAYER + PLAYER_STATE).unwrap(), STATE_PLAYING);
        assert_eq!(
            g.u32(PLAYER + PLAYER_DECODER).unwrap(),
            0,
            "cleared, not torn down"
        );
        assert_eq!(g.u32(SOURCE).unwrap(), 0);
        assert_eq!(g.u8(SOURCE + 4).unwrap(), 1);
        assert_eq!(
            restarted, 0,
            "the restart branch is unreachable absent aliasing"
        );
    }

    #[test]
    fn liveness_checks_the_fifo_then_the_table() {
        let mut g = guest();

        assert!(
            !packet_is_live(&g, PLAYER, PACKET_A).unwrap(),
            "neither list holds it"
        );

        g.set_u32(PLAYER + PLAYER_PACKET_HEAD, PACKET_B).unwrap();
        g.set_u32(PACKET_B + PACKET_NEXT, PACKET_A).unwrap();
        assert!(
            packet_is_live(&g, PLAYER, PACKET_A).unwrap(),
            "found by walking the FIFO"
        );

        // Off the FIFO, in the table: live only when the discriminator is not 2.
        g.set_u32(PLAYER + PLAYER_PACKET_HEAD, 0).unwrap();
        let entry = PLAYER + PLAYER_TABLE + 3 * TABLE_STRIDE;
        g.set_u32(entry, PACKET_A).unwrap();
        g.set_u8(entry + TABLE_DISCRIMINATOR, 1).unwrap();
        assert!(packet_is_live(&g, PLAYER, PACKET_A).unwrap());
        g.set_u8(entry + TABLE_DISCRIMINATOR, 2).unwrap();
        assert!(!packet_is_live(&g, PLAYER, PACKET_A).unwrap());
    }
}
