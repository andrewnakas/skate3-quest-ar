//! The VMX128 layer: RexGlue's lowerings, translated, plus the guest-memory vector accesses.
//!
//! Every function here is a translation of the C++ RexGlue emits into `generated/skate3_recomp.*`
//! and of the helpers in `rex/ppc/intrinsics.h`. It is not a model of Xenon's vector unit and must
//! not be improved into one: this project's oracle is **the recomp's own output**, so a difference
//! between RexGlue and real AltiVec is part of the reference, not a bug to fix
//! (`docs/vmx128-exactness.md`, the note under rule 5 about `vexptefp`/`vlogefp`/`vrefp` being
//! correctly-rounded here where the hardware only estimates).
//!
//! `docs/vmx128-exactness.md` is the authority for everything below and measured 45 of 45
//! operations bit-identical between RexGlue's C++ and a Rust translation over 56,880 lane
//! comparisons in both flush-to-zero states. The five rules it draws, and where each one lands
//! here:
//!
//! 1. **`vmaddfp*` rounds twice in the recomp.** It is built for plain x86-64 plus SSE4.1 with no
//!    `-mfma`, so SIMDe lowers [`vmaddfp`] to `(a * b) + c` and [`vnmsubfp`] to `-(a * b) + c`, which
//!    clang-20 emits as `mulps` then `addps`, and `mulps` then `subps`. Both are translated in that
//!    form. The scalar `fmadds` family is `std::fma` and stays single-rounding. *Corrected
//!    2026-09-13:* this rule used to say the opposite, and the probe agreed only because its
//!    reference was built with `-march=native`, which the game never is.
//! 2. **Flush-to-zero is per instruction class, not per function.** [`Fpscr`] reproduces
//!    `ctx.fpscr`; see its documentation for the part of this that is *not* what the name suggests.
//! 3. **The BE↔LE lane mask is literal data.** [`VECTOR_MASK_L`] and [`VECTOR_MASK_R`] are copied
//!    byte for byte out of `rex/ppc/intrinsics.h`.
//! 4. **Commutative float ops are not NaN-commutative, and the winning operand cannot be derived
//!    from source.** Operand order is preserved verbatim at every call site in [`crate::dsp`], and
//!    that is the most that can be done from source. See "Rule 4" below.
//! 5. **`vexptefp128`/`vlogefp128` go through libm** and match only because Rust's `f32::exp2` and
//!    `f32::log2` lower to the same glibc symbols on this platform. Keep them bit-checked forever.
//!
//! ## The lane convention, which is the thing to get right first
//!
//! Guest memory is big-endian. Every vector load and store passes through a 16-byte reversal, so
//! **host lane 0 holds the guest word at `ea + 12` and host lane 3 holds the word at `ea + 0`**.
//! PPC numbers vector elements from the most significant, so guest element *n* is host lane
//! *3 − n*. That inversion is why the lifted `vspltw128 vD,vS,3` lowers to
//! `shuffle_epi32(..., 0x00)` and not `0xFF`; [`SPLAT_W0`] … [`SPLAT_W3`] name the four immediates
//! so a call site can keep the lifted number without the reader having to re-derive it.
//!
//! The partial stores invert *relative to each other* as well, which is easy to get backwards:
//! [`stvlx128`] walks the register's **high** bytes (`u8[15]` downward) into **ascending**
//! addresses from `ea`, while [`stvrx128`] walks the **low** bytes (`u8[0]` upward) into
//! **descending** addresses below `ea`. Between them they write the sixteen bytes straddling a
//! 16-byte boundary exactly once.
//!
//! ## What the recorded harness covers, and what it does not
//!
//! `probe/vmx128/` holds recorded C++ results for 45 operations over 158 adversarial vectors in
//! both FTZ states. `cargo run --example check_vmx_primitives` replays this module against them.
//! **Covered:** every float arithmetic, estimate, compare, conversion, logical, permute, pack,
//! shift and merge op below, plus [`vspltw128`] at one immediate and the row-0 and row-5 lane
//! masks.
//!
//! **Not covered, and the reason each one matters:**
//!
//! | uncovered | why it is not in the recorded table |
//! |---|---|
//! | [`vrfin128`], [`vrfip128`], [`vrfim128`] | the table records `vrfiz128` only, and `vrfin128` is what [`crate::dsp::sine`] reduces its argument with |
//! | [`stvx128`], [`stvlx128`], [`stvrx128`], [`dcbzl`] | the table has no store side at all — it is a register-to-register probe |
//! | [`lvx128`], [`lvlx128`], [`lvrx128`] | the *data* permutation is covered as `lvx128_swap`/`lvlx128_swap5`; the address masking and the `lvrx` zero case are not |
//! | [`vspltw128`] at three of its four immediates | the table probes `0xAA` only |
//! | [`VECTOR_MASK_R`], and 14 of the 16 [`VECTOR_MASK_L`] rows | only rows 0 and 5 of `MASK_L` are exercised |
//!
//! Each uncovered primitive has a unit test below against a hand-written scalar model of the same
//! lowering. That is a weaker instrument than the recorded compare and is labelled as such.
//!
//! ## Rule 4
//!
//! For `vaddfp*`, `vmulfp*`, `vmaddfp*` and `vnmsubfp*`, *which* operand's payload survives when
//! two lanes are both NaN is decided by register allocation, in GCC and in clang-20 alike, and so
//! is not a property of the source at all. The ops below are written with the operands in the
//! lifted order, which is necessary but not sufficient. `check_vmx_primitives` is what actually
//! settles it, and it has to be re-run on any toolchain change. Nothing in this module may be
//! reasoned about on this point.

// Every intrinsic call below sits in an `unsafe` block even where the compiler no longer asks for
// one: since Rust 1.86 a `core::arch` intrinsic is safe to call inside a `#[target_feature]`
// function, so most of these blocks are redundant *to the compiler*. They are kept because
// `lib.rs` scopes this crate's `unsafe` to intrinsic call sites, because the pointer-taking loads
// and stores in the same bodies genuinely need one, and because which of them the compiler counts
// as necessary has already moved once under this code.
#![allow(unused_unsafe)]

use crate::{Error, Guest, Result};
use core::arch::x86_64::*;

// ---------------------------------------------------------------------------- feature gate

/// Whether this CPU has what RexGlue's lowerings compile to.
///
/// `probe/vmx128/run.sh` builds both sides with `-march=native` / `-C target-cpu=native`, so the
/// reference assumes SSE4.1 and FMA are simply present. Here they are checked once per call at the
/// kernel boundary rather than assumed, because this crate is a library and its caller chooses the
/// build flags.
pub fn supported() -> bool {
    is_x86_feature_detected!("sse4.1") && is_x86_feature_detected!("fma")
}

/// The error a kernel returns rather than running without SSE4.1 and FMA.
///
/// Address 0 because it names no guest location; a wrong answer computed by a software fallback
/// would be worse than no answer, which is the same argument `eval::dispatch` makes for refusing to
/// invent a value for an unported opcode.
pub fn unsupported() -> Error {
    Error::new(
        0,
        "vmx: this build needs sse4.1 and fma (-C target-cpu=native)",
    )
}

// ------------------------------------------------------------------------------- MXCSR / FPSCR

/// MXCSR `FZ`. `rex::platform::FPSCRPlatform::FlushMask` is this or'd with [`DENORMALS_ZERO`].
pub const FLUSH_ZERO: u32 = 0x8000;
/// MXCSR `DAZ`.
pub const DENORMALS_ZERO: u32 = 0x0040;
/// MXCSR `IM|DM|ZM|OM|UM|PM` — `FPSCRPlatform::ExceptionMask`, every FP exception masked.
pub const EXCEPTION_MASK: u32 = (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12);
/// MXCSR `RC`.
pub const ROUND_MASK: u32 = 0x6000;
/// `SIMDE_MM_ROUND_NEAREST`, which is what `GuestToHost[kRoundNearest]` maps to.
pub const ROUND_NEAREST: u32 = 0x0000;

