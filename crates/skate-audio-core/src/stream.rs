//! A stream object's frame delivery, `sub_82B3CA60`, with the stream's fill function supplied by the
//! host.
//!
//! **Unverified.** The C++ body in `recomp/src/audio_ports/` is gate 1 (its two `bctrl` go through
//! the fill pointer at `+20`) and was never compared. This is transcribed from that body. The fill
//! function is the XMA decode path in the game; here it is [`StreamFill`], which is where an engine
//! plugs in PCM it decoded itself (`docs/PLAN.md`, Phase 6, "the source, mapped").
//!
//! The stream object: `+20` fill pointer, `+28` position (advanced by [`advance_segment_position`]),
//! `+36` offset of the 24-byte entry table, `+40` offset of the scratch descriptor, `+44` u16 frames
//! pending in the scratch, `+46` u8 channels, `+49` u8 active entry, `+51` u8 "refill through the
//! scratch". An entry's `+12` is the total. A buffer descriptor: `+4` data (four bytes a sample),
//! `+12` u16 frames the last refill produced, `+14` u16 stride between one channel's run and the
//! next.

use crate::cursors::advance_segment_position;
use crate::mem::memcpy;
use crate::{Guest, Result};

/// The stream's fill function (`stream+20`): write up to `frames` frames into `descriptor`'s buffer,
/// each channel's run `stride` samples after the previous one's, and return how many it produced.
pub trait StreamFill {
    fn fill(&mut self, g: &mut Guest, stream: u32, descriptor: u32, frames: u64) -> Result<u64>;
}

fn word(value: u64) -> i32 {
    value as u32 as i32
}

