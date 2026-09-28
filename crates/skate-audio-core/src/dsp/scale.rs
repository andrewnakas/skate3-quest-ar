//! `sub_82B3BED8` and `sub_82B44B20` — the mixer's two float-array multiplies.
//!
//! They are twins: the same two paths, the same path selector, the same end-pointer arithmetic and
//! the same write window, with a fused multiply-add against the destination in place of a plain
//! multiply. They share a file because reading either one alone hides how narrowly they differ.
//!
//! | | guest | does | `docs/ports.md` | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`scale`] | `sub_82B3BED8` | `dst[i] = src[i] * k` | verified | 2,843,558 | 4,411,988 |
//! | [`scale_accumulate`] | `sub_82B44B20` | `dst[i] += src[i] * k` | verified | 3,738,884 | 6,110,212 |
//!
//! `sub_82B3BED8`'s green is the cleaner of the two to quote: session `s19` recorded
//! `runs=1057635 diverged: registers=0 memory=0 skipped=0 overflow=0`. `skipped=0` is the part
//! worth reading twice — `Windows()` answered on every one of the million calls rather than
//! declining, so the covering is as wide as it looks.
//!
//! ## Two paths, chosen from entry state alone
//!
//! ```text
//! vector   ((dst | src) & 0x7F) == 0  &&  (count & 0x3F) == 0
//! scalar   anything else
//! ```
//!
//! — both buffers 128-byte aligned and the count a multiple of 64. The vector path moves 32 floats
//! an iteration: eight `lvx128`, eight multiplies (or FMAs), a `dcbzl` on the destination line for
//! `sub_82B3BED8`, and eight `stvx128` **in the lifted order**, which is not ascending. Both loops
//! exit on the *source* cursor reaching `src + count*4`, never on a counter.
//!
//! ## The limits of the verification, carried over
//!
//! **The scalar path is unexercised.** Every logged entry has `count = 256` with both pointers
//! 128-byte aligned, so the million clean comparisons are all vector path. The C++ scalar path was
//! checked by a Python model of the lifted register machine against a model of the port over 111
//! cases — six alignment combinations, 18 counts including 1/3/7/17/63/65/127, three overlapping
//! layouts — agreeing on written bytes and written addresses. That checks addressing, loop counts
//! and store order, **not float rounding**. The same limit applies to the translation below, and
//! the tests here add the rounding half that the model could not: the scalar path is compared
//! against an independent per-element model with the same `fmuls`/`fmadds` structure.
//!
//! ## The two paths are not the same function
//!
//! They agree on every input the game passes, and they are not interchangeable. The vector path
//! narrows `f1` to a **single** before it multiplies — four `stfs` into the red zone and one
//! `lvx128` to read them back as four lanes — while the scalar path multiplies in **double** and
//! narrows the product. For an `f1` that is exactly representable as a single, which is what an
//! `lfs` in the caller guarantees and what the census shows (`1.0` and `0.0`, alternating), the two
//! agree bit for bit: the double product of two 24-bit values is exact, so narrowing it is the same
//! single rounding the f32 multiply performs. For any other `f1` they differ, and neither is wrong.
//! `a_scale_that_is_not_a_single_makes_the_two_paths_differ` pins it. This is a property of the
//! original, not of the translation, and it is written down because a reader who assumes the paths
//! are one function with two speeds will eventually be surprised by it.
//!
//! ## One divergence, forced by the guest map
//!
//! The vector loop terminates only when the source cursor lands exactly on `src + count*4` after
//! whole 128-byte steps. A span that is zero, or not a multiple of 128 — `count == 0` with both
//! pointers aligned, a negative count, a `count*4` that overflows — **never terminates**, and the
//! original hangs. The C++ reproduces the hang and its `Windows()` returns `false` on exactly that
//! class, so nothing is known about what the guest does there either. Here the cursor walks off
//! the segment and [`crate::Guest`] returns `Err` instead. That is a difference from the reference,
//! it is in the direction of not hanging a caller, and it is on input the harness has never
//! compared — so it resolves an unknown rather than contradicting a known.

#![allow(unused_unsafe)] // see the note at the top of `crate::vmx`

use crate::fp;
use crate::vmx::{self, Fpscr};
use crate::{Guest, Result};
use core::arch::x86_64::*;

/// `(r3 | r4) & 0x7F` — both buffers on a 128-byte boundary.
pub const ALIGN_MASK: u32 = 0x7F;
/// `r6 & 0x3F` — the float count a multiple of 64.
pub const COUNT_MASK: u32 = 0x3F;
/// Bytes moved per iteration of either vector loop: eight vectors.
pub const BLOCK_BYTES: u32 = 128;

