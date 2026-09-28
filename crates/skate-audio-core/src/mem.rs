//! The XDK block routines the ported bodies call out to, as guest functions.
//!
//! `sub_82EDF460` (memcpy), `sub_82EE5E80` (memset) and `sub_82F52040` (a **second** memset entry
//! point) are not audio functions, have no `.inc` of their own, and are not ported here as
//! instruction streams. What every verified port that calls one of them relies on — and states in its
//! `Windows()` comment — is a claim about their **write set**, established by reading their lifted
//! bodies through rather than assumed:
//!
//! > each writes exactly `[dst, dst + n)` and nothing else, on every path — the `dcbz`/`stvlx`/
//! > `stvrx` forms round into that interval and are then fully overwritten, and their spills go
//! > below their own `r1`.
//!
//! That is the whole of what the callers depend on, so that is what this module provides. It is a
//! narrower claim than "these are ports of `sub_82EDF460` and `sub_82EE5E80`", and the difference
//! matters in exactly one place, named below.
//!
//! ## What is not reproduced
//!
//! **Overlapping `memcpy` is not pinned.** [`memcpy`] snapshots the source before it writes, which
//! is what a non-overlapping copy produces and what the C-library contract permits an
//! implementation to do. The guest routine is a vectorised copy whose behaviour on an overlapping
//! span is a property of its load/store schedule, and nothing in this project has read that
//! schedule out. No call site here passes an overlapping pair: `sub_82B3DB90` copies ring → output
//! buffer and `sub_82B34E08` copies accumulator → row, and both of their `Windows()` builders
//! refuse the layouts where those could coincide. If a future caller can overlap, this is the line
//! to come back to.
//!
//! **A zero length writes nothing and never touches the address.** The guest routines test the
//! count first, so a null or unmapped `dst` with `n == 0` is not a fault there and is not one here.
//! Without that early exit [`crate::Guest`] would reject the address and turn a legal no-op into an
//! `Err`.
//!
//! A length that runs past the end of the guest map is an `Err` rather than a partial write, which
//! is [`crate::Guest`]'s convention everywhere: the harness's own window builders refuse those
//! calls as over-budget or unbounded, so nothing is known about what the guest does with them
//! either.

use crate::{Guest, Result};

/// `sub_82EDF460` — `memcpy(dst, src, len)`.
///
/// Writes `[dst, dst + len)`; reads `[src, src + len)`. Callers pass 64-bit registers and the
/// guest routine addresses with their low words, so both addresses arrive here already truncated
/// by the caller — that truncation is the caller's instruction, not this function's.
pub fn memcpy(g: &mut Guest, dst: u32, src: u32, len: u64) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    // Validate before allocating: a negative word count reaches these routines as a ~4 GB length
    // in the original, and `span` rejects it without trying to materialise it.
    let bytes = g.span(src, len as usize)?.to_vec();
    g.set_span(dst, &bytes)
}

/// `sub_82EE5E80` — `memset(dst, byte, len)`.
///
/// Writes `[dst, dst + len)`. Every call site in this crate passes `0`; the fill byte is a
/// parameter anyway because it is the guest's `r4` and folding it in would hide an argument the
/// replay has to supply.
pub fn memset(g: &mut Guest, dst: u32, byte: u8, len: u64) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    if len > u32::MAX as u64 {
        return Err(crate::Error::new(
            dst,
            "fill length does not fit a guest span",
        ));
    }
    g.fill(dst, byte, len as u32)
}

/// `sub_82F52040` — a **second** guest `memset`, at a different address, with the same contract.
///
/// A separate function rather than a call to [`memset`] spelled differently, so that a call site
/// naming this address is visibly naming *this* routine: the two are distinct guest bodies and nothing
/// in this project has established that they are the same code. What has been established is the write
/// set, and it is the only thing the callers depend on. `sub_82B38B68`'s `.inc` records it directly —
///
/// > `sub_82F52040` is memset (`skate3_recomp.106.cpp:64780`): it writes exactly `[r3, r3 + r5)`,
/// > nothing else, and `r5 == 0` writes nothing.
///
/// — and `sub_82B39FA0`'s `Windows()` adds the derivation: a byte-align prologue, then 16-byte blocks,
/// then `(len>>2)&3` words, then `len&3` bytes, which sums to exactly `len`.
///
/// Callers in this crate: [`crate::stage::run_stage`].
pub fn memset_82f52040(g: &mut Guest, dst: u32, byte: u8, len: u64) -> Result<()> {
    memset(g, dst, byte, len)
}

