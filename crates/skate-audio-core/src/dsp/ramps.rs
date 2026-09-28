//! The two envelope ramp writers `sub_82B238A8` dispatches to: a linear ramp (`sub_82B427D8`) and a
//! square-root ramp (`sub_82B42C98`). Each fills one 256-entry block of a float array.
//!
//! Ported from `recomp/src/audio_ports/sub_82B427D8.inc` (**STATUS: thin**, four calls a session)
//! and `sub_82B42C98.inc` (**STATUS: verified**, 62,842 calls at boot).
//!
//! Both take `out` in `r3`, the block's first index `first` in `r6` (it may be negative), the ramp
//! length in `r7`, `start` in `f1` and `end` in `f2`. The block ends at `first + 255` and the ramp at
//! `last = min(first + 255, length - 1)`. Below index zero the block is flat at `start`, past `last`
//! flat at `end`, and between them the ramp runs four samples to a vector where it can:
//!
//! - linear: `fma(i + 1, step, start)`, `step = (end - start) / length`;
//! - square root: `fma(sqrt(i + 1), step, start)`, `step = (end - start) / sqrt(length)`, or on a
//!   descending ramp `end - sqrt(length - (i + 1)) * step`. The vector root is `vrsqrtefp` refined by
//!   one Newton step, with a mask that passes `+0` and `+inf` through, so it is not a correctly
//!   rounded square root; the scalar tail's is.
//!
//! **The alignment padding after the flat lead-in is linear in both writers**, the square-root one
//! included: those few samples are `fma(i + 1, step, start)` with no root. Reproduced as found.
//!
//! The flat fills store 16-byte units four at a time, then one unit replicated forward over the rest
//! by [`crate::mem::memcpy_chunked`]. Every vector store masks its address to 16 bytes and the output
//! is never checked for alignment, as in the original. The square-root writer also leaves `v29..v31`
//! as its last loop iteration left them; this crate has no register file, so that is not modelled.

use core::arch::x86_64::*;

use crate::vmx::{self, Fpscr};
use crate::{Guest, Result, fp, mem};

/// `lis -32234 ; lfs 23056` — compared with the delta to pick the ascending or descending form.
pub const RAMP_THRESHOLD: u32 = crate::leaves::ZERO_CELL;
/// `lis -32206 ; addi -17776` — the four-lane index stride vector.
pub const RAMP_INDEX_STRIDE: u32 = (((-32206i32 as u32) & 0xFFFF) << 16).wrapping_sub(17776);
/// `lis -32233 ; lfs -8480` — the square-root writer's descending coefficient sign.
pub const RAMP_DESCENDING_SCALE: u32 = (((-32233i32 as u32) & 0xFFFF) << 16).wrapping_sub(8480);
const _: () = assert!(RAMP_INDEX_STRIDE == 0x8231_BA90 && RAMP_DESCENDING_SCALE == 0x8216_DEE0);

/// `srawi rX,rY,2 ; addze` — a signed divide by four, truncating toward zero.
fn div_four(v: i32) -> i32 {
    let carry = v < 0 && (v as u32 & 3) != 0;
    (v >> 2) + i32::from(carry)
}

/// `div_four` then `rlwinm rX,rY,2,0,29` — rounded toward zero to a multiple of four.
fn aligned4(v: i32) -> i32 {
    ((div_four(v) as u32) << 2) as i32
}

/// `extsw ; std ; lfd ; fcfid ; frsp`.
fn int_to_single(v: i32) -> f64 {
    f64::from(f64::from(v) as f32)
}

/// The inline 16-byte-unit fill, emitted twice in each writer: `units` blocks of 16 bytes from
/// `dst`, four vector stores at a time while whole groups remain, then one store replicated over the
/// rest by a forward copy. Nothing for `units <= 0`.
unsafe fn fill_vector_units(g: &mut Guest, dst: u32, units: i32, value: __m128) -> Result<()> {
    // SAFETY: the callers check vmx support; every guest access is bounds-checked.
    unsafe {
        let bulk = aligned4(units);
        if bulk > 0 {
            let mut p = dst.wrapping_add(32);
            let passes = ((bulk as u32).wrapping_sub(1) >> 2) + 1;
            for _ in 0..passes {
                vmx::stvx128_ps(g, p.wrapping_sub(32), value)?;
                vmx::stvx128_ps(g, p.wrapping_sub(16), value)?;
                vmx::stvx128_ps(g, p, value)?;
                vmx::stvx128_ps(g, p.wrapping_add(16), value)?;
                p = p.wrapping_add(64);
            }
        }
        if bulk < units {
            let tail = dst.wrapping_add((bulk as u32) << 4);
            let rest = (units.wrapping_sub(bulk) as u32) << 4;
            vmx::stvx128_ps(g, tail, value)?;
            let len = rest.wrapping_sub(9) & 0xFFFF_FFF8; // addi r9,r11,-9 ; rlwinm r5,r9,0,0,28
            mem::memcpy_chunked(g, tail.wrapping_add(16), tail, u64::from(len))?; // bl 0x82f52fb8
        }
        Ok(())
    }
}