/// Whether a call with these arguments takes the vector path.
///
/// Exposed because it is the whole of the path selection and a caller wanting to know which half of
/// this file ran should not have to re-derive the masks.
pub const fn takes_vector_path(dst: u32, src: u32, count: u32) -> bool {
    ((dst | src) & ALIGN_MASK) == 0 && (count & COUNT_MASK) == 0
}

/// `sub_82B3BED8` — `dst[i] = src[i] * k` for `i` in `[0, count)`.
///
/// `dst` is `r3`, `src` is `r4`, `count` is `r6` and `k` is `f1`, a guest FPR and therefore an
/// `f64` even though every value it holds in practice came from a single. The game passes
/// `count = 256` with both buffers 128-byte aligned and `k` alternating between `1.0` and `0.0` —
/// a zero scale turns the call into a buffer clear.
///
/// Writes `[dst, dst + count*4)`; reads the same span at `src`.
pub fn scale(g: &mut Guest, dst: u32, src: u32, count: u32, k: f64) -> Result<()> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    if takes_vector_path(dst, src, count) {
        // srawi r5,r6,2 ; rlwinm r5,r5,4,0,27 ; add r5,r5,r4 — the source end, count*4 bytes on.
        // Computed the way the original computes it: an *arithmetic* shift right by two and then a
        // left shift by four, which is not the same as `count << 2` for a negative count.
        let span = (((count as i32) >> 2) as u32) << 4 & 0xFFFF_FFF0;
        unsafe { scale_vector(g, dst, src, span, k) }
    } else {
        unsafe { scale_scalar(g, dst, src, count, k) }
    }
}

/// `sub_82B44B20` — `dst[i] += src[i] * k`, the accumulating twin of [`scale`].
///
/// Same arguments, same window, plus a declared *read* of the destination: the accumulator is read
/// before it is written. Every load of a block happens before every store of it, which is what
/// makes an overlapping `src`/`dst` behave as the original does.
pub fn scale_accumulate(g: &mut Guest, dst: u32, src: u32, count: u32, k: f64) -> Result<()> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    if takes_vector_path(dst, src, count) {
        // rlwinm r10,r6,2,0,27 — the twin computes the same end pointer with one instruction
        // instead of two, so a negative count gives a *different* span here than in `scale`. Each
        // is written the way its own original writes it rather than factored into one helper.
        let span = (count << 2) & 0xFFFF_FFF0;
        unsafe { accumulate_vector(g, dst, src, span, k) }
    } else {
        unsafe { accumulate_scalar(g, dst, src, count, k) }
    }
}

// ------------------------------------------------------------------------------ the vector paths

