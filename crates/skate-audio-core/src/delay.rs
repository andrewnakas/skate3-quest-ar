//! The `Del0` circular-history allocation primitive.
//!
//! `Del0`'s module constructor and process function share a small buffer object at module `+100`.
//! The object is allocated by `sub_82B3D590` and retired by `sub_82B3D870`.  It is separate from
//! the delay's state machine: this module owns the exact capacity arithmetic, allocation lifetime,
//! and resize copy, leaving the sample process pass to the following port.

use crate::patch::Heap;
use crate::{Guest, Result};
use crate::{fp, mem};

/// `sub_82B3D590` / `sub_82B3D870` buffer-object fields.
pub const BUFFER: u32 = 0;
pub const REQUEST: u32 = 12;
pub const ALIGNMENT: u32 = 16;
pub const STRIDE: u32 = 20;
pub const CURSOR: u32 = 24;
pub const CAPACITY: u32 = 32;
/// The circular-history position used by the resize copy.
pub const HISTORY: u32 = 36;
pub const CHANNELS: u32 = 48;
pub const WRITTEN: u32 = 52;

/// The final graph, like the other recovered module stages, operates on 256-frame planar blocks.
pub const BLOCK_FRAMES: u32 = 256;
const OWNER_SOURCE: u32 = 28;
const OWNER_DESTINATION: u32 = 32;
const DESCRIPTOR_DATA: u32 = 4;
const DESCRIPTOR_STRIDE: u32 = 14;

/// `sub_82B3D590`: allocate the channel-major `Del0` history buffer.
///
/// `channels` is the module's input channel count, `requested` the target history length, and
/// `minimum` the currently required block length. The title reserves at least `minimum + 255`
/// samples, rounds the sample stride and alignment area independently to 32, then allocates four
/// bytes per sample per channel. The concrete title calls its system allocator with a 128-byte
/// alignment; [`Heap`] is the corresponding host boundary here.
pub fn allocate<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    state: u32,
    channels: u32,
    requested: u32,
    minimum: u32,
) -> Result<bool> {
    let threshold = minimum.wrapping_add(255);
    // `cmpw; bgt`: this is a signed comparison, even though all captured normal paths carry
    // positive frame counts. Keep the high-bit behavior for a malformed/overflowed request.
    let needed = if (requested as i32) > (threshold as i32) {
        requested
    } else {
        threshold
    };
    let stride = needed.wrapping_add(32) & !31;
    let alignment = minimum.wrapping_add(30) & !31;
    let capacity = stride.wrapping_add(alignment);
    let buffer = if needed == 0 {
        0
    } else {
        heap.alloc(g, capacity.wrapping_mul(channels).wrapping_mul(4), 128)?
    };
    if needed != 0 && buffer == 0 {
        return Ok(false);
    }
    g.set_u32(state.wrapping_add(REQUEST), needed)?;
    g.set_u32(state.wrapping_add(ALIGNMENT), minimum)?;
    g.set_u32(state.wrapping_add(CURSOR), 0)?;
    g.set_u32(state.wrapping_add(WRITTEN), 0)?;
    g.set_u32(state.wrapping_add(CHANNELS), channels)?;
    g.set_u32(state.wrapping_add(STRIDE), capacity)?;
    g.set_u32(state.wrapping_add(CAPACITY), capacity)?;
    g.set_u32(state.wrapping_add(BUFFER), buffer)?;
    Ok(true)
}

/// `sub_82B3D870`: release the history allocation and clear its four lifetime words.
pub fn release<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, state: u32) -> Result<()> {
    let buffer = g.u32(state.wrapping_add(BUFFER))?;
    if buffer != 0 {
        heap.free(g, buffer)?;
    }
    for offset in [BUFFER, 4, 8, REQUEST] {
        g.set_u32(state.wrapping_add(offset), 0)?;
    }
    Ok(())
}