/// What both writers do before their vector bodies: the flat `start` lead-in below index zero, then
/// the linear alignment padding. Returns the index and cursor the bodies start from. `threshold` is
/// `None` for the linear writer, which loads it only on this path.
#[allow(clippy::too_many_arguments)]
unsafe fn lead_in(
    g: &mut Guest,
    fpscr: &mut Fpscr,
    out: u32,
    first: i32,
    last: i32,
    ramp: (f64, f64, f64, f64, f64),
    threshold: Option<f64>,
) -> Result<(i32, u32)> {
    // SAFETY: the callers check vmx support; every guest access is bounds-checked.
    unsafe {
        let (start, end, step, length_f, delta) = ramp;
        let (mut index, mut cursor) = (first, out);
        if index >= 0 {
            return Ok((index, cursor));
        }
        let below = 0u32.wrapping_sub(index as u32) as i32; // neg r11,r30
        let leftover = below.wrapping_sub(aligned4(below));
        let mut pad = if leftover == 0 { 0 } else { 4 - leftover }; // subfic ; subfe ; and
        let units = div_four(index).wrapping_neg(); // srawi ; addze ; neg
        fill_vector_units(g, cursor, units, _mm_set1_ps(start as f32))?;
        index = (index as u32).wrapping_add((units as u32) << 2) as i32;
        cursor = cursor.wrapping_add((units as u32) << 4);
        if index < 0 {
            // Fewer than four below zero: one store and a replicating copy.
            fpscr.disable_flush_mode_unconditional();
            fp::store_single(g, cursor, start)?;
            let left = 0u32.wrapping_sub(index as u32);
            let len = (left << 2).wrapping_sub(1) & 0xFFFF_FFFC; // 4*left - 4
            mem::memcpy_chunked(g, cursor.wrapping_add(4), cursor, u64::from(len))?;
            index = (index as u32).wrapping_add(left) as i32;
            cursor = cursor.wrapping_add(left << 2);
        }
        fpscr.disable_flush_mode_unconditional();
        let threshold = match threshold {
            Some(t) => t,
            None => fp::load_single(g, RAMP_THRESHOLD)?, // lfs f0,23056(r11)
        };
        // fcmpu ; blt -- a NaN delta takes the ascending form.
        let ascending = !(delta < threshold);
        if index <= last {
            let mut n = (index as u32).wrapping_add(1) as i32;
            while pad > 0 {
                fpscr.disable_flush_mode_unconditional();
                let value = if ascending {
                    fp::fmadd_single(int_to_single(n), step, start) // fmadds -- linear, no root
                } else {
                    let back = fp::sub_single(length_f, int_to_single(n)); // fsubs
                    fp::nmsub_single(back, step, end) // fnmsubs
                };
                index = index.wrapping_add(1);
                n = n.wrapping_add(1);
                pad -= 1;
                fp::store_single(g, cursor, value)?;
                cursor = cursor.wrapping_add(4);
                if index > last {
                    break;
                }
            }
        }
        Ok((index, cursor))
    }
}

/// Past the ramp: single `end` entries until the count left is a multiple of four, then the
/// 16-byte fill for the unit count taken **before** that loop trimmed it, as the original's `r5`.
unsafe fn trailing_fill(
    g: &mut Guest,
    fpscr: &mut Fpscr,
    mut cursor: u32,
    mut tail_index: i32,
    block_last: i32,
    end: f64,
) -> Result<()> {
    // SAFETY: the callers check vmx support; every guest access is bounds-checked.
    unsafe {
        if tail_index > block_last {
            return Ok(());
        }
        let mut remaining = (block_last as u32)
            .wrapping_sub(tail_index as u32)
            .wrapping_add(1) as i32;
        let units = div_four(remaining);
        if remaining != aligned4(remaining) {
            while tail_index <= block_last {
                remaining -= 1;
                fpscr.disable_flush_mode_unconditional();
                fp::store_single(g, cursor, end)?;
                cursor = cursor.wrapping_add(4);
                tail_index = tail_index.wrapping_add(1);
                if remaining == aligned4(remaining) {
                    break;
                }
            }
        }
        fpscr.disable_flush_mode_unconditional();
        fill_vector_units(g, cursor, units, _mm_set1_ps(end as f32))
    }
}

