//! `sub_82B3CF58` — `z[i] = x[i]·gain + y[i]`, with a parallel copy of `x` into `w`.
//!
//! Four buffers and one gain: a scale-and-add against a separate addend, plus a verbatim copy of the
//! source into a fourth buffer. `docs/ports.md`: **verified**, 179,218 calls a boot and 339,926 in a
//! played session, with 25 vector instructions.
//!
//! It is the three-buffer relative of [`crate::dsp::scale`]'s pair — same 16-byte vector traffic, same
//! `vmaddfp`, same scalar fallback — and it lives in its own file rather than in `scale.rs` because
//! nothing about it is a twin of those two: it selects its path on the **source's** alignment alone,
//! it moves 16 floats an iteration rather than 32, and it has a second destination.
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green: the C++ was
//! compared call-for-call against the original under the shadow harness on those real inputs at zero
//! divergence, and the Rust below has no recorded vectors of its own. A fault here is a transcription
//! error rather than a misreading of the kernel; it is not a number.
//!
//! ## The two paths, and the one bit that picks them
//!
//! ```text
//! vector   (x & 0xF) == 0     16 floats an iteration, blocks = n/16 truncated toward zero
//! scalar   everything else    one float a trip, and the remainder of the vector path as well
//! ```
//!
//! Only `x` is tested. `y`, `z` and `w` may be at any alignment, and that has a consequence the C++
//! `Windows()` builder spells out: `stvx128` **masks its own address down** to the containing 16-byte
//! block, so an unaligned `z` or `w` on the vector path is written *low*, not high — the block below
//! the pointer is clobbered and the last bytes of the run are not written at all.
//! `the_vector_path_masks_its_stores_down_to_the_containing_block` pins that rather than writing
//! around it.
//!
//! ## Three things that are not "just an FMA loop"
//!
//! **The copy re-reads `x` after the `z` store.** Four `lvx128` of the source, in the lifted order
//! 1, 3, 2, 0, *after* the four `stvx128` to `z` — and in the scalar path an `lfs` of the same address
//! after the `stfsx`. So a call with `z == x` copies the **scaled result** into `w`, not the original
//! source. Reproduced, not fixed; `the_copy_is_reloaded_after_the_scaled_store` is the test.
//!
//! **The gain reaches the two paths differently.** The vector path narrows it to a single once (four
//! `stfs` into the red zone, one `lvx128` back) and multiplies in f32; the scalar path keeps the guest
//! FPR and multiplies in double through `fmadds`. For a gain that is exactly representable as a single
//! — which an `lfs` in the caller guarantees — the two agree on almost everything; they are not the
//! same function, and `a_gain_that_is_not_a_single_makes_the_two_paths_differ` pins it. This is a
//! property of the original.
//!
//! **The operand order of the two FMAs is opposite.** `vmaddfp v13,v0,v13,v12` takes the *gain* as its
//! first factor; `fmadds f12,f0,f1,f13` takes the *sample* first. Per
//! `docs/vmx128-exactness.md` rule 4 the winning slot on two NaN operands is part of the semantics and
//! is chosen by register allocation, so both orders are copied from the lifted lines rather than
//! normalised — and no test here can check them, in this language or in the reference.
//!
//! ## Not reproduced
//!
//! The four `stfs f1,-128(r1)`…`-116(r1)` that materialise the splat, which land in the function's own
//! red zone. The four words are identical, so the load's lane reversal is a no-op on them and the
//! splat is built as a register — [`crate::dsp::scale`]'s established precedent, and it holds for the
//! same measured reason: the guest `r1` on the real entries is 16-byte aligned.
//!
//! Also not reproduced: `__restgprlr_20`'s restore of `r20`-`r31`. There is no register file here, and
//! the original's exit values are its entry values.

#![allow(unused_unsafe)] // see the note at the top of `crate::vmx`

use crate::fp;
use crate::vmx::{self, Fpscr};
use crate::{Guest, Result};
use core::arch::x86_64::*;

/// `clrlwi r10,r6,28` — the source's low four bits, and the whole of the path selection.
pub const SOURCE_ALIGN_MASK: u32 = 0xF;
/// Floats moved per iteration of the vector loop: four vectors of four.
pub const BLOCK_SAMPLES: u32 = 16;
/// `addi r9,r9,64` and friends — 64 bytes per buffer per iteration.
pub const BLOCK_BYTES: u32 = 4 * BLOCK_SAMPLES;