/// `sub_82B3D668`: enlarge a live history buffer while retaining its circular contents.
///
/// A smaller request merely changes [`REQUEST`]. A larger one uses the title's exact pointer
/// arithmetic to unfold each old channel ring into the replacement allocation, then folds its
/// current cursor prefix back to the new channel origin. The source intentionally does not clear
/// the untouched tail of a fresh allocation.
pub fn resize<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    state: u32,
    requested: u32,
) -> Result<bool> {
    if g.u32(state.wrapping_add(BUFFER))? == 0 {
        return allocate(
            g,
            heap,
            state,
            g.u32(state.wrapping_add(CHANNELS))?,
            requested,
            g.u32(state.wrapping_add(ALIGNMENT))?,
        );
    }

    let cursor = g.u32(state.wrapping_add(CURSOR))?;
    let capacity = requested.wrapping_add(32) & !31;
    let new_capacity = capacity.wrapping_add(cursor);
    if (g.u32(state.wrapping_add(STRIDE))? as i32) >= new_capacity as i32 {
        g.set_u32(state.wrapping_add(REQUEST), requested)?;
        return Ok(true);
    }

    let channels = g.u32(state.wrapping_add(CHANNELS))?;
    let new_buffer = heap.alloc(g, new_capacity.wrapping_mul(channels).wrapping_mul(4), 128)?;
    if new_buffer == 0 {
        return Ok(false);
    }

    let old_buffer = g.u32(state.wrapping_add(BUFFER))?;
    let old_stride = g.u32(state.wrapping_add(STRIDE))?;
    let written = g.u32(state.wrapping_add(WRITTEN))?;
    let history = g.u32(state.wrapping_add(HISTORY))?;
    let new_stride_bytes = new_capacity.wrapping_mul(4);
    for channel in 0..channels {
        let old_base = old_buffer.wrapping_add(old_stride.wrapping_mul(channel).wrapping_mul(4));
        // `divw` is signed. A zero old stride traps in the title; a constructed state always has
        // a positive stride, so report it as an addressable malformed state instead of dividing.
        if old_stride == 0 {
            return Err(crate::Error::new(
                state + STRIDE,
                "Del0 history stride is zero",
            ));
        }
        let quotient = (written as i32).wrapping_div(old_stride as i32) as u32;
        let remainder = written.wrapping_sub(quotient.wrapping_mul(old_stride));
        let r6 = old_base.wrapping_add(remainder.wrapping_add(cursor).wrapping_mul(4));
        let mut source = r6.wrapping_sub(history.wrapping_mul(4));
        let old_end = old_base.wrapping_add(old_stride.wrapping_mul(4));
        let old_after_cursor = old_end.wrapping_sub(cursor.wrapping_mul(4));
        // `blt` then `bgt` skips this add for a source inside `[old_base, old_end)`. It wraps
        // only sources outside that interval back across the old channel ring.
        if source < old_base || old_end <= source {
            source = source.wrapping_add(old_stride.wrapping_sub(cursor).wrapping_mul(4));
        }
        let destination_at_cursor = new_buffer
            .wrapping_add(new_capacity.wrapping_sub(cursor).wrapping_mul(4))
            .wrapping_add(new_stride_bytes.wrapping_mul(channel));
        let old_to_source = old_after_cursor.wrapping_sub(source);
        let mut first_samples = (old_to_source as i32 >> 2) as u32;
        if (history as i32) < first_samples as i32 {
            first_samples = history;
        }
        // `r27 = r28 - (history << 2)`: retain the history window immediately before the
        // replacement ring's cursor. `written` selects the source read position above, but it
        // does not enter the destination address.
        let destination = destination_at_cursor.wrapping_sub(history.wrapping_mul(4));
        mem::memcpy(
            g,
            destination,
            source,
            u64::from(first_samples.wrapping_mul(4)),
        )?;
        let remaining = history.wrapping_sub(first_samples);
        mem::memcpy(
            g,
            destination.wrapping_add(first_samples.wrapping_mul(4)),
            old_base,
            u64::from(remaining.wrapping_mul(4)),
        )?;
        let channel_destination = new_buffer.wrapping_add(new_stride_bytes.wrapping_mul(channel));
        mem::memcpy(
            g,
            channel_destination,
            destination_at_cursor,
            u64::from(cursor.wrapping_mul(4)),
        )?;
    }
    heap.free(g, old_buffer)?;
    g.set_u32(state.wrapping_add(BUFFER), new_buffer)?;
    g.set_u32(state.wrapping_add(STRIDE), new_capacity)?;
    g.set_u32(state.wrapping_add(WRITTEN), cursor)?;
    g.set_u32(state.wrapping_add(REQUEST), requested)?;
    Ok(true)
}

