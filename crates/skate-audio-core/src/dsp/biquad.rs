//! `sub_82B43AF8` — the mixer's biquad, eight samples per pass.
//!
//! ```text
//! y[k] = (x[k]*b0 + x[k-1]*b1 + x[k-2]*b2 + bias) - y[k-1]*a1 - y[k-2]*a2
//! ```
//!
//! `docs/ports.md`: **verified**, 1,152,252 calls a boot and 1,592,578 in a played session — the
//! heaviest single body in the whole candidate set. Zero divergence, zero skipped.
//!
//! This is **unit-tested against a verified reference**, the crate README's second kind of green:
//! the C++ was compared call-for-call against the original under the shadow harness on those real
//! inputs, and the Rust below has no recorded vectors of its own. A fault here is a transcription
//! error rather than a misreading of the filter.
//!
//! ## The three things that are not "just a biquad"
//!
//! **A bias is added to every feed-forward sum.** `lfs f9,432(r8)` off the audio pool, added after
//! the three taps and before the recursive half — the classic denormal-avoidance constant. It is
//! read live through the guest map, never folded in ([`DENORM_BIAS`]).
//!
//! **The first two samples of each block round their taps in a different order** from the other
//! six, because they reach into the carried history. Sample 0 is
//! `x1*b1 + (x2*b2 + x[0]*b0)`; sample 1 is `x1*b2 + (x[0]*b1 + x[1]*b0)`; samples 2..7 are
//! `x[k]*b0 + (x[k-2]*b2 + x[k-1]*b1)`. Those are three different association orders over the same
//! three products and they do not agree bit for bit. They are spelled out rather than folded into
//! one loop for that reason.
//!
//! **The recursive half uses `fnmsubs`, which is fused.** `y = c - a*b` in one rounding
//! ([`fp::nmsub_single`]); a multiply followed by a subtract rounds twice and is a different
//! filter. Two of them chain per sample, and each output is the next sample's `y[k-1]`, so the
//! chain cannot be reassociated at all.
//!
//! ## What is not translated, and why
//!
//! **A count that is not a nonzero multiple of eight returns an `Err`.** The original hands those
//! straight to `sub_82B43978` — same arguments, same write set, a generic four-at-a-time loop plus
//! a remainder. That function has **no `.inc` in `recomp/src/audio_ports/`**: it is not one of the
//! 216 audio-thread functions the sweep covered, so there is no verified reference for it in either
//! language and translating it would be new analysis dressed as a transcription. Returning an
//! `Err` that names it is the same choice [`crate::eval::dispatch`] makes for an unported opcode,
//! and for the same reason: inventing an answer would turn a gap in coverage into a wrong one.
//!
//! **The frame is not reproduced.** The C++ moves `r1` down by 192 and writes the back chain,
//! solely so that `sub_82B43978`'s `f31` spill at `-8(r1)` lands where the original put it. With
//! that call gone there is no callee to make room for, and the back chain is inside this function's
//! own frame, which no window declares and no comparison sees.
//!
//! ## One thing reproduced rather than fixed
//!
//! **The history is written back even when the loop never runs.** A span that wraps the address
//! space (`in >= in + 4*count`) skips the filter entirely and still stores the four values it just
//! loaded, through a full `lfs`/`stfs` round trip. That is a no-op in value for everything except
//! a signalling NaN, and it is still four stores, so it is kept.

use crate::vmx::Fpscr;
use crate::{Error, Guest, Result, fp};

/// `x[k-1]` carried in, `x[7]` carried out. `lfs f7,0(r3)`.
pub const HISTORY_X1: u32 = 0;
/// `x[k-2]` carried in, `x[6]` carried out. `lfs f4,4(r3)`.
pub const HISTORY_X2: u32 = 4;
/// `y[k-1]`. `lfs f6,8(r3)`.
pub const HISTORY_Y1: u32 = 8;
/// `y[k-2]`. `lfs f3,12(r3)`.
pub const HISTORY_Y2: u32 = 12;