/// The value `InitHost` computes for `vmx_csr`: flush-to-zero, round to nearest, no traps.
pub const VMX_CSR: u32 = FLUSH_ZERO | DENORMALS_ZERO | EXCEPTION_MASK | ROUND_NEAREST;

#[inline]
pub fn get_mxcsr() -> u32 {
    let mut v: u32 = 0;
    unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut v, options(nostack)) };
    v
}

#[inline]
pub fn set_mxcsr(v: u32) {
    unsafe { core::arch::asm!("ldmxcsr [{}]", in(reg) &v, options(nostack)) };
}

/// `ctx.fpscr` — the two MXCSR values a lifted function switches between, and the switch itself.
///
/// **The names are misleading and the misreading is expensive.** `disableFlushMode*` does *not*
/// turn flush-to-zero off. `rex::ppc::FPSCRRegister::InitHost` builds `fpu_csr` as
/// `getcsr() | ExceptionMask | FlushMask`, so the scalar side carries `FZ|DAZ` too; the comment
/// above it in `rex/ppc/context.h` says so outright and explains why (a denormal operand costs a
/// ~100-cycle microcode assist on x86, and Skate 3's reverb tails decay straight through denormal
/// range). What the two modes actually differ in is the **rounding mode**: `vmx_csr` is always
/// round-to-nearest, `fpu_csr` carries whatever the guest last wrote to its FPSCR. On the audio
/// thread, which never leaves round-to-nearest, the two values are equal — so on this build the
/// toggles change nothing, and they are reproduced anyway because the reference performs them and
/// because a guest that changed its rounding mode would make them differ.
///
/// This matters beyond bookkeeping: a translation that cleared `FZ|DAZ` for the scalar paths of
/// [`crate::dsp::scale`] would produce denormal results where the recomp flushes them to zero.
/// [`crate::fp`]'s module documentation used to state the opposite ("MXCSR `0x0000` … denormals
/// preserved"); it was corrected on 2026-09-12 and now agrees with this. Nothing in `fp.rs` ever
/// depended on it either way, because the ops there are called under whatever mode their caller
/// established — which is why every body that uses them holds an [`Fpscr`] of its own.
///
/// **What holding the mode cannot do, measured 2026-09-13.** `examples/flush_probe` sets `FZ|DAZ`,
/// confirms MXCSR reads back `0x9FC0`, and then shows that an `lfs`/`stfs` round trip of the same
/// value flushes a denormal at `opt-level = 0` and **does not** at `opt-level >= 1` — because LLVM
/// folds `fptrunc(fpext(x))` into a no-op and the conversions never execute. Real arithmetic is
/// unaffected: a denormal *product* is `+0` under both profiles, which is the behaviour
/// [`crate::dsp::biquad`]'s denormal-avoidance bias exists for. So the mode is honoured wherever an
/// operation actually happens, and a pure widen/narrow pair is not an operation the optimizer keeps.
/// Two tests used to assert the flush through such a pair; they were wrong, and say so now. clang
/// has the same fold available for the recomp's own `double(f32)`/`float(f64)` pairs, which is why
/// no claim is made about the original's answer either — no recorded vector exercises it.
///
/// **One deliberate divergence.** The guest's FPSCR is sticky: a lifted function leaves the mode
/// it last set and the next one inherits it. [`Fpscr`] restores the entry MXCSR when it is dropped,
/// so a Rust caller's own float code is not silently switched into flush-to-zero. That is safe for
/// everything in [`crate::dsp`] because each of those bodies sets a mode before its first float
/// operation — checked one by one, not assumed — and it is named here rather than hidden because
/// it is a difference from the reference.
pub struct Fpscr {
    entry: u32,
    fpu_csr: u32,
    vmx_csr: u32,
    vmx_mode: bool,
}

impl Fpscr {
    /// `FPSCRRegister::InitHost`: take the host rounding mode, mask the traps, force flush-to-zero,
    /// and enter scalar mode.
    pub fn capture() -> Self {
        let entry = get_mxcsr();
        let fpu_csr = entry | EXCEPTION_MASK | FLUSH_ZERO | DENORMALS_ZERO;
        let vmx_csr = VMX_CSR;
        set_mxcsr(fpu_csr);
        Self {
            entry,
            fpu_csr,
            vmx_csr,
            vmx_mode: false,
        }
    }

    /// `ctx.fpscr.enableFlushModeUnconditional()`.
    #[inline]
    pub fn enable_flush_mode_unconditional(&mut self) {
        self.vmx_mode = true;
        set_mxcsr(self.vmx_csr);
    }

    /// `ctx.fpscr.disableFlushModeUnconditional()`.
    #[inline]
    pub fn disable_flush_mode_unconditional(&mut self) {
        self.vmx_mode = false;
        set_mxcsr(self.fpu_csr);
    }

    /// `ctx.fpscr.enableFlushMode()` — the guarded form, which re-reads MXCSR before deciding.
    #[inline]
    pub fn enable_flush_mode(&mut self) {
        if !self.vmx_mode || get_mxcsr() != self.vmx_csr {
            self.vmx_mode = true;
            set_mxcsr(self.vmx_csr);
        }
    }

    /// `ctx.fpscr.disableFlushMode()`. `sub_82B3C098` uses this form once, mid-body.
    #[inline]
    pub fn disable_flush_mode(&mut self) {
        if self.vmx_mode || get_mxcsr() != self.fpu_csr {
            self.vmx_mode = false;
            set_mxcsr(self.fpu_csr);
        }
    }

    /// The MXCSR in force right now, for tests that assert which mode a body left behind.
    #[inline]
    pub fn current(&self) -> u32 {
        get_mxcsr()
    }
}

impl Drop for Fpscr {
    fn drop(&mut self) {
        set_mxcsr(self.entry);
    }
}

// ------------------------------------------------------------------------------- the lane masks

/// `rex::ppc::VectorMaskL`, verbatim. Row 0 is the plain 16-byte reversal every `lvx128`/`stvx128`
/// uses; row *k* serves `lvlx`/`lvlx128` at an address whose low four bits are *k*, filling the
/// bytes that fall off with `0xFF` so `_mm_shuffle_epi8` zeroes them.
pub static VECTOR_MASK_L: [u8; 256] = [
    0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00,
    0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
    0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02,
    0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03,
    0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D, 0x0C,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E, 0x0D,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x0E,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F,
];

/// `rex::ppc::VectorMaskR`, verbatim. The `lvrx`/`lvrx128` counterpart.
pub static VECTOR_MASK_R: [u8; 256] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
    0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF,
    0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF, 0xFF,
    0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0xFF,
];

/// One 16-byte row of [`VECTOR_MASK_L`], as a vector.
///
/// # Safety
/// Requires SSE2. `row` must be 0..16.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn mask_l(row: usize) -> __m128i {
    unsafe { _mm_loadu_si128(VECTOR_MASK_L.as_ptr().add(row * 16) as *const __m128i) }
}

/// One 16-byte row of [`VECTOR_MASK_R`], as a vector.
///
/// # Safety
/// Requires SSE2. `row` must be 0..16.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn mask_r(row: usize) -> __m128i {
    unsafe { _mm_loadu_si128(VECTOR_MASK_R.as_ptr().add(row * 16) as *const __m128i) }
}

// -------------------------------------------------------------------------- RexGlue's own helpers