/// Render one stable `Del0` block from the owner's source planes into its destination planes.
///
/// This is the non-transition path of `sub_82B3D8E0`: write each input sample into the
/// channel-major history ring, read the requested delayed sample, advance the shared cursor, then
/// exchange the owner's source/destination descriptors. The title's separate transition helpers
/// (`sub_82B3D4F8` / `sub_82B3D578`) still govern fade changes and are intentionally kept outside
/// this stable path until their callback dispatcher is ported.
pub fn process_stable(g: &mut Guest, state: u32, owner: u32, channels: u32) -> Result<u64> {
    let buffer = g.u32(state.wrapping_add(BUFFER))?;
    let stride = g.u32(state.wrapping_add(STRIDE))?;
    if stride == 0 {
        return Err(crate::Error::new(
            state.wrapping_add(STRIDE),
            "Del0 history stride is zero",
        ));
    }
    let requested = g.u32(state.wrapping_add(REQUEST))?;
    let written = g.u32(state.wrapping_add(WRITTEN))? % stride;
    let history = g.u32(state.wrapping_add(HISTORY))?.min(stride);
    let source = g.u32(owner.wrapping_add(OWNER_SOURCE))?;
    let destination = g.u32(owner.wrapping_add(OWNER_DESTINATION))?;
    let source_data = g.u32(source.wrapping_add(DESCRIPTOR_DATA))?;
    let destination_data = g.u32(destination.wrapping_add(DESCRIPTOR_DATA))?;
    let source_stride = u32::from(g.u16(source.wrapping_add(DESCRIPTOR_STRIDE))?);
    let destination_stride = u32::from(g.u16(destination.wrapping_add(DESCRIPTOR_STRIDE))?);
    let delay = requested % stride;

    for channel in 0..channels {
        let history_base = buffer.wrapping_add(stride.wrapping_mul(channel).wrapping_mul(4));
        let input_base =
            source_data.wrapping_add(source_stride.wrapping_mul(channel).wrapping_mul(4));
        let output_base =
            destination_data.wrapping_add(destination_stride.wrapping_mul(channel).wrapping_mul(4));
        let mut position = written;
        let mut available = history;
        for frame in 0..BLOCK_FRAMES {
            let input = fp::load_single(g, input_base.wrapping_add(frame.wrapping_mul(4)))?;
            let output = if delay == 0 {
                input
            } else if available < delay {
                0.0
            } else {
                let read = position.wrapping_add(stride).wrapping_sub(delay) % stride;
                fp::load_single(g, history_base.wrapping_add(read.wrapping_mul(4)))?
            };
            fp::store_single(g, output_base.wrapping_add(frame.wrapping_mul(4)), output)?;
            fp::store_single(
                g,
                history_base.wrapping_add(position.wrapping_mul(4)),
                input,
            )?;
            position = position.wrapping_add(1) % stride;
            available = available.saturating_add(1).min(stride);
        }
    }

    let written = written.wrapping_add(BLOCK_FRAMES) % stride;
    g.set_u32(state.wrapping_add(WRITTEN), written)?;
    g.set_u32(
        state.wrapping_add(HISTORY),
        history.saturating_add(BLOCK_FRAMES).min(stride),
    )?;
    g.set_u32(
        state.wrapping_add(CAPACITY),
        g.u32(state.wrapping_add(CAPACITY))?
            .saturating_add(BLOCK_FRAMES)
            .min(stride),
    )?;
    g.set_u32(owner.wrapping_add(OWNER_SOURCE), destination)?;
    g.set_u32(owner.wrapping_add(OWNER_DESTINATION), source)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::BumpHeap;

    const BASE: u32 = 0x4000_0000;
    const STATE: u32 = BASE + 0x100;
    const OWNER: u32 = BASE + 0x400;
    const SOURCE_DESC: u32 = BASE + 0x500;
    const DEST_DESC: u32 = BASE + 0x540;
    const SOURCE_DATA: u32 = BASE + 0x2000;
    const DEST_DATA: u32 = BASE + 0x2800;

    fn heap() -> BumpHeap {
        BumpHeap {
            next: BASE + 0x1000,
            end: BASE + 0x4000,
        }
    }

    #[test]
    fn allocation_uses_the_titles_two_independent_32_byte_rounds() {
        let mut g = Guest::single(BASE, 0x5000);
        let mut heap = heap();
        assert!(allocate(&mut g, &mut heap, STATE, 2, 300, 256).unwrap());
        // max(300, 256 + 255) = 511; `(511 + 32) & !31` is 512, and
        // `(256 + 30) & !31` is 256. The two-channel allocation is 768 * 2 * 4.
        assert_eq!(g.u32(STATE + REQUEST).unwrap(), 511);
        assert_eq!(g.u32(STATE + STRIDE).unwrap(), 768);
        assert_eq!(g.u32(STATE + CAPACITY).unwrap(), 768);
        assert_eq!(g.u32(STATE + ALIGNMENT).unwrap(), 256);
        assert_eq!(g.u32(STATE + CHANNELS).unwrap(), 2);
        assert_eq!(g.u32(STATE + BUFFER).unwrap(), BASE + 0x1000);
        assert_eq!(heap.next, BASE + 0x1000 + 768 * 2 * 4);
    }

    #[test]
    fn release_frees_the_buffer_and_only_clears_its_lifetime_words() {
        let mut g = Guest::single(BASE, 0x5000);
        let mut heap = heap();
        allocate(&mut g, &mut heap, STATE, 1, 1, 1).unwrap();
        g.set_u32(STATE + 16, 77).unwrap();
        release(&mut g, &mut heap, STATE).unwrap();
        assert_eq!(
            [0, 4, 8, REQUEST].map(|offset| g.u32(STATE + offset).unwrap()),
            [0; 4]
        );
        assert_eq!(g.u32(STATE + 16).unwrap(), 77);
    }

    #[test]
    fn resize_reuses_a_sufficient_history_allocation() {
        let mut g = Guest::single(BASE, 0x5000);
        let mut heap = heap();
        allocate(&mut g, &mut heap, STATE, 1, 1, 1).unwrap();
        let buffer = g.u32(STATE + BUFFER).unwrap();
        let next = heap.next;

        assert!(resize(&mut g, &mut heap, STATE, 200).unwrap());

        assert_eq!(g.u32(STATE + BUFFER).unwrap(), buffer);
        assert_eq!(g.u32(STATE + STRIDE).unwrap(), 288);
        assert_eq!(g.u32(STATE + REQUEST).unwrap(), 200);
        assert_eq!(heap.next, next);
    }

    #[test]
    fn resize_places_the_retained_window_immediately_before_the_new_cursor() {
        let mut g = Guest::single(BASE, 0x5000);
        let mut heap = heap();
        allocate(&mut g, &mut heap, STATE, 1, 1, 1).unwrap();
        let old = g.u32(STATE + BUFFER).unwrap();
        for sample in 0..288 {
            g.set_u32(old + sample * 4, sample).unwrap();
        }
        g.set_u32(STATE + CURSOR, 4).unwrap();
        g.set_u32(STATE + HISTORY, 6).unwrap();
        g.set_u32(STATE + WRITTEN, 10).unwrap();

        assert!(resize(&mut g, &mut heap, STATE, 300).unwrap());

        let replacement = g.u32(STATE + BUFFER).unwrap();
        assert_ne!(replacement, old);
        assert_eq!(g.u32(STATE + STRIDE).unwrap(), 324);
        assert_eq!(g.u32(STATE + WRITTEN).unwrap(), 4);
        assert_eq!(g.u32(STATE + REQUEST).unwrap(), 300);
        assert_eq!(
            (0..6)
                .map(|offset| g.u32(replacement + (314 + offset) * 4).unwrap())
                .collect::<Vec<_>>(),
            vec![8, 9, 10, 11, 12, 13]
        );
    }

    #[test]
    fn stable_process_delays_a_planar_block_and_swaps_the_owner_pair() {
        let mut g = Guest::single(BASE, 0x5000);
        let mut heap = heap();
        allocate(&mut g, &mut heap, STATE, 1, 1, 1).unwrap();
        g.set_u32(STATE + REQUEST, 2).unwrap();
        g.set_u32(OWNER + OWNER_SOURCE, SOURCE_DESC).unwrap();
        g.set_u32(OWNER + OWNER_DESTINATION, DEST_DESC).unwrap();
        g.set_u32(SOURCE_DESC + DESCRIPTOR_DATA, SOURCE_DATA)
            .unwrap();
        g.set_u16(SOURCE_DESC + DESCRIPTOR_STRIDE, 256).unwrap();
        g.set_u32(DEST_DESC + DESCRIPTOR_DATA, DEST_DATA).unwrap();
        g.set_u16(DEST_DESC + DESCRIPTOR_STRIDE, 256).unwrap();
        for frame in 0..BLOCK_FRAMES {
            g.set_u32(SOURCE_DATA + frame * 4, (frame as f32).to_bits())
                .unwrap();
        }

        assert_eq!(process_stable(&mut g, STATE, OWNER, 1).unwrap(), 1);

        assert_eq!(g.f32(DEST_DATA).unwrap(), 0.0);
        assert_eq!(g.f32(DEST_DATA + 4).unwrap(), 0.0);
        assert_eq!(g.f32(DEST_DATA + 8).unwrap(), 0.0);
        assert_eq!(g.f32(DEST_DATA + 3 * 4).unwrap(), 1.0);
        assert_eq!(g.u32(STATE + WRITTEN).unwrap(), 256);
        assert_eq!(g.u32(STATE + HISTORY).unwrap(), 256);
        assert_eq!(g.u32(OWNER + OWNER_SOURCE).unwrap(), DEST_DESC);
        assert_eq!(g.u32(OWNER + OWNER_DESTINATION).unwrap(), SOURCE_DESC);
    }
}