/// Whether a call with this source pointer takes the vector path at all.
///
/// Exposed because it is the whole of the path selection, and a caller wanting to know which half of
/// this file ran should not have to re-derive the mask.
pub const fn takes_vector_path(x: u32) -> bool {
    (x & SOURCE_ALIGN_MASK) == 0
}

/// `sub_82B3CF58` — `z[i] = x[i]·gain + y[i]` and `w[i] = x[i]`, for `i` in `[0, n)`.
///
/// Arguments, by register: `n` is `r3` (the sample count, compared **signed** throughout), `y` is
/// `r5` (the addend, read only), `x` is `r6` (the source, read only — and its low four bits pick the
/// path), `z` is `r7`, `w` is `r8`, and `gain` is `f1`. `r4` is **not** an argument: every use of it in
/// the original is preceded by a write. There is no return value (`kReturnNone`).
///
/// All five integer arguments are `u32` rather than `u64` because the original uses only their low
/// words — every effective address in the body is `base + 16m` or `base + 4i` in 32-bit wrap
/// arithmetic, and the count reaches `srawi`, `cmpw` and `subf` as a word. A replay therefore passes
/// `v.r3 as u32` and so on.
///
/// **Writes** two runs per buffer. On the vector path, `64·blocks` bytes from the 16-byte **floor** of
/// `z` and of `w` (see the module note on `stvx128` masking); then, for the `n - 16·blocks` remaining
/// samples, four bytes each from `z + 4·16·blocks` and `w + 4·16·blocks` exactly. Reads the matching
/// runs of `x` and `y`, with `x` read **twice** per sample.
///
/// A non-positive `n` writes nothing at all on either path, and neither does a count whose blocks the
/// vector loop skips: the `bge` at `loc_82B3D05C` is a signed compare of `16·blocks` against `n`.
pub fn scale_add_with_copy(
    g: &mut Guest,
    n: u32,
    y: u32,
    x: u32,
    z: u32,
    w: u32,
    gain: f64,
) -> Result<()> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    let mut done: u32 = 0; // li r11,0 — samples consumed by the vector body

    // clrlwi r10,r6,28 ; cmplwi cr6,r10,0 ; bne cr6,0x82b3d05c
    if takes_vector_path(x) {
        // srawi r11,r3,4 ; addze r20,r11 — a signed divide by sixteen that truncates *toward zero*.
        // The `srawi`'s carry-out is the rounding correction and `addze` folds it back in, which is
        // what `/ 16` on a signed int does and what `>> 4` does not.
        let blocks = (n as i32) / 16;

        // rlwinm r11,r20,2,0,29 ; cmpwi cr6,r11,0 ; ble cr6,0x82b3d058
        let quarter_count = ((blocks as u32) << 2) as i32;
        if quarter_count > 0 {
            // addi r11,r11,-1 ; rlwinm r11,r11,30,2,31 ; addi r4,r11,1 ; mtctr r4 — a logical shift
            // right by two of `4·blocks - 1`, plus one: exactly `blocks`. Kept in the lifted form so
            // the derivation stays visible.
            let iterations = ((quarter_count as u32 - 1) >> 2) + 1;
            unsafe { vector_body(g, y, x, z, w, gain, iterations)? };
        }
        // loc_82B3D058: rlwinm r11,r20,4,0,27 — sixteen samples per iteration.
        done = (blocks as u32) << 4;
    }

    // loc_82B3D05C: cmpw cr6,r11,r3 ; bge cr6,0x82b3d0a4 — a signed word compare, so a negative or
    // zero count leaves through here having stored nothing outside the frame.
    if (done as i32) >= (n as i32) {
        return Ok(());
    }
    unsafe { scalar_tail(g, n, y, x, z, w, gain, done) }
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn vector_body(
    g: &mut Guest,
    y: u32,
    x: u32,
    z: u32,
    w: u32,
    gain: f64,
    iterations: u32,
) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    // The four stfs into the red zone, then `lvx128 v0,r0,r4`: the gain in all four lanes. Held as a
    // register here — see the module note.
    fpscr.disable_flush_mode_unconditional();
    let gain_v = unsafe { _mm_set1_ps(gain as f32) };

    for k in 0..iterations {
        // bdnz 0x82b3cfe0 — r9/r10/r11 all advance by 64 per iteration, so every lifted effective
        // address reduces to one of the four bases plus this offset. `(A + 16m) & ~0xF ==
        // (A & ~0xF) + 16m`, which is why the store masking distributes out of the loop.
        let u = k * BLOCK_BYTES;

        let x0 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(u))? }; // lvx128 v13,r10,r48
        let y0 = unsafe { vmx::lvx128_ps(g, y.wrapping_add(u))? }; // lvx128 v12,r9,r25

        // vmaddfp v13,v0,v13,v12 — the **gain** is the first factor here; the scalar tail passes the
        // sample first. Rule 4: the slot is part of the semantics, so neither is normalised.
        fpscr.enable_flush_mode_unconditional(); // emitted before this vmaddfp
        let mix0 = unsafe { vmx::vmaddfp(gain_v, x0, y0) };

        let y1 = unsafe { vmx::lvx128_ps(g, y.wrapping_add(16).wrapping_add(u))? }; // lvx128 v10
        let y3 = unsafe { vmx::lvx128_ps(g, y.wrapping_add(48).wrapping_add(u))? }; // lvx128 v12
        let y2 = unsafe { vmx::lvx128_ps(g, y.wrapping_add(32).wrapping_add(u))? }; // lvx128 v11

        unsafe { vmx::stvx128_ps(g, z.wrapping_add(u), mix0)? }; // stvx128 v13,r11,r23

        let x1 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(16).wrapping_add(u))? };
        let mix1 = unsafe { vmx::vmaddfp(gain_v, x1, y1) }; // vmaddfp v13,v0,v13,v10
        unsafe { vmx::stvx128_ps(g, z.wrapping_add(16).wrapping_add(u), mix1)? };

        let x2 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(32).wrapping_add(u))? };
        let mix2 = unsafe { vmx::vmaddfp(gain_v, x2, y2) }; // vmaddfp v13,v0,v13,v11
        unsafe { vmx::stvx128_ps(g, z.wrapping_add(32).wrapping_add(u), mix2)? };

        let x3 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(48).wrapping_add(u))? };
        let mix3 = unsafe { vmx::vmaddfp(gain_v, x3, y3) }; // vmaddfp v13,v0,v13,v12
        unsafe { vmx::stvx128_ps(g, z.wrapping_add(48).wrapping_add(u), mix3)? };

        // The copy re-reads x **after** the four z stores, in the lifted order 1, 3, 2, 0. Kept as
        // loads rather than reusing x0..x3 already in hand: z may alias x.
        let copy1 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(16).wrapping_add(u))? }; // v62
        let copy3 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(48).wrapping_add(u))? }; // v60
        let copy2 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(32).wrapping_add(u))? }; // v61
        let copy0 = unsafe { vmx::lvx128_ps(g, x.wrapping_add(u))? }; // v63

        unsafe { vmx::stvx128_ps(g, w.wrapping_add(u), copy0)? }; // stvx128 v63,r0,r9
        unsafe { vmx::stvx128_ps(g, w.wrapping_add(48).wrapping_add(u), copy3)? }; // v60
        unsafe { vmx::stvx128_ps(g, w.wrapping_add(16).wrapping_add(u), copy1)? }; // v62
        unsafe { vmx::stvx128_ps(g, w.wrapping_add(32).wrapping_add(u), copy2)? }; // v61
    }
    Ok(())
}