/// `sub_82F4DC60` — memmove: `len` bytes from `src` to `dst`, correct for any overlap.
///
/// Written as a copy through a temporary buffer, which is memmove's definition. The guest routine
/// branches on the direction and its forward case tail-calls a memcpy leaf; for the one caller ported
/// so far (`crate::voices::remove_handle`, which shifts an array down by one entry) `dst` is below
/// `src`, so a forward copy and a buffered copy leave the same bytes. The whole of `[dst, dst + len)`
/// is written and nothing else.
pub fn memmove(g: &mut Guest, dst: u32, src: u32, len: u64) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    let bytes = g.span(src, len as usize)?.to_vec();
    g.set_span(dst, &bytes)
}

// ---------------------------------------------------------------- sub_82F52FB8: the chunked copy

/// A forward copy in the original's chunks (`sub_82F52FB8`): single bytes until `dst` is
/// word-aligned, then whole words — one aligned load when `src` is aligned too, four byte loads
/// otherwise, every byte of a word read before any is stored — then the tail bytes.
///
/// `dst` is `r3`, which the original returns unchanged; `src` is `r4`; `len` is `r5` at full width,
/// because the alignment loop's counter is `len + 1` in 64 bits, tested on its low word. An
/// overlapping copy follows the chunking, not memmove's rules: the tests pin one.
pub fn memcpy_chunked(g: &mut Guest, dst: u32, src: u32, len: u64) -> Result<()> {
    let (mut d, mut s, mut len) = (dst, src, len);
    let mut ctr = len.wrapping_add(1); // addi r0,r5,1 ; mtctr r0
    loop {
        let aligned = d & 3 == 0; // andi. r0,r6,3
        ctr = ctr.wrapping_sub(1); // bdnzf eq,0x82f52fc8
        if ctr as u32 == 0 || aligned {
            break;
        }
        len = len.wrapping_sub(1); // addi r5,r5,-1
        let byte = g.u8(s)?;
        s = s.wrapping_add(1);
        g.set_u8(d, byte)?;
        d = d.wrapping_add(1);
    }
    let words = (len as u32) >> 2; // rlwinm. r0,r5,30,2,31
    let aligned_src = s & 3 == 0; // andi. r0,r4,3
    for _ in 0..words {
        let word = if aligned_src {
            g.u32(s)? // lwz r7,0(r4)
        } else {
            let b3 = u32::from(g.u8(s.wrapping_add(3))?);
            let b2 = u32::from(g.u8(s.wrapping_add(2))?);
            let b1 = u32::from(g.u8(s.wrapping_add(1))?);
            let b0 = u32::from(g.u8(s)?);
            (b0 << 24) | (b1 << 16) | (b2 << 8) | b3 // lbz x4 ; rlwimi x3
        };
        s = s.wrapping_add(4);
        g.set_u32(d, word)?; // stw r7,0(r6)
        d = d.wrapping_add(4);
    }
    for _ in 0..(len as u32 & 3) {
        let byte = g.u8(s)?;
        s = s.wrapping_add(1);
        g.set_u8(d, byte)?;
        d = d.wrapping_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;

    fn guest() -> Guest {
        Guest::single(BASE, 0x400)
    }

    #[test]
    fn a_copy_moves_exactly_the_requested_span_and_no_more() {
        let mut g = guest();
        for i in 0..16u32 {
            g.set_u8(BASE + i, 0x10 + i as u8).unwrap();
        }
        g.set_u8(BASE + 0x7F, 0xAA).unwrap();
        g.set_u8(BASE + 0x90, 0xBB).unwrap();
        memcpy(&mut g, BASE + 0x80, BASE, 16).unwrap();
        for i in 0..16u32 {
            assert_eq!(g.u8(BASE + 0x80 + i).unwrap(), 0x10 + i as u8);
        }
        assert_eq!(
            g.u8(BASE + 0x7F).unwrap(),
            0xAA,
            "one byte below the destination"
        );
        assert_eq!(g.u8(BASE + 0x90).unwrap(), 0xBB, "one byte above it");
    }

    #[test]
    fn a_zero_length_never_touches_the_address() {
        // The guest routines test the count first, so `dst` is not dereferenced at all. An
        // unmapped destination has to be legal here or a lawful no-op becomes an Err.
        let mut g = guest();
        memcpy(&mut g, 0, 0, 0).unwrap();
        memset(&mut g, 0, 0, 0).unwrap();
        // And a mapped one is left alone.
        g.set_u32(BASE, 0x1234_5678).unwrap();
        memset(&mut g, BASE, 0xFF, 0).unwrap();
        assert_eq!(g.u32(BASE).unwrap(), 0x1234_5678);
    }

    #[test]
    fn a_length_past_the_map_is_an_error_and_writes_nothing() {
        // The ~4 GB length a negative word count produces in the original. `span` rejects it
        // before anything is allocated or written, which is what keeps the failure cheap.
        let mut g = guest();
        g.set_u32(BASE, 0xDEAD_BEEF).unwrap();
        assert!(memcpy(&mut g, BASE, BASE + 0x100, 0xFFFF_FFFC).is_err());
        assert!(memset(&mut g, BASE, 0, 0xFFFF_FFFC).is_err());
        assert_eq!(
            g.u32(BASE).unwrap(),
            0xDEAD_BEEF,
            "a refused call writes nothing"
        );
        // Past 32 bits is refused before it reaches the map at all.
        assert!(memset(&mut g, BASE, 0, 0x1_0000_0000).is_err());
    }

    #[test]
    fn a_fill_writes_the_byte_it_is_given_over_the_whole_span() {
        let mut g = guest();
        g.set_u8(BASE + 0x0F, 0x11).unwrap();
        g.set_u8(BASE + 0x20, 0x22).unwrap();
        memset(&mut g, BASE + 0x10, 0x5A, 16).unwrap();
        for i in 0..16u32 {
            assert_eq!(g.u8(BASE + 0x10 + i).unwrap(), 0x5A);
        }
        assert_eq!(g.u8(BASE + 0x0F).unwrap(), 0x11);
        assert_eq!(g.u8(BASE + 0x20).unwrap(), 0x22);
    }
}

#[cfg(test)]
mod chunked_copy_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x100);
        for i in 0..0x40u32 {
            g.set_u8(BASE + i, i as u8 + 1).unwrap();
        }
        g
    }

    fn bytes(g: &Guest, at: u32, n: u32) -> Vec<u8> {
        (0..n).map(|i| g.u8(at + i).unwrap()).collect()
    }

    #[test]
    fn an_aligned_copy_moves_every_byte_and_no_more() {
        let mut g = guest();
        memcpy_chunked(&mut g, BASE + 0x80, BASE, 11).unwrap();
        assert_eq!(
            bytes(&g, BASE + 0x80, 12),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0]
        );
    }

    #[test]
    fn misaligned_ends_take_the_byte_paths() {
        let mut g = guest();
        memcpy_chunked(&mut g, BASE + 0x81, BASE + 2, 13).unwrap();
        assert_eq!(
            bytes(&g, BASE + 0x80, 15),
            [0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0]
        );
    }

    #[test]
    fn a_zero_length_copies_nothing_even_misaligned() {
        let mut g = guest();
        memcpy_chunked(&mut g, BASE + 0x81, BASE, 0).unwrap();
        assert_eq!(bytes(&g, BASE + 0x80, 4), [0, 0, 0, 0]);
    }

    #[test]
    fn an_overlapping_copy_follows_the_word_chunks_not_memmove() {
        // The second word is read after the first was stored over it, so the pattern repeats.
        let mut g = guest();
        memcpy_chunked(&mut g, BASE + 4, BASE, 8).unwrap();
        assert_eq!(bytes(&g, BASE, 12), [1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);
    }
}
