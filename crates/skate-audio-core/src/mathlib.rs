//! The guest C runtime's float leaves that the audio thread reaches, and the hole where two of
//! them are.
//!
//! Three functions live at `0x82F4DE80`, `0x82F4DED0` and `0x82F4DFB0`. They are not audio code —
//! they are the image's `floor`, sine and cosine — but the spatial layer cannot be written without
//! them, so they get a module of their own rather than being buried in [`crate::spatial`].
//!
//! | | guest | does | `docs/ports.md` | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`floor`] | `sub_82F4DE80` | `floor(double)`, register-only leaf | verified | 2,872,661 | 2,942,889 |
//! | [`Trig::sine`] | `sub_82F4DED0` | sine | **no `.inc` at all** | — | — |
//! | [`Trig::cosine`] | `sub_82F4DFB0` | cosine | **no `.inc` at all** | — | — |
//! | [`log10`] | `sub_82F55068` | `log10(double)`, one callee | verified | 125,566 | 169,118 |
//! | [`log`] | `sub_82F54ED8` | that callee: the natural log | **outside the corpus** | — | — |
//! | [`atan2`] | `sub_82F52318` | `atan2(y, x)`, octant-reduced rational atan | verified | 87,372 | 56,807 |
//!
//! [`atan2`] is replayed against **3,015 recorded calls**, compared by the bits of `f1` and by the
//! sixteen bytes it spills into its caller's frame.
//!
//! [`log10`] is replayed against **3,000 recorded calls, compared by the bits of `f1`** and so is
//! [`log`] with it, since every one of those calls goes through it. That is what stands behind the
//! transcription of a body with no verified reference of its own.
//!
//! `sub_82F4DE80` is the hottest body anyone has ported in this project: 2.87 million calls in a
//! boot session on `RwAudioCore Dac`, more than the sine kernel and the two buffer multiplies put
//! together.
//!
//! ## The two that are not here, and why they are a trait
//!
//! `sub_82F4DED0` and `sub_82F4DFB0` are **outside the 216 audio-thread functions the native sweep
//! covered**, so neither language has a verified reference for them — the same position
//! `crate::dsp::biquad`'s delegate `sub_82B43978` is in. Writing a sine here would be new analysis
//! dressed as a transcription, and substituting `f64::sin` would be worse: it would turn every
//! future comparison of `sub_82B45788` and `sub_82B269C0` from a failure into a meaningless pass,
//! because the two functions differ in their last mantissa bits and the callers `frsp` the result
//! into a single where that difference survives about half the time.
//!
//! So they are a [`Trig`] parameter. [`Unported`] is the default and returns an `Err` naming the
//! guest address — the same choice `crate::eval::dispatch` makes for an unported opcode. [`Closures`]
//! lets a caller that *has* an implementation supply one. The trait takes `&Guest` because the real
//! bodies almost certainly read a polynomial out of the image, and a future port of them must be
//! able to.
//!
//! **Consequence for replay, stated rather than left to be discovered:** a recorded vector for
//! `sub_82B45788` or `sub_82B269C0` cannot be replayed against this crate until those two are
//! ported. Their results feed every value the two functions store.
//!
//! ## The constants, measured
//!
//! `probe/ports/notes/sub_82F4DE80.md` records that its two pool doubles were **not** read from the
//! image — "by shape they are 1.0 and 2^52" — while being careful to load both through the guest
//! map. Reading the validated dump (`probe/harness/out/image/`) at those addresses:
//!
//! | cell | address | inferred | measured |
//! |---|---|---|---|
//! | [`POOL_STEP_DOWN`] | `0x82010108` | 1.0 | **1.0** |
//! | [`POOL_INTEGRAL_MAGNITUDE`] | `0x820514D8` | 2^52 | **1e18** (`0x43ABC16D674EC800`) |
//!
//! The second inference is **wrong**, and the port is unaffected because it reads the cell live.
//! 1e18 is a coherent choice where 2^52 is only a plausible one: it is the largest round decimal
//! magnitude comfortably below `2^63`, which is the bound `fctidz` needs in order not to produce
//! its integer indefinite. Above it every double is already an integer anyway, so the arm that
//! returns `x` unchanged is correct for the whole range it covers. That is noted here and recorded
//! in this session's report; nothing else in the tree depends on the old guess.

use crate::vmx::Fpscr;
use crate::{Error, Guest, Result, fp};

// The two pool doubles. Each address is `((lis_imm & 0xFFFF) << 16) + offset`, computed from the
// immediate and asserted, never read off the disassembly.
const LIS_82010000: u32 = ((-32255i32 as u32) & 0xFFFF) << 16;
const LIS_82050000: u32 = ((-32251i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82010000 == 0x8201_0000, "lis -32255");
const _: () = assert!(LIS_82050000 == 0x8205_0000, "lis -32251");

/// `lfd f13,264(r11)` — measured 1.0, the step taken when the truncation rounded the wrong way.
pub const POOL_STEP_DOWN: u32 = LIS_82010000 + 264;
/// `lfd f0,5336(r10)` — measured 1e18, the magnitude above which a double is already integral.
pub const POOL_INTEGRAL_MAGNITUDE: u32 = LIS_82050000 + 5336;

const _: () = assert!(POOL_STEP_DOWN == 0x8201_0108);
const _: () = assert!(POOL_INTEGRAL_MAGNITUDE == 0x8205_14D8);

/// `sub_82F4DE80` — `floor(double)`.
///
/// `x` is `f1` and the result is `f1`; the mask is `kReturnF1`, so the returned value **is** the
/// whole of the comparison — this function has no stores at all. `r3`…`r8` carry nothing. It reads
/// the two pool doubles and nothing else.
///
/// Four steps, and the third is the one a reader would not write unprompted:
///
/// 1. `trunc = fcfid(fctidz(x))`, truncation toward zero through a 64-bit integer;
/// 2. `fsel` on the signed fraction steps down by [`POOL_STEP_DOWN`] when `x` was negative and had
///    one — which is what turns truncation into a floor;
/// 3. `fsel` on `1e18 - |x|` passes large magnitudes through **unchanged**, because they are
///    already integral and because `fctidz` above `2^63` would not be;
/// 4. `fsel` on `-|x|` passes `±0` through with its sign, since `-0.0 >= 0.0` holds and the
///    subtraction in step 2 would otherwise have turned `-0.0` into `+0.0`.
///
/// A NaN falls to the *else* side of every `fsel` — `>=` is false when unordered — so it comes out
/// through the step-3 arm as `x` itself, payload intact. The port keeps that as three selects
/// rather than collapsing it into `f64::floor`, which differs from this on `±0` and would hide the
/// whole point of steps 3 and 4.
pub fn floor(g: &Guest, x: f64) -> Result<f64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted before fctidz

    let truncated_bits = fp::fctidz(x); // fctidz f12,f1
    let abs_x = fp::abs_double(x); // fabs f11,f1

    let step_down = fp::load_double(g, POOL_STEP_DOWN)?; // lfd f13,264(r11)
    let integral_mag = fp::load_double(g, POOL_INTEGRAL_MAGNITUDE)?; // lfd f0,5336(r10)

    let truncated = fp::fcfid(truncated_bits); // fcfid f12,f12
    // Plain `fsub`, not `fsubs`: nothing on this path narrows to single. All three subtractions
    // below are double, and the port must not "tidy" them into the `.s` forms the rest of the
    // crate is full of.
    let magnitude_margin = integral_mag - abs_x; // fsub f0,f0,f11
    let neg_abs_x = fp::neg_double(abs_x); // fneg f11,f11
    let fraction = x - truncated; // fsub f10,f1,f12
    let stepped_down = truncated - step_down; // fsub f13,f12,f13

    let floored = fp::fsel(fraction, truncated, stepped_down); // fsel f13,f10,f12,f13
    let bounded = fp::fsel(magnitude_margin, floored, x); // fsel f0,f0,f13,f1
    Ok(fp::fsel(neg_abs_x, x, bounded)) // fsel f1,f11,f1,f0
}

/// The image's sine and cosine, which **have no port in either language**.
///
/// `sub_82F4DED0` is the sine and `sub_82F4DFB0` the cosine; both take `f1` and return `f1`, and
/// `probe/ports/notes/sub_82B45788.md` records that each writes only its own red zone — no guest
/// state, no indirect call, no timebase. That is why the two callers here are gate-1 clean while
/// their trigonometry is not available: the census could see inside those bodies, but the sweep
/// never gave them an `.inc`, so there is no verified reference to transcribe.
///
/// Implementations take `&Guest` so that a real port of either can read the image the way every
/// other body in this crate does.
pub trait Trig {
    /// `sub_82F4DED0` — `f1` in, `f1` out.
    fn sine(&mut self, g: &Guest, x: f64) -> Result<f64>;
    /// `sub_82F4DFB0` — `f1` in, `f1` out.
    fn cosine(&mut self, g: &Guest, x: f64) -> Result<f64>;
}

