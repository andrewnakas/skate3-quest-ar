//! A drop-in stand-in for `core::arch::x86_64`'s SSE intrinsics, so [`crate::vmx`] and every DSP
//! kernel that calls `_mm_*` directly can compile unchanged on both architectures: every name here
//! means exactly what the SSE intrinsic of the same name means, and on x86_64 this module *is*
//! `core::arch::x86_64` (a plain re-export, zero cost, the already-verified path untouched).
//!
//! On aarch64 each function is translated from the x86 instruction's documented semantics, not
//! "whatever the nearest NEON instruction happens to do" — the two differ in exactly the cases
//! called out below. **There is no recorded-vector harness for this path** (`docs/vmx128-exactness.md`
//! isn't part of this checkout), so nothing here carries the bit-exactness guarantee
//! [`crate::vmx`]'s module docs describe for x86_64. Treat it as reasoned-through, not measured.
//!
//! Known, deliberate divergences from literal x86 behaviour, and why they don't matter here:
//! - [`_mm_max_ps`]/[`_mm_min_ps`]: implemented as the exact `if a>b {a} else {b}` / `if a<b {a}
//!   else {b}` MAXPS/MINPS compute (not IEEE `maxNum`), via compare-and-select, so the "NaN in either
//!   operand loses to the second operand" behaviour matches bit for bit.
//! - [`_mm_cvttps_epi32`]: ARM's `FCVTZS` returns 0 for NaN and saturates out-of-range values toward
//!   the correct sign (x86's `CVTTPS2DQ` returns `i32::MIN` for *both* NaN and any overflow). Every
//!   call site in this crate ([`crate::vmx::vctsxs`], [`crate::vmx::vctuxs`]) already re-derives
//!   overflow and NaN from the *input* independently and overwrites those lanes explicitly, so it
//!   never actually depends on which sentinel the raw conversion produced.
//! - [`_mm_shuffle_epi8`]: a byte index with its high bit set reads as zero either way; an index in
//!   `16..128` (no high bit, so PSHUFB would mask it to its low nibble) would disagree with NEON's
//!   table lookup (which just returns zero past the table), but every mask this crate ever builds
//!   ([`crate::vmx::VECTOR_MASK_L`]/[`VECTOR_MASK_R`], and `0x0F - (c & 0x0F)` in
//!   [`crate::vmx::perm_epi8`]) only ever contains `0x00..=0x0F` or the `0xFF` zero sentinel, so that
//!   disagreement never triggers.
//! - [`_mm_dp_ps`]: computed by hand as a **sequential** `p0 + p1 + p2 + p3` (summing straight scalar
//!   `f32` adds in lane order, skipping excluded lanes), deliberately *not* any hardware horizontal-add,
//!   because float addition isn't associative and a tree-reduction would round differently for some
//!   inputs. This reproduces DPPS's documented accumulation order exactly, independent of either
//!   platform's native reduction instruction.
#![allow(non_snake_case, non_camel_case_types)]

#[cfg(target_arch = "x86_64")]
pub use core::arch::x86_64::*;

#[cfg(target_arch = "aarch64")]
mod aarch64_shim {
    use core::arch::aarch64::*;

    /// The universal 128-bit container, standing in for SSE's single reinterpretable `__m128i`.
    pub type __m128i = uint8x16_t;
    pub type __m128 = float32x4_t;