/// `stfs float(i+1..i+4)` into the frame, read back by `lvx128`: the lane reversal puts `i + 4` in
/// lane 0.
unsafe fn index_vector(index: i32) -> __m128 {
    // SAFETY: the callers check vmx support; every guest access is bounds-checked.
    unsafe {
        let i0 = index as u32;
        let f = |k: u32| int_to_single(i0.wrapping_add(k) as i32) as f32;
        _mm_setr_ps(f(4), f(3), f(2), f(1))
    }
}

/// The ramp span in whole vectors, and those taken four at a time.
fn groups_for(last: i32, index: i32) -> (i32, i32) {
    let span = (last as u32).wrapping_sub(index as u32).wrapping_add(1) as i32;
    let groups = div_four(span);
    (groups, aligned4(groups))
}

fn block_bounds(first: i32, length: i32) -> (i32, i32) {
    let block_last = (first as u32).wrapping_add(255) as i32; // addi r26,r6,255
    let ramp_last = (length as u32).wrapping_sub(1) as i32; // addi r11,r7,-1
    (
        block_last,
        if block_last > ramp_last {
            ramp_last
        } else {
            block_last
        },
    )
}

/// The linear ramp writer (`sub_82B427D8`). Returns 1.
pub fn linear_ramp(
    g: &mut Guest,
    out: u32,
    first: i32,
    length: i32,
    start: f64,
    end: f64,
) -> Result<u64> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    let (block_last, last) = block_bounds(first, length);
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let delta = fp::sub_single(end, start); // fsubs f28,f2,f1
    let length_f = int_to_single(length); // fcfid ; frsp f29
    let step = fp::div_single(delta, length_f); // fdivs f31,f28,f29
    // SAFETY: vmx support was checked above; guest accesses are bounds-checked.
    unsafe {
        let ramp = (start, end, step, length_f, delta);
        let (index, mut cursor) = lead_in(g, &mut fpscr, out, first, last, ramp, None)?;
        fpscr.disable_flush_mode_unconditional();
        let step_v = _mm_set1_ps(step as f32);
        let start_v = _mm_set1_ps(start as f32);
        let mut idx = index_vector(index);
        let stride = vmx::lvx128_ps(g, RAMP_INDEX_STRIDE)?; // lvx128 v63,r0,r4
        let (groups, bulk) = groups_for(last, index);
        if bulk > 0 {
            for _ in 0..((bulk as u32 - 1) >> 2) + 1 {
                fpscr.enable_flush_mode_unconditional();
                let idx1 = _mm_add_ps(idx, stride);
                let v0 = vmx::vmaddfp(step_v, idx, start_v); // step first, index second, as lifted
                let idx2 = _mm_add_ps(idx1, stride);
                let v1 = vmx::vmaddfp(step_v, idx1, start_v);
                vmx::stvx128_ps(g, cursor, v0)?;
                let idx3 = _mm_add_ps(idx2, stride);
                let v2 = vmx::vmaddfp(step_v, idx2, start_v);
                vmx::stvx128_ps(g, cursor.wrapping_add(16), v1)?;
                let v3 = vmx::vmaddfp(step_v, idx3, start_v);
                vmx::stvx128_ps(g, cursor.wrapping_add(32), v2)?;
                idx = _mm_add_ps(idx3, stride);
                vmx::stvx128_ps(g, cursor.wrapping_add(48), v3)?;
                cursor = cursor.wrapping_add(64);
            }
        }
        if bulk < groups {
            for _ in 0..(groups - bulk) as u32 {
                fpscr.enable_flush_mode_unconditional();
                let v = vmx::vmaddfp(step_v, idx, start_v);
                idx = _mm_add_ps(idx, stride);
                vmx::stvx128_ps(g, cursor, v)?;
                cursor = cursor.wrapping_add(16);
            }
        }
        let mut tail_index = (index as u32).wrapping_add((groups as u32) << 2) as i32;
        if tail_index <= last {
            let passes = (last.wrapping_sub(tail_index) as u32).wrapping_add(1);
            let mut n = (tail_index as u32).wrapping_add(1) as i32;
            tail_index = (tail_index as u32).wrapping_add(passes) as i32;
            for _ in 0..passes {
                fpscr.disable_flush_mode_unconditional();
                let value = fp::fmadd_single(int_to_single(n), step, start); // fmadds f11,f12,f31,f30
                n = n.wrapping_add(1);
                fp::store_single(g, cursor, value)?;
                cursor = cursor.wrapping_add(4);
            }
        }
        trailing_fill(g, &mut fpscr, cursor, tail_index, block_last, end)?;
    }
    Ok(1) // li r3,1
}

