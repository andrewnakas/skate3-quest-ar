//! `sub_82B46B30` — six planar float channels into 24-byte frames, with a channel remap.
//!
//! Ported from `recomp/src/audio_ports/sub_82B46B30.inc`, **STATUS: verified** — 54,900 calls in a
//! played session on `RwAudioCore Dac`, compared against the original under the shadow harness.
//!
//! Replayed against **600 recorded calls, 0 disagreements**, and compared live against the original
//! 14,338 times in the same session.
//!
//! The last stage before the mixer hands a block to the driver: six separate plane buffers become
//! one run of interleaved frames. Three things about it are not what the name suggests.
//!
//! **It is a remap, not a straight interleave.** Plane *p* does not land at slot *p*. The six store
//! offsets, read off the instructions and identical in both loops, are:
//!
//! | plane | 0 | 1 | 2 | 3 | 4 | 5 |
//! |---|---|---|---|---|---|---|
//! | byte slot in the frame | +0 | +8 | +4 | +16 | +20 | +12 |
//!
//! and the *store order* is plane 1, 2, 4, 3, 0, 5 — which matters only if the source aliases the
//! destination, and is kept for that reason. [`LANES`] is that order.
//!
//! **The length is a literal, not a field.** `addi r4,r5,1024` adds 1024 bytes to the cursor's own
//! register, so every call that gets past the first compare writes exactly 256 frames — 6,144
//! bytes — whatever the descriptor says. That constant is what makes the write set enumerable from
//! entry state, and it is why 30 flagged stores collapse to one span.
//!
//! **The guard is a 32-bit compare of 64-bit values.** `start` and `start + 1024` are compared on
//! their low words, so the function returns having written nothing only when the addition carries
//! out of 32 bits. [`the_low_word_compare_is_the_only_early_out`] builds that case.
//!
//! The tail loop is **unreachable** for a 1024-byte span: the unrolled loop's trip count,
//! `(span - 13) / 16 + 1`, is 64, and 64 × 4 frames consumes all 256. It is transcribed anyway, as
//! the C++ port does, so the body stays faithful if the literal ever changes.
//!
//! One thing the round trip does *not* buy: the C++ note says the `lfs`/`stfs` pair quiets a
//! signalling NaN the way the lifted form does. That is true only unoptimised — LLVM folds
//! `fptrunc(fpext(x))` away, and clang can do the same to the recomp (`examples/flush_probe`, and
//! the note in [`crate::vmx::Fpscr`]). The pair is written here because it is the shape of the
//! original, and no claim is made about what either language does to a NaN.

use crate::{Guest, Result, fp};

/// `lwz r10,4(r4)` — the planar buffer's base.
pub const PLANE_BASE: u32 = 4;
/// `lhz r11,14(r4)` — floats per plane, i.e. the plane stride over four.
pub const PLANE_FRAMES: u32 = 14;
/// `addi r4,r5,1024` — the span, as a literal.
pub const SPAN_BYTES: u32 = 1024;
/// Six floats out per input frame.
pub const FRAME_BYTES: u32 = 24;
/// The planes, and the frame slots they land in: `(plane, byte slot)`, **in store order**.
pub const LANES: [(u32, u32); 6] = [(1, 8), (2, 4), (4, 20), (3, 16), (0, 0), (5, 12)];
/// How many planes there are.
pub const PLANES: u32 = 6;

/// One output frame: six `lfs`/`stfs` pairs, in the original's order.
fn frame(g: &mut Guest, planes: &[u64; 6], element: u32, out: u32) -> Result<()> {
    for (plane, slot) in LANES {
        // lfs f,4k(rp) -- the plane pointer is 64-bit and truncates at the access
        let value = fp::load_single(g, (planes[plane as usize] as u32).wrapping_add(4 * element))?;
        fp::store_single(g, out.wrapping_add(slot), value)?; // stfs f,slot(r3)
    }
    Ok(())
}