/// The default: an `Err` naming the guest function, never a value.
///
/// Feeding `f64::sin` to a caller here would produce a number that is right to about fifteen
/// digits and wrong in the bits that survive the callers' `frsp`. A wrong answer that looks right
/// is exactly what this crate refuses to produce for an unported callee.
#[derive(Clone, Copy, Debug, Default)]
pub struct Unported;

impl Trig for Unported {
    fn sine(&mut self, _g: &Guest, _x: f64) -> Result<f64> {
        Err(Error::new(
            0x82F4_DED0,
            "sub_82F4DED0 (sine) has no verified port in either language",
        ))
    }
    fn cosine(&mut self, _g: &Guest, _x: f64) -> Result<f64> {
        Err(Error::new(
            0x82F4_DFB0,
            "sub_82F4DFB0 (cosine) has no verified port in either language",
        ))
    }
}

/// The image's own sine and cosine, transcribed from their C++ ports.
///
/// Those ports (`sub_82F4DED0`, `sub_82F4DFB0`) were written on 2026-09-13 and shadow-verified at
/// zero divergence: sine over 436,266 comparable calls under scripted play and 414,229 at boot,
/// cosine over 268,332 and 228,467. Both are a Cody-Waite reduction by pi in two parts followed by
/// an odd Taylor polynomial through x^19, over one table read live. Cosine shifts by pi/2 and
/// offsets the quotient by one half so the sine's polynomial serves unchanged.
///
/// This is what [`Unported`] was waiting for. Substituting `f64::sin` would agree to fifteen digits
/// and differ in the bits the callers keep; this reproduces the guest's own arithmetic instead,
/// down to the fused multiply-adds and the operand order of every multiply.
#[derive(Clone, Copy, Debug, Default)]
pub struct Image;

const TRIG_TABLE: u32 = 0x82FB_0000 + 20000; // lis r11,-32005 ; addi r11,r11,20000
const TRIG_ONE: u32 = 0x8201_0000 + 264; // lis r10,-32255 ; lfd 264(r10) = 1.0
const TRIG_NAN: u32 = 0x82FB_0000 + 22728; // lis r11,-32005 ; lfd 22728(r11) = NaN
const _: () = assert!(
    TRIG_TABLE == 0x82FB_4E20 && TRIG_ONE == 0x8201_0108 && TRIG_NAN == 0x82FB_58C8,
    "((imm & 0xFFFF) << 16) + offset, computed rather than read by eye"
);

/// `lfs`: a single widened exactly to double.
fn lfs(g: &Guest, ea: u32) -> Result<f64> {
    Ok(f32::from_bits(g.u32(ea)?) as f64)
}

/// The odd polynomial both functions share: `p(r^2) * r`, in the guest's Horner order.
fn trig_poly(g: &Guest, r: f64) -> Result<f64> {
    let one = fp::load_double(g, TRIG_ONE)?;
    let c9 = fp::load_double(g, TRIG_TABLE + 112)?;
    let c8 = fp::load_double(g, TRIG_TABLE + 104)?;
    let c7 = fp::load_double(g, TRIG_TABLE + 96)?;
    let c6 = fp::load_double(g, TRIG_TABLE + 88)?;
    let c5 = fp::load_double(g, TRIG_TABLE + 80)?;
    let c4 = fp::load_double(g, TRIG_TABLE + 72)?;
    let c3 = fp::load_double(g, TRIG_TABLE + 64)?;
    let c2 = fp::load_double(g, TRIG_TABLE + 56)?;
    let r2 = r * r; // fmul f13,f9,f9
    let mut p = c9.mul_add(r2, c8); // fmadd f11,f8,f13,f7
    p = p.mul_add(r2, c7);
    p = p.mul_add(r2, c6);
    p = p.mul_add(r2, c5);
    p = p.mul_add(r2, c4);
    p = p.mul_add(r2, c3);
    p = p.mul_add(r2, c2);
    p = p.mul_add(r2, one); // fmadd f3,f4,f13,f30 (f31 in the cosine)
    Ok(p * r) // fmul f13,f3,f9
}

/// `sub_82F4DED0`. `f1` in, `f1` out. `±0` returns the input unchanged, sign included.
fn image_sine(g: &Guest, x: f64) -> Result<f64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // at stfd f30,-16(r1)
    let abs_bits = x.to_bits() & !0x8000_0000_0000_0000;
    let ax = f64::from_bits(abs_bits); // fabs f0,f1
    let inv_pi = fp::load_double(g, TRIG_TABLE + 8)?; // lfd f11,8(r11)
    let minus_one = lfs(g, TRIG_TABLE + 32)?; // lfs f12,32(r11)
    let pi_hi = fp::load_double(g, TRIG_TABLE + 40)?; // lfd f10,40(r11)
    let q = inv_pi * ax; // fmul f13,f11,f0
    let pi_lo = fp::load_double(g, TRIG_TABLE + 48)?; // lfd f9,48(r11)
    let n_rounded = fp::fctid(q); // fctid f11,f13
    let plus_one = lfs(g, TRIG_TABLE + 28)?; // lfs f13,28(r11)
    let sign = fp::fsel(x, plus_one, minus_one); // fsel f12,f1,f13,f12
    let n = fp::fcfid(n_rounded); // fcfid f13,f11
    let r_hi = -pi_hi.mul_add(n, -ax); // fnmsub f11,f10,f13,f0
    let odd = (fp::fctidz(n) & 1) != 0; // fctidz ; stfd ; ld ; clrldi r8,r9,63
    let r = -pi_lo.mul_add(n, -r_hi); // fnmsub f9,f9,f13,f11
    let mut s = trig_poly(g, r)?;
    if odd {
        s = fp::neg_double(s);
    } // fneg f13,f13 (sine)
    fpscr.disable_flush_mode_unconditional(); // loc_82F4DF80
    let signed_s = s * sign; // fmul f12,f13,f12
    let mut result = x; // the +/-0 path never writes f1
    if abs_bits != 0 {
        let limit = fp::load_double(g, TRIG_TABLE + 16)?; // lfd f13,16(r11)
        let over = ax - limit; // fsub f13,f0,f13
        let nan = fp::load_double(g, TRIG_NAN)?; // lfd f0,22728(r11)
        result = fp::fsel(over, nan, signed_s); // fsel f1,f13,f0,f12
    }
    fpscr.disable_flush_mode_unconditional(); // loc_82F4DFA4
    Ok(result)
}

/// `sub_82F4DFB0`. `f1` in, `f1` out. `|x| == 0` returns exactly 1.0.
fn image_cosine(g: &Guest, x: f64) -> Result<f64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // at stfd f31,-8(r1)
    let ax = fp::abs_double(x); // fabs f0,f1
    let half_pi = fp::load_double(g, TRIG_TABLE)?; // lfd f13,0(r11)
    let half = lfs(g, TRIG_TABLE + 36)?; // lfs f11,36(r11)
    let pi_hi = fp::load_double(g, TRIG_TABLE + 40)?; // lfd f10,40(r11)
    let shifted = half_pi + ax; // fadd f12,f13,f0
    let inv_pi = fp::load_double(g, TRIG_TABLE + 8)?; // lfd f13,8(r11)
    let pi_lo = fp::load_double(g, TRIG_TABLE + 48)?; // lfd f9,48(r11)
    let q = inv_pi * shifted; // fmul f13,f13,f12
    let n = fp::fcfid(fp::fctid(q)); // fctid ; fcfid
    let m = n - half; // fsub f11,f13,f11
    let odd = (fp::fctidz(n) & 1) != 0; // fctidz f13,f13 ; clrldi
    let r_hi = -pi_hi.mul_add(m, -ax); // fnmsub f10,f10,f11,f0
    let r = -pi_lo.mul_add(m, -r_hi); // fnmsub f9,f9,f11,f10
    let mut c = trig_poly(g, r)?;
    if odd {
        c = fp::neg_double(c);
    } // fneg f13,f13 (cosine)
    fpscr.disable_flush_mode_unconditional(); // loc_82F4E05C
    let zero = lfs(g, TRIG_TABLE + 24)?; // lfs f11,24(r11)
    if ax == zero {
        return lfs(g, TRIG_TABLE + 28); // fcmpu ; bne not taken ; lfs f1,28(r11) = 1.0
    }
    fpscr.disable_flush_mode_unconditional(); // loc_82F4E074
    let limit = fp::load_double(g, TRIG_TABLE + 16)?; // lfd f0,16(r11)
    let over = shifted - limit; // fsub f12,f12,f0
    let nan = fp::load_double(g, TRIG_NAN)?; // lfd f0,22728(r11)
    Ok(fp::fsel(over, nan, c)) // fsel f1,f12,f0,f13
}