/// Scales `y[k-1]`; subtracted. `lfs f0,0(r6)`.
pub const COEFF_A1: u32 = 0;
/// Scales `y[k-2]`; subtracted. `lfs f13,4(r6)`.
pub const COEFF_A2: u32 = 4;
/// Scales `x[k]`. `lfs f12,8(r6)`.
pub const COEFF_B0: u32 = 8;
/// Scales `x[k-1]`. `lfs f11,12(r6)`.
pub const COEFF_B1: u32 = 12;
/// Scales `x[k-2]`. `lfs f10,16(r6)`.
pub const COEFF_B2: u32 = 16;

/// The unrolled pass. The entry test is `count % 8`.
pub const BLOCK_SAMPLES: usize = 8;
/// 32 bytes of input and 32 of output per pass.
pub const BLOCK_BYTES: u32 = 4 * BLOCK_SAMPLES as u32;

const LIS_82300000: u32 = ((-32208i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82300000 == 0x8230_0000, "lis r11,-32208");

/// `lis -32208 ; addi -31232` — the audio pool `sub_82B2FEA8` and `sub_82B3C668` both name.
///
/// Computed as `((imm & 0xFFFF) << 16) + offset` and asserted below, never read off by eye: one
/// misread digit in `sub_82B2FE00` cost this project its first shadow divergence.
pub const POOL: u32 = LIS_82300000.wrapping_add(-31232i32 as u32);
const _: () = assert!(POOL == 0x822F_8600, "lis -32208 ; addi -31232");

/// `lfs f9,432(r8)` — added to every feed-forward sum. Read **live**, never folded in.
pub const DENORM_BIAS: u32 = POOL + 432;
const _: () = assert!(DENORM_BIAS == 0x822F_87B0, "lfs f9,432(r8)");

const _: () = assert!(HISTORY_Y2 == HISTORY_X1 + 12, "four adjacent singles at r3");
const _: () = assert!(COEFF_B2 == COEFF_A1 + 16, "five adjacent singles at r6");

/// The guest address of the generic path this port does not translate.
pub const GENERIC_BIQUAD: u32 = 0x82B4_3978;

/// `sub_82B43AF8` — filter `count` singles from `input` into `output`, carrying four singles of
/// state through `history`.
///
/// Arguments, by register: `history` is `r3`, `output` is `r4`, `input` is `r5`, `coeffs` is `r6`
/// and `count` is `r7`. Only the low words are ever used — every one of them is either an address
/// or a `rlwinm`'d span — so all five are `u32`. There is no return value (`kReturnNone`); `r3` on
/// exit still holds the history pointer.
///
/// **Writes** the four-single history at `history` on every path, and `[output, output + 4*count)`
/// when the filter runs. Reads `[input, input + 4*count)`, the five coefficients at `coeffs`, the
/// four history singles, and [`DENORM_BIAS`].
///
/// **In-place is safe and is why the load order is kept**: all eight inputs of a block are read
/// before any output of that block is written, so `output == input` behaves as the original does.
///
/// Returns `Err` naming [`GENERIC_BIQUAD`] when `count` is zero or not a multiple of eight — see
/// the module note.
pub fn biquad(
    g: &mut Guest,
    history: u32,
    output: u32,
    input: u32,
    coeffs: u32,
    count: u32,
) -> Result<()> {
    // clrlwi r11,r7,29 ; bne, then cmplwi cr6,r7,0 ; beq — anything but a nonzero multiple of
    // eight is handed to the generic routine with r3..r7 exactly as they arrived.
    if (count & 7) != 0 || count == 0 {
        return Err(Error::new(
            GENERIC_BIQUAD,
            "sub_82B43978 (the generic biquad) has no verified C++ body and is not ported: \
             count must be a nonzero multiple of 8",
        ));
    }

    // rlwinm r11,r7,2,0,29 ; add r11,r11,r5 — the byte span, and one past the last input word.
    let span = (count << 2) & 0xFFFF_FFFC;
    let in_end = span.wrapping_add(input);

    // Every load below happens before the branch in the original, and before any store on either
    // side of it.
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let a1 = fp::load_single(g, coeffs.wrapping_add(COEFF_A1))?; // lfs f0,0(r6)
    let a2 = fp::load_single(g, coeffs.wrapping_add(COEFF_A2))?; // lfs f13,4(r6)
    let b0 = fp::load_single(g, coeffs.wrapping_add(COEFF_B0))?; // lfs f12,8(r6)
    let b1 = fp::load_single(g, coeffs.wrapping_add(COEFF_B1))?; // lfs f11,12(r6)
    let b2 = fp::load_single(g, coeffs.wrapping_add(COEFF_B2))?; // lfs f10,16(r6)
    let mut x1 = fp::load_single(g, history.wrapping_add(HISTORY_X1))?; // lfs f7,0(r3)
    let mut x2 = fp::load_single(g, history.wrapping_add(HISTORY_X2))?; // lfs f4,4(r3)
    let mut y1 = fp::load_single(g, history.wrapping_add(HISTORY_Y1))?; // lfs f6,8(r3)
    let mut y2 = fp::load_single(g, history.wrapping_add(HISTORY_Y2))?; // lfs f3,12(r3)

    // cmplw cr6,r5,r11 ; bge 0x82b43c8c — a span that wraps the address space skips the whole loop
    // and writes the history back untouched. It cannot be empty here: the count is a nonzero
    // multiple of eight.
    if input < in_end {
        // addi r9,r11,-1 ; rlwinm r9,r9,27,5,31 ; addi r9,r9,1 ; mtctr r9
        let blocks = ((span - 1) >> 5) + 1;
        let bias = fp::load_single(g, DENORM_BIAS)?; // lfs f9,432(r8)
        let mut src = input; // addi r11,r5,-4, then read through +4(r11) .. lfsu 32(r11)
        let mut dst = output; // addi r10,r4,-4, then written through +4(r10) .. stfsu 32(r10)
        for _ in 0..blocks {
            fpscr.disable_flush_mode_unconditional();
            // All eight inputs are read before any output is written, which is what lets a caller
            // run this in place. The order is kept for that reason, not for the values.
            let mut x = [0f64; BLOCK_SAMPLES];
            for (k, slot) in x.iter_mut().enumerate() {
                *slot = fp::load_single(g, src.wrapping_add(4 * k as u32))?;
            }
            src = src.wrapping_add(BLOCK_BYTES);

            // The feed-forward half: fmuls, two fmadds, fadds per sample. The first two samples
            // reach into the history and associate their three taps differently from the other six.
            let mut t = [0f64; BLOCK_SAMPLES];
            t[0] = fp::add_single(
                fp::fmadd_single(x1, b1, fp::fmadd_single(x2, b2, fp::mul_single(x[0], b0))),
                bias,
            );
            t[1] = fp::add_single(
                fp::fmadd_single(x1, b2, fp::fmadd_single(x[0], b1, fp::mul_single(x[1], b0))),
                bias,
            );
            for k in 2..BLOCK_SAMPLES {
                t[k] = fp::add_single(
                    fp::fmadd_single(
                        x[k],
                        b0,
                        fp::fmadd_single(x[k - 2], b2, fp::mul_single(x[k - 1], b1)),
                    ),
                    bias,
                );
            }

            // The recursive half: two fnmsubs and a store per sample, in order.
            for (k, tk) in t.iter().enumerate() {
                let y = fp::nmsub_single(y2, a2, fp::nmsub_single(y1, a1, *tk));
                fp::store_single(g, dst.wrapping_add(4 * k as u32), y)?; // stfs / stfsu
                y2 = y1;
                y1 = y;
            }
            dst = dst.wrapping_add(BLOCK_BYTES);
            x2 = x[BLOCK_SAMPLES - 2]; // fmr f4,f8 — x[6] becomes the next pass's x[k-2]
            x1 = x[BLOCK_SAMPLES - 1]; // fmr f7,f5 — x[7] becomes its x[k-1]
        }
    }

    // loc_82B43C8C: the history written back, on both paths.
    fpscr.disable_flush_mode_unconditional();
    fp::store_single(g, history.wrapping_add(HISTORY_X1), x1)?; // stfs f7,0(r3)
    fp::store_single(g, history.wrapping_add(HISTORY_X2), x2)?; // stfs f4,4(r3)
    fp::store_single(g, history.wrapping_add(HISTORY_Y1), y1)?; // stfs f6,8(r3)
    fp::store_single(g, history.wrapping_add(HISTORY_Y2), y2)?; // stfs f3,12(r3)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const HISTORY: u32 = BASE;
    const COEFFS: u32 = BASE + 0x40;
    const SRC: u32 = BASE + 0x100;
    const DST: u32 = BASE + 0x400;

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x800);
        g.put(DENORM_BIAS, vec![0u8; 4]);
        g
    }

    fn put(g: &mut Guest, at: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            g.set_u32(at + 4 * i as u32, v.to_bits()).unwrap();
        }
    }

    fn get(g: &Guest, at: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| g.f32(at + 4 * i as u32).unwrap()).collect()
    }

    fn coeffs(g: &mut Guest, a1: f32, a2: f32, b0: f32, b1: f32, b2: f32) {
        put(g, COEFFS, &[a1, a2, b0, b1, b2]);
    }

    fn history(g: &mut Guest, x1: f32, x2: f32, y1: f32, y2: f32) {
        put(g, HISTORY, &[x1, x2, y1, y2]);
    }

    fn bias(g: &mut Guest, v: f32) {
        g.set_u32(DENORM_BIAS, v.to_bits()).unwrap();
    }

    /// A per-sample model, written from the difference equation rather than from the loop
    /// structure — but with the three association orders the original uses, because those are part
    /// of the answer and not of the plumbing.
    ///
    /// **It therefore shares those orders with the port by construction**, so what it checks is the
    /// structure — which tap goes where, how the history is carried across a block boundary, that
    /// every sample is written once — and not the rounding order itself. The rounding order is
    /// pinned separately by `sample_zero_associates_its_three_taps_in_the_lifted_order` and by
    /// `the_recursive_half_is_subtracted_and_fused`, which is where a transcription that got it
    /// wrong would actually be caught.
    ///
    /// Everything arrives as `f32` and is widened here. Passing `f64` literals instead is how the
    /// first version of this test disagreed with a correct port: `0.6f64` is not `0.6f32 as f64`.
    fn model(input: &[f32], c: [f32; 5], bias: f32, hist: [f32; 4]) -> (Vec<f32>, [f64; 4]) {
        let [a1, a2, b0, b1, b2] = [
            c[0] as f64,
            c[1] as f64,
            c[2] as f64,
            c[3] as f64,
            c[4] as f64,
        ];
        let bias = bias as f64;
        let (mut x1, mut x2, mut y1, mut y2) = (
            hist[0] as f64,
            hist[1] as f64,
            hist[2] as f64,
            hist[3] as f64,
        );
        let mut out = Vec::new();
        for block in input.chunks(BLOCK_SAMPLES) {
            let x: Vec<f64> = block.iter().map(|v| *v as f64).collect();
            let mut t = vec![0f64; x.len()];
            t[0] = fp::add_single(
                fp::fmadd_single(x1, b1, fp::fmadd_single(x2, b2, fp::mul_single(x[0], b0))),
                bias,
            );
            t[1] = fp::add_single(
                fp::fmadd_single(x1, b2, fp::fmadd_single(x[0], b1, fp::mul_single(x[1], b0))),
                bias,
            );
            for k in 2..x.len() {
                t[k] = fp::add_single(
                    fp::fmadd_single(
                        x[k],
                        b0,
                        fp::fmadd_single(x[k - 2], b2, fp::mul_single(x[k - 1], b1)),
                    ),
                    bias,
                );
            }
            for tk in &t {
                let y = fp::nmsub_single(y2, a2, fp::nmsub_single(y1, a1, *tk));
                out.push(y as f32);
                y2 = y1;
                y1 = y;
            }
            x2 = x[x.len() - 2];
            x1 = x[x.len() - 1];
        }
        (out, [x1, x2, y1, y2])
    }

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i % 13) as f32) * 0.125 - 0.5).collect()
    }

    #[test]
    fn one_block_matches_the_difference_equation() {
        let mut g = guest();
        coeffs(&mut g, -1.5, 0.6, 0.25, 0.5, 0.25);
        history(&mut g, 0.0, 0.0, 0.0, 0.0);
        bias(&mut g, 1.0e-20);
        let x = ramp(8);
        put(&mut g, SRC, &x);

        biquad(&mut g, HISTORY, DST, SRC, COEFFS, 8).unwrap();

        let (expected, h) = model(
            &x,
            [-1.5, 0.6, 0.25, 0.5, 0.25],
            1.0e-20,
            [0.0, 0.0, 0.0, 0.0],
        );
        assert_eq!(get(&g, DST, 8), expected);
        // And the history carries out the right four values: x[7], x[6], y[7], y[6].
        assert_eq!(g.f32(HISTORY + HISTORY_X1).unwrap(), h[0] as f32);
        assert_eq!(g.f32(HISTORY + HISTORY_X2).unwrap(), h[1] as f32);
        assert_eq!(g.f32(HISTORY + HISTORY_Y1).unwrap(), h[2] as f32);
        assert_eq!(g.f32(HISTORY + HISTORY_Y2).unwrap(), h[3] as f32);
    }

    #[test]
    fn many_blocks_chain_through_the_history_registers() {
        // Four blocks in one call must equal four calls of one block each: that is the only thing
        // that pins the `x2 = x[6]; x1 = x[7]` carry and the running `y1`/`y2` at a block boundary.
        let c = [-1.2f32, 0.45, 0.3, -0.2, 0.1];
        let x = ramp(32);

        let mut whole = guest();
        coeffs(&mut whole, -1.2, 0.45, 0.3, -0.2, 0.1);
        history(&mut whole, 0.75, -0.25, 0.5, -0.5);
        bias(&mut whole, 0.0);
        put(&mut whole, SRC, &x);
        biquad(&mut whole, HISTORY, DST, SRC, COEFFS, 32).unwrap();

        let mut piece = guest();
        coeffs(&mut piece, -1.2, 0.45, 0.3, -0.2, 0.1);
        history(&mut piece, 0.75, -0.25, 0.5, -0.5);
        bias(&mut piece, 0.0);
        for b in 0..4u32 {
            put(&mut piece, SRC, &x[8 * b as usize..8 * b as usize + 8]);
            biquad(&mut piece, HISTORY, DST + 32 * b, SRC, COEFFS, 8).unwrap();
        }

        assert_eq!(get(&whole, DST, 32), get(&piece, DST, 32));
        assert_eq!(get(&whole, HISTORY, 4), get(&piece, HISTORY, 4));
        let (expected, _) = model(&x, c, 0.0, [0.75, -0.25, 0.5, -0.5]);
        assert_eq!(get(&whole, DST, 32), expected);
    }

    #[test]
    fn the_first_two_samples_use_the_carried_history_and_the_rest_do_not() {
        // With b0 = b1 = 0 and only b2 live, output sample 0 is x[k-2]*b2 — which for k = 0 is the
        // *history's* x2 and for k = 2 is x[0]. Changing the history must move exactly the first
        // two outputs and nothing after them.
        let run = |x2: f32, x1: f32| {
            let mut g = guest();
            coeffs(&mut g, 0.0, 0.0, 0.0, 0.0, 1.0);
            history(&mut g, x1, x2, 0.0, 0.0);
            bias(&mut g, 0.0);
            put(&mut g, SRC, &ramp(8));
            biquad(&mut g, HISTORY, DST, SRC, COEFFS, 8).unwrap();
            get(&g, DST, 8)
        };
        let a = run(3.0, 5.0);
        let b = run(-3.0, 5.0);
        assert_ne!(a[0], b[0], "sample 0's x[k-2] is the history's x2");
        assert_eq!(
            a[1], b[1],
            "sample 1's x[k-2] is the history's x1, which did not move"
        );
        assert_eq!(a[2..], b[2..], "samples 2..7 read only this block's inputs");

        let c = run(3.0, -5.0);
        assert_eq!(a[0], c[0]);
        assert_ne!(a[1], c[1], "and sample 1 moves when x1 does");
        assert_eq!(a[2..], c[2..]);
    }

    #[test]
    fn the_recursive_half_is_subtracted_and_fused() {
        // Two properties in one input. The a-taps are *subtracted* (fnmsub, not fmadd): with
        // a1 = 1 and a feed-forward of 1 per sample, the outputs alternate 1, 0, 1, 0 rather than
        // running away. And the fusion is visible where the product needs 48 bits.
        let mut g = guest();
        coeffs(&mut g, 1.0, 0.0, 1.0, 0.0, 0.0);
        history(&mut g, 0.0, 0.0, 0.0, 0.0);
        bias(&mut g, 0.0);
        put(&mut g, SRC, &[1.0f32; 8]);
        biquad(&mut g, HISTORY, DST, SRC, COEFFS, 8).unwrap();
        assert_eq!(
            get(&g, DST, 8),
            vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]
        );

        // The fusion. y[0] = t - y1*a1 with y1*a1 needing 48 bits and t their f32 rounding: a
        // single-rounded fnmsubs returns the discarded low bits, a multiply-then-subtract zero.
        let a = 1.0f32 + f32::EPSILON;
        let b = 1.0f32 - f32::EPSILON;
        let rounded = a * b;
        let mut h = guest();
        coeffs(&mut h, b, 0.0, 1.0, 0.0, 0.0); // a1 = b, everything else inert
        history(&mut h, 0.0, 0.0, a, 0.0); // y1 = a
        bias(&mut h, 0.0);
        put(&mut h, SRC, &[rounded, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        biquad(&mut h, HISTORY, DST, SRC, COEFFS, 8).unwrap();
        let fused = fp::nmsub_single(a as f64, b as f64, rounded as f64) as f32;
        assert_ne!(fused, 0.0, "the distinguishing input has to distinguish");
        assert_eq!(h.f32(DST).unwrap(), fused);
    }

    #[test]
    fn sample_zero_associates_its_three_taps_in_the_lifted_order() {
        // The one property `model` cannot check, because it shares the order by construction.
        //
        // Sample 0 is `fmadds(x1, b1, fmadds(x2, b2, fmuls(x[0], b0)))`. The obvious
        // mistranscription is to give it the *uniform* shape samples 2..7 have — which uses the
        // same three products, in the same three roles, and differs only in which one gets rounded
        // to a single first. On most inputs the two agree; this searches a deterministic set for
        // one where they do not, and then pins the port to the lifted answer.
        let mut xorshift = 0x1234_5678u32;
        let mut next = || {
            xorshift ^= xorshift << 13;
            xorshift ^= xorshift >> 17;
            xorshift ^= xorshift << 5;
            // Exponents around 1.0, so nothing overflows or goes denormal.
            f32::from_bits((xorshift & 0x007F_FFFF) | 0x3F80_0000)
                * if xorshift & 1 == 0 { 1.0 } else { -1.0 }
        };

        let mut found = 0;
        for _ in 0..4000 {
            let (x0, x1, x2) = (next(), next(), next());
            let (b0, b1, b2) = (next(), next(), next());
            let lifted = fp::fmadd_single(
                x1 as f64,
                b1 as f64,
                fp::fmadd_single(x2 as f64, b2 as f64, fp::mul_single(x0 as f64, b0 as f64)),
            );
            let uniform = fp::fmadd_single(
                x0 as f64,
                b0 as f64,
                fp::fmadd_single(x2 as f64, b2 as f64, fp::mul_single(x1 as f64, b1 as f64)),
            );
            if lifted as f32 == uniform as f32 {
                continue;
            }
            found += 1;

            let mut g = guest();
            coeffs(&mut g, 0.0, 0.0, b0, b1, b2); // a1 = a2 = 0, so y[0] == t[0]
            history(&mut g, x1, x2, 0.0, 0.0);
            bias(&mut g, 0.0);
            put(&mut g, SRC, &[x0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            biquad(&mut g, HISTORY, DST, SRC, COEFFS, 8).unwrap();

            assert_eq!(
                g.f32(DST).unwrap(),
                lifted as f32,
                "x {x0} {x1} {x2}, b {b0} {b1} {b2}"
            );
            assert_ne!(
                g.f32(DST).unwrap(),
                uniform as f32,
                "the two orders have to differ here"
            );
            if found == 8 {
                break;
            }
        }
        assert!(
            found > 0,
            "no distinguishing input found: this test would prove nothing"
        );
    }

    #[test]
    fn the_bias_is_read_live_and_added_to_every_sample() {
        // Not folded in, not assumed zero: change the cell and every output moves by it.
        let run = |value: f32| {
            let mut g = guest();
            coeffs(&mut g, 0.0, 0.0, 0.0, 0.0, 0.0); // all taps dead, so y[k] == bias
            history(&mut g, 0.0, 0.0, 0.0, 0.0);
            bias(&mut g, value);
            put(&mut g, SRC, &ramp(16));
            biquad(&mut g, HISTORY, DST, SRC, COEFFS, 16).unwrap();
            get(&g, DST, 16)
        };
        assert_eq!(run(0.0), vec![0.0f32; 16]);
        assert_eq!(run(0.25), vec![0.25f32; 16], "every sample, both blocks");
        assert_eq!(run(-1.5), vec![-1.5f32; 16]);
    }

    #[test]
    fn it_runs_in_place_because_a_block_is_read_before_it_is_written() {
        // r4 == r5 is the caller's normal case. All eight inputs of a block are loaded before any
        // of its eight outputs is stored, so filtering in place gives the same answer as out of
        // place. Hoisting a store above a load inside the block would break exactly this.
        let x = ramp(24);
        let c = [-1.7f32, 0.72, 0.4, 0.8, 0.4];

        let mut apart = guest();
        coeffs(&mut apart, -1.7, 0.72, 0.4, 0.8, 0.4);
        history(&mut apart, 0.1, 0.2, 0.3, 0.4);
        bias(&mut apart, 0.0);
        put(&mut apart, SRC, &x);
        biquad(&mut apart, HISTORY, DST, SRC, COEFFS, 24).unwrap();

        let mut place = guest();
        coeffs(&mut place, -1.7, 0.72, 0.4, 0.8, 0.4);
        history(&mut place, 0.1, 0.2, 0.3, 0.4);
        bias(&mut place, 0.0);
        put(&mut place, SRC, &x);
        biquad(&mut place, HISTORY, SRC, SRC, COEFFS, 24).unwrap();

        assert_eq!(get(&apart, DST, 24), get(&place, SRC, 24));
        let (expected, _) = model(&x, c, 0.0, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(get(&place, SRC, 24), expected);
    }

    #[test]
    fn a_count_that_is_not_a_nonzero_multiple_of_eight_is_an_error_and_writes_nothing() {
        for count in [0u32, 1, 7, 9, 15, 255] {
            let mut g = guest();
            coeffs(&mut g, 1.0, 1.0, 1.0, 1.0, 1.0);
            history(&mut g, 9.0, 9.0, 9.0, 9.0);
            put(&mut g, SRC, &ramp(256));
            put(&mut g, DST, &vec![7.0f32; 256]);

            let e = biquad(&mut g, HISTORY, DST, SRC, COEFFS, count).unwrap_err();
            assert_eq!(e.address, GENERIC_BIQUAD, "count {count}");

            assert_eq!(
                get(&g, HISTORY, 4),
                vec![9.0f32; 4],
                "count {count}: history untouched"
            );
            assert_eq!(
                get(&g, DST, 8),
                vec![7.0f32; 8],
                "count {count}: output untouched"
            );
        }
        // 8 and 16 are fine, so the test above is not simply refusing everything.
        let mut g = guest();
        coeffs(&mut g, 0.0, 0.0, 0.0, 0.0, 0.0);
        history(&mut g, 0.0, 0.0, 0.0, 0.0);
        put(&mut g, SRC, &ramp(16));
        assert!(biquad(&mut g, HISTORY, DST, SRC, COEFFS, 8).is_ok());
        assert!(biquad(&mut g, HISTORY, DST, SRC, COEFFS, 16).is_ok());
    }

    #[test]
    fn a_span_that_wraps_the_address_space_filters_nothing() {
        // `cmplw cr6,r5,r11 ; bge` — when `in + 4*count` wraps back below `in`, the whole loop is
        // skipped. Nothing is filtered and nothing is written to the destination.
        //
        // **What this test cannot establish**, and it is worth being explicit rather than letting a
        // reader assume otherwise: the original *also* stores the four history singles back on this
        // path, through the same `lfs`/`stfs` round trip they were loaded with. That store is a
        // no-op in value — the only input for which it is not is a signalling NaN, and Rust's
        // `as` casts on this target leave sNaN payloads alone rather than quieting them the way
        // the guest's widening load does (measured, not assumed). So removing those four stores
        // leaves every test in this crate passing. They are reproduced because the original has
        // them, and they join the crate README's list of writes no single-threaded comparison can
        // see in either direction.
        let mut g = Guest::single(0xFFFF_F000, 0x1000);
        g.put(DENORM_BIAS, vec![0u8; 4]);
        let history_at = 0xFFFF_F000u32;
        let coeffs_at = 0xFFFF_F040u32;
        let output_at = 0xFFFF_F100u32;
        let input_at = 0xFFFF_FF00u32; // + 4*256 wraps to 0xFFFF_FF00 + 0x400 == 0x0000_0300
        for i in 0..4u32 {
            g.set_u32(history_at + 4 * i, (1.0f32 + i as f32).to_bits())
                .unwrap();
        }
        for i in 0..5u32 {
            g.set_u32(coeffs_at + 4 * i, 1.0f32.to_bits()).unwrap();
        }
        for i in 0..8u32 {
            g.set_u32(output_at + 4 * i, 0x7F7F_7F7F).unwrap();
        }
        assert!(
            input_at.wrapping_add(1024) < input_at,
            "the span has to wrap for this to test it"
        );

        biquad(&mut g, history_at, output_at, input_at, coeffs_at, 256).unwrap();

        for i in 0..8u32 {
            assert_eq!(
                g.u32(output_at + 4 * i).unwrap(),
                0x7F7F_7F7F,
                "word {i} was filtered"
            );
        }
        // And the history carries through unchanged, which is what the four stores leave behind.
        for i in 0..4u32 {
            assert_eq!(g.f32(history_at + 4 * i).unwrap(), 1.0 + i as f32);
        }
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest();
        coeffs(&mut g, 0.5, 0.5, 0.5, 0.5, 0.5);
        history(&mut g, 0.0, 0.0, 0.0, 0.0);
        put(&mut g, SRC, &ramp(16));
        let before = crate::vmx::get_mxcsr();
        biquad(&mut g, HISTORY, DST, SRC, COEFFS, 16).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }
}