    #[inline(always)]
    unsafe fn s8(a: __m128i) -> int8x16_t {
        unsafe { vreinterpretq_s8_u8(a) }
    }
    #[inline(always)]
    unsafe fn s16(a: __m128i) -> int16x8_t {
        unsafe { vreinterpretq_s16_u8(a) }
    }
    #[inline(always)]
    unsafe fn s32(a: __m128i) -> int32x4_t {
        unsafe { vreinterpretq_s32_u8(a) }
    }
    #[inline(always)]
    unsafe fn u32_(a: __m128i) -> uint32x4_t {
        unsafe { vreinterpretq_u32_u8(a) }
    }
    #[inline(always)]
    unsafe fn from_s8(a: int8x16_t) -> __m128i {
        unsafe { vreinterpretq_u8_s8(a) }
    }
    #[inline(always)]
    unsafe fn from_s16(a: int16x8_t) -> __m128i {
        unsafe { vreinterpretq_u8_s16(a) }
    }
    #[inline(always)]
    unsafe fn from_s32(a: int32x4_t) -> __m128i {
        unsafe { vreinterpretq_u8_s32(a) }
    }
    #[inline(always)]
    unsafe fn from_u32(a: uint32x4_t) -> __m128i {
        unsafe { vreinterpretq_u8_u32(a) }
    }
    #[inline(always)]
    unsafe fn from_f32(a: float32x4_t) -> __m128i {
        unsafe { vreinterpretq_u8_f32(a) }
    }
    #[inline(always)]
    pub unsafe fn _mm_castps_si128(a: __m128) -> __m128i {
        unsafe { from_f32(a) }
    }
    #[inline(always)]
    pub unsafe fn _mm_castsi128_ps(a: __m128i) -> __m128 {
        unsafe { vreinterpretq_f32_u8(a) }
    }