impl Trig for Image {
    fn sine(&mut self, g: &Guest, x: f64) -> Result<f64> {
        image_sine(g, x)
    }
    fn cosine(&mut self, g: &Guest, x: f64) -> Result<f64> {
        image_cosine(g, x)
    }
}

/// Two caller-supplied implementations, for a caller that has them.
///
/// The closure form is what [`crate::player`] already uses for its two unportable callees. It takes
/// the pair explicitly so that nothing in this crate can reach a sine by default.
pub struct Closures<S, C> {
    sine: S,
    cosine: C,
}

impl<S, C> Closures<S, C>
where
    S: FnMut(&Guest, f64) -> Result<f64>,
    C: FnMut(&Guest, f64) -> Result<f64>,
{
    pub fn new(sine: S, cosine: C) -> Self {
        Self { sine, cosine }
    }
}

impl<S, C> Trig for Closures<S, C>
where
    S: FnMut(&Guest, f64) -> Result<f64>,
    C: FnMut(&Guest, f64) -> Result<f64>,
{
    fn sine(&mut self, g: &Guest, x: f64) -> Result<f64> {
        (self.sine)(g, x)
    }
    fn cosine(&mut self, g: &Guest, x: f64) -> Result<f64> {
        (self.cosine)(g, x)
    }
}

// ------------------------------------------------------------- the natural log, and log10 over it

/// `lis r11,-32005` — `0x82FB0000`, where the two infinity cells live.
const LIS_82FB0000: u32 = ((-32005i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82FB0000 == 0x82FB_0000, "lis -32005");

/// `lfd f1,248(r11)` — measured `0.0`, the result of `log(1.0)`.
pub const POOL_ZERO: u32 = LIS_82010000 + 248;
/// `lfd f6,264(r11)` — measured `1.0`. The same cell [`POOL_STEP_DOWN`] names, reached by a
/// different function through a different immediate.
pub const POOL_ONE: u32 = LIS_82010000 + 264;
/// `lfd f0,648(r9)` — measured `0.5`.
pub const POOL_HALF: u32 = LIS_82010000 + 648;
/// `lfd f0,22720(r11)` — measured `+inf`. Returned **negated**, for `log(0)`.
pub const POOL_INFINITY: u32 = LIS_82FB0000 + 22720;
/// `lfd f0,22728(r11)` — measured `0xFFF8000000000000`, a negative quiet NaN. Also returned
/// negated, so a negative argument *and* a NaN argument both come back as `0x7FF8000000000000`.
pub const POOL_NAN: u32 = LIS_82FB0000 + 22728;

/// `addi r11,r11,8344` — the natural log's own pool: eighteen doubles from `0x82052098`.
///
/// Every value below was read out of `probe/harness/out/image/` rather than inferred, which matters
/// here more than usual: the coefficients of a rational approximation are exactly the kind of
/// constant that looks right while being one cell off.
pub const LOG_POOL: u32 = LIS_82050000 + 8344;

/// `+0`, measured `0.7071067811865476` — sqrt(2)/2, which side of it the significand falls on
/// choosing the reduction.
pub const LOG_SQRT_HALF: u32 = LOG_POOL;
/// `+8`, measured `0.693359375` — the high part of ln 2, exact in 12 bits.
pub const LOG_LN2_HI: u32 = LOG_POOL + 8;
/// `+24`, measured `0.4342944819032518` — log10(e), used only by [`log10`]. One ULP from what
/// `1.0 / 10f64.ln()` computes, so it is a tabulated constant rather than a derived one.
pub const LOG10_OF_E: u32 = LOG_POOL + 24;
/// `+40`, measured `16.383943563021536`.
pub const LOG_NUM_B: u32 = LOG_POOL + 40;
/// `+64`, measured `312.03222091924533`.
pub const LOG_DEN_B: u32 = LOG_POOL + 64;
/// `+80`, measured `0.00021219444005469057` — the low part of ln 2. `LN2_HI - LN2_LO` is
/// `0.6931471805599453`, the correctly rounded ln 2, which is what makes the split exact.
pub const LOG_LN2_LO: u32 = LOG_POOL + 80;
/// `+88`, measured `769.4993210849487`.
pub const LOG_DEN_C: u32 = LOG_POOL + 88;
/// `+96`, measured `64.12494342374558`.
pub const LOG_NUM_C: u32 = LOG_POOL + 96;
/// `+104`, measured `35.66797773903465`.
pub const LOG_DEN_A: u32 = LOG_POOL + 104;
/// `+112`, measured `0.7895611288749126`.
pub const LOG_NUM_A: u32 = LOG_POOL + 112;
/// `+120`, measured `2^53` — the scale that lifts a denormal into the normal range.
pub const LOG_TWO_53: u32 = LOG_POOL + 120;
/// `+128`, measured `DBL_MIN`.
pub const LOG_MIN_NORMAL: u32 = LOG_POOL + 128;
/// `+136`, measured `DBL_MAX`.
pub const LOG_MAX_FINITE: u32 = LOG_POOL + 136;

const _: () = assert!(LOG_POOL == 0x8205_2098, "lis -32251 ; addi 8344");
const _: () = assert!(LOG10_OF_E == 0x8205_20B0, "lis -32251 ; lfd 8368");
const _: () = assert!(LOG_MAX_FINITE == 0x8205_2120 && LOG_MIN_NORMAL == 0x8205_2118);
const _: () = assert!(POOL_NAN == 0x82FB_58C8 && POOL_INFINITY == 0x82FB_58C0);

/// `sub_82F54ED8` — the C runtime's natural log, `f1` in and `f1` out.
///
/// **Outside the audio corpus, and so outside the native sweep**: there is no `.inc` for it, and
/// `probe/ports/notes/sub_82F55068.md` records that it was extracted with
/// `tools/extract_lifted.py --one 82F54ED8` for exactly this reason. Its caller [`log10`] *is*
/// verified, which is the only reason this can be transcribed rather than invented — the lifted
/// body is the specification, line by line, and the constants above are measured.
///
/// Cody-Waite, in the shape every CRT of that generation uses: split the argument into `2^n · y`
/// with `y` in `[0.5, 1)`, reduce `y` to `z = (y − 1)/(y + 1)` (or the `y − 0.5` variant, depending
/// which side of sqrt(2)/2 it fell), evaluate a rational in `z²`, and add `n · ln 2` back in two
/// parts so the multiplication by the large `n` cannot lose the low bits.
///
/// The parts a rewrite gets wrong, all of them pinned by the tests:
///
/// - **Four special cases come out of the image, not out of arithmetic.** `log(1)` returns the
///   `0.0` *cell*; `log(0)` returns the negation of the `+inf` cell; a negative argument and a NaN
///   argument both return the negation of the NaN cell, which is `0x7FF8000000000000` — so an input
///   NaN's payload is **discarded**, not propagated. `log(+inf)` returns its argument, decided by
///   a `> DBL_MAX` test rather than by a class check.
/// - **The exponent test is integer.** `lhz` of the double's top halfword, `& 0x7FF0`, compared
///   against `0x7FF0`. The body never asks `is_nan`.
/// - **The denormal branch is dead in this build, and reproduced anyway.** It scales by `2^53` and
///   biases the exponent by 1075 instead of 1022. Under the flush mode RexGlue actually runs
///   (`FZ|DAZ` in *both* of its modes — [`crate::vmx::Fpscr`]), a denormal argument compares equal
///   to zero, so the body returns −inf before reaching it. On hardware without `DAZ` the branch is
///   live, which is why it is here.
/// - **Every multiply-add is a scalar `fma`,** one rounding, and the divides are plain `fdiv`.
///   That is `std::fma` in the lifted body and `mul_add` here; it is unrelated to the two-rounding
///   vector multiply-adds of [`crate::vmx`].
///
/// Not reproduced: the spills at `16(r1)` and `−16(r1)`. They move a double's bits into a GPR and
/// back, which is what `to_bits`/`from_bits` do here, and the `.inc` for [`log10`] reproduces the
/// stack frame only so those spills land at the same guest addresses. No window covers them.
pub fn log(g: &Guest, x: f64) -> Result<f64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at stfd f1,16(r1)

    let one = fp::load_double(g, POOL_ONE)?; // lfd f6,264(r11)
    if x == one {
        // fcmpu cr6,f1,f6 ; bne ; lfd f1,248(r11) ; blr
        return fp::load_double(g, POOL_ZERO);
    }

    // lhz r10,16(r1) ; rlwinm r11,r10,0,17,27 ; cmplwi cr6,r11,32752
    let high = (x.to_bits() >> 48) as u32;
    if high & 0x7FF0 == 0x7FF0 {
        if x > fp::load_double(g, LOG_MAX_FINITE)? {
            return Ok(x); // bgtlr cr6 -- +inf returns itself; a NaN is not greater
        }
        // loc_82F54F1C: -inf and every NaN leave through the NaN cell, negated
        return Ok(fp::neg_double(fp::load_double(g, POOL_NAN)?));
    }

    let zero = fp::load_double(g, POOL_ZERO)?; // lfd f0,248(r10)
    if !(x > zero) {
        // fcmpu cr6,f1,f0 ; bgt -> the normal path
        if x == zero {
            // log(±0) = -inf, and the sign of the zero does not matter
            return Ok(fp::neg_double(fp::load_double(g, POOL_INFINITY)?));
        }
        return Ok(fp::neg_double(fp::load_double(g, POOL_NAN)?)); // x < 0
    }

    // loc_82F54F50. Under FZ|DAZ this test is unreachable for a denormal, which compares equal to
    // zero above; the branch is the original's and is kept.
    let min_normal = fp::load_double(g, LOG_MIN_NORMAL)?; // lfd f0,8472(r10)
    let (value, mut n) = if x < min_normal {
        let scaled = x * fp::load_double(g, LOG_TWO_53)?; // lfd f0,8464(r11) ; fmul f1,f1,f0
        let high = (scaled.to_bits() >> 48) as u32; // stfd f1,16(r1) ; lhz r11,16(r1)
        (scaled, ((high >> 4) & 0x7FF) as i32 - 1075) // rlwinm r10,r11,28,21,31 ; addi -1075
    } else {
        // loc_82F54F88: `rlwinm r11,r11,28,20,31` runs on the *masked* halfword from above, so the
        // sign bit it could have kept is already gone and this is the same eleven bits.
        (x, ((high >> 4) & 0x7FF) as i32 - 1022) // addi r10,r11,-1022
    };

    // loc_82F54F90. andi. r9,r9,32783 ; ori r9,r9,16352 ; sth r9,-16(r1) ; lfd f13,-16(r1):
    // replace the exponent with 0x3FE and keep the sign and the mantissa, which puts the
    // significand in [0.5, 1). The `andi.` sets cr0 and nothing reads it.
    let bits = value.to_bits();
    let significand_high = ((bits >> 48) as u32 & 32783) | 16352;
    let y = f64::from_bits((bits & 0x0000_FFFF_FFFF_FFFF) | (u64::from(significand_high) << 48));

    let half = fp::load_double(g, POOL_HALF)?; // lfd f0,648(r9) / lfd f12,648(r9)
    let (numerator_arg, denominator_arg) = if y > fp::load_double(g, LOG_SQRT_HALF)? {
        // fadd f12,f13,f6 ; fsub f11,f13,f0 ; fmul f13,f12,f0 ; fsub f0,f11,f0
        let den = (y + one) * half;
        ((y - half) - half, den) // y - 1, in two steps, which is not the same as one
    } else {
        // loc_82F54FD4: addi r10,r10,-1 ; fsub f0,f13,f12 ; fadd f13,f0,f6 ; fmul f13,f13,f12
        n -= 1;
        let num = y - half;
        (num, (num + one) * half)
    };

    let z = numerator_arg / denominator_arg; // fdiv f5,f0,f13
    let w = z * z; // fmul f3,f5,f5

    // extsw r11,r10 ; std r11,-16(r1) ; lfd f0,-16(r1) ; fcfid f4,f0
    let n_double = fp::fcfid(i64::from(n));

    let ln2_lo = fp::load_double(g, LOG_LN2_LO)?; // lfd f0,8424(r10)
    let ln2_hi = fp::load_double(g, LOG_LN2_HI)?; // lfd f7,8(r11)
    let correction = n_double * ln2_lo; // fmul f0,f4,f0
    let num_a = fp::load_double(g, LOG_NUM_A)?; // lfd f13,8456(r9)
    let num_b = fp::load_double(g, LOG_NUM_B)?; // lfd f12,40(r11)
    let num_c = fp::load_double(g, LOG_NUM_C)?; // lfd f10,8440(r7)
    let den_a = fp::load_double(g, LOG_DEN_A)?; // lfd f11,8448(r8)
    let den_b = fp::load_double(g, LOG_DEN_B)?; // lfd f9,64(r11)
    let den_c = fp::load_double(g, LOG_DEN_C)?; // lfd f8,8432(r6)

    // The two chains are interleaved in the original and kept apart here; no value depends on the
    // interleaving, and each line's mnemonic is named.
    let mut numerator = -w.mul_add(num_a, -num_b); // fnmsub f13,f3,f13,f12
    let mut denominator = w - den_a; // fsub f12,f3,f11
    numerator = numerator.mul_add(w, -num_c); // fmsub f13,f13,f3,f10
    denominator = denominator.mul_add(w, den_b); // fmadd f12,f12,f3,f9
    numerator *= w; // fmul f13,f13,f3
    denominator = denominator.mul_add(w, -den_c); // fmsub f12,f12,f3,f8

    let ratio = numerator / denominator + one; // fdiv f13,f13,f12 ; fadd f13,f13,f6
    let low = ratio.mul_add(z, -correction); // fmsub f0,f13,f5,f0
    Ok(n_double.mul_add(ln2_hi, low)) // fmadd f1,f4,f7,f0
}