/// `rex::ppc::ppc_vrsqrtefp_bits` — the reciprocal-square-root estimate as an integer table
/// lookup, not `_mm_rsqrt_ps`.
///
/// This is why `vrsqrtefp` cannot drift between the two languages: it is data plus integer
/// arithmetic and has no hardware estimate in it at all.
pub fn ppc_vrsqrtefp_bits(bits: u32) -> u32 {
    const TABLE: [u32; 32] = [
        0x0568B4FD, 0x04F3AF97, 0x048DAAA5, 0x0435A618, 0x03E7A1E4, 0x03A29DFE, 0x03659A5C,
        0x032E96F8, 0x02FC93CA, 0x02D090CE, 0x02A88DFE, 0x02838B57, 0x026188D4, 0x02438673,
        0x02268431, 0x020B820B, 0x03D27FFA, 0x03807C29, 0x033878AA, 0x02F97572, 0x02C27279,
        0x02926FB7, 0x02666D26, 0x023F6AC0, 0x021D6881, 0x01FD6665, 0x01E16468, 0x01C76287,
        0x01AF60C1, 0x01995F12, 0x01855D79, 0x01735BF4,
    ];
    let sign = bits >> 31;
    let biased_exp = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;

    if bits == 0xFF80_0000 {
        return 0x7FC0_0000;
    }
    if biased_exp == 0 {
        return if sign != 0 { 0xFF80_0000 } else { 0x7F80_0000 };
    }
    if biased_exp == 0xFF {
        if mantissa == 0 {
            return 0;
        }
        return bits | 0x0040_0000;
    }
    if sign != 0 {
        return 0x7FC0_0000;
    }

    let unbiased_exp = biased_exp as i32 - 127;
    let index = ((((unbiased_exp as u32) << 4) & 16) | (mantissa >> 19)) ^ 16;
    let interp = (mantissa >> 9) & 1023;
    let mut result_exp = (127 - biased_exp as i32) >> 1;
    let entry = TABLE[index as usize];
    let slope = entry >> 16;
    let base = (entry << 10) & 0x03FF_FC00;
    // C++ computes `int32_t(base) - int32_t(interp * slope)`; both operands wrap.
    let mut raw = (base as i32).wrapping_sub(interp.wrapping_mul(slope) as i32);

    if (raw & (1 << 25)) == 0 {
        let val = (raw as u32) & 0x01FF_FFFF;
        let lz = val.leading_zeros() as i32;
        let shift = lz - 6;
        result_exp += 6 - lz;
        // C++ `raw <<= shift` with a negative shift is UB, but x86 masks the count to five bits.
        // Reproduce the machine, not the abstract rule.
        raw = ((raw as u32) << ((shift as u32) & 31)) as i32;
    }

    if (raw & 5) != 0 && (raw & 2) != 0 {
        raw = raw.wrapping_add(4);
    }

    let mut result =
        ((result_exp << 23).wrapping_add(0x3F80_0000)) as u32 | (((raw as u32) >> 2) & 0x007F_FFFF);
    if ((result >> 23) & 0xFF) == 0 && (result & 0x007F_FFFF) != 0 {
        result = 0;
    }
    result
}

/// `rex::ppc::ppc_vmsumfp_result`: overflow becomes the default QNaN, then the guest's own
/// flush-to-zero is applied to the mantissa by hand.
pub fn ppc_vmsumfp_result_scalar(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::from_bits(0x7FC0_0000);
    }
    let mut bits = value.to_bits();
    if ((bits >> 23) & 0xFF) == 0 && (bits & 0x007F_FFFF) != 0 {
        bits &= 0x8000_0000;
    }
    f32::from_bits(bits)
}

/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn vmsumfp_result(v: __m128) -> __m128 {
    unsafe {
        let mut l = [0f32; 4];
        _mm_storeu_ps(l.as_mut_ptr(), v);
        _mm_set1_ps(ppc_vmsumfp_result_scalar(l[0]))
    }
}

/// `simde_mm_cvtepu32_ps_`: unsigned word to float, which SSE has no instruction for.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn cvtepu32_ps(src1: __m128i) -> __m128 {
    unsafe {
        let xmm1 = _mm_add_epi32(src1, _mm_set1_epi32(127));
        let mut xmm0 = _mm_slli_epi32::<{ 31 - 8 }>(src1);
        xmm0 = _mm_srli_epi32::<31>(xmm0);
        xmm0 = _mm_add_epi32(xmm0, xmm1);
        xmm0 = _mm_srai_epi32::<8>(xmm0);
        xmm0 = _mm_add_epi32(xmm0, _mm_set1_epi32(0x4F80_0000u32 as i32));
        let xmm2 = _mm_cvtepi32_ps(src1);
        _mm_blendv_ps(xmm2, _mm_castsi128_ps(xmm0), _mm_castsi128_ps(src1))
    }
}

/// `simde_mm_perm_epi8_`: the `vperm` byte permute, whose selector is big-endian byte numbering,
/// hence the `0x0F -` complement.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn perm_epi8(a: __m128i, b: __m128i, c: __m128i) -> __m128i {
    unsafe {
        let d = _mm_set1_epi8(0x0F);
        let e = _mm_sub_epi8(d, _mm_and_si128(c, d));
        _mm_blendv_epi8(
            _mm_shuffle_epi8(a, e),
            _mm_shuffle_epi8(b, e),
            _mm_slli_epi32::<3>(c),
        )
    }
}

/// `simde_mm_vctsxs`: float to signed word with PPC saturation, and NaN to zero.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vctsxs(src1: __m128) -> __m128i {
    unsafe {
        let xmm2 = _mm_cmpunord_ps(src1, src1);
        let xmm0 = _mm_cvttps_epi32(src1);
        let mut xmm1 = _mm_cmpeq_epi32(xmm0, _mm_set1_epi32(i32::MIN));
        xmm1 = _mm_andnot_si128(_mm_castps_si128(src1), xmm1);
        let dest = _mm_blendv_ps(
            _mm_castsi128_ps(xmm0),
            _mm_castsi128_ps(_mm_set1_epi32(i32::MAX)),
            _mm_castsi128_ps(xmm1),
        );
        _mm_andnot_si128(_mm_castps_si128(xmm2), _mm_castps_si128(dest))
    }
}

/// `simde_mm_vctuxs`: float to unsigned word with PPC saturation.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vctuxs(src1: __m128) -> __m128i {
    unsafe {
        let nan_mask = _mm_cmpunord_ps(src1, src1);
        let neg_mask = _mm_cmplt_ps(src1, _mm_setzero_ps());
        let max_val = _mm_set1_ps(4294967295.0f32);
        let overflow_mask = _mm_cmpge_ps(src1, max_val);
        let mut clamped = _mm_max_ps(src1, _mm_setzero_ps());
        clamped = _mm_min_ps(clamped, max_val);
        let half_range = _mm_set1_ps(2147483648.0f32);
        let high_bit_mask = _mm_cmpge_ps(clamped, half_range);
        let adjusted = _mm_sub_ps(clamped, _mm_and_ps(high_bit_mask, half_range));
        let low_bits = _mm_cvttps_epi32(adjusted);
        let high_bit = _mm_and_si128(
            _mm_castps_si128(high_bit_mask),
            _mm_set1_epi32(0x8000_0000u32 as i32),
        );
        let mut result = _mm_or_si128(low_bits, high_bit);
        result = _mm_andnot_si128(_mm_castps_si128(nan_mask), result);
        result = _mm_andnot_si128(_mm_castps_si128(neg_mask), result);
        result = _mm_or_si128(
            _mm_andnot_si128(_mm_castps_si128(overflow_mask), result),
            _mm_and_si128(_mm_castps_si128(overflow_mask), _mm_set1_epi32(-1)),
        );
        result
    }
}

// -------------------------------------------------------------------------------- float arithmetic

