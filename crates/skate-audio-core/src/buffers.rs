//! `rwaudio_InitBufferPair`: record two buffers with their lengths and zero them.
//!
//! Ported from `sub_82B7F828`, shadow-verified over 226 comparable calls at zero divergence.
//! PLAN section 6 does not name a module for this; it is filed here rather than folded into
//! `system.rs`, because it belongs to the measure/allocate split described in
//! `docs/buffer-size-bug.md` alongside `sub_82B7F998`, not to the command ring.

use crate::{Guest, Result};

/// The object's written span is +0x00..+0x27 inclusive: two pointer/length pairs and six words
/// cleared between them.
pub const PAIR_FIRST: u32 = 0x00;
pub const PAIR_FIRST_LEN: u32 = 0x08;
pub const PAIR_SECOND: u32 = 0x14;
pub const PAIR_SECOND_LEN: u32 = 0x1C;
pub const PAIR_SPAN: u32 = 40;

const _: () = assert!(PAIR_FIRST_LEN == 0x08, "first length");
const _: () = assert!(PAIR_SECOND == 0x14, "second pointer");
const _: () = assert!(PAIR_SECOND_LEN == 0x1C, "second length");

/// `sub_82B7F828`. Returns the object it initialised, as the original does.
///
/// Argument order follows the guest's: `second`/`second_len` arrive in r4/r5 and
/// `first`/`first_len` in r6/r7, which reads oddly and is kept because that is what the
/// recorded vectors carry.
///
/// Store order is the original's: the first pair and its three cleared words, then the first
/// fill, then the second pair and its three cleared words, then the second fill. The order is
/// only observable if a buffer overlaps the object, which is why it is preserved rather than
/// tidied.
///
/// **A guarded unknown.** The original zeroes through the guest's own `memset`
/// (`sub_82F52040`), whose behaviour at length 0 is not obvious from its alignment preamble, and
/// no observed call has length 0 — measured lengths are 192 and 196. A zero length here fills
/// nothing, which is the reasonable reading but is **not** verified against the guest.
pub fn init_buffer_pair(
    g: &mut Guest,
    object: u32,
    second: u32,
    second_len: u32,
    first: u32,
    first_len: u32,
) -> Result<u32> {
    g.set_u32(object + PAIR_FIRST, first)?;
    g.set_u32(object + PAIR_FIRST_LEN, first_len)?;
    g.set_u32(object + 0x04, 0)?;
    g.set_u32(object + 0x0C, 0)?;
    g.set_u32(object + 0x10, 0)?;
    if first != 0 {
        g.fill(first, 0, first_len)?;
    }

    g.set_u32(object + PAIR_SECOND, second)?;
    g.set_u32(object + PAIR_SECOND_LEN, second_len)?;
    g.set_u32(object + 0x18, 0)?;
    g.set_u32(object + 0x20, 0)?;
    g.set_u32(object + 0x24, 0)?;
    if second != 0 {
        g.fill(second, 0, second_len)?;
    }
    Ok(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECT: u32 = 0x7018_E110;
    const FIRST: u32 = 0x4017_3794;
    const SECOND: u32 = 0x4017_36D0;

    /// The spans a real recorded vector carries, at their real addresses: the object on the
    /// guest stack and the buffers on the heap, 768 MB apart.
    fn guest() -> Guest {
        let mut g = Guest::default();
        g.put(OBJECT, vec![0xAAu8; PAIR_SPAN as usize]);
        g.put(FIRST, vec![0xBBu8; 192]);
        g.put(SECOND, vec![0xCCu8; 196]);
        g
    }

    #[test]
    fn writes_the_ten_words_and_zeroes_both_buffers() {
        let mut g = guest();
        assert_eq!(
            init_buffer_pair(&mut g, OBJECT, SECOND, 196, FIRST, 192).unwrap(),
            OBJECT
        );

        assert_eq!(g.u32(OBJECT + PAIR_FIRST).unwrap(), FIRST);
        assert_eq!(g.u32(OBJECT + PAIR_FIRST_LEN).unwrap(), 192);
        assert_eq!(g.u32(OBJECT + PAIR_SECOND).unwrap(), SECOND);
        assert_eq!(g.u32(OBJECT + PAIR_SECOND_LEN).unwrap(), 196);
        for off in [0x04u32, 0x0C, 0x10, 0x18, 0x20, 0x24] {
            assert_eq!(
                g.u32(OBJECT + off).unwrap(),
                0,
                "word at +{off:#x} must be cleared"
            );
        }
        assert!(
            g.span(FIRST, 192).unwrap().iter().all(|&b| b == 0),
            "first buffer zeroed"
        );
        assert!(
            g.span(SECOND, 196).unwrap().iter().all(|&b| b == 0),
            "second buffer zeroed"
        );
    }

    #[test]
    fn a_null_buffer_is_recorded_but_not_filled() {
        let mut g = guest();
        init_buffer_pair(&mut g, OBJECT, 0, 0, FIRST, 192).unwrap();
        assert_eq!(g.u32(OBJECT + PAIR_SECOND).unwrap(), 0);
        assert_eq!(g.u32(OBJECT + PAIR_SECOND_LEN).unwrap(), 0);
        // The second buffer's memory is untouched, because the original skips the fill.
        assert!(g.span(SECOND, 196).unwrap().iter().all(|&b| b == 0xCC));
    }
}