/// `sub_82F55068` — `log10(f1)`, verified: the natural log, times log10(e).
///
/// 169,118 calls in a played session on `RwAudioCore Dac`. Ported from
/// `recomp/src/audio_ports/sub_82F55068.inc`, **STATUS: verified**; six lifted call sites, one of
/// which reads the result straight back through `frsp`, so `f1` is the whole of its result.
///
/// The multiply is a plain `fmul`, not a multiply-add: there is no adjacent add for anything to
/// contract with, and writing it as one would change the last bit. [`LOG10_OF_E`] is loaded rather
/// than written as a literal — a wrong literal would be invisible to a harness that read the same
/// word, which is the argument `probe/ports/notes/sub_82F55068.md` makes.
///
/// Its 96-byte stack frame is not reproduced. The frame exists so `sub_82F54ED8`'s spills land
/// below the caller's parameter area instead of inside it; [`log`] here keeps those values in
/// registers, so there is nothing to place.
pub fn log10(g: &Guest, x: f64) -> Result<f64> {
    let natural = log(g, x)?; // bl 0x82f54ed8
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfd f0,8368(r11)
    let multiplier = fp::load_double(g, LOG10_OF_E)?;
    Ok(natural * multiplier) // fmul f1,f1,f0
}

// ------------------------------------------------------------------------------------- atan2

/// `lis r11,-32005 ; addi r11,r11,20584` — the atan2 pool at `0x82FB5068`, twenty-four doubles.
///
/// Every value below was read out of `probe/harness/out/image/`. The body loads all of them, so a
/// wrong offset here is a wrong answer rather than a compile error — which is why each is named by
/// what it measured rather than by what it ought to be.
pub const ATAN_POOL: u32 = LIS_82FB0000 + 20584;
/// `+8`, measured `1.5707963267948966` — pi/2, the angle when `x` is zero.
pub const ATAN_HALF_PI: u32 = ATAN_POOL + 8;
/// `+16`, measured `3.141592653589793`.
pub const ATAN_PI: u32 = ATAN_POOL + 16;
/// `+24`, measured `0.2679491924311227` — tan(pi/12), the threshold for the second reduction.
pub const ATAN_TAN_PI_12: u32 = ATAN_POOL + 24;
/// `+40`, measured `1.7320508075688772` — sqrt(3).
pub const ATAN_SQRT3: u32 = ATAN_POOL + 40;
/// `+128`, measured `{0, pi/6, pi/2, pi/3}` — four doubles, indexed by the octant code.
pub const ATAN_OFFSETS: u32 = ATAN_POOL + 128;
/// `+168`, measured `0.0` as a **single**, which is what the zero tests compare against.
pub const ATAN_ZERO_SINGLE: u32 = ATAN_POOL + 168;
/// `+176`, measured `1.0` as a single.
pub const ATAN_ONE_SINGLE: u32 = ATAN_POOL + 176;
/// The rational's numerator coefficients, at `+56`, `+64`, `+72` and `+80`: measured
/// `-13.688768894191927`, `-20.505855195861653`, `-8.494624035132068`, `-0.8375829936815006`.
pub const ATAN_NUM: [u32; 4] = [
    ATAN_POOL + 56,
    ATAN_POOL + 64,
    ATAN_POOL + 72,
    ATAN_POOL + 80,
];
/// The denominator's, at `+88`, `+96`, `+104` and `+112`: measured `41.06630668257578`,
/// `86.15734959713025`, `59.57843614259735`, `15.024001160028575`.
pub const ATAN_DEN: [u32; 4] = [
    ATAN_POOL + 88,
    ATAN_POOL + 96,
    ATAN_POOL + 104,
    ATAN_POOL + 112,
];