/// `sub_82B3CA60`: deliver `requested` frames from `stream` into the caller's descriptor `dest`.
/// Returns the frames delivered, all 64 bits of the accumulator as the original leaves it.
pub fn deliver_frames(
    g: &mut Guest,
    fill: &mut dyn StreamFill,
    stream: u32,
    dest: u32,
    requested: u64,
) -> Result<u64> {
    let mut delivered: u64 = 0;
    if g.u8(stream + 51)? != 0 {
        let scratch = g.u32(stream + 40)?.wrapping_add(stream);
        let pending = g.u16(stream + 44)? as u64;
        if pending != 0 {
            // min(pending, requested), signed; when requested wins its full 64 bits are kept.
            delivered = if word(pending) < word(requested) {
                pending
            } else {
                requested
            };
            if g.u8(stream + 46)? != 0 {
                let bytes = ((delivered as u32) << 2) as u64;
                let mut channel: u64 = 0;
                loop {
                    let src_stride = g.u16(scratch + 14)? as u64;
                    let pending_now = g.u16(stream + 44)? as u64;
                    let src_channel = word(src_stride) as i64 * word(channel) as i64;
                    let filled = g.u16(scratch + 12)? as u64;
                    let dst_stride = g.u16(dest + 14)? as u64;
                    let src_base = g.u32(scratch + 4)? as u64;
                    let dst_base = g.u32(dest + 4)? as u64;
                    let src_index = (src_channel as u64)
                        .wrapping_sub(pending_now)
                        .wrapping_add(filled);
                    let dst_channel = word(dst_stride) as i64 * word(channel) as i64;
                    let dst_ptr = (((dst_channel as u32) << 2) as u64).wrapping_add(dst_base);
                    let src_ptr = (((src_index as u32) << 2) as u64).wrapping_add(src_base);
                    memcpy(g, dst_ptr as u32, src_ptr as u32, bytes)?;
                    let channels = g.u8(stream + 46)? as u32;
                    channel += 1;
                    if !((channel as u32) < channels) {
                        break;
                    }
                }
            }
            let left = (g.u16(stream + 44)? as u64).wrapping_sub(delivered);
            g.set_u16(stream + 44, left as u16)?;
            advance_segment_position(g, stream, delivered as u32)?;
        }

        while word(delivered) < word(requested) {
            let entry = (24 * g.u8(stream + 49)? as u32)
                .wrapping_add(stream)
                .wrapping_add(g.u32(stream + 36)?);
            if word(g.u32(entry + 12)? as u64) == 0 {
                break;
            }
            let src_stride = g.u16(scratch + 14)? as u64;
            let remaining = requested.wrapping_sub(delivered);
            let ask = if word(remaining) < word(src_stride) {
                remaining
            } else {
                src_stride
            };
            let produced = fill.fill(g, stream, scratch, ask)?;

            let total = g.u32(entry + 12)? as u64;
            let consumed = g.u32(stream + 28)? as u64;
            let mut accepted = total.wrapping_sub(consumed);
            if word(produced) < word(accepted) {
                accepted = produced;
            }
            g.set_u16(stream + 44, accepted as u16)?;
            g.set_u16(scratch + 12, accepted as u16)?;
            let pending_now = g.u16(stream + 44)? as u64; // reloaded: the two stores may alias
            let chunk = if word(pending_now) < word(remaining) {
                pending_now
            } else {
                remaining
            };

            if g.u8(stream + 46)? != 0 {
                let bytes = ((chunk as u32) << 2) as u64;
                let mut channel: u64 = 0;
                loop {
                    let dst_stride = g.u16(dest + 14)? as u64;
                    let stride = g.u16(scratch + 14)? as u64;
                    let dst_channel = word(dst_stride) as i64 * word(channel) as i64;
                    let src_base = g.u32(scratch + 4)? as u64;
                    let dst_base = g.u32(dest + 4)? as u64;
                    let dst_index = (dst_channel as u64).wrapping_add(delivered);
                    let src_channel = word(stride) as i64 * word(channel) as i64;
                    let src_ptr = (((src_channel as u32) << 2) as u64).wrapping_add(src_base);
                    let dst_ptr = (((dst_index as u32) << 2) as u64).wrapping_add(dst_base);
                    memcpy(g, dst_ptr as u32, src_ptr as u32, bytes)?;
                    let channels = g.u8(stream + 46)? as u32;
                    channel += 1;
                    if !((channel as u32) < channels) {
                        break;
                    }
                }
            }
            let left = (g.u16(stream + 44)? as u64).wrapping_sub(chunk);
            delivered = chunk.wrapping_add(delivered);
            g.set_u16(stream + 44, left as u16)?;
            advance_segment_position(g, stream, chunk as u32)?;
        }
        return Ok(delivered);
    }

    // No scratch: the fill writes straight into the caller's descriptor, and what it returns is
    // ignored -- the request size is credited.
    if word(requested) > 0 {
        loop {
            let entry = (24 * g.u8(stream + 49)? as u32)
                .wrapping_add(stream)
                .wrapping_add(g.u32(stream + 36)?);
            let total = g.u32(entry + 12)? as u64;
            if word(total) == 0 {
                break;
            }
            let consumed = g.u32(stream + 28)? as u64;
            let remaining = requested.wrapping_sub(delivered);
            let mut chunk = total.wrapping_sub(consumed);
            if word(remaining) < word(chunk) {
                chunk = remaining;
            }
            fill.fill(g, stream, dest, chunk)?;
            delivered = chunk.wrapping_add(delivered);
            advance_segment_position(g, stream, chunk as u32)?;
            if !(word(delivered) < word(requested)) {
                break;
            }
        }
    }
    Ok(delivered)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x5000_0000;
    const STREAM: u32 = MEM;
    const SCRATCH_DATA: u32 = MEM + 0x400;
    const DEST: u32 = MEM + 0x800;
    const DEST_DATA: u32 = MEM + 0xC00;

    /// Writes a continuous ramp per channel (`channel * 10000 + n`), counting across calls. With
    /// `overfill` it fills a whole stride whatever it was asked for, which is what leaves frames
    /// pending: a fill that returns only what it was asked never does.
    struct Ramp {
        produced: u32,
        calls: Vec<u64>,
        overfill: bool,
    }

    impl StreamFill for Ramp {
        fn fill(
            &mut self,
            g: &mut Guest,
            stream: u32,
            descriptor: u32,
            frames: u64,
        ) -> Result<u64> {
            self.calls.push(frames);
            let channels = g.u8(stream + 46)? as u32;
            let stride = g.u16(descriptor + 14)? as u32;
            let frames = if self.overfill { stride as u64 } else { frames };
            let data = g.u32(descriptor + 4)?;
            for ch in 0..channels {
                for i in 0..frames as u32 {
                    g.set_u32(data + 4 * (stride * ch + i), ch * 10000 + self.produced + i)?;
                }
            }
            self.produced += frames as u32;
            Ok(frames)
        }
    }

    fn guest(scratch: bool, total: u32) -> Guest {
        let mut g = Guest::single(MEM, 0x2000);
        g.set_u32(STREAM + 36, 64).unwrap(); // entry table at +64
        g.set_u32(STREAM + 40, 200).unwrap(); // scratch descriptor at +200
        g.set_u8(STREAM + 46, 2).unwrap(); // two channels
        g.set_u8(STREAM + 50, 1).unwrap(); // one entry
        g.set_u8(STREAM + 51, scratch as u8).unwrap();
        g.set_u32(STREAM + 64 + 12, total).unwrap();
        g.set_u32(STREAM + 200 + 4, SCRATCH_DATA).unwrap();
        g.set_u16(STREAM + 200 + 14, 4).unwrap(); // a refill is capped at 4 frames
        g.set_u32(DEST + 4, DEST_DATA).unwrap();
        g.set_u16(DEST + 14, 16).unwrap();
        g
    }

    fn dest_channel(g: &Guest, ch: u32, frames: u32) -> Vec<u32> {
        (0..frames)
            .map(|i| g.u32(DEST_DATA + 4 * (16 * ch + i)).unwrap())
            .collect()
    }

    #[test]
    fn through_the_scratch_the_refills_are_capped_and_copied_in_order() {
        let mut g = guest(true, 1000);
        let mut fill = Ramp {
            produced: 0,
            calls: vec![],
            overfill: false,
        };
        assert_eq!(
            deliver_frames(&mut g, &mut fill, STREAM, DEST, 10).unwrap(),
            10
        );
        assert_eq!(fill.calls, vec![4, 4, 2]);
        assert_eq!(dest_channel(&g, 0, 10), (0..10).collect::<Vec<_>>());
        assert_eq!(dest_channel(&g, 1, 10), (10000..10010).collect::<Vec<_>>());
        assert_eq!(
            g.u32(STREAM + 28).unwrap(),
            10,
            "the position advanced by what was delivered"
        );
        assert_eq!(g.u16(STREAM + 44).unwrap(), 0, "nothing left pending");
    }

    #[test]
    fn a_short_entry_leaves_the_rest_pending_for_the_next_call() {
        let mut g = guest(true, 1000);
        let mut fill = Ramp {
            produced: 0,
            calls: vec![],
            overfill: true,
        };
        // Ask for 6: two refills of 4 (the second asked for 2), the second only half consumed.
        assert_eq!(
            deliver_frames(&mut g, &mut fill, STREAM, DEST, 6).unwrap(),
            6
        );
        assert_eq!(g.u16(STREAM + 44).unwrap(), 2);
        // The next call drains the two pending frames first, then refills.
        assert_eq!(
            deliver_frames(&mut g, &mut fill, STREAM, DEST, 3).unwrap(),
            3
        );
        assert_eq!(dest_channel(&g, 0, 3), vec![6, 7, 8]);
    }

    #[test]
    fn without_a_scratch_the_fill_writes_directly_and_the_request_is_credited() {
        let mut g = guest(false, 1000);
        let mut fill = Ramp {
            produced: 0,
            calls: vec![],
            overfill: false,
        };
        assert_eq!(
            deliver_frames(&mut g, &mut fill, STREAM, DEST, 10).unwrap(),
            10
        );
        assert_eq!(fill.calls, vec![10]);
        assert_eq!(dest_channel(&g, 1, 3), vec![10000, 10001, 10002]);
    }

    #[test]
    fn an_empty_entry_delivers_nothing() {
        let mut g = guest(true, 0);
        let mut fill = Ramp {
            produced: 0,
            calls: vec![],
            overfill: false,
        };
        assert_eq!(
            deliver_frames(&mut g, &mut fill, STREAM, DEST, 10).unwrap(),
            0
        );
        assert!(fill.calls.is_empty());
    }
}