/// `loc_82B3D05C` — one sample a trip, through three constant inter-buffer deltas.
///
/// The deltas are the original's: `z - x`, `y - z` and `w - x`, each a 32-bit `subf`, with the
/// addend reached as `y_delta + z_ptr` rather than as `y + offset`. Written that way because the two
/// forms differ when the sum wraps, and because it is one instruction in the lifted body.
#[allow(clippy::too_many_arguments)]
unsafe fn scalar_tail(
    g: &mut Guest,
    n: u32,
    y: u32,
    x: u32,
    z: u32,
    w: u32,
    gain: f64,
    done: u32,
) -> Result<()> {
    let tail = n.wrapping_sub(done); // subf r4,r11,r3 ; mtctr r4
    let mut x_ptr = ((done << 2) & 0xFFFF_FFFC).wrapping_add(x); // rlwinm ; add r11,r11,r6
    let z_delta = z.wrapping_sub(x); // subf r10,r6,r7
    let y_delta = y.wrapping_sub(z); // subf r9,r7,r5
    let w_delta = w.wrapping_sub(x); // subf r8,r6,r8

    let mut fpscr = Fpscr::capture();
    for _ in 0..tail {
        // bdnz 0x82b3d080
        let z_ptr = z_delta.wrapping_add(x_ptr); // add r7,r10,r11
        fpscr.disable_flush_mode_unconditional();
        let sample = fp::load_single(g, x_ptr)?; // lfs f0,0(r11)
        let addend = fp::load_single(g, y_delta.wrapping_add(z_ptr))?; // lfsx f13,r9,r7
        // fmadds f12,f0,f1,f13 — the **sample** is the first factor here, unlike the vector path.
        fp::store_single(g, z_ptr, fp::fmadd_single(sample, gain, addend))?; // stfsx f12,r10,r11
        let copy = fp::load_single(g, x_ptr)?; // lfs f11,0(r11) — reloaded after the store above
        fp::store_single(g, x_ptr.wrapping_add(w_delta), copy)?; // stfsx f11,r11,r8
        x_ptr = x_ptr.wrapping_add(4); // addi r11,r11,4
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const Y: u32 = BASE + 0x0100;
    const X: u32 = BASE + 0x0400;
    const Z: u32 = BASE + 0x0700;
    const W: u32 = BASE + 0x0A00;
    /// A source four bytes off a 16-byte boundary, which forces the scalar path.
    const X_ODD: u32 = BASE + 0x0D04;

    fn guest() -> Guest {
        Guest::single(BASE, 0x1000)
    }

    fn put(g: &mut Guest, at: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            g.set_u32(at + 4 * i as u32, v.to_bits()).unwrap();
        }
    }

    fn get(g: &Guest, at: u32, n: u32) -> Vec<f32> {
        (0..n).map(|i| g.f32(at + 4 * i).unwrap()).collect()
    }

    fn poison(g: &mut Guest, at: u32, n: u32) {
        for i in 0..n {
            g.set_u32(at + 4 * i, 0x7F7F_7F7F).unwrap();
        }
    }

    fn source(n: u32) -> Vec<f32> {
        (0..n).map(|i| (i as f32) * 0.25 - 3.0).collect()
    }

    fn addend(n: u32) -> Vec<f32> {
        (0..n).map(|i| 100.0 - (i as f32) * 0.5).collect()
    }

    /// The expected `z` run, per element, split at the vector/scalar boundary.
    ///
    /// The two halves are **different functions** and the model says so: the vector path is one f32
    /// FMA against a single-narrowed gain, the scalar path is an f64 FMA rounded to single. For a gain
    /// that is exactly a single they agree on ordinary inputs; the model keeps them apart so that
    /// `a_gain_that_is_not_a_single_makes_the_two_paths_differ` has something to compare against.
    fn model_z(x: &[f32], y: &[f32], gain: f64, vector_samples: u32) -> Vec<f32> {
        x.iter()
            .zip(y)
            .enumerate()
            .map(|(i, (xi, yi))| {
                if (i as u32) < vector_samples {
                    (gain as f32).mul_add(*xi, *yi)
                } else {
                    fp::fmadd_single(*xi as f64, gain, *yi as f64) as f32
                }
            })
            .collect()
    }

    #[test]
    fn the_scalar_path_scales_adds_and_copies_every_sample() {
        let n = 7u32;
        let (x, y) = (source(n), addend(n));
        let mut g = guest();
        put(&mut g, X_ODD, &x);
        put(&mut g, Y, &y);
        poison(&mut g, Z, n + 1);
        poison(&mut g, W, n + 1);

        scale_add_with_copy(&mut g, n, Y, X_ODD, Z, W, 0.5).unwrap();

        assert!(
            !takes_vector_path(X_ODD),
            "this test has to be on the scalar path"
        );
        assert_eq!(get(&g, Z, n), model_z(&x, &y, 0.5, 0));
        assert_eq!(get(&g, W, n), x, "w is a verbatim copy of x");
        assert_eq!(
            g.u32(Z + 4 * n).unwrap(),
            0x7F7F_7F7F,
            "one sample past the z run"
        );
        assert_eq!(
            g.u32(W + 4 * n).unwrap(),
            0x7F7F_7F7F,
            "one sample past the w run"
        );
        // And the source and addend come out unchanged.
        assert_eq!(get(&g, X_ODD, n), x);
        assert_eq!(get(&g, Y, n), y);
    }

    #[test]
    fn the_vector_path_takes_whole_sixteens_and_the_tail_takes_the_remainder() {
        // n = 35 is two blocks of sixteen plus three. The split point is what this pins: a port that
        // used `>> 4` for the block count, or that let the tail start at the wrong sample, would move
        // it. Both halves are checked against their own model.
        let n = 35u32;
        let (x, y) = (source(n), addend(n));
        let mut g = guest();
        put(&mut g, X, &x);
        put(&mut g, Y, &y);
        poison(&mut g, Z, n + 1);
        poison(&mut g, W, n + 1);

        scale_add_with_copy(&mut g, n, Y, X, Z, W, 0.5).unwrap();

        assert!(takes_vector_path(X));
        assert_eq!(get(&g, Z, n), model_z(&x, &y, 0.5, 32));
        assert_eq!(get(&g, W, n), x);
        assert_eq!(
            g.u32(Z + 4 * n).unwrap(),
            0x7F7F_7F7F,
            "one sample past the run"
        );
        assert_eq!(g.u32(W + 4 * n).unwrap(), 0x7F7F_7F7F);

        // Fifteen samples is a *whole* tail and no vector iteration at all, which is the other side
        // of the same boundary.
        let mut h = guest();
        put(&mut h, X, &x[..15]);
        put(&mut h, Y, &y[..15]);
        poison(&mut h, Z, 16);
        scale_add_with_copy(&mut h, 15, Y, X, Z, W, 0.5).unwrap();
        assert_eq!(get(&h, Z, 15), model_z(&x[..15], &y[..15], 0.5, 0));
        assert_eq!(h.u32(Z + 60).unwrap(), 0x7F7F_7F7F);
    }

    #[test]
    fn a_non_positive_count_writes_nothing_on_either_path() {
        // `cmpw cr6,r11,r3 ; bge cr6` is signed, so 0 and every negative count fall straight out.
        // -16 is the interesting one: it gives `blocks = -1`, so `done` is `-16` too and the compare
        // is an equality rather than a strict inequality.
        for count in [
            0u32,
            1u32.wrapping_neg(),
            16u32.wrapping_neg(),
            100u32.wrapping_neg(),
        ] {
            for src in [X, X_ODD] {
                let mut g = guest();
                put(&mut g, src, &source(16));
                put(&mut g, Y, &addend(16));
                poison(&mut g, Z, 16);
                poison(&mut g, W, 16);
                scale_add_with_copy(&mut g, count, Y, src, Z, W, 0.5).unwrap();
                assert_eq!(
                    get(&g, Z, 16),
                    vec![f32::from_bits(0x7F7F_7F7F); 16],
                    "count {count:#x}"
                );
                assert_eq!(
                    get(&g, W, 16),
                    vec![f32::from_bits(0x7F7F_7F7F); 16],
                    "count {count:#x}"
                );
            }
        }
    }

    #[test]
    fn the_copy_is_reloaded_after_the_scaled_store() {
        // `z == x` is the aliasing case the reload makes observable: `w` gets the **scaled result**,
        // not the source. Reproduced rather than fixed, and it has to hold on both paths — in the
        // vector body because the four copy loads come after the four z stores, and in the tail
        // because the `lfs` comes after the `stfsx`.
        for (name, src, n) in [("vector", X, 16u32), ("scalar", X_ODD, 5u32)] {
            let (x, y) = (source(n), addend(n));
            let mut g = guest();
            put(&mut g, src, &x);
            put(&mut g, Y, &y);
            poison(&mut g, W, n);

            // z aliases x exactly.
            scale_add_with_copy(&mut g, n, Y, src, src, W, 0.5).unwrap();

            let expected = model_z(&x, &y, 0.5, if src == X { n } else { 0 });
            assert_eq!(get(&g, src, n), expected, "{name}: z");
            assert_eq!(
                get(&g, W, n),
                expected,
                "{name}: w copied the result, not the source"
            );
            assert_ne!(
                expected, x,
                "{name}: the two have to differ for this to mean anything"
            );
        }
    }

    #[test]
    fn a_gain_that_is_not_a_single_makes_the_two_paths_differ() {
        // The vector path narrows the gain once and multiplies in f32; the scalar path multiplies the
        // guest FPR in double. A gain needing more than 24 mantissa bits separates them, which is a
        // property of the original and not of this translation.
        let gain = 1.0f64 + f64::from_bits(0x3E70_0000_0000_0000); // 1 + 2^-24: not an f32
        assert_ne!(gain, gain as f32 as f64);
        let n = 16u32;
        let x: Vec<f32> = (0..n).map(|i| 1.0 + i as f32).collect();
        let y = vec![0.0f32; n as usize];

        let mut vector = guest();
        put(&mut vector, X, &x);
        put(&mut vector, Y, &y);
        scale_add_with_copy(&mut vector, n, Y, X, Z, W, gain).unwrap();

        let mut scalar = guest();
        put(&mut scalar, X_ODD, &x);
        put(&mut scalar, Y, &y);
        scale_add_with_copy(&mut scalar, n, Y, X_ODD, Z, W, gain).unwrap();

        assert_eq!(get(&vector, Z, n), model_z(&x, &y, gain, n));
        assert_eq!(get(&scalar, Z, n), model_z(&x, &y, gain, 0));
        assert_ne!(
            get(&vector, Z, n),
            get(&scalar, Z, n),
            "the two paths are not one function"
        );
    }

    #[test]
    fn the_vector_path_masks_its_stores_down_to_the_containing_block() {
        // Only `x`'s alignment picks the path, so `z` may be unaligned on it — and `stvx128` forces
        // its address to a 16-byte boundary, writing the block **below** the pointer. The
        // consequence, which the C++ `Windows()` builder declares as `z & ~0xF`: the four words
        // before an unaligned `z` are clobbered and the last words of the nominal run are not
        // written at all.
        let n = 16u32;
        let (x, y) = (source(n), addend(n));
        let z_odd = Z + 4; // one word past the boundary
        let mut g = guest();
        put(&mut g, X, &x);
        put(&mut g, Y, &y);
        poison(&mut g, Z, n + 8);
        poison(&mut g, W, n + 8);

        scale_add_with_copy(&mut g, n, Y, X, z_odd, W, 0.5).unwrap();

        // The run landed at Z, not at Z + 4.
        assert_eq!(
            get(&g, Z, n),
            model_z(&x, &y, 0.5, n),
            "the store floored its address"
        );
        // And the four words the caller asked for at the top of the run are untouched.
        for i in 0..4u32 {
            assert_eq!(
                g.u32(Z + 4 * n + 4 * i).unwrap(),
                0x7F7F_7F7F,
                "word {i} past the floored run"
            );
        }
        // `w` was aligned, so its copy is where the caller expects it.
        assert_eq!(get(&g, W, n), x);
    }

    #[test]
    fn the_addend_is_reached_through_two_deltas_and_not_through_its_own_base() {
        // `subf r9,r7,r5` then `lfsx f13,r9,r7`: the addend address is `(y - z) + (z - x + x_ptr)`.
        // That is the same value as `y + offset` on every input the game passes, and it is one
        // instruction in the lifted body rather than two — so it is written that way. What the test
        // can check is that the addend is genuinely read per sample from `y`, offset by the *sample
        // index*: a port that read `y[0]` every trip, or that indexed from the wrong base, fails.
        let n = 6u32;
        let x = vec![0.0f32; n as usize];
        let y: Vec<f32> = (0..n).map(|i| (i as f32) + 1.0).collect();
        let mut g = guest();
        put(&mut g, X_ODD, &x);
        put(&mut g, Y, &y);
        scale_add_with_copy(&mut g, n, Y, X_ODD, Z, W, 7.0).unwrap();
        // gain·0 + y[i] == y[i], so z is the addend run verbatim, in order.
        assert_eq!(get(&g, Z, n), y);
    }

    #[test]
    fn the_tail_starts_at_the_sample_the_vector_body_stopped_on() {
        // `rlwinm r11,r20,4,0,27` then `rlwinm ; add r11,r11,r6`: the tail's first source address is
        // `x + 4·16·blocks`. Aiming the two destinations at *disjoint* regions per half is the only
        // way to see the boundary directly, so this run gives the tail its own `z` and `w` by moving
        // the source: 17 samples from an aligned x writes sixteen by vector and one by tail, and the
        // seventeenth z word has to be the seventeenth sample, not the first.
        let n = 17u32;
        let (x, y) = (source(n), addend(n));
        let mut g = guest();
        put(&mut g, X, &x);
        put(&mut g, Y, &y);
        poison(&mut g, Z, n + 1);
        poison(&mut g, W, n + 1);
        scale_add_with_copy(&mut g, n, Y, X, Z, W, 0.5).unwrap();

        let expected = model_z(&x, &y, 0.5, 16);
        assert_eq!(
            g.f32(Z + 64).unwrap(),
            expected[16],
            "the tail's one sample"
        );
        assert_eq!(g.f32(W + 64).unwrap(), x[16]);
        assert_eq!(g.u32(Z + 68).unwrap(), 0x7F7F_7F7F, "and nothing past it");
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest();
        put(&mut g, X, &source(20));
        put(&mut g, Y, &addend(20));
        let before = crate::vmx::get_mxcsr();
        scale_add_with_copy(&mut g, 20, Y, X, Z, W, 0.5).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }
}