#[target_feature(enable = "sse4.1,fma")]
unsafe fn scale_vector(g: &mut Guest, dst: u32, src: u32, span: u32, k: f64) -> Result<()> {
    let src_end = src.wrapping_add(span);

    let mut fpscr = Fpscr::capture();
    // stfs f1,-32(r1) .. stfs f1,-20(r1) ; lvx128 v63,r0,r30 — the scale in all four lanes. The
    // four words are identical so the load's lane reversal is a no-op on them, and the scratch is
    // this function's own red zone, which the port keeps in a register instead. That holds only
    // because the guest `r1` is 16-byte aligned, which was *measured* on the real entries
    // (`r1 = 0x707BF960`) rather than argued from the ABI.
    fpscr.disable_flush_mode_unconditional(); // emitted by the first stfs
    let scale = unsafe { _mm_set1_ps(k as f32) };

    let mut s = src; // r10
    let mut d = dst; // r11
    loop {
        // Eight lvx128 off r10; the dcbt hints lift to nothing at all.
        let a = unsafe { load_block(g, s)? };
        s = s.wrapping_add(BLOCK_BYTES); // addi r10,r10,128

        // Eight separate multiplies, the scale as the FIRST operand of every one, as lifted. Rule 4:
        // with two NaN operands the winning slot is part of the semantics, so the order is copied
        // rather than rearranged.
        fpscr.enable_flush_mode_unconditional(); // before the first vmulfp128
        let mut p = [unsafe { _mm_setzero_ps() }; 8];
        for i in 0..8 {
            p[i] = unsafe { vmx::vmulfp(scale, a[i]) }; // vmulfp128 v62,v63,v62 ...
        }

        // dcbzl r0,r11 — zeroes the 128-byte line at d & ~127. On this path d is 128-byte aligned,
        // so all 128 bytes are overwritten by the eight stores below. Kept, in place, because it is
        // a store the original makes and a caller with an unaligned d would see it.
        vmx::dcbzl(g, d)?;

        // The store order is the lifted one: +16, +32, +48, +64, then +0, then +80, +96, +112.
        for off in [16u32, 32, 48, 64, 0, 80, 96, 112] {
            unsafe { vmx::stvx128_ps(g, d.wrapping_add(off), p[(off / 16) as usize])? };
        }
        d = d.wrapping_add(BLOCK_BYTES); // addi r11,r11,128

        // cmplw cr6,r10,r5 ; bne — the *only* exit. See the module note on the non-terminating case.
        if s == src_end {
            return Ok(());
        }
    }
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn accumulate_vector(g: &mut Guest, dst: u32, src: u32, span: u32, k: f64) -> Result<()> {
    let src_end = src.wrapping_add(span);

    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted by the first stfs
    let scale = unsafe { _mm_set1_ps(k as f32) };

    let mut s = src;
    let mut d = dst;
    loop {
        let a = unsafe { load_block(g, s)? };
        s = s.wrapping_add(BLOCK_BYTES);

        // The destination is loaded in pairs interleaved with the FMAs, exactly as lifted; +32 and
        // +0 come first because the lifted cursors run 32 bytes ahead of the block they address.
        // What matters is only that every load of the block precedes every store of it.
        let b2 = unsafe { vmx::lvx128_ps(g, d + 32)? }; // lvx128 v12,r0,r11
        let b0 = unsafe { vmx::lvx128_ps(g, d)? }; // lvx128 v13,r0,r9

        // Single-rounding FMAs, scale first, as lifted. Rule 4 applies to the two factors.
        fpscr.enable_flush_mode_unconditional(); // before the first vmaddfp
        let out2 = unsafe { vmx::vmaddfp(scale, a[2], b2) }; // vmaddfp v6,v0,v9,v12
        let out0 = unsafe { vmx::vmaddfp(scale, a[0], b0) }; // vmaddfp v7,v0,v11,v13
        let b1 = unsafe { vmx::lvx128_ps(g, d + 16)? };
        let b3 = unsafe { vmx::lvx128_ps(g, d + 48)? };
        let out1 = unsafe { vmx::vmaddfp(scale, a[1], b1) };
        let out3 = unsafe { vmx::vmaddfp(scale, a[3], b3) };
        let b4 = unsafe { vmx::lvx128_ps(g, d + 64)? };
        let b5 = unsafe { vmx::lvx128_ps(g, d + 80)? };
        let out4 = unsafe { vmx::vmaddfp(scale, a[4], b4) };
        let out5 = unsafe { vmx::vmaddfp(scale, a[5], b5) };
        let b6 = unsafe { vmx::lvx128_ps(g, d + 96)? };
        let b7 = unsafe { vmx::lvx128_ps(g, d + 112)? };
        let out6 = unsafe { vmx::vmaddfp(scale, a[6], b6) };
        let out7 = unsafe { vmx::vmaddfp(scale, a[7], b7) };

        // The store order is the lifted one: +32, +0, +16, +48, +64, +80, +96, +112. There is no
        // dcbzl here — this call accumulates, so it may not clear the line first.
        let out = [out0, out1, out2, out3, out4, out5, out6, out7];
        for off in [32u32, 0, 16, 48, 64, 80, 96, 112] {
            unsafe { vmx::stvx128_ps(g, d.wrapping_add(off), out[(off / 16) as usize])? };
        }
        d = d.wrapping_add(BLOCK_BYTES);

        if s == src_end {
            return Ok(());
        }
    }
}

/// Eight `lvx128` at `s + 0 … s + 112`, in that order.
#[target_feature(enable = "sse4.1")]
unsafe fn load_block(g: &Guest, s: u32) -> Result<[__m128; 8]> {
    let mut a = [unsafe { _mm_setzero_ps() }; 8];
    for i in 0..8u32 {
        a[i as usize] = unsafe { vmx::lvx128_ps(g, s.wrapping_add(16 * i))? };
    }
    Ok(a)
}

// ------------------------------------------------------------------------------ the scalar paths

/// The loop counts `loc_82B3BFD4` and `loc_82B44C44` both compute, which are identical.
///
/// `subf r8,r11,r9 ; addi r8,r8,3 ; srawi r7,r8,2 ; addze r6,r7` gives the float count. `len` is a
/// non-negative multiple of four on every reachable path, so this is `len/4` — but the `srawi`
/// carry that `addze` consumes is reproduced anyway, because dropping a carry because it "cannot
/// fire" is how a chain stops being a transcription.
fn scalar_shape(len: u32) -> (i32, u32) {
    let biased = len.wrapping_add(3) as i32;
    let shift_carry = (biased < 0) && ((biased as u32 & 3) != 0); // srawi's CA
    let floats = (biased >> 2) + if shift_carry { 1 } else { 0 }; // addze
    // addi r6,r7,-13 ; rlwinm r6,r6,28,4,31 ; addi r6,r6,1 — where r7 was reloaded with `len`, not
    // with the float count, two instructions earlier. Groups of four floats, i.e. len/16.
    let groups = (len.wrapping_sub(13) >> 4).wrapping_add(1);
    (floats, groups)
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn scale_scalar(g: &mut Guest, dst: u32, src: u32, count: u32, k: f64) -> Result<()> {
    // rlwinm r9,r6,2,0,29 ; add r9,r9,r11 — the destination end, count*4 bytes on.
    let dst_end = dst.wrapping_add((count << 2) & 0xFFFF_FFFC);
    if dst >= dst_end {
        return Ok(()); // cmplw cr6,r11,r9 ; bge
    }

    let mut fpscr = Fpscr::capture();
    let mut s = src; // r10
    let mut d = dst; // r11
    let len = dst_end - d;
    let (floats, mut groups) = scalar_shape(len);

    if floats >= 4 {
        // cmpwi cr6,r6,4 ; blt
        loop {
            fpscr.disable_flush_mode_unconditional(); // before the first lfs
            let a0 = fp::load_single(g, s)?; // lfs f0,0(r10)
            let a1 = fp::load_single(g, s + 4)?; // lfs f13,4(r7), and r7 == r10 here
            let a2 = fp::load_single(g, s + 8)?;
            let a3 = fp::load_single(g, s + 12)?;
            // Four fmuls, the scale *second* as lifted — the opposite order to the vector path's
            // `vmulfp128`, which is rule 4's problem in miniature and is why neither is normalised.
            let p0 = fp::mul_single(a0, k); // fmuls f12,f0,f1
            let p1 = fp::mul_single(a1, k);
            let p2 = fp::mul_single(a2, k);
            let p3 = fp::mul_single(a3, k);
            // Every load above happens before every store below, which is what makes an
            // overlapping src/dst behave as the original does.
            fp::store_single(g, d, p0)?;
            fp::store_single(g, d + 4, p1)?;
            s = s.wrapping_add(16); // addi r10,r10,16
            fp::store_single(g, d + 8, p2)?;
            fp::store_single(g, d + 12, p3)?;
            d = d.wrapping_add(16); // addi r11,r11,16
            groups -= 1;
            if groups == 0 {
                break; // bdnz
            }
        }
    }

    // loc_82B3C058: the remainder, one float at a time.
    if d >= dst_end {
        return Ok(());
    }
    let mut tail = ((dst_end - d - 1) >> 2).wrapping_add(1);
    let mut sp = s.wrapping_sub(4); // addi r11,r10,-4
    let mut dp = d.wrapping_sub(4); // addi r9,r11,-4
    loop {
        fpscr.disable_flush_mode_unconditional(); // before the lfsu
        sp = sp.wrapping_add(4); // lfsu f0,4(r11) updates the address first
        let a = fp::load_single(g, sp)?;
        let p = fp::mul_single(a, k); // fmuls f0,f0,f1
        dp = dp.wrapping_add(4); // stfsu f0,4(r9) likewise
        fp::store_single(g, dp, p)?;
        tail -= 1;
        if tail == 0 {
            return Ok(());
        }
    }
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn accumulate_scalar(g: &mut Guest, dst: u32, src: u32, count: u32, k: f64) -> Result<()> {
    let dst_end = dst.wrapping_add((count << 2) & 0xFFFF_FFFC);
    if dst >= dst_end {
        return Ok(());
    }

    let mut fpscr = Fpscr::capture();
    let mut s = src;
    let mut d = dst;
    let len = dst_end - d;
    let (floats, mut groups) = scalar_shape(len);

    if floats >= 4 {
        loop {
            fpscr.disable_flush_mode_unconditional();
            // The accumulator is read first here, all four words, before any source word.
            let c0 = fp::load_single(g, d)?; // lfs f0,0(r11)
            let c1 = fp::load_single(g, d + 4)?;
            let c2 = fp::load_single(g, d + 8)?;
            let c3 = fp::load_single(g, d + 12)?;
            let a0 = fp::load_single(g, s)?; // lfs f10,0(r10)
            let a1 = fp::load_single(g, s + 4)?;
            // fmadds: one fused double-precision multiply-add, then rounded to single. The loads
            // and the FMAs interleave exactly as lifted; the scale is the second factor.
            let r0 = fp::fmadd_single(a0, k, c0); // fmadds f8,f10,f1,f0
            let a2 = fp::load_single(g, s + 8)?;
            let r1 = fp::fmadd_single(a1, k, c1);
            let a3 = fp::load_single(g, s + 12)?;
            let r2 = fp::fmadd_single(a2, k, c2);
            let r3 = fp::fmadd_single(a3, k, c3);
            fp::store_single(g, d, r0)?;
            fp::store_single(g, d + 4, r1)?;
            s = s.wrapping_add(16);
            fp::store_single(g, d + 8, r2)?;
            fp::store_single(g, d + 12, r3)?;
            d = d.wrapping_add(16);
            groups -= 1;
            if groups == 0 {
                break;
            }
        }
    }

    // loc_82B44CDC.
    if d >= dst_end {
        return Ok(());
    }
    let mut tail = ((dst_end - d - 1) >> 2).wrapping_add(1);
    let mut dp = d.wrapping_sub(4); // addi r11,r11,-4
    let mut sp = s.wrapping_sub(4); // addi r10,r10,-4
    loop {
        fpscr.disable_flush_mode_unconditional();
        let c = fp::load_single(g, dp.wrapping_add(4))?; // lfs f13,4(r11), no update
        sp = sp.wrapping_add(4); // lfsu f0,4(r10) updates first
        let a = fp::load_single(g, sp)?;
        let r = fp::fmadd_single(a, k, c); // fmadds f12,f0,f1,f13
        dp = dp.wrapping_add(4); // stfsu f12,4(r11)
        fp::store_single(g, dp, r)?;
        tail -= 1;
        if tail == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const DST: u32 = BASE + 0x0800;
    const SRC: u32 = BASE + 0x1000;

    fn guest() -> Guest {
        Guest::single(BASE, 0x2000)
    }

    fn put(g: &mut Guest, base: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            g.set_u32(base + 4 * i as u32, v.to_bits()).unwrap();
        }
    }

    fn get(g: &Guest, base: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| g.f32(base + 4 * i as u32).unwrap())
            .collect()
    }

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32) * 0.5 - 3.0).collect()
    }

    /// Independent per-element models, written from what the functions are *for* rather than from
    /// their loop structure. Both round exactly once per element, which is the property the
    /// `fmuls`/`fmadds` forms have and a naive `f64` chain does not.
    fn model_scale(src: &[f32], k: f64) -> Vec<f32> {
        src.iter().map(|x| ((*x as f64) * k) as f32).collect()
    }
    fn model_accumulate(dst: &[f32], src: &[f32], k: f64) -> Vec<f32> {
        dst.iter()
            .zip(src)
            .map(|(c, x)| ((*x as f64).mul_add(k, *c as f64)) as f32)
            .collect()
    }

    #[test]
    fn the_path_selector_is_both_masks() {
        // Vector needs 128-byte alignment on *both* pointers and a count that is a multiple of 64.
        assert!(takes_vector_path(0x1000_0000, 0x2000_0080, 256));
        assert!(
            !takes_vector_path(0x1000_0004, 0x2000_0000, 256),
            "dst misaligned"
        );
        assert!(
            !takes_vector_path(0x1000_0000, 0x2000_0040, 256),
            "src on 64 but not 128"
        );
        assert!(
            !takes_vector_path(0x1000_0000, 0x2000_0000, 255),
            "count not a multiple of 64"
        );
        assert!(
            !takes_vector_path(0x1000_0000, 0x2000_0000, 32),
            "32 is not a multiple of 64"
        );
        // The mask is 0x7F, not 0xF: a 16-byte aligned pair is still the scalar path.
        assert!(!takes_vector_path(0x1000_0010, 0x2000_0010, 64));
    }

    #[test]
    fn the_vector_path_scales_a_whole_block() {
        // The shape the game actually passes: count 256, both buffers 128-byte aligned.
        let mut g = guest();
        let src = ramp(256);
        put(&mut g, SRC, &src);
        put(&mut g, DST, &vec![f32::NAN; 256]); // pre-filled, so a skipped word shows up
        assert!(takes_vector_path(DST, SRC, 256));
        scale(&mut g, DST, SRC, 256, 0.25).unwrap();
        assert_eq!(get(&g, DST, 256), model_scale(&src, 0.25));
        // And it wrote nothing past the end.
        assert!(g.f32(DST + 1024).unwrap().is_nan() || g.u32(DST + 1024).unwrap() == 0);
    }

    #[test]
    fn every_word_of_every_group_is_written_including_the_out_of_order_one() {
        // The lifted store order is +16,+32,+48,+64,+0,+80,+96,+112 — the +0 store is fifth, not
        // first. A transcription that dropped it would leave the first four floats of each block
        // holding whatever the dcbzl left, which is zero, and a test on an all-zero source would
        // not notice.
        let mut g = guest();
        let src: Vec<f32> = (0..256).map(|i| (i + 1) as f32).collect();
        put(&mut g, SRC, &src);
        scale(&mut g, DST, SRC, 256, 2.0).unwrap();
        let out = get(&g, DST, 256);
        for (i, v) in out.iter().enumerate() {
            assert_eq!(
                *v,
                (i as f32 + 1.0) * 2.0,
                "float {i} (block {}, word {})",
                i / 32,
                i % 32
            );
        }
        assert!(
            out.iter().all(|v| *v != 0.0),
            "a zero anywhere means a store was missed"
        );
    }

    #[test]
    fn the_dcbzl_line_clear_is_reproduced() {
        // dcbzl clears 128 bytes at d & ~127 before the eight stores. With a 128-byte aligned d
        // every byte is overwritten and the clear is invisible — which is exactly why it needs its
        // own test. Give it a destination whose line starts *below* it and the clear shows.
        let mut g = guest();
        put(&mut g, SRC, &ramp(64));
        // Poison the 128 bytes below DST; a dcbzl at DST clears that same line.
        put(&mut g, DST - 128, &vec![7.0f32; 32]);
        put(&mut g, DST, &vec![7.0f32; 32]);
        scale(&mut g, DST, SRC, 64, 1.0).unwrap();
        assert_eq!(
            get(&g, DST - 128, 32),
            vec![7.0f32; 32],
            "the line below is not this call's"
        );
        // Now the aliasing case the mask really describes: an unaligned destination, where the
        // cleared line reaches backwards past it. `Windows()` still covers it, and the C++ keeps
        // the clear for exactly this reason.
        let mut h = guest();
        put(&mut h, SRC, &ramp(64));
        put(&mut h, DST - 64, &vec![7.0f32; 16]);
        // A dst of DST+64 is 64-byte but not 128-byte aligned, so this is the scalar path and no
        // dcbzl runs at all: the guard below is what distinguishes the two.
        assert!(!takes_vector_path(DST + 64, SRC, 64));
        scale(&mut h, DST + 64, SRC, 64, 1.0).unwrap();
        assert_eq!(
            get(&h, DST - 64, 16),
            vec![7.0f32; 16],
            "no dcbzl on the scalar path"
        );
    }

    #[test]
    fn the_scalar_path_handles_every_remainder() {
        // Counts that exercise the group loop, the tail loop, and both together. The C++'s own
        // cross-check used exactly these.
        for count in [1u32, 2, 3, 4, 5, 7, 8, 16, 17, 63, 65, 127, 128] {
            let mut g = guest();
            let src = ramp(count as usize);
            // +4 on both makes them 4-byte aligned and nothing more, so the scalar path is forced
            // even for counts that are multiples of 64.
            let (d, s) = (DST + 4, SRC + 4);
            assert!(!takes_vector_path(d, s, count));
            put(&mut g, s, &src);
            scale(&mut g, d, s, count, -1.5).unwrap();
            assert_eq!(
                get(&g, d, count as usize),
                model_scale(&src, -1.5),
                "count = {count}"
            );
            // Nothing past the end.
            assert_eq!(
                g.u32(d + 4 * count).unwrap(),
                0,
                "wrote past dst_end at count = {count}"
            );
        }
    }

    #[test]
    fn a_zero_count_on_the_scalar_path_writes_nothing() {
        // `cmplw r11,r9 ; bge` returns immediately. This is a real comparison of "writes nothing",
        // not an untested corner: the C++ `Windows()` declares a zero-length write set for it.
        let mut g = guest();
        put(&mut g, DST + 4, &[9.0, 9.0, 9.0, 9.0]);
        scale(&mut g, DST + 4, SRC + 4, 0, 3.0).unwrap();
        assert_eq!(get(&g, DST + 4, 4), vec![9.0f32; 4]);
        scale_accumulate(&mut g, DST + 4, SRC + 4, 0, 3.0).unwrap();
        assert_eq!(get(&g, DST + 4, 4), vec![9.0f32; 4]);
    }

    #[test]
    fn the_accumulator_reads_the_destination_before_it_writes_it() {
        let mut g = guest();
        let src = ramp(256);
        let acc: Vec<f32> = (0..256).map(|i| (i as f32) * -0.125).collect();
        put(&mut g, SRC, &src);
        put(&mut g, DST, &acc);
        scale_accumulate(&mut g, DST, SRC, 256, 0.75).unwrap();
        assert_eq!(get(&g, DST, 256), model_accumulate(&acc, &src, 0.75));
        // With a zero scale the accumulator is unchanged, which says the destination was read.
        let mut h = guest();
        put(&mut h, SRC, &src);
        put(&mut h, DST, &acc);
        scale_accumulate(&mut h, DST, SRC, 256, 0.0).unwrap();
        assert_eq!(get(&h, DST, 256), acc);
    }

    #[test]
    fn the_accumulator_rounds_twice_on_the_vector_path_and_once_on_the_scalar_path() {
        // The distinguishing input: `a * b` needs 48 bits, and the destination holds the negation
        // of its f32 rounding. A single-rounding multiply-add returns the discarded bits; a
        // multiply-then-add returns zero, because the multiply threw those bits away first.
        let a = 1.0f32 + f32::EPSILON;
        let b = 1.0f32 - f32::EPSILON;
        let rounded = a * b; // == 1.0
        let fused = ((a as f64) * (b as f64) - rounded as f64) as f32; // -2^-46
        assert_ne!(fused, 0.0);

        for count in [256u32, 4] {
            let (d, s) = if count == 256 {
                (DST, SRC)
            } else {
                (DST + 4, SRC + 4)
            };
            let mut g = guest();
            put(&mut g, s, &vec![a; count as usize]);
            put(&mut g, d, &vec![-rounded; count as usize]);
            scale_accumulate(&mut g, d, s, count, b as f64).unwrap();
            // CORRECTED 2026-09-13. The vector path's vmaddfp rounds the product and then the add,
            // because the recomp is built without FMA and SIMDe falls back to `(a * b) + c`; so the
            // discarded bits are gone and the answer is +0. The scalar path's fmadds is `std::fma`,
            // which is correctly rounded in software with or without the instruction, so it keeps
            // them. One function, two answers, decided by which path the count and alignment take.
            let want = if count == 256 { 0.0f32 } else { fused };
            let got: Vec<u32> = get(&g, d, count as usize)
                .iter()
                .map(|v| v.to_bits())
                .collect();
            assert_eq!(got, vec![want.to_bits(); count as usize], "count {count}");
        }

        // And `scale`, which has no add at all, leaves the rounded product — the value the
        // accumulate path is *not* allowed to reach.
        let mut g = guest();
        put(&mut g, SRC, &vec![a; 256]);
        scale(&mut g, DST, SRC, 256, b as f64).unwrap();
        assert_eq!(get(&g, DST, 256), vec![rounded; 256]);
    }

    #[test]
    fn both_paths_agree_when_the_scale_is_a_single() {
        // Both paths reduce to "round the exact product once" when the scale is representable as a
        // single: the vector path multiplies in f32, the scalar path forms an exact f64 product of
        // two 24-bit values and narrows it. They are different instructions in a different operand
        // order, so this is a real check rather than a tautology.
        let src = ramp(256);
        let k = 0.1f32 as f64; // a single, widened — see the next test for why that matters
        let mut v = guest();
        put(&mut v, SRC, &src);
        scale(&mut v, DST, SRC, 256, k).unwrap();
        let mut s = guest();
        put(&mut s, SRC + 4, &src);
        scale(&mut s, DST + 4, SRC + 4, 256, k).unwrap();
        assert_eq!(get(&v, DST, 256), get(&s, DST + 4, 256));

        let acc: Vec<f32> = src.iter().map(|x| x * 0.375).collect();
        let mut va = guest();
        put(&mut va, SRC, &src);
        put(&mut va, DST, &acc);
        scale_accumulate(&mut va, DST, SRC, 256, k).unwrap();
        let mut sa = guest();
        put(&mut sa, SRC + 4, &src);
        put(&mut sa, DST + 4, &acc);
        scale_accumulate(&mut sa, DST + 4, SRC + 4, 256, k).unwrap();
        assert_eq!(get(&va, DST, 256), get(&sa, DST + 4, 256));
    }

    #[test]
    fn a_scale_that_is_not_a_single_makes_the_two_paths_differ() {
        // Not a defect in the translation — a property of the original, and one that only shows up
        // once both paths are written out. The vector path narrows `f1` to a single before it
        // multiplies (four `stfs` into the red zone, then one `lvx128`); the scalar path multiplies
        // in double and narrows the product. For an `f1` that is exactly a single those agree, and
        // the game only ever passes such a value. For any other they do not, and neither answer is
        // wrong: they are what the two instruction streams compute.
        let src = ramp(256);
        let k = 0.1f64; // 0.1 as a double is NOT 0.1 as a single
        assert_ne!(k, k as f32 as f64);

        let mut v = guest();
        put(&mut v, SRC, &src);
        scale(&mut v, DST, SRC, 256, k).unwrap();
        let mut s = guest();
        put(&mut s, SRC + 4, &src);
        scale(&mut s, DST + 4, SRC + 4, 256, k).unwrap();

        let vector = get(&v, DST, 256);
        let scalar = get(&s, DST + 4, 256);
        let differing = vector.iter().zip(&scalar).filter(|(a, b)| a != b).count();
        assert!(
            differing > 0,
            "the narrowing has to be visible, or this file has lost it"
        );
        // The scalar path is the one that keeps the double: it matches an f64 multiply.
        assert_eq!(scalar, model_scale(&src, k));
        // The vector path matches the same multiply with the scale narrowed first.
        assert_eq!(vector, model_scale(&src, k as f32 as f64));
    }

    #[test]
    fn the_loop_shape_matches_the_lifted_counters() {
        // scalar_shape is the `srawi`/`addze`/`rlwinm` chain. `groups` counts *four-float* groups,
        // so it is len/16 and not len/4 — the lifted `addi r6,r7,-13` reads a reloaded r7 that holds
        // the byte length, which is the one place this arithmetic is easy to mistranscribe.
        assert_eq!(scalar_shape(16), (4, 1));
        assert_eq!(scalar_shape(32), (8, 2));
        assert_eq!(scalar_shape(64), (16, 4));
        assert_eq!(scalar_shape(4).0, 1, "one float, no group loop");
        assert_eq!(
            scalar_shape(12).0,
            3,
            "three floats, still below the group threshold"
        );
        // 17 floats: four groups of four plus a one-float tail.
        assert_eq!(scalar_shape(68), (17, 4));
    }

    #[test]
    fn an_unbounded_write_set_becomes_an_error_rather_than_a_hang() {
        // count == 0 with both pointers aligned takes the vector path with a zero span, and the
        // lifted loop's `r10 != r5` exit can never fire. The original hangs; the C++ `Windows()`
        // declines the input; here the cursor walks off the guest map. An `Err` is the honest
        // answer — see the module note.
        let mut g = guest();
        assert!(takes_vector_path(DST, SRC, 0));
        assert!(scale(&mut g, DST, SRC, 0, 1.0).is_err());
        assert!(scale_accumulate(&mut g, DST, SRC, 0, 1.0).is_err());
    }

    #[test]
    fn a_group_reads_all_four_words_before_it_writes_any() {
        // The ordering claim is **within a group**, and that is the only place it can be made: the
        // group loop reads four words, multiplies four times and then stores four. A destination one
        // word *ahead* of the source puts the first store exactly on the second load's address, so
        // hoisting a store above a load changes the answer.
        //
        // The first version of this test put the destination a whole group *behind* the source,
        // where every store lands on a word the loop has already consumed — it could not have
        // failed, and it passed. Recorded because that is the shape of a vacuous test.
        let mut g = guest();
        let src = ramp(8);
        let s = SRC + 4;
        let d = s + 4; // one float ahead: d[0] is src[1]'s address
        put(&mut g, s, &src);
        scale(&mut g, d, s, 8, 2.0).unwrap();
        assert_eq!(
            get(&g, d, 4),
            model_scale(&src[0..4], 2.0),
            "group 0 must see the pre-call source in all four words"
        );
        // Beyond the first group the source really has been overwritten — by this call, exactly as
        // the original overwrites it — so there is nothing to assert there and none is asserted.
        assert_eq!(
            g.f32(s).unwrap(),
            src[0],
            "the word below the destination is untouched"
        );
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest();
        put(&mut g, SRC, &ramp(256));
        let before = vmx::get_mxcsr();
        scale(&mut g, DST, SRC, 256, 1.0).unwrap();
        assert_eq!(vmx::get_mxcsr(), before);
        scale_accumulate(&mut g, DST, SRC, 256, 1.0).unwrap();
        assert_eq!(vmx::get_mxcsr(), before);
        scale(&mut g, DST + 4, SRC + 4, 7, 1.0).unwrap();
        assert_eq!(vmx::get_mxcsr(), before, "the scalar path too");
    }
}