macro_rules! vop {
    ($(#[$m:meta])* $name:ident($a:ident: __m128 $(, $b:ident: __m128)*) = $body:expr) => {
        $(#[$m])*
        ///
        /// # Safety
        /// Requires SSE4.1 and FMA.
        #[inline]
        #[target_feature(enable = "sse4.1,fma")]
        pub unsafe fn $name($a: __m128 $(, $b: __m128)*) -> __m128 {
            unsafe { $body }
        }
    };
}

vop! {
    /// `vaddfp`, `vaddfp128`. **Rule 4 applies:** operand order is part of the semantics.
    vaddfp(a: __m128, b: __m128) = _mm_add_ps(a, b)
}
vop! {
    /// `vsubfp`, `vsubfp128`.
    vsubfp(a: __m128, b: __m128) = _mm_sub_ps(a, b)
}
vop! {
    /// `vmulfp128`. **Rule 4 applies.** Must never be fused with an adjacent add.
    vmulfp(a: __m128, b: __m128) = _mm_mul_ps(a, b)
}
vop! {
    /// `vmaddfp`, `vmaddfp128`, `vmaddcfp128` — `a * b + c` with **two** roundings.
    ///
    /// Rule 1: the recomp has no FMA, so SIMDe multiplies, rounds, adds and rounds again, and
    /// clang-20 emits exactly `mulps` then `addps` with the product as the first source. LLVM will
    /// not contract the pair without fast-math, so it stays unfused here too. **Rule 4 applies** to
    /// the multiply's two factors and to the add.
    vmaddfp(a: __m128, b: __m128, c: __m128) = _mm_add_ps(_mm_mul_ps(a, b), c)
}
vop! {
    /// `vnmsubfp` — `-(a * b) + c`, which clang-20 emits as `c - a * b`: `mulps` then `subps`, two
    /// roundings. A subtraction is not commutative, so which NaN wins is fixed — `c`, then the
    /// product with its sign *not* flipped. **Rule 4 applies** only to the multiply's factors.
    vnmsubfp(a: __m128, b: __m128, c: __m128) = _mm_sub_ps(c, _mm_mul_ps(a, b))
}
vop! {
    /// `vmaxfp128`. Needs no operand pinning: SSE defines `maxps` to return its second operand
    /// when either is NaN, so no compiler may commute it.
    vmaxfp(a: __m128, b: __m128) = _mm_max_ps(a, b)
}
vop! {
    /// `vminfp128`. See [`vmaxfp`].
    vminfp(a: __m128, b: __m128) = _mm_min_ps(a, b)
}
vop! {
    /// `vrefp` — a **true** division, not `rcpps`. RexGlue is more accurate than the hardware
    /// estimate here and the recomp's output is the oracle, so this is correct as written.
    vrefp(a: __m128) = _mm_div_ps(_mm_set1_ps(1.0), a)
}
vop! {
    /// `vrfin128` — round to nearest. **Not covered by the recorded harness**; see the module
    /// table. [`crate::dsp::sine`] reduces its argument with this.
    vrfin(a: __m128) = _mm_round_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(a)
}
vop! {
    /// `vrfiz128` — round toward zero.
    vrfiz(a: __m128) = _mm_round_ps::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(a)
}
vop! {
    /// `vrfip128` — round toward +infinity. Not covered by the recorded harness.
    vrfip(a: __m128) = _mm_round_ps::<{ _MM_FROUND_TO_POS_INF | _MM_FROUND_NO_EXC }>(a)
}
vop! {
    /// `vrfim128` — round toward -infinity. Not covered by the recorded harness.
    vrfim(a: __m128) = _mm_round_ps::<{ _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC }>(a)
}
vop! {
    /// `vcmpeqfp128` — an all-ones or all-zeroes lane mask.
    vcmpeqfp(a: __m128, b: __m128) = _mm_cmpeq_ps(a, b)
}
vop! {
    /// `vcmpgefp128`.
    vcmpgefp(a: __m128, b: __m128) = _mm_cmpge_ps(a, b)
}
vop! {
    /// `vcmpgtfp128`.
    vcmpgtfp(a: __m128, b: __m128) = _mm_cmpgt_ps(a, b)
}
vop! {
    /// `vmsum3fp128`: the three-lane dot product, broadcast, then put through
    /// [`ppc_vmsumfp_result_scalar`].
    vmsum3fp(a: __m128, b: __m128) = vmsumfp_result(_mm_dp_ps::<0xEF>(a, b))
}
vop! {
    /// `vmsum4fp128`: the four-lane dot product.
    vmsum4fp(a: __m128, b: __m128) = vmsumfp_result(_mm_dp_ps::<0xFF>(a, b))
}

/// `vrsqrtefp` — the table lookup of [`ppc_vrsqrtefp_bits`], one lane at a time.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vrsqrtefp(v: __m128) -> __m128 {
    unsafe {
        let mut l = [0f32; 4];
        _mm_storeu_ps(l.as_mut_ptr(), v);
        for x in l.iter_mut() {
            *x = f32::from_bits(ppc_vrsqrtefp_bits(x.to_bits()));
        }
        _mm_loadu_ps(l.as_ptr())
    }
}

/// `vexptefp128` — scalar `exp2f` per lane.
///
/// **Rule 5: this is the one operation whose exactness rests on something outside the
/// translation.** It matches only because Rust's `f32::exp2` and glibc's `exp2f` are the same
/// symbol on this platform. Keep it in the recorded bit-compare permanently and re-check it on any
/// toolchain or libc change; do not treat it as settled.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vexptefp(v: __m128) -> __m128 {
    unsafe {
        let mut l = [0f32; 4];
        _mm_storeu_ps(l.as_mut_ptr(), v);
        for x in l.iter_mut() {
            *x = x.exp2();
        }
        _mm_loadu_ps(l.as_ptr())
    }
}

/// `vlogefp128` — scalar `log2f` per lane. See [`vexptefp`]: rule 5 applies identically.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vlogefp(v: __m128) -> __m128 {
    unsafe {
        let mut l = [0f32; 4];
        _mm_storeu_ps(l.as_mut_ptr(), v);
        for x in l.iter_mut() {
            *x = x.log2();
        }
        _mm_loadu_ps(l.as_ptr())
    }
}

// -------------------------------------------------------------------------------- conversions

/// `vcsxwfp128 vD,vS,n` — signed word to float, scaled by `2^-n`.
///
/// `n == 0` is a plain `cvtdq2ps`; any other `n` multiplies by the single whose exponent field is
/// `(127 - n) << 23`, which is what the lifted form materialises.
///
/// # Safety
/// Requires SSE4.1 and FMA.
#[inline]
#[target_feature(enable = "sse4.1,fma")]
pub unsafe fn vcsxwfp<const N: u32>(a: __m128i) -> __m128 {
    unsafe {
        let converted = _mm_cvtepi32_ps(a);
        if N == 0 {
            converted
        } else {
            let scale = ((127 - N) << 23) as i32;
            _mm_mul_ps(converted, _mm_castsi128_ps(_mm_set1_epi32(scale)))
        }
    }
}

/// `vcuxwfp128 vD,vS,n` — unsigned word to float, scaled by `2^-n`.
///
/// # Safety
/// Requires SSE4.1 and FMA.
#[inline]
#[target_feature(enable = "sse4.1,fma")]
pub unsafe fn vcuxwfp<const N: u32>(a: __m128i) -> __m128 {
    unsafe {
        let converted = cvtepu32_ps(a);
        if N == 0 {
            converted
        } else {
            let scale = ((127 - N) << 23) as i32;
            _mm_mul_ps(converted, _mm_castsi128_ps(_mm_set1_epi32(scale)))
        }
    }
}

/// `vcfpsxws128` — float to signed word, PPC saturation.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vcfpsxws(a: __m128) -> __m128i {
    unsafe { vctsxs(a) }
}

/// `vcfpuxws128` — float to unsigned word, PPC saturation.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vcfpuxws(a: __m128) -> __m128i {
    unsafe { vctuxs(a) }
}

// ------------------------------------------------------------------------ logic, permute, integer