/// `vspltisw128 1 ; vcsxwfp128 v0,v61,1` — 0.5 in every lane, kept as the lifted expression.
unsafe fn half_vector() -> __m128 {
    // SAFETY: the callers check vmx support; every guest access is bounds-checked.
    unsafe {
        _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_set1_epi32(1)),
            _mm_castsi128_ps(_mm_set1_epi32(0x3F00_0000)),
        )
    }
}

/// The vector square root: the `vrsqrtefp` estimate, one Newton step, and a mask selecting `x` in
/// exactly the lanes where the estimate cannot be refined (`+0` and `+inf`, their own roots). Every
/// multiply is separate and both multiply-adds round twice; the operand order is the lifted one.
unsafe fn sqrt_vector(x: __m128, half: __m128) -> __m128 {
    // SAFETY: the callers check vmx support; every guest access is bounds-checked.
    unsafe {
        let e = vmx::vrsqrtefp(x); // vrsqrtefp128 v13,v63
        let scaled = _mm_mul_ps(x, half); // vmulfp128 v10,v63,v0
        let square = _mm_mul_ps(e, e); // vmulfp128 v7,v13,v13
        let mask_e = _mm_cmpeq_ps(e, e);
        let c = vmx::vnmsubfp(scaled, square, half); // vnmsubfp v1,v10,v7,v0
        let refined = vmx::vmaddfp(e, c, e); // vmaddfp v31,v13,v1,v13
        let mask_c = _mm_cmpeq_ps(c, c);
        let product = _mm_mul_ps(x, refined); // vmulfp128 v12,v63,v31
        let mask = _mm_xor_si128(_mm_castps_si128(mask_c), _mm_castps_si128(mask_e));
        _mm_castsi128_ps(_mm_or_si128(
            _mm_andnot_si128(mask, _mm_castps_si128(product)),
            _mm_and_si128(mask, _mm_castps_si128(x)),
        )) // vsel v2,v12,v4,v31
    }
}