/// `stfd f1,16(r1)` — the spill of `y`, in the **caller's** frame; this leaf allocates none.
pub const ATAN_SPILL_Y: u32 = 16;
/// `stfd f2,24(r1)` — the spill of `x`, reloaded for its sign bit.
pub const ATAN_SPILL_X: u32 = 24;
/// `rlwinm. rN,rN,0,0,0` — the sign bit of a double's high word.
const SIGN_BIT: u32 = 0x8000_0000;

const _: () = assert!(ATAN_POOL == 0x82FB_5068, "lis -32005 ; addi 20584");
const _: () = assert!(ATAN_OFFSETS == 0x82FB_50E8 && ATAN_ONE_SINGLE == 0x82FB_5118);

/// `sub_82F52318` — `atan2(y, x)` in double, octant-reduced rational atan.
///
/// Verified, 87,372 calls in a boot session; all 22 lifted call sites read the result through
/// `frsp`, so `f1` is the whole of it. `y` arrives in `f1` and `x` in `f2`.
///
/// **It writes sixteen bytes of the caller's frame, and that is part of the port.** `stfd f1,16(r1)`
/// and `stfd f2,24(r1)` spill both arguments above the entry `r1` — this leaf never opens a frame of
/// its own — and the body reloads their high words for the sign tests. The C++ port declares them as
/// its write window for a specific reason its note records: unwindowed, the native run would read
/// the *lifted* run's spill back and agree for the wrong reason. So `sp` is an argument here and the
/// two stores actually happen.
///
/// The shape, and the three places a rewrite drifts:
///
/// - **Zero is a single, not a double.** The `x == 0` and `y == 0` tests compare against the pool's
///   `f32` zero widened, and the four-way result for `(±0, ±0)` comes from the sign *bits* read back
///   out of the spill: `x` positive returns `y` **unchanged** (so `atan2(-0, +0)` is `-0`), `x`
///   negative returns `±pi` from the pool.
/// - **The octant code is built from two comparisons**, `|y| > |x|` giving 2 and `t > tan(pi/12)`
///   giving 1, and it indexes a four-entry table of `{0, pi/6, pi/2, pi/3}`. Codes above 1 negate
///   `t` first. Getting the table order wrong is a plausible curve that is wrong in two octants.
/// - **The second reduction is `(sqrt(3)·t − 1)/(sqrt(3) + t)`** with the numerator a single
///   `fma` and the denominator a separate add — scalar, one rounding, as `std::fma`.
///
/// The final choice is an `fsel` on `x`, so `x = -0.0` takes the *non*-reflected arm (`-0.0 >= 0.0`
/// holds) and a NaN `x` takes the reflected one. Then `y`'s sign bit, read from memory, negates.
pub fn atan2(g: &mut Guest, y: f64, x: f64, sp: u32) -> Result<f64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at stfd f1,16(r1)

    g.set_u64(sp + ATAN_SPILL_Y, y.to_bits())?; // stfd f1,16(r1)
    g.set_u64(sp + ATAN_SPILL_X, x.to_bits())?; // stfd f2,24(r1)
    let zero = fp::load_single(g, ATAN_ZERO_SINGLE)?; // lfs f0,168(r11)

    let angle;
    // fcmpu cr6,f2,f0 ; bne cr6 -> the main path (a NaN x is unordered, so it takes it)
    if x == zero {
        // fcmpu cr6,f1,f0 ; bne cr6 -> x is zero but y is not
        if y == zero {
            // lwz r10,24(r1) ; rlwinm. r10,r10,0,0,0 ; beqlr -- +0 for x returns y as it came in
            if g.u32(sp + ATAN_SPILL_X)? & SIGN_BIT == 0 {
                return Ok(y);
            }
            // lwz r10,16(r1) ; rlwinm. r10,r10,0,0,0
            if g.u32(sp + ATAN_SPILL_Y)? & SIGN_BIT == 0 {
                return fp::load_double(g, ATAN_PI); // lfd f1,16(r11)
            }
            return Ok(fp::neg_double(fp::load_double(g, ATAN_PI)?)); // lfd f0,16(r11) ; fneg f1,f0
        }
        angle = fp::load_double(g, ATAN_HALF_PI)?; // lfd f0,8(r11) ; b loc_82F52428
    } else {
        // loc_82F52370
        let abs_x = fp::abs_double(x); // fabs f13,f2
        let mut octant = 0u32; // li r10,0
        let mut num = fp::abs_double(y); // fabs f0,f1
        let mut den = abs_x; // fmr f12,f13
        // fcmpu cr6,f0,f13 ; ble cr6 -> keep the order
        if num > abs_x {
            den = num; // fmr f12,f0
            octant = 2; // li r10,2
            num = abs_x; // fmr f0,f13
        }

        // loc_82F52394
        let mut t = num / den; // fdiv f0,f0,f12
        if t > fp::load_double(g, ATAN_TAN_PI_12)? {
            let sqrt3 = fp::load_double(g, ATAN_SQRT3)?; // lfd f13,40(r11)
            octant += 1; // addi r10,r10,1
            let one = fp::load_single(g, ATAN_ONE_SINGLE)?; // lfs f12,176(r11)
            let denominator = sqrt3 + t; // fadd f11,f13,f0
            t = sqrt3.mul_add(t, -one); // fmsub f0,f13,f0,f12
            t /= denominator; // fdiv f0,f0,f11
        }

        // loc_82F523BC: the two chains, interleaved in the original.
        let t2 = t * t; // fmul f5,f0,f0
        let n = [
            fp::load_double(g, ATAN_NUM[0])?, // lfd f8,56(r11)
            fp::load_double(g, ATAN_NUM[1])?, // lfd f10,64(r11)
            fp::load_double(g, ATAN_NUM[2])?, // lfd f12,72(r11)
            fp::load_double(g, ATAN_NUM[3])?, // lfd f13,80(r11)
        ];
        let d = [
            fp::load_double(g, ATAN_DEN[0])?, // lfd f6,88(r11)
            fp::load_double(g, ATAN_DEN[1])?, // lfd f7,96(r11)
            fp::load_double(g, ATAN_DEN[2])?, // lfd f9,104(r11)
            fp::load_double(g, ATAN_DEN[3])?, // lfd f11,112(r11)
        ];
        let mut p = n[3].mul_add(t2, n[2]); // fmadd f13,f13,f5,f12
        let mut q = d[3] + t2; // fadd f12,f11,f5
        p = p.mul_add(t2, n[1]); // fmadd f13,f13,f5,f10
        q = q.mul_add(t2, d[2]); // fmadd f12,f12,f5,f9
        p = p.mul_add(t2, n[0]); // fmadd f13,f13,f5,f8
        q = q.mul_add(t2, d[1]); // fmadd f12,f12,f5,f7
        p *= t2; // fmul f13,f13,f5
        q = q.mul_add(t2, d[0]); // fmadd f12,f12,f5,f6
        p *= t; // fmul f13,f13,f0
        p /= q; // fdiv f13,f13,f12
        t = p + t; // fadd f0,f13,f0

        // cmpwi cr6,r10,1 ; ble cr6 -- signed, and the codes are 0..3
        if octant as i32 > 1 {
            t = fp::neg_double(t); // fneg f0,f0
        }
        // loc_82F52418: rlwinm r10,r10,3,0,28 ; addi r9,r11,128 ; lfdx f13,r10,r9
        let offset = fp::load_double(g, ATAN_OFFSETS + (octant << 3))?;
        angle = offset + t; // fadd f0,f13,f0
    }

    // loc_82F52428
    let pi = fp::load_double(g, ATAN_PI)?; // lfd f13,16(r11)
    let reflected = pi - angle; // fsub f13,f13,f0
    let y_negative = g.u32(sp + ATAN_SPILL_Y)? & SIGN_BIT != 0; // lwz r11,16(r1) ; rlwinm.
    let result = fp::fsel(x, angle, reflected); // fsel f1,f2,f0,f13
    if !y_negative {
        return Ok(result); // beqlr
    }
    Ok(fp::neg_double(result)) // fneg f1,f1
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The two measured pool values, each as its own eight-byte segment at the address the body
    /// computes.
    ///
    /// One segment per cell rather than one rodata window: a body that read the wrong address gets
    /// an `Err` out of [`Guest`] instead of a plausible number. The words are the ones read out of
    /// `probe/harness/out/image/`, `1e18` included.
    pub fn with_pool_segments(g: &mut Guest) {
        g.put(POOL_STEP_DOWN, 1.0f64.to_bits().to_be_bytes().to_vec());
        g.put(
            POOL_INTEGRAL_MAGNITUDE,
            1e18f64.to_bits().to_be_bytes().to_vec(),
        );
    }

    /// A guest map holding the rodata cells `mathlib` reads, and nothing else.
    fn guest() -> Guest {
        let mut g = Guest::default();
        with_pool_segments(&mut g);
        g
    }

    /// The natural log's eighteen-double pool and the four single cells, each as its own segment,
    /// with the values read out of `probe/harness/out/image/` — not inferred.
    pub fn with_log_pool(g: &mut Guest) {
        let cells: [(u32, u64); 4] = [
            (POOL_ZERO, 0x0000_0000_0000_0000),
            (POOL_ONE, 0x3FF0_0000_0000_0000),
            (POOL_HALF, 0x3FE0_0000_0000_0000),
            (POOL_INFINITY, 0x7FF0_0000_0000_0000),
        ];
        for (addr, bits) in cells {
            g.put(addr, bits.to_be_bytes().to_vec());
        }
        // The NaN cell is *negative* in the image, and the body negates it on the way out.
        g.put(POOL_NAN, 0xFFF8_0000_0000_0000u64.to_be_bytes().to_vec());
        let pool: [u64; 18] = [
            0x3FE6_A09E_667F_3BCD, // +0   sqrt(2)/2
            0x3FE6_3000_0000_0000, // +8   0.693359375, ln2 high
            0xBF2B_D010_5C61_0CA8, // +16  -ln2 low (unused by this pair)
            0x3FDB_CB7B_1526_E50E, // +24  log10(e)
            0xC050_07FF_12B3_B59A, // +32  -64.12494342374558
            0x4030_624A_2016_AFED, // +40  16.383943563021536
            0xBFE9_4415_B356_BD29, // +48  -0.7895611288749126
            0xC088_0BFE_9C0D_9077, // +56  -769.4993210849487
            0x4073_8083_FA15_267E, // +64  312.03222091924533
            0xC041_D580_4B67_CE0F, // +72  -35.66797773903465
            0x3F2B_D010_5C61_0CA8, // +80  ln2 low
            0x4088_0BFE_9C0D_9077, // +88  769.4993210849487
            0x4050_07FF_12B3_B59A, // +96  64.12494342374558
            0x4041_D580_4B67_CE0F, // +104 35.66797773903465
            0x3FE9_4415_B356_BD29, // +112 0.7895611288749126
            0x4340_0000_0000_0000, // +120 2^53
            0x0010_0000_0000_0000, // +128 DBL_MIN
            0x7FEF_FFFF_FFFF_FFFF, // +136 DBL_MAX
        ];
        let mut bytes = Vec::with_capacity(18 * 8);
        for word in pool {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        g.put(LOG_POOL, bytes);
    }

    fn log_guest() -> Guest {
        let mut g = Guest::default();
        with_log_pool(&mut g);
        g
    }

    #[test]
    fn the_log_pool_addresses_come_from_the_lis_immediates() {
        assert_eq!(LOG_POOL, 0x8205_0000 + 8344);
        assert_eq!(LOG10_OF_E, 0x8205_0000 + 8368);
        assert_eq!(LOG_LN2_LO, 0x8205_0000 + 8424);
        assert_eq!(LOG_DEN_C, 0x8205_0000 + 8432);
        assert_eq!(LOG_NUM_C, 0x8205_0000 + 8440);
        assert_eq!(LOG_DEN_A, 0x8205_0000 + 8448);
        assert_eq!(LOG_NUM_A, 0x8205_0000 + 8456);
        assert_eq!(LOG_TWO_53, 0x8205_0000 + 8464);
        assert_eq!(POOL_INFINITY, 0x82FB_0000 + 22720);
        assert_eq!(POOL_NAN, 0x82FB_0000 + 22728);
        // The split is exact: the two halves sum to the correctly rounded ln 2.
        let g = log_guest();
        let hi = fp::load_double(&g, LOG_LN2_HI).unwrap();
        let lo = fp::load_double(&g, LOG_LN2_LO).unwrap();
        assert_eq!(hi - lo, std::f64::consts::LN_2);
    }

    #[test]
    fn log_matches_an_independent_oracle_across_the_range() {
        // `f64::ln` is outside this translation entirely. The agreement is not bit-for-bit — the
        // guest's rational approximation is its own — so this bounds the relative error instead,
        // which is what catches a wrong coefficient, a wrong exponent bias or a swapped branch.
        let g = log_guest();
        let mut x = 1e-300;
        while x < 1e300 {
            let got = log(&g, x).unwrap();
            let want = x.ln();
            let tolerance = 4.0 * f64::EPSILON * want.abs().max(1.0);
            assert!(
                (got - want).abs() <= tolerance,
                "log({x:e}): got {got}, oracle {want}, diff {}",
                got - want
            );
            x *= 7.3;
        }
    }

    #[test]
    fn both_reduction_branches_run_and_agree_with_the_oracle() {
        // The branch is chosen by where the significand falls relative to sqrt(2)/2, so these two
        // arguments differ only in that, and the `n -= 1` on the lower branch is what a rewrite
        // drops. 1.5 has significand 0.75 (above), 1.1 has 0.55 (below).
        let g = log_guest();
        for x in [1.5f64, 1.1, 0.75, 0.55, 2.0, 1023.0] {
            let got = log(&g, x).unwrap();
            assert!(
                (got - x.ln()).abs() <= 4.0 * f64::EPSILON * x.ln().abs().max(1.0),
                "log({x})"
            );
        }
    }

    #[test]
    fn the_four_special_cases_come_out_of_the_image() {
        let mut g = log_guest();
        // log(1) is the zero *cell*, not a computed zero: patch it and the answer moves.
        assert_eq!(log(&g, 1.0).unwrap().to_bits(), 0);
        g.put(POOL_ZERO, 0.25f64.to_bits().to_be_bytes().to_vec());
        assert_eq!(log(&g, 1.0).unwrap(), 0.25, "the cell is read, not assumed");

        let g = log_guest();
        assert_eq!(log(&g, 0.0).unwrap(), f64::NEG_INFINITY);
        assert_eq!(log(&g, -0.0).unwrap(), f64::NEG_INFINITY, "either zero");
        assert_eq!(log(&g, f64::INFINITY).unwrap(), f64::INFINITY);

        // A negative argument and a NaN argument both leave through the negated NaN cell, so an
        // input payload is discarded rather than propagated. That is the assertion a `f64::ln`
        // substitute would fail: Rust's ln(-1.0) is a NaN of its own making.
        assert_eq!(log(&g, -1.0).unwrap().to_bits(), 0x7FF8_0000_0000_0000);
        let signalling = f64::from_bits(0x7FF4_0000_0000_0DAD);
        assert_eq!(
            log(&g, signalling).unwrap().to_bits(),
            0x7FF8_0000_0000_0000
        );
    }

    #[test]
    fn a_denormal_argument_returns_minus_infinity_under_the_guest_flush_mode() {
        // The body has a 2^53 scaling branch for denormals, and in this build it is unreachable:
        // both of RexGlue's MXCSR values carry DAZ, so a denormal compares equal to zero and the
        // zero arm wins. This is the test that fails if the port stops holding an `Fpscr` — Rust's
        // default MXCSR would take the scaling branch and return about -744.
        let g = log_guest();
        let denormal = f64::from_bits(1);
        assert_eq!(log(&g, denormal).unwrap(), f64::NEG_INFINITY);
    }

    #[test]
    fn log10_is_the_natural_log_times_the_tabulated_constant() {
        let g = log_guest();
        for x in [10.0f64, 1000.0, 0.001, 2.0, 1e100] {
            // The image's word, by its bits: 0.4342944819032518, which is log10(e) to the last
            // bit but is a *tabulated* constant here, so it is spelled the way the image holds it.
            let want = log(&g, x).unwrap() * f64::from_bits(0x3FDB_CB7B_1526_E50E);
            assert_eq!(
                log10(&g, x).unwrap(),
                want,
                "log10({x}) is one plain multiply"
            );
            assert!((log10(&g, x).unwrap() - x.log10()).abs() <= 1e-14 * x.log10().abs().max(1.0));
        }
        assert_eq!(log10(&g, 0.0).unwrap(), f64::NEG_INFINITY);
    }

    #[test]
    fn log10_reads_its_multiplier_from_the_image() {
        // A hardcoded log10(e) would be invisible to a harness that read the same word, which is
        // why the port loads it. Patching the cell has to move the result.
        let mut g = log_guest();
        // `set_u64`, not `put`: the cell lives *inside* the pool segment, and a second overlapping
        // segment would be shadowed by the first — which is how this test failed the first time,
        // reporting the correct answer 2.0 and proving nothing.
        g.set_u64(LOG10_OF_E, 2.0f64.to_bits()).unwrap();
        assert_eq!(log10(&g, 100.0).unwrap(), log(&g, 100.0).unwrap() * 2.0);
        assert_ne!(
            log10(&g, 100.0).unwrap(),
            2.0,
            "the patched constant has to matter"
        );
    }

    #[test]
    fn log_restores_the_entry_flush_mode() {
        let g = log_guest();
        let before = crate::vmx::get_mxcsr();
        log10(&g, 42.0).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }

    /// The atan2 pool's twenty-four doubles, as read out of `probe/harness/out/image/`.
    pub fn with_atan_pool(g: &mut Guest) {
        let pool: [u64; 24] = [
            0x3E46_A09E_667F_3BCD, // +0    (unread by this body)
            0x3FF9_21FB_5444_2D18, // +8    pi/2
            0x4009_21FB_5444_2D18, // +16   pi
            0x3FD1_2614_5E9E_CD56, // +24   tan(pi/12)
            0x3FE7_6CF5_D0B0_9955, // +32   (unread)
            0x3FFB_B67A_E858_4CAA, // +40   sqrt(3)
            0x7FD0_0000_0000_0000, // +48   (unread)
            0xC02B_60A6_5106_1CE2, // +56   numerator
            0xC034_817F_B9E2_BCCB, // +64
            0xC020_FD3F_5C8D_6A63, // +72
            0xBFEA_CD7A_D9B1_87BD, // +80
            0x4044_887C_BCC4_95A9, // +88   denominator
            0x4055_8A12_040B_6DA5, // +96
            0x404D_CA0A_320D_A3D7, // +104
            0x402E_0C49_E14A_C710, // +112
            0x3FF0_0000_0000_0000, // +120  (unread)
            0x0000_0000_0000_0000, // +128  octant 0: 0
            0x3FE0_C152_382D_7366, // +136  octant 1: pi/6
            0x3FF9_21FB_5444_2D18, // +144  octant 2: pi/2
            0x3FF0_C152_382D_7366, // +152  octant 3: pi/3
            0x0010_0000_0000_0000, // +160  (unread)
            0x0000_0000_3F00_0000, // +168  the f32 zero is the high half of this word
            0x3F80_0000_0000_0000, // +176  the f32 one, likewise
            0x4415_AF1D_78B5_8C40, // +184  (unread)
        ];
        let mut bytes = Vec::with_capacity(24 * 8);
        for word in pool {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        g.put(ATAN_POOL, bytes);
    }

    const ATAN_SP: u32 = 0x5000_0000;

    /// A guest with the atan2 pool and a stack page for the two spills.
    fn atan_guest() -> Guest {
        let mut g = Guest::default();
        with_atan_pool(&mut g);
        g.put(ATAN_SP, vec![0xAA; 64]);
        g
    }

    #[test]
    fn the_atan_pool_addresses_come_from_the_lis_immediates() {
        assert_eq!(ATAN_POOL, 0x82FB_0000 + 20584);
        assert_eq!(ATAN_HALF_PI, 0x82FB_5070);
        assert_eq!(ATAN_OFFSETS, 0x82FB_50E8);
        assert_eq!(ATAN_ZERO_SINGLE, 0x82FB_5110);
        assert_eq!(ATAN_ONE_SINGLE, 0x82FB_5118);
        // The two "singles" are the *high* halves of their words, which is what `lfs` reads.
        let g = atan_guest();
        assert_eq!(fp::load_single(&g, ATAN_ZERO_SINGLE).unwrap(), 0.0);
        assert_eq!(fp::load_single(&g, ATAN_ONE_SINGLE).unwrap(), 1.0);
        assert_eq!(fp::load_double(&g, ATAN_PI).unwrap(), std::f64::consts::PI);
    }

    #[test]
    fn atan2_matches_an_independent_oracle_in_every_quadrant() {
        // `f64::atan2` is outside this translation. Both octant branches and both swap arms are
        // covered by the ratios below, in all four quadrants and at three magnitudes.
        let mut g = atan_guest();
        let ratios = [0.0, 0.05, 0.2, 0.2679, 0.3, 0.7, 1.0, 1.4, 5.0, 40.0];
        for scale in [1e-8f64, 1.0, 1e9] {
            for r in ratios {
                for (sy, sx) in [(1.0f64, 1.0f64), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                    let (y, x) = (sy * r * scale, sx * scale);
                    let got = atan2(&mut g, y, x, ATAN_SP).unwrap();
                    let want = y.atan2(x);
                    assert!(
                        (got - want).abs() <= 8.0 * f64::EPSILON * want.abs().max(1.0),
                        "atan2({y:e}, {x:e}): got {got}, oracle {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_zero_cases_are_decided_by_the_spilled_sign_bits() {
        let mut g = atan_guest();
        // x = +0: y comes back *unchanged*, sign and all — not recomputed.
        assert_eq!(atan2(&mut g, 0.0, 0.0, ATAN_SP).unwrap().to_bits(), 0);
        assert_eq!(
            atan2(&mut g, -0.0, 0.0, ATAN_SP).unwrap().to_bits(),
            (-0.0f64).to_bits(),
            "atan2(-0, +0) keeps the negative zero"
        );
        // x = -0: the pool's pi, with y's sign.
        assert_eq!(
            atan2(&mut g, 0.0, -0.0, ATAN_SP).unwrap(),
            std::f64::consts::PI
        );
        assert_eq!(
            atan2(&mut g, -0.0, -0.0, ATAN_SP).unwrap(),
            -std::f64::consts::PI
        );
        // x = 0 with y non-zero: pi/2, and `-0.0 >= 0.0` holds so a negative zero x is not
        // reflected either.
        assert_eq!(
            atan2(&mut g, 2.0, 0.0, ATAN_SP).unwrap(),
            std::f64::consts::FRAC_PI_2
        );
        assert_eq!(
            atan2(&mut g, -2.0, 0.0, ATAN_SP).unwrap(),
            -std::f64::consts::FRAC_PI_2
        );
        assert_eq!(
            atan2(&mut g, 2.0, -0.0, ATAN_SP).unwrap(),
            std::f64::consts::FRAC_PI_2
        );
    }

    #[test]
    fn it_spills_both_arguments_into_the_callers_frame() {
        // The spills are the port's declared write window, so they have to happen — on every path,
        // including the one that returns before any arithmetic.
        let mut g = atan_guest();
        atan2(&mut g, -0.0, 0.0, ATAN_SP).unwrap();
        assert_eq!(g.u64(ATAN_SP + ATAN_SPILL_Y).unwrap(), (-0.0f64).to_bits());
        assert_eq!(g.u64(ATAN_SP + ATAN_SPILL_X).unwrap(), 0.0f64.to_bits());

        atan2(&mut g, 3.5, -2.5, ATAN_SP).unwrap();
        assert_eq!(g.u64(ATAN_SP + ATAN_SPILL_Y).unwrap(), 3.5f64.to_bits());
        assert_eq!(g.u64(ATAN_SP + ATAN_SPILL_X).unwrap(), (-2.5f64).to_bits());
        // Nothing outside the sixteen bytes.
        assert_eq!(g.u64(ATAN_SP + 8).unwrap(), u64::from_be_bytes([0xAA; 8]));
        assert_eq!(g.u64(ATAN_SP + 32).unwrap(), u64::from_be_bytes([0xAA; 8]));
    }

    #[test]
    fn each_octant_takes_its_own_table_entry() {
        // The four codes come from `|y| > |x|` (2) and `t > tan(pi/12)` (1). Patching one entry has
        // to move exactly the inputs that use it, which is what a wrong table order would fail:
        // these four arguments cover codes 0, 1, 2 and 3 in that sequence.
        let inputs = [(0.1f64, 1.0f64), (0.9, 1.0), (1.0, 0.1), (1.0, 0.9)];
        for (code, (y, x)) in inputs.iter().enumerate() {
            let mut g = atan_guest();
            let before = atan2(&mut g, *y, *x, ATAN_SP).unwrap();
            let cell = ATAN_OFFSETS + (code as u32) * 8;
            let patched = fp::load_double(&g, cell).unwrap() + 1.0;
            g.set_u64(cell, patched.to_bits()).unwrap();
            let after = atan2(&mut g, *y, *x, ATAN_SP).unwrap();
            assert!(
                (after - before - 1.0).abs() < 1e-15,
                "octant {code}: patching its offset moved the result by {}",
                after - before
            );
            // And the *other* three entries must not matter for this input.
            for other in 0..4u32 {
                if other == code as u32 {
                    continue;
                }
                let mut h = atan_guest();
                let cell = ATAN_OFFSETS + other * 8;
                let bumped = fp::load_double(&h, cell).unwrap() + 1.0;
                h.set_u64(cell, bumped.to_bits()).unwrap();
                assert_eq!(
                    atan2(&mut h, *y, *x, ATAN_SP).unwrap(),
                    before,
                    "octant {code} vs {other}"
                );
            }
        }
    }

    #[test]
    fn a_nan_argument_comes_out_nan() {
        let mut g = atan_guest();
        assert!(atan2(&mut g, f64::NAN, 1.0, ATAN_SP).unwrap().is_nan());
        assert!(atan2(&mut g, 1.0, f64::NAN, ATAN_SP).unwrap().is_nan());
    }

    #[test]
    fn atan2_restores_the_entry_flush_mode() {
        let mut g = atan_guest();
        let before = crate::vmx::get_mxcsr();
        atan2(&mut g, 1.0, 2.0, ATAN_SP).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }

    /// A `Trig` that answers with values the test chose, so no test here depends on libm.
    #[derive(Default)]
    pub struct Scripted {
        pub sine: f64,
        pub cosine: f64,
        pub asked: Vec<(char, f64)>,
    }

    impl Trig for Scripted {
        fn sine(&mut self, _g: &Guest, x: f64) -> Result<f64> {
            self.asked.push(('s', x));
            Ok(self.sine)
        }
        fn cosine(&mut self, _g: &Guest, x: f64) -> Result<f64> {
            self.asked.push(('c', x));
            Ok(self.cosine)
        }
    }

    #[test]
    fn it_floors_the_ordinary_range_and_agrees_with_the_library_there() {
        let g = guest();
        for x in [
            0.5,
            1.0,
            1.5,
            2.75,
            -0.5,
            -1.0,
            -1.5,
            -2.75,
            1e15,
            -1e15,
            1e17,
            -1e17,
            4.9,
            -4.9,
            0.9999999999999999,
        ] {
            assert_eq!(floor(&g, x).unwrap(), x.floor(), "floor({x})");
        }
    }

    #[test]
    fn the_step_down_is_what_makes_it_a_floor_and_not_a_truncation() {
        // The distinguishing input set is the negatives with a fraction: fctidz gives -2 for -2.75
        // and the floor is -3. Reading the cell live is what supplies the 1.0, so a map whose cell
        // holds something else steps by that instead — which is how a patched image reaches this.
        let g = guest();
        assert_eq!(floor(&g, -2.75).unwrap(), -3.0);
        assert_eq!(floor(&g, -0.25).unwrap(), -1.0);
        // Exact integers do not step: the fraction is +0.0, and +0.0 >= 0.0 takes the first arm.
        assert_eq!(floor(&g, -3.0).unwrap(), -3.0);

        let mut patched = guest();
        patched.set_u64(POOL_STEP_DOWN, 10.0f64.to_bits()).unwrap();
        assert_eq!(
            floor(&patched, -2.75).unwrap(),
            -12.0,
            "the cell is read, not folded in"
        );
    }

    #[test]
    fn the_magnitude_cell_is_1e18_and_it_is_the_threshold_that_is_read() {
        // Measured from the image, against a note that predicted 2^52. Pinned here because the
        // difference is observable: at 1e17 the magnitude arm is not taken and the fsel chain
        // computes a floor; the answer is the same either way, so the test has to use the cell.
        let g = guest();
        assert_eq!(
            g.u64(POOL_INTEGRAL_MAGNITUDE).unwrap(),
            0x43AB_C16D_674E_C800
        );
        assert_eq!(f64::from_bits(0x43AB_C16D_674E_C800), 1e18);

        // Lower the threshold under an input that has a fraction, and the pass-through arm takes
        // over: floor(2.75) stops being 2.0 and becomes 2.75.
        let mut patched = guest();
        patched
            .set_u64(POOL_INTEGRAL_MAGNITUDE, 1.0f64.to_bits())
            .unwrap();
        assert_eq!(
            floor(&patched, 2.75).unwrap(),
            2.75,
            "|x| > cell passes through unchanged"
        );
        assert_eq!(
            floor(&patched, 0.75).unwrap(),
            0.0,
            "|x| <= cell still floors"
        );
    }

    #[test]
    fn zero_keeps_its_sign_and_nan_comes_back_with_its_payload() {
        let g = guest();
        // -0.0 is the whole reason for the third fsel: trunc(-0.0) is +0.0 through fctidz/fcfid,
        // and without the select the function would lose the sign.
        assert_eq!(floor(&g, -0.0).unwrap().to_bits(), 0x8000_0000_0000_0000);
        assert_eq!(floor(&g, 0.0).unwrap().to_bits(), 0);
        // NaN takes the else arm of every select, so it returns through the `x` path untouched.
        let nan = f64::from_bits(0x7FF8_0000_DEAD_BEEF);
        assert_eq!(floor(&g, nan).unwrap().to_bits(), 0x7FF8_0000_DEAD_BEEF);
        // Infinities are above the magnitude cell, so they pass through as well.
        assert_eq!(floor(&g, f64::INFINITY).unwrap(), f64::INFINITY);
        assert_eq!(floor(&g, f64::NEG_INFINITY).unwrap(), f64::NEG_INFINITY);
    }

    #[test]
    fn it_reads_both_pool_cells_on_every_call() {
        // The cells are read live, so a map without them is an error rather than a guess — and both
        // are read unconditionally, including on the calls whose answer does not depend on either.
        let bare = Guest::default();
        assert!(floor(&bare, 1.5).is_err(), "neither cell mapped");
        let mut only_step = Guest::default();
        only_step.put(POOL_STEP_DOWN, 1.0f64.to_bits().to_be_bytes().to_vec());
        assert!(
            floor(&only_step, 8.0).is_err(),
            "the magnitude cell is read even for 8.0"
        );
        let mut only_mag = Guest::default();
        only_mag.put(
            POOL_INTEGRAL_MAGNITUDE,
            1e18f64.to_bits().to_be_bytes().to_vec(),
        );
        assert!(
            floor(&only_mag, 8.0).is_err(),
            "the step cell is read even with no fraction"
        );
    }

    #[test]
    fn the_unported_trig_refuses_rather_than_answering() {
        let g = guest();
        let mut t = Unported;
        let e = t.sine(&g, 0.5).unwrap_err();
        assert_eq!(e.address, 0x82F4_DED0);
        assert!(e.message.contains("82F4DED0"));
        assert_eq!(t.cosine(&g, 0.5).unwrap_err().address, 0x82F4_DFB0);

        // And the closure adapter passes the argument straight through, in both slots.
        let mut c = Closures::new(
            |_g: &Guest, x: f64| Ok(x + 1.0),
            |_g: &Guest, x: f64| Ok(x + 2.0),
        );
        assert_eq!(c.sine(&g, 10.0).unwrap(), 11.0);
        assert_eq!(c.cosine(&g, 10.0).unwrap(), 12.0);
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let g = guest();
        let before = crate::vmx::get_mxcsr();
        floor(&g, -2.75).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }
}