macro_rules! iop {
    ($(#[$m:meta])* $name:ident($a:ident: __m128i $(, $b:ident: __m128i)*) = $body:expr) => {
        $(#[$m])*
        ///
        /// # Safety
        /// Requires SSE4.1.
        #[inline]
        #[target_feature(enable = "sse4.1")]
        pub unsafe fn $name($a: __m128i $(, $b: __m128i)*) -> __m128i {
            unsafe { $body }
        }
    };
}

iop! {
    /// `vand128`.
    vand(a: __m128i, b: __m128i) = _mm_and_si128(a, b)
}
iop! {
    /// `vandc128` — `a & ~b`, so the SSE operands are the other way round.
    vandc(a: __m128i, b: __m128i) = _mm_andnot_si128(b, a)
}
iop! {
    /// `vor128`. Also the register move the lifted code writes as `vor128 vD,vS,vS`.
    vor(a: __m128i, b: __m128i) = _mm_or_si128(a, b)
}
iop! {
    /// `vnor128`.
    vnor(a: __m128i, b: __m128i) = _mm_xor_si128(_mm_or_si128(a, b), _mm_set1_epi32(-1))
}
iop! {
    /// `vxor128`.
    vxor(a: __m128i, b: __m128i) = _mm_xor_si128(a, b)
}
iop! {
    /// `vsel` — per **bit**, not per lane, which is what `blendv_epi8` gives once the selector is
    /// all-ones or all-zeroes bytes. RexGlue uses `blendv_epi8` verbatim.
    vsel(a: __m128i, b: __m128i, c: __m128i) = _mm_blendv_epi8(a, b, c)
}
iop! {
    /// `vperm128`.
    vperm(a: __m128i, b: __m128i, c: __m128i) = perm_epi8(a, b, c)
}
iop! {
    /// `vadduwm` — modular word add.
    vadduwm(a: __m128i, b: __m128i) = _mm_add_epi32(a, b)
}
iop! {
    /// `vaddsws`. RexGlue lowers it to the **modular** add, which differs from the PPC instruction
    /// on overflow. Reproduced because the recomp is the reference.
    vaddsws(a: __m128i, b: __m128i) = _mm_add_epi32(a, b)
}
iop! {
    /// `vsubsws`. Modular, like [`vaddsws`].
    vsubsws(a: __m128i, b: __m128i) = _mm_sub_epi32(a, b)
}
iop! {
    /// `vaddshs` — saturating signed halfword add.
    vaddshs(a: __m128i, b: __m128i) = _mm_adds_epi16(a, b)
}
iop! {
    /// `vpkshus128` — signed halfword to unsigned byte, saturating.
    vpkshus(a: __m128i, b: __m128i) = _mm_packus_epi16(a, b)
}
iop! {
    /// `vpkswss128` — signed word to signed halfword, saturating.
    vpkswss(a: __m128i, b: __m128i) = _mm_packs_epi32(a, b)
}
iop! {
    /// `vupkhsb128` — the low eight bytes sign-extended to halfwords.
    vupkhsb(a: __m128i) = _mm_cvtepi8_epi16(a)
}
iop! {
    /// `vmrghw128`.
    vmrghw(a: __m128i, b: __m128i) = _mm_unpackhi_epi32(a, b)
}
iop! {
    /// `vmrglw128`.
    vmrglw(a: __m128i, b: __m128i) = _mm_unpacklo_epi32(a, b)
}

/// `vsraw128 vD,vS,n` — arithmetic word shift right by a constant.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vsraw<const N: i32>(a: __m128i) -> __m128i {
    unsafe { _mm_srai_epi32::<N>(a) }
}

/// `vslw128 vD,vS,n`.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vslw<const N: i32>(a: __m128i) -> __m128i {
    unsafe { _mm_slli_epi32::<N>(a) }
}

/// `vsrw128 vD,vS,n`.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vsrw<const N: i32>(a: __m128i) -> __m128i {
    unsafe { _mm_srli_epi32::<N>(a) }
}

/// `vsldoi128 vD,vA,vB,n` — the 32-byte pair `vA:vB` shifted left by `n` bytes, big-endian.
///
/// RexGlue emits `_mm_alignr_epi8(a, b, 16 - n)` … except that the lifted form the probe records
/// is `alignr(a, b, 4)`, so the immediate here is the **x86** one, kept as lifted.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vsldoi<const N: i32>(a: __m128i, b: __m128i) -> __m128i {
    unsafe { _mm_alignr_epi8::<N>(a, b) }
}

/// `vspltw128 vD,vS,0` — splat PPC element 0, which is **host lane 3**.
pub const SPLAT_W0: i32 = 0xFF;
/// `vspltw128 vD,vS,1` — host lane 2.
pub const SPLAT_W1: i32 = 0xAA;
/// `vspltw128 vD,vS,2` — host lane 1.
pub const SPLAT_W2: i32 = 0x55;
/// `vspltw128 vD,vS,3` — host lane 0.
pub const SPLAT_W3: i32 = 0x00;

/// The `shuffle_epi32` immediate for a given PPC element number, computed rather than recalled.
///
/// The lifted code carries the already-reversed immediate, so call sites keep that number and use
/// this only to assert the mapping at compile time.
pub const fn splat_imm(ppc_element: u32) -> i32 {
    let lane = 3 - ppc_element; // PPC counts from the most significant word
    (lane | (lane << 2) | (lane << 4) | (lane << 6)) as i32
}

const _: () = assert!(
    splat_imm(0) == SPLAT_W0,
    "vspltw128 element 0 splats host lane 3"
);
const _: () = assert!(splat_imm(1) == SPLAT_W1);
const _: () = assert!(splat_imm(2) == SPLAT_W2);
const _: () = assert!(splat_imm(3) == SPLAT_W3);

/// `vspltw128 vD,vS,n`, with the **reversed** immediate the lifted code carries.
///
/// Pass [`SPLAT_W0`] … [`SPLAT_W3`], or the raw number from the lifted line — they are the same
/// values. Getting this backwards is silent: it compiles, runs, and returns a different
/// coefficient.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn vspltw128<const IMM8: i32>(a: __m128i) -> __m128i {
    unsafe { _mm_shuffle_epi32::<IMM8>(a) }
}

// ------------------------------------------------------------------------------- guest memory

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn bytes_of(v: __m128i) -> [u8; 16] {
    let mut out = [0u8; 16];
    unsafe { _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, v) };
    out
}

/// `lvx128 vD,rA,rB` — sixteen bytes at `ea & ~0xF`, reversed.
///
/// The low four address bits are **discarded**, not faulted on: a misaligned `lvx128` reads the
/// containing 16-byte block. Every call site in `crate::dsp` relies on that being a no-op because
/// its operand is 16-byte aligned, and each says so.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn lvx128(g: &Guest, ea: u32) -> Result<__m128i> {
    let block = ea & !0xF;
    let src = g.span(block, 16)?;
    unsafe {
        Ok(_mm_shuffle_epi8(
            _mm_loadu_si128(src.as_ptr() as *const __m128i),
            mask_l(0),
        ))
    }
}

/// `lvx128` returning float lanes. The bits are the same; this only saves a cast at the call site.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn lvx128_ps(g: &Guest, ea: u32) -> Result<__m128> {
    unsafe { Ok(_mm_castsi128_ps(lvx128(g, ea)?)) }
}

/// `stvx128 vS,rA,rB` — the same masking and the same reversal on the way out.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn stvx128(g: &mut Guest, ea: u32, value: __m128i) -> Result<()> {
    let block = ea & !0xF;
    let swapped = unsafe { bytes_of(_mm_shuffle_epi8(value, mask_l(0))) };
    g.set_span(block, &swapped)
}

/// `stvx128` taking float lanes.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn stvx128_ps(g: &mut Guest, ea: u32, value: __m128) -> Result<()> {
    unsafe { stvx128(g, ea, _mm_castps_si128(value)) }
}