/// The square-root ramp writer (`sub_82B42C98`). Returns 1.
pub fn sqrt_ramp(
    g: &mut Guest,
    out: u32,
    first: i32,
    length: i32,
    start: f64,
    end: f64,
) -> Result<u64> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    let (block_last, last) = block_bounds(first, length);
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let delta = fp::sub_single(end, start); // fsubs f27,f2,f1
    let length_f = int_to_single(length); // fcfid ; frsp f30
    let threshold = fp::load_single(g, RAMP_THRESHOLD)?; // lfs f26,23056(r10)
    let sqrt_length = fp::sqrt_single(length_f); // fsqrts f12,f30
    let step = fp::div_single(delta, sqrt_length); // fdivs f31,f27,f12
    // SAFETY: vmx support was checked above; guest accesses are bounds-checked.
    unsafe {
        let ramp = (start, end, step, length_f, delta);
        let (index, mut cursor) = lead_in(g, &mut fpscr, out, first, last, ramp, Some(threshold))?;
        fpscr.disable_flush_mode_unconditional();
        let step_v = _mm_set1_ps(step as f32);
        let start_v = _mm_set1_ps(start as f32);
        let idx = index_vector(index);
        let (groups, bulk) = groups_for(last, index);
        let stride_up = vmx::lvx128_ps(g, RAMP_INDEX_STRIDE)?; // lvx128 v62,r0,r8
        let ascending = !(delta < threshold);
        let (mut x, stride, coef, base_v) = if ascending {
            (idx, stride_up, step_v, start_v)
        } else {
            let scale = fp::load_single(g, RAMP_DESCENDING_SCALE)?; // lfs f0,-8480(r6)
            let coef = _mm_set1_ps(fp::mul_single(step, scale) as f32); // fmuls f0,f31,f0
            let base_v = _mm_set1_ps(end as f32);
            let length_v = _mm_set1_ps(length_f as f32);
            fpscr.enable_flush_mode(); // emitted by vsubfp128 v63,v63,v62
            let x = _mm_sub_ps(length_v, idx);
            let stride = _mm_castsi128_ps(_mm_set1_epi32(0xC080_0000u32 as i32)); // lis r11,-16256
            (x, stride, coef, base_v)
        };
        if bulk > 0 {
            fpscr.enable_flush_mode_unconditional();
            let half = half_vector();
            for _ in 0..((bulk as u32 - 1) >> 2) + 1 {
                fpscr.enable_flush_mode_unconditional();
                let x0 = x;
                let x1 = _mm_add_ps(x0, stride);
                let x2 = _mm_add_ps(x1, stride);
                let x3 = _mm_add_ps(x2, stride);
                x = _mm_add_ps(x3, stride);
                let s = [
                    sqrt_vector(x0, half),
                    sqrt_vector(x1, half),
                    sqrt_vector(x2, half),
                    sqrt_vector(x3, half),
                ];
                for (k, root) in s.iter().enumerate() {
                    let a = vmx::vmaddfp(coef, *root, base_v); // coefficient first, root second
                    vmx::stvx128_ps(g, cursor.wrapping_add(16 * k as u32), a)?;
                }
                cursor = cursor.wrapping_add(64);
            }
        }
        if bulk < groups {
            fpscr.enable_flush_mode_unconditional();
            let half = half_vector();
            for _ in 0..(groups - bulk) as u32 {
                fpscr.enable_flush_mode_unconditional();
                let root = sqrt_vector(x, half);
                x = _mm_add_ps(x, stride);
                let a = vmx::vmaddfp(coef, root, base_v);
                vmx::stvx128_ps(g, cursor, a)?;
                cursor = cursor.wrapping_add(16);
            }
        }
        let mut tail_index = (index as u32).wrapping_add((groups as u32) << 2) as i32;
        fpscr.disable_flush_mode_unconditional();
        if tail_index <= last {
            let passes = (last.wrapping_sub(tail_index) as u32).wrapping_add(1);
            let mut n = (tail_index as u32).wrapping_add(1) as i32;
            tail_index = (tail_index as u32).wrapping_add(passes) as i32;
            for _ in 0..passes {
                fpscr.disable_flush_mode_unconditional();
                let value = if ascending {
                    // fcfid ; fsqrts -- the root of the double, with no frsp between.
                    fp::fmadd_single(fp::sqrt_single(f64::from(n)), step, start)
                } else {
                    let back = fp::sub_single(length_f, int_to_single(n)); // frsp ; fsubs
                    fp::nmsub_single(fp::sqrt_single(back), step, end) // fsqrts ; fnmsubs
                };
                n = n.wrapping_add(1);
                fp::store_single(g, cursor, value)?;
                cursor = cursor.wrapping_add(4);
            }
        }
        trailing_fill(g, &mut fpscr, cursor, tail_index, block_last, end)?;
    }
    Ok(1) // li r3,1
}

// ===================================================== sub_82B43340: the sine-curve writer

/// `lis -32250 ; lfs f13,3140(r10)` — the phase step's numerator.
pub const RAMP_ANGLE_NUMERATOR: u32 = (((-32250i32 as u32) & 0xFFFF) << 16) + 3140;
/// `addi r9,r10,3140 ; lfs f0,12(r9)` — the length's scale in the phase step's denominator.
pub const RAMP_ANGLE_LENGTH_SCALE: u32 = RAMP_ANGLE_NUMERATOR + 12;
const _: () = assert!(RAMP_ANGLE_NUMERATOR == 0x8206_0C44);

/// The four-lane sine kernel on a vector register's lanes.
unsafe fn sine_lanes(g: &Guest, x: __m128) -> Result<__m128> {
    // SAFETY: plain lane copies; the kernel checks vmx support itself.
    unsafe {
        let mut lanes = [0u32; 4];
        _mm_storeu_si128(lanes.as_mut_ptr() as *mut __m128i, _mm_castps_si128(x));
        let s = crate::dsp::sine::sine4_value(g, lanes)?; // bl 0x824531c8
        Ok(_mm_castsi128_ps(_mm_loadu_si128(
            s.as_ptr() as *const __m128i
        )))
    }
}