/// Interleave 256 frames of six planes into `out`, returning the cursor the original leaves in `r3`.
///
/// `out` is the guest's `r3` at full width and `desc` its `r4`. The returned value is `r3` on exit:
/// the cursor the unrolled loop advanced, which the tail loop reads but never moves.
pub fn interleave_six(g: &mut Guest, out: u64, desc: u32) -> Result<u64> {
    let frames_per_plane = u64::from(g.u16(desc + PLANE_FRAMES)?); // lhz r11,14(r4)
    let buffer = u64::from(g.u32(desc + PLANE_BASE)?); // lwz r10,4(r4)

    // Six rotates and two `rlwinm`s of a zero-extended halfword, which are plain shifts at these
    // magnitudes: the plane pointers are `buffer + 4 * frames * p`, each a 64-bit add of two
    // zero-extended words. Kept 64-bit, cast only at the access.
    let mut planes = [0u64; 6];
    for (p, slot) in planes.iter_mut().enumerate() {
        *slot = buffer + 4 * frames_per_plane * p as u64;
    }

    // add r5,r9,r10 -- the cursor starts at plane 2, which is also what the limit is measured from.
    let start = planes[2];
    let limit = start as i64 + i64::from(SPAN_BYTES);
    // cmplw cr6,r5,r4 ; bge -- a LOW-WORD compare, so this fires only on a carry out of 32 bits.
    if start as u32 >= limit as u32 {
        return Ok(out); // nothing at all is written on that path
    }

    // subf ; addi 3 ; srawi 2 ; addze -- the floats in the span, rounded up: 256 on every call.
    let span = limit as u64 - start;
    let rounded = (span as u32).wrapping_add(3) as i32;
    let carry = i64::from(rounded < 0 && (rounded as u32) & 3 != 0);
    let elements = i64::from(rounded >> 2) + carry;

    let mut out = out as i64;
    let mut element = 0u32;
    // cmpwi cr6,r5,4 ; blt -- fewer than four frames skips the unrolled loop entirely.
    if elements as u32 as i32 >= 4 {
        // (span - 13) / 16 + 1 trips of four frames: 64 for a 1024-byte span, which consumes it.
        let adjusted = span as i64 - 13;
        let trips = u64::from(adjusted as u32 >> 4) + 1;
        for _ in 0..trips {
            for k in 0..4u32 {
                frame(
                    g,
                    &planes,
                    element + k,
                    (out as u32).wrapping_add(FRAME_BYTES * k),
                )?;
            }
            element += 4; // all six plane pointers advance by 16
            out += 4 * i64::from(FRAME_BYTES); // addi r3,r3,96
        }
    }
    // The tail loop reads this cursor (addi r11,r3,-12) and never advances it, so this is the value
    // the original returns on every path.
    let returned = out as u64;

    // cmplw cr6,r11,r4 -- plane 2's cursor against the limit, low words again.
    let cursor = start + 4 * u64::from(element);
    if cursor as u32 >= limit as u32 {
        return Ok(returned);
    }

    // One frame per trip over whatever is left. Unreachable at 1024 bytes; transcribed anyway.
    let left = (limit as u64 - cursor) as i64 - 1;
    let tail_trips = u64::from(left as u32 >> 2) + 1;
    let mut tail = out as u32;
    for _ in 0..tail_trips {
        frame(g, &planes, element, tail)?;
        element += 1;
        tail = tail.wrapping_add(FRAME_BYTES);
    }

    Ok(returned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const DESC: u32 = BASE + 0x20;
    const PLANES_AT: u32 = BASE + 0x1000;
    const OUT: u32 = BASE + 0x8000;
    const FRAMES_PER_PLANE: u32 = 300; // deliberately not 256, so the stride is visible
    const POISON: u32 = 0xDEAD_BEEF;

    /// Plane `p`, frame `f` holds `p * 1000 + f`, so a swapped plane or a wrong stride is a wrong
    /// value rather than a plausible one.
    fn value(plane: u32, frame_index: u32) -> f32 {
        (plane * 1000 + frame_index) as f32
    }

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x1_0000);
        g.set_u32(DESC + PLANE_BASE, PLANES_AT).unwrap();
        g.set_u16(DESC + PLANE_FRAMES, FRAMES_PER_PLANE as u16)
            .unwrap();
        for plane in 0..PLANES {
            for f in 0..FRAMES_PER_PLANE {
                let at = PLANES_AT + 4 * (plane * FRAMES_PER_PLANE + f);
                g.set_u32(at, value(plane, f).to_bits()).unwrap();
            }
        }
        // Poison the output, one frame past the 256 the call should write.
        for i in 0..(257 * FRAME_BYTES) / 4 {
            g.set_u32(OUT + i * 4, POISON).unwrap();
        }
        g
    }

    fn slot_of(plane: u32) -> u32 {
        LANES.iter().find(|(p, _)| *p == plane).unwrap().1
    }

    #[test]
    fn every_plane_lands_in_its_remapped_slot() {
        let mut g = guest();
        let end = interleave_six(&mut g, u64::from(OUT), DESC).unwrap();

        assert_eq!(
            end,
            u64::from(OUT) + 256 * u64::from(FRAME_BYTES),
            "the cursor advanced 6,144"
        );
        for f in [0u32, 1, 5, 128, 255] {
            for plane in 0..PLANES {
                let at = OUT + f * FRAME_BYTES + slot_of(plane);
                assert_eq!(
                    g.f32(at).unwrap(),
                    value(plane, f),
                    "frame {f}, plane {plane}"
                );
            }
        }
    }

    #[test]
    fn the_remap_is_not_a_straight_interleave() {
        // The distinguishing check: plane 1 at +8 and plane 2 at +4, not the other way round; and
        // planes 3 and 5 swapped relative to their slot order.
        assert_eq!((slot_of(0), slot_of(1), slot_of(2)), (0, 8, 4));
        assert_eq!((slot_of(3), slot_of(4), slot_of(5)), (16, 20, 12));
        // Which means a straight interleave would disagree on five of the six lanes: only plane 0
        // coincides, at +0.
        let straight: Vec<u32> = (0..PLANES).map(|p| 4 * p).collect();
        let actual: Vec<u32> = (0..PLANES).map(slot_of).collect();
        assert_eq!(
            straight
                .iter()
                .zip(&actual)
                .filter(|(a, b)| *a != *b)
                .count(),
            5
        );
        assert_eq!(straight[0], actual[0], "plane 0 is the only agreement");

        // And the values really follow it: plane 1's first sample is at +8 of the first frame.
        let mut g = guest();
        interleave_six(&mut g, u64::from(OUT), DESC).unwrap();
        assert_eq!(g.f32(OUT + 8).unwrap(), value(1, 0));
        assert_eq!(g.f32(OUT + 4).unwrap(), value(2, 0));
    }

    #[test]
    fn it_writes_exactly_256_frames_whatever_the_descriptor_says() {
        // The length is the literal 1024 bytes of plane stride, not `frames_per_plane`. A plane of
        // 300 frames still produces 256 output frames, and the 257th is untouched.
        let mut g = guest();
        interleave_six(&mut g, u64::from(OUT), DESC).unwrap();
        for word in 0..FRAME_BYTES / 4 {
            assert_eq!(
                g.u32(OUT + 256 * FRAME_BYTES + word * 4).unwrap(),
                POISON,
                "word {word} of frame 256"
            );
        }
    }

    #[test]
    fn the_plane_stride_comes_from_the_descriptor() {
        // Halve the stride and every plane but the first reads from a different place, so the same
        // output slot holds a different value. This is what a hardcoded 256-frame stride would fail.
        let mut g = guest();
        g.set_u16(DESC + PLANE_FRAMES, 256).unwrap();
        interleave_six(&mut g, u64::from(OUT), DESC).unwrap();
        // Plane 1 now starts 1,024 bytes in, which is frame 256 of the 300-frame layout.
        assert_eq!(g.f32(OUT + slot_of(1)).unwrap(), value(0, 256));
        // Plane 0 is unaffected: it starts at the buffer base either way.
        assert_eq!(g.f32(OUT + slot_of(0)).unwrap(), value(0, 0));
    }

    #[test]
    fn the_low_word_compare_is_the_only_early_out() {
        // `start` and `start + 1024` are compared on their low words, so the body returns having
        // written nothing exactly when that addition carries out of 32 bits. Plane 2's pointer is
        // the cursor, and with a stride of zero it is the buffer base itself.
        let mut g = Guest::single(BASE, 0x200);
        g.set_u32(DESC + PLANE_BASE, 0xFFFF_FC00).unwrap();
        g.set_u16(DESC + PLANE_FRAMES, 0).unwrap();

        // No plane memory is mapped at all, which is itself the assertion: nothing is read either.
        let end = interleave_six(&mut g, u64::from(OUT), DESC).unwrap();
        assert_eq!(end, u64::from(OUT), "the cursor did not move");

        // And one byte lower it does not fire, so the guard is the carry and not the address.
        g.set_u32(DESC + PLANE_BASE, 0xFFFF_FBFF).unwrap();
        assert!(
            interleave_six(&mut g, u64::from(OUT), DESC).is_err(),
            "without the carry it runs, and then the unmapped plane is an error"
        );
    }

    #[test]
    fn the_field_offsets_and_the_span_are_the_lifted_ones() {
        assert_eq!((PLANE_BASE, PLANE_FRAMES), (4, 14));
        assert_eq!(SPAN_BYTES, 1024);
        assert_eq!(FRAME_BYTES, 24);
        assert_eq!(
            SPAN_BYTES / 4 * FRAME_BYTES,
            6144,
            "256 frames of six floats"
        );
        // The unrolled trip count the original computes, for the span it always gets.
        assert_eq!(((1024u32 - 13) >> 4) + 1, 64);
        assert_eq!(
            64 * 4,
            256,
            "and it consumes the span exactly, leaving no tail"
        );
    }
}