/// `lvlx128 vD,rA,rB` — the bytes from `ea` up to the end of its 16-byte block, left-justified,
/// the rest zero.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn lvlx128(g: &Guest, ea: u32) -> Result<__m128i> {
    let block = ea & !0xF;
    let src = g.span(block, 16)?;
    unsafe {
        Ok(_mm_shuffle_epi8(
            _mm_loadu_si128(src.as_ptr() as *const __m128i),
            mask_l((ea & 0xF) as usize),
        ))
    }
}

/// `lvrx128 vD,rA,rB` — the bytes below `ea` inside its 16-byte block, right-justified.
///
/// An `ea` that is already 16-byte aligned reads **nothing**: the lifted form short-circuits to
/// `setzero` rather than indexing row 0 of the mask, and row 0 of `VectorMaskR` is all `0xFF`
/// anyway, so the two agree. The branch is reproduced because the reference has it.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn lvrx128(g: &Guest, ea: u32) -> Result<__m128i> {
    if (ea & 0xF) == 0 {
        return Ok(unsafe { _mm_setzero_si128() });
    }
    let block = ea & !0xF;
    let src = g.span(block, 16)?;
    unsafe {
        Ok(_mm_shuffle_epi8(
            _mm_loadu_si128(src.as_ptr() as *const __m128i),
            mask_r((ea & 0xF) as usize),
        ))
    }
}

/// `stvlx128 vS,rA,rB` — `16 - (ea & 0xF)` bytes, from the register's **high** end, into
/// **ascending** addresses starting at `ea`.
///
/// The lifted lowering is a byte loop, not a masked store:
/// ```text
/// for (i = 0; i < 16 - (ea & 0xF); i++) REX_STORE_U8(ea + i, v.u8[15 - i]);
/// ```
/// An aligned `ea` writes all sixteen bytes and is then exactly a `stvx128`.
/// Contrast [`stvrx128`], whose lane order is the opposite in both respects.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn stvlx128(g: &mut Guest, ea: u32, value: __m128i) -> Result<()> {
    let lanes = unsafe { bytes_of(value) };
    let n = 16 - (ea & 0xF) as usize;
    let mut out = [0u8; 16];
    for i in 0..n {
        out[i] = lanes[15 - i];
    }
    g.set_span(ea, &out[..n])
}

/// `stvrx128 vS,rA,rB` — `ea & 0xF` bytes, from the register's **low** end, into **descending**
/// addresses below `ea`.
///
/// ```text
/// for (i = 0; i < (ea & 0xF); i++) REX_STORE_U8(ea - i - 1, v.u8[i]);
/// ```
/// An aligned `ea` writes nothing at all. Together with a [`stvlx128`] at the same address the pair
/// stores one whole vector across a block boundary.
///
/// # Safety
/// Requires SSE4.1.
#[inline]
#[target_feature(enable = "sse4.1")]
pub unsafe fn stvrx128(g: &mut Guest, ea: u32, value: __m128i) -> Result<()> {
    let lanes = unsafe { bytes_of(value) };
    let n = (ea & 0xF) as usize;
    if n == 0 {
        return Ok(());
    }
    // Descending addresses `ea-1 … ea-n` hold `lanes[0 … n-1]`, so the block starting at `ea - n`
    // is that run reversed.
    let mut out = [0u8; 16];
    for i in 0..n {
        out[n - 1 - i] = lanes[i];
    }
    g.set_span(ea - n as u32, &out[..n])
}

/// `dcbzl rA,rB` — establish the 128-byte cache line containing `ea` as zeroes.
///
/// **This is a real write to guest memory**, 128 bytes at `ea & ~127`, and it is inside the write
/// window the C++ ports declare. It is not a prefetch hint and must not be dropped; `dcbt`, which
/// sits next to it in the same loops, *is* a hint and lifts to nothing at all.
/// The message a body refuses with when it would splat a constant through a stack frame that is not
/// 16-byte aligned.
///
/// Several originals build a four-lane constant by storing a single four times into their own frame
/// and reloading it with one `lvx128`, which masks the low four address bits. With a frame on a
/// 16-byte boundary the reload reads back exactly what was stored, which is the only case the ports
/// model. With a misaligned frame it would read four bytes somewhere else, so the input is refused
/// rather than answered with a value that merely looks right.
pub const SPLAT_FRAME_UNALIGNED: &str = "a vector loop would splat through a stack frame that is not 16-byte aligned; refused rather than modelled";

/// The error for [`SPLAT_FRAME_UNALIGNED`], carrying the offending stack pointer.
pub fn splat_frame_unaligned(sp: u32) -> crate::Error {
    crate::Error::new(sp, SPLAT_FRAME_UNALIGNED)
}