/// The sine-curve ramp writer (`sub_82B43340`). Returns 1.
///
/// The siblings' arguments, lead-in, flat fills and window, with the phase step
/// `k / (float(length) * c)` and the curve `fma(sin(step * (i + 1)), delta, start)` — or, descending,
/// `end - sin(step * (length - (i + 1))) * delta` in the scalar tail and the lifted vector form
/// `delta * sin((-step) * (length - idx)) + end` in the body. The vector sine is
/// [`crate::dsp::sine::sine4_value`], the scalar one `trig`. Its alignment padding is linear in the
/// phase step, as in both siblings.
pub fn sine_ramp<T: crate::mathlib::Trig>(
    g: &mut Guest,
    trig: &mut T,
    out: u32,
    first: i32,
    length: i32,
    start: f64,
    end: f64,
) -> Result<u64> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    let (block_last, last) = block_bounds(first, length);
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let delta = fp::sub_single(end, start); // fsubs f29,f2,f1
    let length_f = int_to_single(length); // fcfid ; frsp f30
    let length_scale = fp::load_single(g, RAMP_ANGLE_LENGTH_SCALE)?; // lfs f0,12(r9)
    let numerator = fp::load_single(g, RAMP_ANGLE_NUMERATOR)?; // lfs f13,3140(r10)
    let threshold = fp::load_single(g, RAMP_THRESHOLD)?; // lfs f26,23056(r8)
    let step = fp::div_single(numerator, fp::mul_single(length_f, length_scale)); // fmuls ; fdivs f31
    // SAFETY: vmx support was checked above; guest accesses are bounds-checked.
    unsafe {
        let ramp = (start, end, step, length_f, delta);
        let (index, mut cursor) = lead_in(g, &mut fpscr, out, first, last, ramp, Some(threshold))?;
        fpscr.disable_flush_mode_unconditional();
        let step_v = _mm_set1_ps(step as f32);
        let delta_v = _mm_set1_ps(delta as f32);
        let start_v = _mm_set1_ps(start as f32);
        let idx = index_vector(index);
        let (groups, bulk) = groups_for(last, index);
        let stride_up = vmx::lvx128_ps(g, RAMP_INDEX_STRIDE)?; // lvx128 v123,r0,r9
        let ascending = !(delta < threshold);
        let (mut x, stride, angle, base_v) = if ascending {
            (idx, stride_up, step_v, start_v)
        } else {
            let scale = fp::load_single(g, RAMP_DESCENDING_SCALE)?; // lfs f0,-8480(r8)
            let angle = _mm_set1_ps(fp::mul_single(step, scale) as f32); // fmuls f0,f31,f0
            let base_v = _mm_set1_ps(end as f32);
            let length_v = _mm_set1_ps(length_f as f32);
            fpscr.enable_flush_mode(); // vsubfp128 v127,v63,v62
            let stride = _mm_castsi128_ps(_mm_set1_epi32(0xC080_0000u32 as i32)); // lis r11,-16256
            (_mm_sub_ps(length_v, idx), stride, angle, base_v)
        };
        // Four vectors a pass while whole passes remain, then one at a time: the same count either way.
        let vectors = bulk.max(0) + (groups - bulk).max(0);
        for _ in 0..vectors {
            fpscr.enable_flush_mode_unconditional();
            let arg = _mm_mul_ps(angle, x); // vmulfp128 v1,v126,v127
            let s = sine_lanes(g, arg)?; // bl 0x824531c8
            fpscr.enable_flush_mode_unconditional();
            x = _mm_add_ps(x, stride); // vaddfp128 v127,v127,v123
            let a = vmx::vmaddfp(delta_v, s, base_v); // vmaddfp v0,v125,v0,v124
            vmx::stvx128_ps(g, cursor, a)?;
            cursor = cursor.wrapping_add(16);
        }
        let mut tail_index = (index as u32).wrapping_add((groups as u32) << 2) as i32;
        fpscr.disable_flush_mode_unconditional();
        if tail_index <= last {
            let passes = (last.wrapping_sub(tail_index) as u32).wrapping_add(1);
            let mut n = (tail_index as u32).wrapping_add(1) as i32;
            tail_index = (tail_index as u32).wrapping_add(passes) as i32;
            for _ in 0..passes {
                fpscr.disable_flush_mode_unconditional();
                let arg = if ascending {
                    fp::mul_single(int_to_single(n), step) // fmuls f1,f12,f31
                } else {
                    fp::mul_single(fp::sub_single(length_f, int_to_single(n)), step) // fsubs ; fmuls
                };
                let s = fp::frsp(trig.sine(g, arg)?); // bl 0x82f4ded0 ; frsp
                fpscr.disable_flush_mode_unconditional();
                let value = if ascending {
                    fp::fmadd_single(s, delta, start) // fmadds f10,f11,f29,f28
                } else {
                    fp::nmsub_single(s, delta, end) // fnmsubs f9,f10,f29,f27
                };
                n = n.wrapping_add(1);
                fp::store_single(g, cursor, value)?;
                cursor = cursor.wrapping_add(4);
            }
        }
        trailing_fill(g, &mut fpscr, cursor, tail_index, block_last, end)?;
    }
    Ok(1) // li r3,1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;

    const BASE: u32 = 0x4000_0000;
    const OUT: u32 = BASE + 0x100;
    const POISON: u32 = 0xDEAD_BEEF;

    fn guest() -> Guest {
        let mut g = Guest::from_segments(vec![
            Segment {
                base: BASE,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8216_0000,
                bytes: vec![0u8; 0x10000],
            },
            Segment {
                base: 0x8231_B000,
                bytes: vec![0u8; 0x1000],
            },
        ]);
        g.set_u32(RAMP_THRESHOLD, 0.0f32.to_bits()).unwrap();
        g.set_u32(RAMP_DESCENDING_SCALE, (-1.0f32).to_bits())
            .unwrap();
        for k in 0..4 {
            g.set_u32(RAMP_INDEX_STRIDE + 4 * k, 4.0f32.to_bits())
                .unwrap();
        }
        for k in 0..300u32 {
            g.set_u32(OUT + 4 * k, POISON).unwrap();
        }
        g
    }

    fn out(g: &Guest, n: u32) -> Vec<f32> {
        (0..n).map(|k| g.f32(OUT + 4 * k).unwrap()).collect()
    }

    #[test]
    fn a_unit_linear_ramp_counts_one_to_256() {
        let mut g = guest();
        assert_eq!(linear_ramp(&mut g, OUT, 0, 256, 0.0, 256.0).unwrap(), 1);
        assert_eq!(
            out(&g, 256),
            (1..=256).map(|v| v as f32).collect::<Vec<_>>()
        );
        assert_eq!(
            g.u32(OUT + 4 * 256).unwrap(),
            POISON,
            "one block and no more"
        );
    }

    #[test]
    fn indices_below_zero_are_flat_at_the_start() {
        let mut g = guest();
        linear_ramp(&mut g, OUT, -8, 256, 0.0, 256.0).unwrap();
        let o = out(&g, 256);
        assert_eq!(&o[..8], &[0.0; 8]);
        assert_eq!(o[8], 1.0);
        assert_eq!(o[255], 248.0);
        assert_eq!(g.u32(OUT + 4 * 256).unwrap(), POISON);
    }

    #[test]
    fn past_the_ramp_the_block_is_flat_at_the_end() {
        let mut g = guest();
        linear_ramp(&mut g, OUT, 0, 100, 0.0, 100.0).unwrap();
        let o = out(&g, 256);
        assert_eq!(o[99], 100.0);
        assert!(o[100..].iter().all(|v| *v == 100.0));
    }

    #[test]
    fn a_descending_lead_in_pads_from_the_end_side() {
        // first -2: two flat samples, two padding samples by the descending form, then the vectors.
        let mut g = guest();
        linear_ramp(&mut g, OUT, -2, 10, 10.0, 0.0).unwrap();
        assert_eq!(
            out(&g, 12),
            [10.0, 10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0]
        );
        assert!(out(&g, 256)[12..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_square_root_ramp_tracks_the_root_through_the_vector_estimate() {
        let mut g = guest();
        assert_eq!(sqrt_ramp(&mut g, OUT, 0, 256, 0.0, 16.0).unwrap(), 1);
        for (i, v) in out(&g, 256).iter().enumerate() {
            let want = ((i + 1) as f32).sqrt();
            assert!(
                (v - want).abs() <= want * 1e-4,
                "entry {i}: {v} against {want}"
            );
        }
    }

    #[test]
    fn the_square_root_ramp_goes_flat_at_its_length() {
        let mut g = guest();
        sqrt_ramp(&mut g, OUT, 0, 36, 0.0, 6.0).unwrap();
        let o = out(&g, 256);
        assert!((o[35] - 6.0).abs() < 1e-4);
        assert!(
            o[36..].iter().all(|v| *v == 6.0),
            "the trailing fill is exact"
        );
    }

    #[test]
    fn the_scalar_tail_takes_a_correctly_rounded_root() {
        // Length 6: one vector of four, then two scalar samples.
        let mut g = guest();
        sqrt_ramp(&mut g, OUT, 0, 6, 0.0, 1.0).unwrap();
        let step = fp::div_single(1.0, fp::sqrt_single(6.0));
        for (i, v) in out(&g, 6).iter().enumerate().skip(4) {
            let want = fp::fmadd_single(fp::sqrt_single((i + 1) as f64), step, 0.0) as f32;
            assert_eq!(*v, want, "entry {i}");
        }
    }

    #[test]
    fn a_descending_square_root_ramp_counts_down_the_root() {
        let mut g = guest();
        sqrt_ramp(&mut g, OUT, 0, 256, 16.0, 0.0).unwrap();
        for (i, v) in out(&g, 256).iter().enumerate() {
            let want = ((255 - i) as f32).sqrt();
            assert!(
                (v - want).abs() <= want.max(1.0) * 1e-4,
                "entry {i}: {v} against {want}"
            );
        }
    }
}

#[cfg(test)]
mod sine_ramp_tests {
    use super::*;
    use crate::mathlib::tests::Scripted;

    const BASE: u32 = 0x4000_0000;
    const OUT: u32 = BASE + 0x100;

    /// The sine kernel's pool, the ramp constants, and a numerator of pi/2 over a length scale of 1.
    fn guest() -> Guest {
        let mut g = crate::dsp::sine::tests::image();
        g.put(BASE, vec![0u8; 0x1000]);
        g.put(0x8206_0000, vec![0u8; 0x1000]);
        g.put(0x8216_0000, vec![0u8; 0x10000]);
        g.put(0x8231_B000, vec![0u8; 0x1000]);
        let cell = |g: &mut Guest, at: u32, v: f32| g.set_u32(at, v.to_bits()).unwrap();
        cell(&mut g, RAMP_ANGLE_NUMERATOR, core::f32::consts::FRAC_PI_2);
        cell(&mut g, RAMP_ANGLE_LENGTH_SCALE, 1.0);
        cell(&mut g, RAMP_THRESHOLD, 0.0);
        cell(&mut g, RAMP_DESCENDING_SCALE, -1.0);
        for k in 0..4 {
            cell(&mut g, RAMP_INDEX_STRIDE + 4 * k, 4.0);
        }
        g
    }

    fn out(g: &Guest, n: u32) -> Vec<f32> {
        (0..n).map(|k| g.f32(OUT + 4 * k).unwrap()).collect()
    }

    fn scripted(sine: f64) -> Scripted {
        Scripted {
            sine,
            cosine: 0.0,
            asked: vec![],
        }
    }

    #[test]
    fn a_quarter_period_over_the_block_tracks_the_sine() {
        let mut g = guest();
        assert_eq!(
            sine_ramp(&mut g, &mut scripted(0.0), OUT, 0, 256, 0.0, 1.0).unwrap(),
            1
        );
        let step = core::f64::consts::FRAC_PI_2 / 256.0;
        for (i, v) in out(&g, 256).iter().enumerate() {
            let want = ((i + 1) as f64 * step).sin() as f32;
            assert!((v - want).abs() < 2e-4, "entry {i}: {v} against {want}");
        }
    }

    #[test]
    fn the_scalar_tail_uses_the_scalar_sine_and_the_block_goes_flat() {
        // Length 6: one vector of four, two scalar samples through `trig`, then the flat end.
        let mut g = guest();
        let mut trig = scripted(0.5);
        sine_ramp(&mut g, &mut trig, OUT, 0, 6, 0.0, 2.0).unwrap();
        let o = out(&g, 256);
        assert_eq!(
            (o[4], o[5]),
            (1.0, 1.0),
            "0 + 0.5 * 2 from the scripted sine"
        );
        assert_eq!(trig.asked.len(), 2);
        assert!(o[6..].iter().all(|v| *v == 2.0));
    }
}