    // ---------------------------------------------------------------- float arithmetic
    #[inline(always)]
    pub unsafe fn _mm_add_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vaddq_f32(a, b) }
    }
    #[inline(always)]
    pub unsafe fn _mm_sub_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vsubq_f32(a, b) }
    }
    #[inline(always)]
    pub unsafe fn _mm_mul_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vmulq_f32(a, b) }
    }
    #[inline(always)]
    pub unsafe fn _mm_div_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vdivq_f32(a, b) }
    }
    /// MAXPS: `if a > b { a } else { b }`, not IEEE `maxNum` — see the module docs.
    #[inline(always)]
    pub unsafe fn _mm_max_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vbslq_f32(vcgtq_f32(a, b), a, b) }
    }
    /// MINPS: `if a < b { a } else { b }`.
    #[inline(always)]
    pub unsafe fn _mm_min_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vbslq_f32(vcltq_f32(a, b), a, b) }
    }
    #[inline(always)]
    pub unsafe fn _mm_cmpeq_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vreinterpretq_f32_u32(vceqq_f32(a, b)) }
    }
    #[inline(always)]
    pub unsafe fn _mm_cmpge_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vreinterpretq_f32_u32(vcgeq_f32(a, b)) }
    }
    #[inline(always)]
    pub unsafe fn _mm_cmpgt_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vreinterpretq_f32_u32(vcgtq_f32(a, b)) }
    }
    #[inline(always)]
    pub unsafe fn _mm_cmplt_ps(a: __m128, b: __m128) -> __m128 {
        unsafe { vreinterpretq_f32_u32(vcltq_f32(a, b)) }
    }
    /// "Unordered": true wherever either lane is NaN, derived from `a == a` (false only for NaN).
    #[inline(always)]
    pub unsafe fn _mm_cmpunord_ps(a: __m128, b: __m128) -> __m128 {
        unsafe {
            let ordered = vandq_u32(vceqq_f32(a, a), vceqq_f32(b, b));
            vreinterpretq_f32_u32(vmvnq_u32(ordered))
        }
    }
    #[inline(always)]
    pub unsafe fn _mm_and_ps(a: __m128, b: __m128) -> __m128 {
        unsafe {
            vreinterpretq_f32_u32(vandq_u32(vreinterpretq_u32_f32(a), vreinterpretq_u32_f32(b)))
        }
    }
    #[inline(always)]
    pub unsafe fn _mm_or_ps(a: __m128, b: __m128) -> __m128 {
        unsafe {
            vreinterpretq_f32_u32(vorrq_u32(vreinterpretq_u32_f32(a), vreinterpretq_u32_f32(b)))
        }
    }
    /// ANDNOTPS: `(!a) & b` — see [`_mm_andnot_si128`] for the same operand-order note.
    #[inline(always)]
    pub unsafe fn _mm_andnot_ps(a: __m128, b: __m128) -> __m128 {
        unsafe {
            vreinterpretq_f32_u32(vbicq_u32(vreinterpretq_u32_f32(b), vreinterpretq_u32_f32(a)))
        }
    }

    /// DPPS, computed as a sequential scalar sum in lane order — see the module docs on why this
    /// isn't a hardware horizontal-add.
    #[inline(always)]
    pub unsafe fn _mm_dp_ps<const IMM8: i32>(a: __m128, b: __m128) -> __m128 {
        unsafe {
            let prod = vmulq_f32(a, b);
            let p0 = if IMM8 & 0b0001_0000 != 0 { vgetq_lane_f32::<0>(prod) } else { 0.0 };
            let p1 = if IMM8 & 0b0010_0000 != 0 { vgetq_lane_f32::<1>(prod) } else { 0.0 };
            let p2 = if IMM8 & 0b0100_0000 != 0 { vgetq_lane_f32::<2>(prod) } else { 0.0 };
            let p3 = if IMM8 & 0b1000_0000 != 0 { vgetq_lane_f32::<3>(prod) } else { 0.0 };
            let sum = p0 + p1 + p2 + p3;
            let mut out = [0f32; 4];
            if IMM8 & 0b0001 != 0 {
                out[0] = sum;
            }
            if IMM8 & 0b0010 != 0 {
                out[1] = sum;
            }
            if IMM8 & 0b0100 != 0 {
                out[2] = sum;
            }
            if IMM8 & 0b1000 != 0 {
                out[3] = sum;
            }
            vld1q_f32(out.as_ptr())
        }
    }

    // ---------------------------------------------------------------------- rounding
    #[inline(always)]
    pub unsafe fn _mm_round_ps<const ROUNDING: i32>(a: __m128) -> __m128 {
        const NEAREST: i32 = _MM_FROUND_TO_NEAREST_INT;
        const ZERO: i32 = _MM_FROUND_TO_ZERO;
        const POS_INF: i32 = _MM_FROUND_TO_POS_INF;
        unsafe {
            match ROUNDING & 0b111 {
                NEAREST => vrndnq_f32(a),
                ZERO => vrndq_f32(a),
                POS_INF => vrndpq_f32(a),
                _ => vrndmq_f32(a),
            }
        }
    }
    // The four modes used at call sites, named the same way `core::arch::x86_64` names them.
    pub const _MM_FROUND_TO_NEAREST_INT: i32 = 0x00;
    pub const _MM_FROUND_TO_NEG_INF: i32 = 0x01;
    pub const _MM_FROUND_TO_POS_INF: i32 = 0x02;
    pub const _MM_FROUND_TO_ZERO: i32 = 0x03;
    pub const _MM_FROUND_NO_EXC: i32 = 0x08;

    // -------------------------------------------------------------------- conversions
    #[inline(always)]
    pub unsafe fn _mm_cvtepi32_ps(a: __m128i) -> __m128 {
        unsafe { vcvtq_f32_s32(s32(a)) }
    }
    /// CVTTPS2DQ (truncating). See the module docs: NaN/overflow sentinels differ from x86, and
    /// every caller already corrects for that independently of this function.
    #[inline(always)]
    pub unsafe fn _mm_cvttps_epi32(a: __m128) -> __m128i {
        unsafe { from_s32(vcvtq_s32_f32(a)) }
    }
    #[inline(always)]
    pub unsafe fn _mm_cvtepi8_epi16(a: __m128i) -> __m128i {
        unsafe { from_s16(vmovl_s8(vget_low_s8(s8(a)))) }
    }

    // ------------------------------------------------------------------------ bitwise
    #[inline(always)]
    pub unsafe fn _mm_and_si128(a: __m128i, b: __m128i) -> __m128i {
        unsafe { vandq_u8(a, b) }
    }
    /// ANDNOTPS/PANDN: `(!a) & b` — note the operand order relative to `vbicq` (`x & !y`).
    #[inline(always)]
    pub unsafe fn _mm_andnot_si128(a: __m128i, b: __m128i) -> __m128i {
        unsafe { vbicq_u8(b, a) }
    }
    #[inline(always)]
    pub unsafe fn _mm_or_si128(a: __m128i, b: __m128i) -> __m128i {
        unsafe { vorrq_u8(a, b) }
    }
    #[inline(always)]
    pub unsafe fn _mm_xor_si128(a: __m128i, b: __m128i) -> __m128i {
        unsafe { veorq_u8(a, b) }
    }

    /// PBLENDVB: per-byte select on each mask byte's sign bit (not a plain bit-select — a mask byte
    /// of e.g. `0x01` must still select `a`, matching x86's "top bit only" rule).
    #[inline(always)]
    pub unsafe fn _mm_blendv_epi8(a: __m128i, b: __m128i, mask: __m128i) -> __m128i {
        unsafe {
            let hi = vshrq_n_s8::<7>(s8(mask));
            vbslq_u8(vreinterpretq_u8_s8(hi), b, a)
        }
    }
    /// BLENDVPS: per-lane select on each mask lane's sign bit.
    #[inline(always)]
    pub unsafe fn _mm_blendv_ps(a: __m128, b: __m128, mask: __m128) -> __m128 {
        unsafe {
            let hi = vshrq_n_s32::<31>(vreinterpretq_s32_f32(mask));
            vbslq_f32(vreinterpretq_u32_s32(hi), b, a)
        }
    }

    // -------------------------------------------------------------------- integer ops
    #[inline(always)]
    pub unsafe fn _mm_add_epi32(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s32(vaddq_s32(s32(a), s32(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_sub_epi32(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s32(vsubq_s32(s32(a), s32(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_sub_epi8(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s8(vsubq_s8(s8(a), s8(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_adds_epi16(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s16(vqaddq_s16(s16(a), s16(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_cmpeq_epi32(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_u32(vceqq_s32(s32(a), s32(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_packus_epi16(a: __m128i, b: __m128i) -> __m128i {
        unsafe { vcombine_u8(vqmovun_s16(s16(a)), vqmovun_s16(s16(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_packs_epi32(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s16(vcombine_s16(vqmovn_s32(s32(a)), vqmovn_s32(s32(b)))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_unpackhi_epi32(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s32(vzip2q_s32(s32(a), s32(b))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_unpacklo_epi32(a: __m128i, b: __m128i) -> __m128i {
        unsafe { from_s32(vzip1q_s32(s32(a), s32(b))) }
    }

    /// Variable-amount shift (positive = left) stands in for the immediate-shift intrinsics, which
    /// on aarch64 reject a shift count of 0 as an invalid immediate; this accepts it as a no-op.
    #[inline(always)]
    pub unsafe fn _mm_slli_epi32<const N: i32>(a: __m128i) -> __m128i {
        unsafe { from_s32(vshlq_s32(s32(a), vdupq_n_s32(N))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_srai_epi32<const N: i32>(a: __m128i) -> __m128i {
        unsafe { from_s32(vshlq_s32(s32(a), vdupq_n_s32(-N))) }
    }
    #[inline(always)]
    pub unsafe fn _mm_srli_epi32<const N: i32>(a: __m128i) -> __m128i {
        unsafe { from_u32(vshlq_u32(u32_(a), vdupq_n_s32(-N))) }
    }

    /// PALIGNR: bytes `N..N+15` of the 32-byte concatenation `[a (high), b (low)]`.
    #[inline(always)]
    pub unsafe fn _mm_alignr_epi8<const N: i32>(a: __m128i, b: __m128i) -> __m128i {
        unsafe { vextq_u8::<N>(b, a) }
    }

    const fn shuffle32_table(imm8: i32) -> [u8; 16] {
        let mut out = [0u8; 16];
        let mut lane = 0usize;
        while lane < 4 {
            let src = ((imm8 >> (lane * 2)) & 0b11) as u8;
            let mut byte = 0u8;
            while byte < 4 {
                out[lane * 4 + byte as usize] = src * 4 + byte;
                byte += 1;
            }
            lane += 1;
        }
        out
    }

    /// PSHUFD: four 32-bit lanes gathered by `IMM8`'s four 2-bit fields, lane *i* of the result
    /// takes `a`'s lane `(IMM8 >> 2*i) & 3`.
    #[inline(always)]
    pub unsafe fn _mm_shuffle_epi32<const IMM8: i32>(a: __m128i) -> __m128i {
        let table: [u8; 16] = shuffle32_table(IMM8);
        unsafe {
            let idx = vld1q_u8(table.as_ptr());
            vqtbl1q_u8(a, idx)
        }
    }

    /// PSHUFB. See the module docs: correct for every mask this crate actually builds.
    #[inline(always)]
    pub unsafe fn _mm_shuffle_epi8(a: __m128i, mask: __m128i) -> __m128i {
        unsafe { vqtbl1q_u8(a, mask) }
    }

    #[inline(always)]
    pub unsafe fn _mm_set1_ps(x: f32) -> __m128 {
        unsafe { vdupq_n_f32(x) }
    }
    #[inline(always)]
    pub unsafe fn _mm_set1_epi32(x: i32) -> __m128i {
        unsafe { from_s32(vdupq_n_s32(x)) }
    }
    #[inline(always)]
    pub unsafe fn _mm_set1_epi8(x: i8) -> __m128i {
        unsafe { from_s8(vdupq_n_s8(x)) }
    }
    #[inline(always)]
    pub unsafe fn _mm_setzero_ps() -> __m128 {
        unsafe { vdupq_n_f32(0.0) }
    }
    #[inline(always)]
    pub unsafe fn _mm_setzero_si128() -> __m128i {
        unsafe { vdupq_n_u8(0) }
    }
    /// `_mm_set_epi32(e3,e2,e1,e0)`: highest argument becomes the highest lane.
    #[inline(always)]
    pub unsafe fn _mm_set_epi32(e3: i32, e2: i32, e1: i32, e0: i32) -> __m128i {
        let lanes = [e0, e1, e2, e3];
        unsafe { from_s32(vld1q_s32(lanes.as_ptr())) }
    }
    #[inline(always)]
    pub unsafe fn _mm_set_ps(e3: f32, e2: f32, e1: f32, e0: f32) -> __m128 {
        let lanes = [e0, e1, e2, e3];
        unsafe { vld1q_f32(lanes.as_ptr()) }
    }
    #[inline(always)]
    pub unsafe fn _mm_setr_ps(e0: f32, e1: f32, e2: f32, e3: f32) -> __m128 {
        let lanes = [e0, e1, e2, e3];
        unsafe { vld1q_f32(lanes.as_ptr()) }
    }

    #[inline(always)]
    pub unsafe fn _mm_loadu_ps(ptr: *const f32) -> __m128 {
        unsafe { vld1q_f32(ptr) }
    }
    #[inline(always)]
    pub unsafe fn _mm_loadu_si128(ptr: *const __m128i) -> __m128i {
        unsafe { vld1q_u8(ptr as *const u8) }
    }
    #[inline(always)]
    pub unsafe fn _mm_store_ps(ptr: *mut f32, a: __m128) {
        unsafe { vst1q_f32(ptr, a) }
    }
    #[inline(always)]
    pub unsafe fn _mm_storeu_ps(ptr: *mut f32, a: __m128) {
        unsafe { vst1q_f32(ptr, a) }
    }
    #[inline(always)]
    pub unsafe fn _mm_storeu_si128(ptr: *mut __m128i, a: __m128i) {
        unsafe { vst1q_u8(ptr as *mut u8, a) }
    }
}

#[cfg(target_arch = "aarch64")]
pub use aarch64_shim::*;