pub fn dcbzl(g: &mut Guest, ea: u32) -> Result<()> {
    g.fill(ea & !127, 0, 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Host lane order out of an `__m128i`, low lane first.
    fn lanes(v: __m128i) -> [u32; 4] {
        let mut out = [0u32; 4];
        unsafe { _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, v) };
        out
    }

    fn lanes_ps(v: __m128) -> [u32; 4] {
        lanes(unsafe { _mm_castps_si128(v) })
    }

    fn from_lanes(v: [u32; 4]) -> __m128i {
        unsafe { _mm_loadu_si128(v.as_ptr() as *const __m128i) }
    }

    fn from_f32(v: [f32; 4]) -> __m128 {
        unsafe { _mm_loadu_ps(v.as_ptr()) }
    }

    fn guest() -> Guest {
        Guest::single(0x4000_0000, 0x400)
    }

    #[test]
    fn the_splat_immediates_are_reversed() {
        // The trap this names: a reader who assumes `vspltw128 vD,vS,0` is `_MM_SHUFFLE(0,0,0,0)`
        // gets the wrong coefficient, silently. Element 0 is the *most significant* guest word,
        // which after the load reversal is host lane 3.
        assert_eq!(SPLAT_W0, 0xFF);
        assert_eq!(SPLAT_W3, 0x00);
        assert_ne!(
            SPLAT_W0, SPLAT_W3,
            "if these ever coincide the mapping is not being tested"
        );

        let v = from_lanes([10, 11, 12, 13]);
        assert!(supported());
        unsafe {
            // PPC element 0 is the guest's first word, which lvx128 puts in host lane 3 = 13.
            assert_eq!(lanes(vspltw128::<SPLAT_W0>(v)), [13; 4]);
            assert_eq!(lanes(vspltw128::<SPLAT_W1>(v)), [12; 4]);
            assert_eq!(lanes(vspltw128::<SPLAT_W2>(v)), [11; 4]);
            assert_eq!(lanes(vspltw128::<SPLAT_W3>(v)), [10; 4]);
        }
    }

    /// Two f32-exact factors whose product needs 48 bits, the f32 rounding of that product, and the
    /// error the rounding discards. `a * b - rounded` is exactly `error`, and it is representable,
    /// so a fused multiply-add returns it and a separate multiply-then-add returns **zero** — the
    /// two differ by the whole of the discarded bits.
    ///
    /// The first version of this test had the two the wrong way round, asserting the fused form
    /// cancels to zero. It failed, which is how the shape below got found; recorded because a test
    /// that asserts the *unfused* answer would have passed against an unfused translation.
    fn fma_probe() -> (f32, f32, f32, f32) {
        let a = 1.0f32 + f32::EPSILON;
        let b = 1.0f32 - f32::EPSILON;
        let rounded = a * b; // == 1.0
        let error = ((a as f64) * (b as f64) - rounded as f64) as f32; // -2^-46
        assert_ne!(error, 0.0, "the probe needs a product that actually rounds");
        (a, b, rounded, error)
    }

    #[test]
    fn vmaddfp_rounds_twice_as_the_recomp_computes_it() {
        // CORRECTED 2026-09-13: this test used to assert a single rounding. The recomp is built for
        // plain x86-64 with SSE4.1 and no FMA, so SIMDe lowers vmaddfp to `(a * b) + c` in float --
        // the multiply rounds, then the add rounds. Replaying 2,000 recorded calls of the four-lane
        // sine kernel settled it: a two-rounding emulation matches the original on 16 of 16 distinct
        // inputs where a fused one matches 12, and the probe rebuilt with the recomp's own flags
        // agrees on this primitive where the -march=native build did not.
        let (a, b, rounded, error) = fma_probe();
        assert!(supported());
        unsafe {
            let r = vmaddfp(from_f32([a; 4]), from_f32([b; 4]), from_f32([-rounded; 4]));
            // The multiply rounds away exactly the bits the addend then cancels: +0, by bits.
            assert_eq!(
                lanes_ps(r),
                [0; 4],
                "vmaddfp rounds the product before adding"
            );
            // A single-rounding FMA would have kept them. If it agreed, this would prove nothing.
            assert_ne!(
                error.to_bits(),
                0,
                "the input must tell the two forms apart"
            );
        }
    }

    #[test]
    fn vnmsubfp_negates_the_rounded_product_then_adds() {
        // CORRECTED 2026-09-13, for the reason above: SIMDe's fallback is `-(a * b) + c`, the product
        // rounded, negated exactly, then the add rounded.
        let (a, b, rounded, error) = fma_probe();
        assert!(supported());
        unsafe {
            let r = vnmsubfp(from_f32([a; 4]), from_f32([b; 4]), from_f32([rounded; 4]));
            assert_eq!(lanes_ps(r), [0; 4]);
            assert_ne!(
                (-error).to_bits(),
                0,
                "the input must tell the two forms apart"
            );
            // And the sign really is on the product, not the addend.
            let s = vnmsubfp(from_f32([2.0; 4]), from_f32([3.0; 4]), from_f32([1.0; 4]));
            assert_eq!(lanes_ps(s), [(-5.0f32).to_bits(); 4]);
        }
    }

    #[test]
    fn vrfin_rounds_to_nearest_even_and_vrfiz_toward_zero() {
        // vrfin is the uncovered one: the recorded harness has vrfiz only, so this scalar model is
        // all that stands behind it.
        assert!(supported());
        unsafe {
            let v = from_f32([2.5, -2.5, 1.5, -0.5]);
            assert_eq!(
                lanes_ps(vrfin(v)),
                [
                    2.0f32.to_bits(),
                    (-2.0f32).to_bits(),
                    2.0f32.to_bits(),
                    (-0.0f32).to_bits()
                ],
                "round to nearest, ties to even"
            );
            assert_eq!(
                lanes_ps(vrfiz(v)),
                [
                    2.0f32.to_bits(),
                    (-2.0f32).to_bits(),
                    1.0f32.to_bits(),
                    (-0.0f32).to_bits()
                ],
            );
            assert_eq!(lanes_ps(vrfip(v))[0], 3.0f32.to_bits());
            assert_eq!(lanes_ps(vrfim(v))[0], 2.0f32.to_bits());
        }
    }

    #[test]
    fn lvx128_puts_the_first_guest_word_in_the_top_lane() {
        let mut g = guest();
        for i in 0..4u32 {
            g.set_u32(0x4000_0000 + i * 4, 0x1000 + i).unwrap();
        }
        assert!(supported());
        unsafe {
            // Guest word 0 -> host lane 3. Getting this backwards is the whole BE/LE trap.
            assert_eq!(
                lanes(lvx128(&g, 0x4000_0000).unwrap()),
                [0x1003, 0x1002, 0x1001, 0x1000]
            );
            // The low four address bits are discarded, so every address in the block reads the same
            // sixteen bytes.
            for off in 0..16u32 {
                assert_eq!(
                    lanes(lvx128(&g, 0x4000_0000 + off).unwrap()),
                    [0x1003, 0x1002, 0x1001, 0x1000]
                );
            }
        }
    }

    #[test]
    fn stvx128_is_the_inverse_of_lvx128() {
        let mut g = guest();
        assert!(supported());
        unsafe {
            stvx128(
                &mut g,
                0x4000_0020,
                from_lanes([0xAAAA_AAAA, 0xBBBB_BBBB, 0xCCCC_CCCC, 0xDDDD_DDDD]),
            )
            .unwrap();
            // Host lane 3 lands at the lowest guest address.
            assert_eq!(g.u32(0x4000_0020).unwrap(), 0xDDDD_DDDD);
            assert_eq!(g.u32(0x4000_002C).unwrap(), 0xAAAA_AAAA);
            assert_eq!(
                lanes(lvx128(&g, 0x4000_0020).unwrap()),
                [0xAAAA_AAAA, 0xBBBB_BBBB, 0xCCCC_CCCC, 0xDDDD_DDDD]
            );
            // Address masking on the store side too.
            stvx128(&mut g, 0x4000_002F, from_lanes([1, 2, 3, 4])).unwrap();
            assert_eq!(g.u32(0x4000_0020).unwrap(), 4);
        }
    }

    #[test]
    fn the_partial_stores_run_in_opposite_directions() {
        // stvlx takes the register's high bytes upward from ea; stvrx takes its low bytes downward
        // from ea. Writing either one with the other's order compiles and is silently wrong.
        let mut g = guest();
        let v = from_lanes([0x0302_0100, 0x0706_0504, 0x0B0A_0908, 0x0F0E_0D0C]);
        // Register bytes, low to high: 00 01 02 ... 0F.
        assert!(supported());
        unsafe {
            stvlx128(&mut g, 0x4000_0005, v).unwrap();
            // 11 bytes at 0x...05: u8[15], u8[14], ... u8[5] == 0x0F, 0x0E, ... 0x05.
            assert_eq!(
                g.span(0x4000_0005, 11).unwrap(),
                &[
                    0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05
                ]
            );
            assert_eq!(g.u8(0x4000_0004).unwrap(), 0, "nothing below ea");
            assert_eq!(g.u8(0x4000_0010).unwrap(), 0, "nothing past the block");

            let mut h = guest();
            stvrx128(&mut h, 0x4000_0105, v).unwrap();
            // 5 bytes below 0x...05: address 0x04 gets u8[0], 0x03 gets u8[1] ... 0x00 gets u8[4].
            assert_eq!(
                h.span(0x4000_0100, 5).unwrap(),
                &[0x04, 0x03, 0x02, 0x01, 0x00]
            );
            assert_eq!(h.u8(0x4000_0105).unwrap(), 0, "nothing at or above ea");

            // The two halves of a boundary-straddling vector, together, are one whole vector.
            let mut j = guest();
            stvlx128(&mut j, 0x4000_0205, v).unwrap();
            stvrx128(&mut j, 0x4000_0215, v).unwrap();
            assert_eq!(
                j.span(0x4000_0205, 16).unwrap(),
                &[
                    0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03,
                    0x02, 0x01, 0x00
                ]
            );
        }
    }

    #[test]
    fn the_partial_stores_degenerate_correctly_when_aligned() {
        let mut g = guest();
        let v = from_lanes([1, 2, 3, 4]);
        assert!(supported());
        unsafe {
            stvlx128(&mut g, 0x4000_0010, v).unwrap();
            // Aligned stvlx is a whole stvx128.
            let mut h = guest();
            stvx128(&mut h, 0x4000_0010, v).unwrap();
            assert_eq!(
                g.span(0x4000_0010, 16).unwrap(),
                h.span(0x4000_0010, 16).unwrap()
            );

            // Aligned stvrx writes nothing.
            let mut j = guest();
            stvrx128(&mut j, 0x4000_0010, v).unwrap();
            assert_eq!(j.span(0x4000_0000, 0x40).unwrap(), &[0u8; 0x40][..]);
        }
    }

    #[test]
    fn lvlx_and_lvrx_split_a_block_at_the_address() {
        let mut g = guest();
        for i in 0..16u32 {
            g.set_u8(0x4000_0000 + i, i as u8).unwrap();
        }
        assert!(supported());
        unsafe {
            // lvlx at +5 takes bytes 5..16, left-justified: register byte 15 is guest byte 5.
            let l = bytes_of(lvlx128(&g, 0x4000_0005).unwrap());
            assert_eq!(l[15], 5);
            assert_eq!(l[5], 15);
            assert_eq!(
                &l[0..5],
                &[0, 0, 0, 0, 0],
                "the tail is zero-filled by the 0xFF mask bytes"
            );

            // lvrx at +5 takes bytes 0..5, right-justified: register byte 0 is guest byte 4.
            let r = bytes_of(lvrx128(&g, 0x4000_0005).unwrap());
            assert_eq!(r[0], 4);
            assert_eq!(r[4], 0);
            assert_eq!(&r[5..16], &[0u8; 11][..]);

            // Aligned lvrx is the zero short-circuit.
            assert_eq!(bytes_of(lvrx128(&g, 0x4000_0000).unwrap()), [0u8; 16]);
        }
    }

    #[test]
    fn dcbzl_clears_a_whole_128_byte_line() {
        let mut g = Guest::single(0x4000_0000, 0x400);
        g.fill(0x4000_0000, 0xAB, 0x400).unwrap();
        // An address in the middle of a line clears the whole line, not from the address.
        dcbzl(&mut g, 0x4000_0190).unwrap();
        assert_eq!(g.span(0x4000_0180, 128).unwrap(), &[0u8; 128][..]);
        assert_eq!(
            g.u8(0x4000_017F).unwrap(),
            0xAB,
            "below the line is untouched"
        );
        assert_eq!(
            g.u8(0x4000_0200).unwrap(),
            0xAB,
            "above the line is untouched"
        );
        // 128, not 32 and not 64: a smaller clear would leave the tail set.
        assert_eq!(
            g.span(0x4000_0180, 128)
                .unwrap()
                .iter()
                .filter(|b| **b == 0)
                .count(),
            128
        );
    }

    #[test]
    fn flush_mode_is_on_in_both_directions_and_restores_on_drop() {
        let before = get_mxcsr();
        {
            let mut f = Fpscr::capture();
            f.disable_flush_mode_unconditional();
            assert_eq!(
                f.current() & (FLUSH_ZERO | DENORMALS_ZERO),
                FLUSH_ZERO | DENORMALS_ZERO,
                "`disableFlushMode` does NOT clear FZ/DAZ — InitHost or's FlushMask into fpu_csr"
            );
            f.enable_flush_mode_unconditional();
            assert_eq!(f.current(), VMX_CSR);
            assert_eq!(f.current() & ROUND_MASK, ROUND_NEAREST);
            assert_eq!(
                f.current() & EXCEPTION_MASK,
                EXCEPTION_MASK,
                "no FP traps under guest code"
            );
        }
        assert_eq!(get_mxcsr(), before, "the entry MXCSR is restored on drop");
    }

    #[test]
    fn flush_to_zero_actually_flushes() {
        // Not decoration: the scalar paths of `dsp::scale` run under `disable_flush_mode`, and a
        // translation that cleared FZ/DAZ there would keep denormals the recomp discards.
        let tiny = f32::from_bits(0x0080_0000); // smallest normal
        assert!(supported());
        let mut f = Fpscr::capture();
        f.enable_flush_mode_unconditional();
        let flushed = unsafe { lanes_ps(vmulfp(from_f32([tiny; 4]), from_f32([0.5; 4]))) };
        assert_eq!(flushed, [0x0000_0000; 4], "a denormal result flushes to +0");
        drop(f);

        // With FZ off the same product is the denormal it mathematically is, which is what makes
        // the assertion above a real one.
        set_mxcsr(get_mxcsr() & !(FLUSH_ZERO | DENORMALS_ZERO));
        let kept = unsafe { lanes_ps(vmulfp(from_f32([tiny; 4]), from_f32([0.5; 4]))) };
        set_mxcsr(get_mxcsr() | FLUSH_ZERO | DENORMALS_ZERO);
        set_mxcsr(get_mxcsr() & !(FLUSH_ZERO | DENORMALS_ZERO));
        assert_eq!(kept, [0x0040_0000; 4]);
    }

    #[test]
    fn vrsqrtefp_is_a_table_lookup_not_an_estimate_instruction() {
        // Exact powers of four land on table entries; the point is that the answer is reproducible
        // arithmetic rather than whatever the host's rsqrtps approximates.
        assert_eq!(ppc_vrsqrtefp_bits(0x0000_0000), 0x7F80_0000, "+0 -> +inf");
        assert_eq!(ppc_vrsqrtefp_bits(0x8000_0000), 0xFF80_0000, "-0 -> -inf");
        assert_eq!(
            ppc_vrsqrtefp_bits(0xBF80_0000),
            0x7FC0_0000,
            "negative -> QNaN"
        );
        assert_eq!(ppc_vrsqrtefp_bits(0x7F80_0000), 0, "+inf -> +0");
        let one = f32::from_bits(ppc_vrsqrtefp_bits(1.0f32.to_bits()));
        assert!(
            (one - 1.0).abs() < 1.0 / 4096.0,
            "1/sqrt(1) within estimate tolerance, got {one}"
        );
        let quarter = f32::from_bits(ppc_vrsqrtefp_bits(4.0f32.to_bits()));
        assert!(
            (quarter - 0.5).abs() < 1.0 / 4096.0,
            "1/sqrt(4), got {quarter}"
        );
        // And it is not _mm_rsqrt_ps: the two differ in the low mantissa bits on most inputs.
        assert!(supported());
        let hw = unsafe { lanes_ps(_mm_rsqrt_ps(from_f32([3.0; 4])))[0] };
        let table = ppc_vrsqrtefp_bits(3.0f32.to_bits());
        assert_ne!(
            hw, table,
            "if these agreed, substituting rsqrtps would go unnoticed"
        );
    }

    #[test]
    fn vandc_takes_the_complement_of_its_second_operand() {
        // The operand order flips relative to SSE's andnot, which is the kind of one-character
        // mistake that survives every test that only uses symmetric inputs.
        assert!(supported());
        unsafe {
            let a = from_lanes([0xFFFF_0000; 4]);
            let b = from_lanes([0xFF00_FF00; 4]);
            assert_eq!(lanes(vandc(a, b)), [0x00FF_0000; 4]);
            assert_ne!(
                lanes(vandc(a, b)),
                lanes(vandc(b, a)),
                "asymmetric inputs, on purpose"
            );
        }
    }

    #[test]
    fn vsel_selects_per_bit() {
        assert!(supported());
        unsafe {
            let a = from_lanes([0x0000_0000; 4]);
            let b = from_lanes([0xFFFF_FFFF; 4]);
            // A whole-byte selector, which is all blendv_epi8 can express.
            let c = from_lanes([0xFF00_FF00; 4]);
            assert_eq!(lanes(vsel(a, b, c)), [0xFF00_FF00; 4]);
        }
    }

    #[test]
    fn integer_ops_that_are_not_what_their_mnemonic_says() {
        assert!(supported());
        unsafe {
            // vaddsws is "add signed word saturate" on PPC, but RexGlue lowers it to the modular
            // add. The recomp is the reference, so this wraps.
            let big = from_lanes([0x7FFF_FFFF; 4]);
            let one = from_lanes([1; 4]);
            assert_eq!(
                lanes(vaddsws(big, one)),
                [0x8000_0000; 4],
                "modular, not saturating"
            );
            // vaddshs really is saturating.
            let h = from_lanes([0x7FFF_7FFF; 4]);
            let hone = from_lanes([0x0001_0001; 4]);
            assert_eq!(lanes(vaddshs(h, hone)), [0x7FFF_7FFF; 4]);
        }
    }
}
