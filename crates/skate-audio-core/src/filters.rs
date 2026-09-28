//! The per-channel filter stages: a low-pass, a high-pass, and the coefficient builder they share.
//!
//! Both stages take the same filter object in `r3` and the same stream in `r4`, normalise a cutoff
//! out of the object's `+52` and the stream's sample rate, and then either run
//! [`crate::dsp::biquad`] over 256 frames of every channel and swap the stream's descriptor pair, or
//! **bypass** — clear the filter history and leave the pair alone, so the caller keeps reading the
//! untouched input block. They are mirror images of one another and they are not symmetric, which is
//! the main thing this file exists to make visible.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`lowpass_stage`] | `sub_82B27E20` | verified | 214 | 183,427 | 291,143 |
//! | [`highpass_stage`] | `sub_82B26568` | verified | 287 | 171,012 | 276,388 |
//! | [`build_lowpass_coefficients`] | `sub_82B43CC0` | verified | 111 | 4,400 | 3,226 |
//! | [`shelf_stage`] | `sub_82B26740` | verified | 263 | 24,936 | 29,673 |
//! | [`build_shelf_coefficients`] | `sub_82B43D78` | thin | 163 | 82 | 514 |
//! | [`peaking_stage`] | `sub_82B2C658` | verified | 386 | 61,716 | 90,841 |
//!
//! The shelf pair is replayed against **3,000 recorded calls, 0 disagreements** — 2,996 for the stage
//! and only 4 for the thin builder, which rebuilds coefficients that rarely; its unit tests carry the
//! rest. The peaking equaliser is **unit-tested only** for now: its recording session lost its display
//! (the frame clock stopped, no audio thread came up) and never reached gameplay.
//!
//! Every one of those `.inc` headers leads with `// STATUS: verified` and `docs/ports.md` agrees on
//! all three — checked before translating, because a header reading `thin`, `partial` or `gate-`
//! would not be a reference at all.
//!
//! `build_lowpass_coefficients` is not in the same call-count league as the other two and it is here
//! because [`lowpass_stage`] **calls** it: the low-pass builds its five coefficients in a separate
//! guest function, and the high-pass builds the same five inline. Porting the stages without it would
//! have left the low-pass's recompute path as a stub, which is the only part of it a test can drive.
//!
//! ## What the green here means
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green. Each C++
//! body was compared call-for-call against the original under the shadow harness, on the real inputs
//! those call counts come from, at zero divergence. The Rust has no recorded vectors of its own, and
//! for these three it cannot have any yet — see the [`Trig`] note below. A fault here is a
//! transcription error rather than a misreading of the engine; it is not a number.
//!
//! ## Nothing in `docs/rw_audio_structs.h` names this object
//!
//! Every offset below is the raw offset plus the use the two lifted bodies make of it. No field name
//! here is recovered from RTTI or from plug-in metadata, and none should be read as one. The
//! `+56`/`+184` pair does carry one real constraint: `(184 - 56) / 16 == 8`, so the state array holds
//! at most eight channels and a ninth would run into the coefficients. Both C++ `Windows()` builders
//! decline a channel count above eight for that reason — a corrupted object rather than a case to
//! compare — and [`MAX_CHANNELS`] records it without enforcing it, because the original does not.
//!
//! ## The two stages are mirrors, and the mirror is not exact
//!
//! | | [`lowpass_stage`] | [`highpass_stage`] |
//! |---|---|---|
//! | bypasses when | `!(cutoff < ceiling)` | `!(cutoff > floor)` |
//! | clears history when the cache was | `< ceiling` | `> floor` |
//! | then clamps to | the **floor**, from below | the **ceiling**, from above |
//! | coefficients | `sub_82B43CC0` | built inline |
//! | numerator | `1 - cos` | `1 + cos` |
//! | `b1` | `+(1-cos)/a0` | `-(1+cos)/a0`, through an `fneg` |
//!
//! Both bypass tests are **unordered**: a NaN cutoff fails `<` and fails `>`, so both stages bypass
//! on it. Both clamps are strict, so a cutoff exactly on its bound is left alone.
//!
//! Reading the four coefficient cells out of the validated image dump makes the arithmetic legible
//! rather than merely transcribed. They are `0.5`, `1.0`, `-2.0` and `2.0` — three of which this crate
//! already names from [`crate::spatial`]'s own reading of the same dump — so with
//! `alpha = 0.5·sin w` and `a0 = 1 + alpha` both bodies are the textbook RBJ biquad at Q = 1, every
//! coefficient divided through by `a0`:
//!
//! ```text
//! a1 = -2cos/a0   a2 = (1-alpha)/a0
//! low-pass    b0 = b2 = (1-cos)/(2a0)   b1 =  (1-cos)/a0
//! high-pass   b0 = b2 = (1+cos)/(2a0)   b1 = -(1+cos)/a0
//! ```
//!
//! ## `Trig`, and what it costs
//!
//! Both recompute paths reach the image's sine and cosine, `sub_82F4DED0` and `sub_82F4DFB0`, which
//! **have no port in either language** — outside the 216 audio-thread functions the sweep covered, so
//! there is no `.inc` to transcribe. They are therefore a [`Trig`] parameter, exactly as in
//! [`crate::spatial`], whose default [`crate::mathlib::Unported`] returns an `Err` naming the guest
//! address rather than a plausible number.
//!
//! The consequence is narrower here than in `spatial`, and worth stating precisely: a call whose
//! cutoff equals the cached cutoff never asks for a sine, so it runs to completion under `Unported`
//! and **is** replayable. Only a call that recomputes is not. The same applies to the whole bypass
//! path, which never reaches trigonometry at all.
//!
//! ## Reproduced rather than fixed
//!
//! - **The channel count is re-read from `+42` at the bottom of every loop**, in all three loops here,
//!   so a filter pass that stored over the object's own header would change its own trip count. Both
//!   C++ `Windows()` builders decline a layout where that could happen, so nothing is known about what
//!   the guest does with it; the reload is kept because the original has it.
//! - **The state pointer walks on the callee's `r3`.** `addi r3,r3,16` is applied to whatever
//!   `sub_82B43AF8` left in `r3`, and that function never writes it — so the walk depends on the
//!   callee returning its first argument untouched. [`crate::dsp::biquad`] has no return value to read,
//!   so the walk is written as `state + 16` with this note attached rather than as a register read.
//! - **`sub_82B43CC0` computes `b2` with a second `fdivs` of the same two operands** instead of copying
//!   `b0`. Two single-rounded divides of identical operands agree, so this is invisible in value; it is
//!   written out because a `fmuls` by `0.5` or a copy would not be the same instruction.
//! - **`sub_82B26568` stores its five coefficients in the order 184, 188, 192, 200, 196**, which is
//!   not ascending. Kept.
//!
//! ## What no test here can catch
//!
//! Two things, each measured the same way the crate README's other entries were — the change made,
//! the whole suite re-run, all still green — rather than argued from the arithmetic.
//!
//! - **The coefficient store order.** The five stores go to five **distinct** words with five distinct
//!   values, so a single-threaded comparison cannot order them in either direction: reversing `+196`
//!   and `+200` leaves every byte of the object identical. Written in the original's order because the
//!   original has it; `b1` and `b2` are separately asserted to hold the right values, which is all a
//!   test here can establish.
//! - **The `frsp` on the *sine*, in all three bodies.** Its only consumer is `alpha = fmuls(sine, 0.5)`
//!   and scaling by a power of two commutes with rounding to single, so `(x·0.5)→f32` equals
//!   `(x→f32)·0.5` for every non-denormal `x`. Removing that one `frsp` leaves the suite green. The
//!   `frsp` on the **cosine** is a different matter and is pinned, because its consumer is
//!   `fsubs(1, cos)` / `fadds(cos, 1)`, where a tail below the single's last bit survives as a
//!   non-zero difference — see `the_cosines_double_tail_is_discarded_by_the_frsp`.
//!
//! ## Not reproduced, and why
//!
//! Neither stage's register spill is reproduced: the originals save `f29`-`f31` and `r26`-`r31` because
//! their callees may clobber the volatile file, and the C++ ports keep the entry values in locals for
//! the same reason. There is no register file here, so nothing is written to memory in any version.
//!
//! `sub_82B43CC0`'s `stwu r1,-112(r1)` is **also** not reproduced, and that one is a decision rather
//! than an absence. The C++ port does reproduce it, because its two callees are real guest bodies whose
//! spills land below `r1` and would otherwise overwrite this function's own. With the trigonometry as a
//! parameter there is no callee to make room for — the same reasoning `spatial::add_angular` records
//! for its own `stwu r1,-160(r1)`.

use crate::mathlib::Trig;
use crate::vmx::Fpscr;
use crate::{Guest, Result, dsp, fp, gains};

// ------------------------------------------------------------------------------- the filter object

/// `u8` at `+42` — the channel count, **re-read** at the bottom of every loop.
pub const CHANNEL_COUNT: u32 = 42;
/// `f32` at `+52` — the numerator of the normalised cutoff.
pub const CUTOFF_INPUT: u32 = 52;
/// 16 bytes per channel, `{x1, x2, y1, y2}`: exactly what [`crate::dsp::biquad`] carries.
pub const FILTER_STATE: u32 = 56;
/// Five singles at `+184`, in the order `sub_82B43AF8` consumes them.
pub const COEFFICIENTS: u32 = 184;
/// `f32` at `+204` — the cutoff the coefficients at [`COEFFICIENTS`] were built from.
pub const CACHED_CUTOFF: u32 = 204;
/// `(184 - 56) / 16` — the state array's capacity, not a bound either original enforces.
pub const MAX_CHANNELS: u32 = 8;

const _: () = assert!(
    (COEFFICIENTS - FILTER_STATE) / 16 == MAX_CHANNELS,
    "eight state slots"
);
const _: () = assert!(
    CACHED_CUTOFF == COEFFICIENTS + 20,
    "the cache sits past the five singles"
);

// --------------------------------------------------------------------------------- the stream

/// `rw_ptr` at `+28` — the descriptor read this call, and written on the swap.
pub const STREAM_BUFFER_A: u32 = 28;
/// `rw_ptr` at `+32` — the descriptor written this call.
pub const STREAM_BUFFER_B: u32 = 32;
/// `rw_ptr` at `+40` — the format block. Dereferenced unconditionally, before any store.
pub const STREAM_FORMAT: u32 = 40;
/// `f32` at `+12` of the format block — the sample rate the cutoff is normalised by.
pub const FORMAT_SAMPLE_RATE: u32 = 12;

const _: () = assert!(
    STREAM_BUFFER_B == STREAM_BUFFER_A + 4,
    "the pair is swapped as two words"
);

// ------------------------------------------------------------------------------- the descriptor

/// `u32` at `+4` of a buffer descriptor — channel 0's float array.
pub const DESC_BASE: u32 = 4;
/// `u16` at `+14` — singles between one channel and the next.
pub const DESC_STRIDE: u32 = 14;

// The same descriptor `gains.rs` walks, so its `mullw`/`rlwinm`/`add` chain is reused rather than
// written a fourth time. These two assertions are what makes the reuse legitimate: if either offset
// ever disagreed, the shared helper would be reading a different structure.
const _: () = assert!(
    DESC_BASE == gains::BUFFER_BASE,
    "the descriptor gains.rs walks"
);
const _: () = assert!(DESC_STRIDE == gains::CHANNEL_STRIDE);

/// `li r7,256` — both stages filter a fixed 256-frame block.
pub const BLOCK_FRAMES: u32 = 256;

// ------------------------------------------------------------------------ the five coefficients

// The offsets `dsp::biquad` reads them back from. Asserted rather than re-stated, because the whole
// point of writing coefficients here is that that body consumes them.
const _: () = assert!(dsp::biquad::COEFF_A1 == 0);
const _: () = assert!(dsp::biquad::COEFF_A2 == 4);
const _: () = assert!(dsp::biquad::COEFF_B0 == 8);
const _: () = assert!(dsp::biquad::COEFF_B1 == 12);
const _: () = assert!(dsp::biquad::COEFF_B2 == 16);
/// Five singles, `{a1, a2, b0, b1, b2}`.
pub const COEFFICIENT_BYTES: u32 = 20;

// ------------------------------------------------------------------------------- the rodata cells

/// `((imm & 0xFFFF) << 16)` — the `lis` half of a constant's address, **computed**.
///
/// Never read off the disassembly by eye: one misread digit in `sub_82B2FE00` cost this project its
/// first shadow divergence, and `CLAUDE.md` records a second wrong value reaching a committed note.
const fn lis(imm: i32) -> u32 {
    ((imm as u32) & 0xFFFF) << 16
}

const _: () = assert!(lis(-32245) == 0x820B_0000, "lis r11,-32245");
const _: () = assert!(lis(-32208) == 0x8230_0000, "lis r11,-32208");
const _: () = assert!(lis(-32234) == 0x8216_0000, "lis r9,-32234");
const _: () = assert!(lis(-32246) == 0x820A_0000, "lis r11,-32246");
const _: () = assert!(lis(-32206) == 0x8232_0000, "lis r10,-32206");
const _: () = assert!(lis(-32247) == 0x8209_0000, "lis r9,-32247");
const _: () = assert!(lis(-32250) == 0x8206_0000, "lis r8,-32250");

/// `lis -32208 ; addi -31232` — the audio pool [`crate::dsp::biquad`] also names.
const POOL: u32 = lis(-32208).wrapping_add(-31232i32 as u32);
const _: () = assert!(POOL == 0x822F_8600, "lis -32208 ; addi -31232");
const _: () = assert!(
    POOL == dsp::biquad::POOL,
    "the same pool the kernel loads its bias from"
);

/// `lis -32245 ; lfs 16668` — multiplies the normalised `numerator / rate` back up.
pub const CUTOFF_SCALE: u32 = lis(-32245).wrapping_add(16668);
/// `lfs 2172` off [`POOL`] — the low bound. [`lowpass_stage`] clamps **up** to it; the high-pass
/// **bypasses** at or below it.
pub const CUTOFF_FLOOR: u32 = POOL.wrapping_add(2172);
/// `lfs 2176` off [`POOL`] — the high bound, with the two roles the other way round.
pub const CUTOFF_CEILING: u32 = POOL.wrapping_add(2176);
/// `lis -32234 ; lfs 23056` — the single both bypass paths clear the filter history with.
pub const ZERO_SINGLE: u32 = lis(-32234).wrapping_add(23056);
/// `lfs 432` off [`POOL`] — read by the kernel, not by either body here. Named so that the read
/// spans in this module's documentation account for it.
pub const KERNEL_BIAS: u32 = POOL.wrapping_add(432);

/// `lis -32246 ; lfs -26788` — measured **0.5**. `alpha = 0.5 · sin w`.
pub const HALF_SINGLE: u32 = lis(-32246).wrapping_add(-26788i32 as u32);
/// `lis -32206 ; lfs -22460` — measured **1.0**. The `1` in `a0 = 1 + alpha` and in `1 ∓ cos`.
pub const ONE_SINGLE: u32 = lis(-32206).wrapping_add(-22460i32 as u32);
/// `lis -32247 ; lfs 16760` — measured **-2.0**. The `-2` in `a1 = -2cos/a0`.
pub const MINUS_TWO_SINGLE: u32 = lis(-32247).wrapping_add(16760);
/// `lis -32250 ; lfs 3152` — measured **2.0**. The `2` in `b0 = num/(2·a0)`.
pub const TWO_SINGLE: u32 = lis(-32250).wrapping_add(3152);

const _: () = assert!(CUTOFF_SCALE == 0x820B_411C);
const _: () = assert!(
    CUTOFF_FLOOR == 0x822F_8E7C,
    "lis -32208 ; addi -31232 ; lfs 2172"
);
const _: () = assert!(CUTOFF_CEILING == 0x822F_8E80, "lfs 2176");
const _: () = assert!(CUTOFF_CEILING == CUTOFF_FLOOR + 4, "two adjacent words");
const _: () = assert!(ZERO_SINGLE == 0x8216_5A10, "lis -32234 ; lfs 23056");
const _: () = assert!(KERNEL_BIAS == 0x822F_87B0);
const _: () = assert!(
    KERNEL_BIAS == dsp::biquad::DENORM_BIAS,
    "the kernel's own bias cell"
);
const _: () = assert!(HALF_SINGLE == 0x8209_975C, "lis -32246 ; lfs -26788");
const _: () = assert!(ONE_SINGLE == 0x8231_A844, "lis -32206 ; lfs -22460");
const _: () = assert!(MINUS_TWO_SINGLE == 0x8209_4178, "lis -32247 ; lfs 16760");
const _: () = assert!(TWO_SINGLE == 0x8206_0C50, "lis -32250 ; lfs 3152");

// Three of the five are cells this crate has already read out of the validated image dump, from two
// other modules. Asserting the addresses agree is what turns "0.5, 1.0, 0.0" from an inference about
// this file's constants into the same measurement `spatial.rs` records.
const _: () = assert!(ZERO_SINGLE == crate::spatial::ZERO_SINGLE, "0.0f, measured");
const _: () = assert!(ONE_SINGLE == crate::spatial::ONE_SINGLE, "1.0f, measured");
const _: () = assert!(HALF_SINGLE == crate::spatial::HALF_SINGLE, "0.5f, measured");

// ================================================= sub_82B43CC0: the low-pass coefficients

/// `sub_82B43CC0` — build the five normalised low-pass biquad coefficients from one angle.
///
/// Arguments, by register: `coefficients` is `r3` (addressed through the low word, `mr r31,r3`) and
/// `angle` is `f1`, in radians. There is no return value (`kReturnNone`); `r3` on exit still holds
/// the pointer.
///
/// **Writes** exactly [`COEFFICIENT_BYTES`] at `coefficients`, in ascending order, each single stored
/// immediately after it is computed. Reads [`HALF_SINGLE`], [`ONE_SINGLE`], [`MINUS_TWO_SINGLE`] and
/// [`TWO_SINGLE`], all four **live** through the guest map, plus whatever `trig` reads.
///
/// `b2` is computed by a **second** `fdivs` of the same two operands rather than copied from `b0` —
/// see the module note. Both call sites pass the interior of a filter object (`+184` from
/// [`lowpass_stage`], `+172` from `sub_82B2D5D4`) and neither reads a register result, so those twenty
/// bytes are the whole observable output.
pub fn build_lowpass_coefficients<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    coefficients: u32,
    angle: f64,
) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();

    // bl 0x82f4ded0 — sine, then cosine, each `frsp`'d before it is used. The narrowing is what
    // keeps the double-precision tail of the helper's answer out of everything below.
    let sine_raw = trig.sine(g, angle)?;
    fpscr.disable_flush_mode_unconditional(); // emitted on return, before the frsp
    let sine = fp::frsp(sine_raw); // frsp f30,f1
    let cosine_raw = trig.cosine(g, angle)?; // fmr f1,f31 ; bl 0x82f4dfb0
    fpscr.disable_flush_mode_unconditional();
    let cosine = fp::frsp(cosine_raw); // frsp f11,f1

    let half = fp::load_single(g, HALF_SINGLE)?; // lfs f0,-26788(r11)
    let alpha = fp::mul_single(sine, half); // fmuls f10,f30,f0
    let one = fp::load_single(g, ONE_SINGLE)?; // lfs f0,-22460(r10)
    let minus_two = fp::load_single(g, MINUS_TWO_SINGLE)?; // lfs f13,16760(r9)
    let two = fp::load_single(g, TWO_SINGLE)?; // lfs f12,3152(r8)
    let numerator = fp::sub_single(one, cosine); // fsubs f9,f0,f11 — the low-pass form, `1 - cos`
    let neg_two_cos = fp::mul_single(cosine, minus_two); // fmuls f8,f11,f13
    let a0 = fp::add_single(alpha, one); // fadds f7,f10,f0
    let one_minus_alpha = fp::sub_single(one, alpha); // fsubs f6,f0,f10
    let inv_a0 = fp::div_single(one, a0); // fdivs f5,f0,f7
    let two_a0 = fp::mul_single(a0, two); // fmuls f4,f7,f12

    let at = |k: u32| coefficients.wrapping_add(k);
    fp::store_single(
        g,
        at(dsp::biquad::COEFF_A1),
        fp::mul_single(neg_two_cos, inv_a0),
    )?;
    fp::store_single(
        g,
        at(dsp::biquad::COEFF_A2),
        fp::mul_single(one_minus_alpha, inv_a0),
    )?;
    fp::store_single(
        g,
        at(dsp::biquad::COEFF_B0),
        fp::div_single(numerator, two_a0),
    )?;
    fp::store_single(
        g,
        at(dsp::biquad::COEFF_B1),
        fp::mul_single(numerator, inv_a0),
    )?;
    // fdivs f13,f9,f4 — the same quotient a second time, not a copy of b0. See the module note.
    fp::store_single(
        g,
        at(dsp::biquad::COEFF_B2),
        fp::div_single(numerator, two_a0),
    )
}

// ------------------------------------------------------------------------------ shared by the stages

/// `fdivs`, then `fmuls` — two single-rounded operations, **not** one FMA.
///
/// `numerator / rate * scale`, where the division's result is narrowed to a single before it is
/// multiplied. Written once because both stages open with the identical three instructions.
fn normalised_cutoff(g: &Guest, object: u32, format: u32) -> Result<f64> {
    let numerator = fp::load_single(g, object.wrapping_add(CUTOFF_INPUT))?; // lfs f0,52(r3)
    let rate = fp::load_single(g, format.wrapping_add(FORMAT_SAMPLE_RATE))?; // lfs f13,12(r10)
    let scale = fp::load_single(g, CUTOFF_SCALE)?;
    // fdivs f12,f0,f13 ; fmuls f31,f12,f13
    Ok(fp::mul_single(fp::div_single(numerator, rate), scale))
}

/// The bypass path's history clear: four singles per channel, walked `stfsu`-style.
///
/// The cursor starts at `object + 52` — the cutoff word, *not* the state array — and the four stores
/// per pass land at `+4`, `+8`, `+12` and then at the cursor after it advances by 16. That is what
/// puts channel 0's four words at `+56`…`+68` and channel `i`'s at `+56 + 16i`. Written as the
/// instruction pair forms it rather than as `FILTER_STATE + 16*i`, because the pre-increment is the
/// reason the array starts four bytes above the cursor's base.
///
/// The count is **reloaded** at the bottom of every pass, as the original does.
fn clear_history(g: &mut Guest, fpscr: &mut Fpscr, object: u32, mut channels: u32) -> Result<()> {
    if channels == 0 {
        return Ok(()); // beq cr6 — checked by the caller too, and by the loop's shape
    }
    let zero = fp::load_single(g, ZERO_SINGLE)?; // lfs f0,23056(r9)
    let mut cursor = object.wrapping_add(CUTOFF_INPUT); // addi r11,r3,52
    let mut index: u32 = 0; // li r10,0
    loop {
        fpscr.disable_flush_mode_unconditional();
        fp::store_single(g, cursor.wrapping_add(4), zero)?; // stfs f0,4(r11)
        index += 1; // addi r10,r10,1
        fp::store_single(g, cursor.wrapping_add(8), zero)?; // stfs f0,8(r11)
        fp::store_single(g, cursor.wrapping_add(12), zero)?; // stfs f0,12(r11)
        cursor = cursor.wrapping_add(16); // stfsu f0,16(r11): store at r11+16, then r11 = ea
        fp::store_single(g, cursor, zero)?;
        channels = g.u8(object.wrapping_add(CHANNEL_COUNT))? as u32; // lbz r9,42(r30)
        if !(index < channels) {
            return Ok(()); // cmplw cr6,r10,r9 ; blt
        }
    }
}

/// The filter pass and the descriptor swap, shared verbatim by both stages.
///
/// Both bodies emit these instructions identically, down to the order the two strides are loaded
/// before the two bases. `state` is kept as an `i64` because the original's is: `addi r3,r30,56` on a
/// 64-bit register, then `addi r3,r3,16` per channel.
fn filter_channels_and_swap(g: &mut Guest, object: u64, stream: u32) -> Result<()> {
    let object_low = object as u32;
    let mut channels = g.u8(object_low.wrapping_add(CHANNEL_COUNT))? as u32; // lbz r11,42(r30)
    let src_desc = g.u32(stream.wrapping_add(STREAM_BUFFER_A))?; // lwz r29,28(r26)
    let dst_desc = g.u32(stream.wrapping_add(STREAM_BUFFER_B))?; // lwz r28,32(r26)

    if channels != 0 {
        let coefficients = object.wrapping_add(COEFFICIENTS as u64); // addi r27,r30,184
        let mut state = (object as i64).wrapping_add(FILTER_STATE as i64); // addi r3,r30,56
        let mut index: u32 = 0; // li r31,0
        loop {
            let src_stride = g.u16(src_desc.wrapping_add(DESC_STRIDE))? as u32; // lhz 14(r29)
            let dst_stride = g.u16(dst_desc.wrapping_add(DESC_STRIDE))? as u32; // lhz 14(r28)
            let src_base = g.u32(src_desc.wrapping_add(DESC_BASE))?; // lwz 4(r29)
            let dst_base = g.u32(dst_desc.wrapping_add(DESC_BASE))?; // lwz 4(r28)
            // mullw ; rlwinm ; add — 64-bit product, 32-bit shift, 64-bit sum. The index cannot
            // exceed the u8 count that bounds the loop, so the `mullw`'s sign extension of it is the
            // identity here.
            let src_ptr = gains::channel_address(src_stride, index, src_base);
            let dst_ptr = gains::channel_address(dst_stride, index, dst_base);
            // bl 0x82b43af8 — reads 256 singles at r5, writes 256 at r4, rewrites four state words
            // at r3. r7 is 256, so the kernel never reaches its generic fallback.
            dsp::biquad::biquad(
                g,
                state as u64 as u32,
                dst_ptr as u32,
                src_ptr as u32,
                coefficients as u32,
                BLOCK_FRAMES,
            )?;
            channels = g.u8(object_low.wrapping_add(CHANNEL_COUNT))? as u32; // lbz r11,42(r30)
            index += 1; // addi r31,r31,1
            // addi r3,r3,16 on whatever the callee left in r3 — which sub_82B43AF8 never writes.
            // See the module note: this is a read of the callee's r3 in the original.
            state = state.wrapping_add(16);
            if !(index < channels) {
                break; // cmplw cr6,r31,r11 ; blt
            }
        }
    }

    // The ping-pong swap. Both loads happen before both stores, so a reordering here would write the
    // same descriptor into both slots.
    let new_a = g.u32(stream.wrapping_add(STREAM_BUFFER_B))?; // lwz r11,32(r26)
    let new_b = g.u32(stream.wrapping_add(STREAM_BUFFER_A))?; // lwz r10,28(r26)
    g.set_u32(stream.wrapping_add(STREAM_BUFFER_A), new_a)?; // stw r11,28(r26)
    g.set_u32(stream.wrapping_add(STREAM_BUFFER_B), new_b) // stw r10,32(r26)
}

// ============================================================== sub_82B27E20: the low-pass

/// `sub_82B27E20` — run the per-channel low-pass over a 256-frame block, or bypass it.
///
/// Arguments, by register: `object` is `r3` (the filter object) and `stream` is `r4`. Both arrive as
/// full 64-bit registers — the original keeps them in `r30`/`r26` and the state pointer it hands the
/// kernel is `r30 + 56` computed on the **whole** register — and both are addressed through their low
/// words. **Returns `1`** (`li r3,1`) on every path; `kReturnR3`, so that constant is the whole of the
/// register comparison.
///
/// **Writes**, as the union of both paths: the five coefficients and the cache at `+184`…`+207`; the
/// four state words of every channel at `+56 + 16i`; the two descriptor words at `stream + 28`; and,
/// on the filtering path only, 1 KB per channel through the destination descriptor. Reads `+42`,
/// `+52`, `+184`…`+207`, the format pointer at `stream + 40` and the rate at `+12` of it, both
/// descriptors, 1 KB per channel through the source descriptor, and the rodata cells
/// [`CUTOFF_SCALE`], [`CUTOFF_FLOOR`], [`CUTOFF_CEILING`], [`ZERO_SINGLE`], [`KERNEL_BIAS`] — plus,
/// through [`build_lowpass_coefficients`] on the recompute path, [`HALF_SINGLE`], [`ONE_SINGLE`],
/// [`MINUS_TWO_SINGLE`] and [`TWO_SINGLE`].
///
/// **Those last four, and `stream + 40` itself, are reads the C++ `Windows()` does not declare.**
/// That is recorded here rather than worked around: a recorded vector that reaches the recompute path
/// has none of those five cells in its read set and is unreplayable until one `spec.read` line per
/// cell is added. The high-pass twin declares the four coefficient cells, because it builds the
/// coefficients itself; the low-pass reaches them through a callee.
///
/// **The bypass is at the ceiling.** `!(cutoff < ceiling)` bypasses — which a NaN cutoff satisfies,
/// the compare being unordered — and then the descriptor pair is **not** swapped, so the caller keeps
/// reading the untouched input block. The history is cleared only when the *cached* cutoff was itself
/// below the ceiling, i.e. only when those words hold filter state worth clearing.
pub fn lowpass_stage<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    object: u64,
    stream: u64,
) -> Result<u64> {
    let object_low = object as u32;
    let stream_low = stream as u32;

    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let format = g.u32(stream_low.wrapping_add(STREAM_FORMAT))?; // lwz r10,40(r4)
    let mut cutoff = normalised_cutoff(g, object_low, format)?;
    let ceiling = fp::load_single(g, CUTOFF_CEILING)?; // lfs f0,2176(r11)

    // blt cr6,0x82b27ec4 — at or above the ceiling (and on a NaN, which is unordered) the stage is
    // bypassed.
    if !(cutoff < ceiling) {
        let cached = fp::load_single(g, object_low.wrapping_add(CACHED_CUTOFF))?; // lfs f13,204(r3)
        // bge cr6,0x82b27eb0
        if cached < ceiling {
            let channels = g.u8(object_low.wrapping_add(CHANNEL_COUNT))? as u32; // lbz r11,42(r3)
            clear_history(g, &mut fpscr, object_low, channels)?;
        }
        // loc_82B27EB0
        fpscr.disable_flush_mode_unconditional();
        fp::store_single(g, object_low.wrapping_add(CACHED_CUTOFF), cutoff)?; // stfs f31,204(r30)
        return Ok(1); // li r3,1
    }

    // loc_82B27EC4
    fpscr.disable_flush_mode_unconditional();
    let floor_value = fp::load_single(g, CUTOFF_FLOOR)?; // lfs f0,2172(r11)
    // bge cr6,0x82b27ed4 — clamp up to the floor only when strictly below it.
    if cutoff < floor_value {
        cutoff = floor_value; // fmr f31,f0
    }

    // loc_82B27ED4
    fpscr.disable_flush_mode_unconditional();
    let cached = fp::load_single(g, object_low.wrapping_add(CACHED_CUTOFF))?; // lfs f0,204(r30)
    // beq cr6,0x82b27ef0 — an unchanged cutoff reuses the coefficients already at +184, and so never
    // reaches the trigonometry. That is the majority path and it needs no `Trig`.
    if cutoff != cached {
        // addi r3,r30,184 ; fmr f1,f31 ; bl 0x82b43cc0
        build_lowpass_coefficients(
            g,
            trig,
            object.wrapping_add(COEFFICIENTS as u64) as u32,
            cutoff,
        )?;
        fpscr.disable_flush_mode_unconditional();
        fp::store_single(g, object_low.wrapping_add(CACHED_CUTOFF), cutoff)?; // stfs f31,204(r30)
    }

    // loc_82B27EF0
    filter_channels_and_swap(g, object, stream_low)?;
    Ok(1) // li r3,1
}

// ============================================================== sub_82B26568: the high-pass

/// `sub_82B26568` — run the per-channel high-pass over a 256-frame block, or bypass it.
///
/// The mirror of [`lowpass_stage`]: same arguments, same return of `1`, same write set, and the same
/// `Trig` cost on the recompute path. What differs is in the module's comparison table — the bypass
/// is at the **floor**, the clamp is **down** to the ceiling, the coefficients are built **inline**
/// rather than through `sub_82B43CC0`, and the numerator is `1 + cos` with `b1` **negated**.
///
/// **Writes** and reads as [`lowpass_stage`] does, with one difference in each direction: the four
/// coefficient cells are read by this body itself, so the C++ `Windows()` **does** declare them, and
/// the only read it leaves undeclared is the format pointer at `stream + 40`. The store order for the
/// five coefficients is `184, 188, 192, 200, 196` — not ascending — followed by the cache at `+204`.
///
/// One instruction is deliberately absent: `fmr f30,f31` between the two math calls. `f30` is never
/// read again and the epilogue reloads it, so the C++ port leaves the register at its entry value and
/// there is nothing here for it to be.
pub fn highpass_stage<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    object: u64,
    stream: u64,
) -> Result<u64> {
    let object_low = object as u32;
    let stream_low = stream as u32;

    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let format = g.u32(stream_low.wrapping_add(STREAM_FORMAT))?; // lwz r10,40(r4)
    let mut cutoff = normalised_cutoff(g, object_low, format)?;
    let floor_value = fp::load_single(g, CUTOFF_FLOOR)?; // lfs f0,2172(r11)

    // bgt cr6,0x82b26608 — at or below the floor (and on a NaN) the stage is bypassed and the pair
    // is not swapped. The mirror of the low-pass, which bypasses at the ceiling instead.
    if !(cutoff > floor_value) {
        let cached = fp::load_single(g, object_low.wrapping_add(CACHED_CUTOFF))?; // lfs f13,204(r3)
        // ble cr6,0x82b26600
        if cached > floor_value {
            let channels = g.u8(object_low.wrapping_add(CHANNEL_COUNT))? as u32; // lbz r11,42(r3)
            clear_history(g, &mut fpscr, object_low, channels)?;
        }
        // loc_82B26600, then straight to loc_82B26724: no kernel, no descriptor swap.
        fpscr.disable_flush_mode_unconditional();
        fp::store_single(g, object_low.wrapping_add(CACHED_CUTOFF), cutoff)?; // stfs f31,204(r30)
        return Ok(1); // li r3,1
    }

    // loc_82B26608
    fpscr.disable_flush_mode_unconditional();
    let ceiling = fp::load_single(g, CUTOFF_CEILING)?; // lfs f0,2176(r11)
    // ble cr6,0x82b26618 — clamp down to the ceiling only when strictly above it.
    if cutoff > ceiling {
        cutoff = ceiling; // fmr f31,f0
    }

    // loc_82B26618
    fpscr.disable_flush_mode_unconditional();
    let cached = fp::load_single(g, object_low.wrapping_add(CACHED_CUTOFF))?; // lfs f0,204(r30)
    // beq cr6,0x82b266ac
    if cutoff != cached {
        let sine_raw = trig.sine(g, cutoff)?; // fmr f1,f31 ; bl 0x82f4ded0
        fpscr.disable_flush_mode_unconditional();
        let sine = fp::frsp(sine_raw); // frsp f29,f1
        let cosine_raw = trig.cosine(g, cutoff)?; // fmr f1,f31 ; bl 0x82f4dfb0
        fpscr.disable_flush_mode_unconditional();
        let cosine = fp::frsp(cosine_raw); // frsp f11,f1

        let alpha = fp::mul_single(sine, fp::load_single(g, HALF_SINGLE)?); // fmuls f10,f29,f0
        let one = fp::load_single(g, ONE_SINGLE)?; // lfs f0,-22460(r10)
        // fadds f9,f11,f0 — the high-pass form, `1 + cos`. sub_82B43CC0 does an fsubs here.
        let numerator = fp::add_single(cosine, one);
        // fmuls f8,f11,f13
        let neg_two_cos = fp::mul_single(cosine, fp::load_single(g, MINUS_TWO_SINGLE)?);
        let a0 = fp::add_single(alpha, one); // fadds f7,f10,f0
        let one_minus_alpha = fp::sub_single(one, alpha); // fsubs f6,f0,f10
        let inv_a0 = fp::div_single(one, a0); // fdivs f5,f0,f7
        let two_a0 = fp::mul_single(a0, fp::load_single(g, TWO_SINGLE)?); // fmuls f4,f7,f12
        let half_num = fp::mul_single(numerator, inv_a0); // fmuls f3,f9,f5

        // Store order 184, 188, 192, 200, 196, 204 — kept as the original has it.
        let at = |k: u32| object_low.wrapping_add(COEFFICIENTS).wrapping_add(k);
        fp::store_single(
            g,
            at(dsp::biquad::COEFF_A1),
            fp::mul_single(neg_two_cos, inv_a0),
        )?;
        fp::store_single(
            g,
            at(dsp::biquad::COEFF_A2),
            fp::mul_single(one_minus_alpha, inv_a0),
        )?;
        fp::store_single(
            g,
            at(dsp::biquad::COEFF_B0),
            fp::div_single(numerator, two_a0),
        )?;
        fp::store_single(
            g,
            at(dsp::biquad::COEFF_B2),
            fp::div_single(numerator, two_a0),
        )?;
        // fneg f12,f3 — a sign-bit flip on the doubleword, not an arithmetic negate. This single
        // instruction is what makes the stage a high-pass rather than a low-pass.
        fp::store_single(g, at(dsp::biquad::COEFF_B1), fp::neg_double(half_num))?;
        fp::store_single(g, object_low.wrapping_add(CACHED_CUTOFF), cutoff)?; // stfs f31,204(r30)
    }

    // loc_82B266AC
    filter_channels_and_swap(g, object, stream_low)?;
    Ok(1) // loc_82B26724: li r3,1
}

// ============================================================ sub_82B43D78 + sub_82B26740: the shelf

/// `lis -32208 ; addi -31232 ; lfs 2128` — measured 0.70710653, the alpha scale of an RBJ shelf at
/// slope S = 1. Next to the low-pass's pool cells, and read live like them.
pub const SHELF_ALPHA_SCALE: u32 = 0x822F_8E50;
/// `lis -32250 ; lfs 3152` — 2.0.
pub const SHELF_TWO: u32 = (((-32250i32 as u32) & 0xFFFF) << 16) + 3152;
/// `lis -32247 ; lfs 16760` — −2.0.
pub const SHELF_MINUS_TWO: u32 = (((-32247i32 as u32) & 0xFFFF) << 16) + 16760;
/// `lis -32206 ; lfs -22460` — 1.0, which is also the "no shelf" gain the stage bypasses on.
pub const SHELF_ONE: u32 = (((-32206i32 as u32) & 0xFFFF) << 16).wrapping_sub(22460);
const _: () = assert!(SHELF_ALPHA_SCALE == 0x822F_0000 + 0x8E50);
const _: () =
    assert!(SHELF_TWO == 0x8206_0C50 && SHELF_MINUS_TWO == 0x8209_4178 && SHELF_ONE == 0x8231_A844);

/// `f32` at `+60` — the shelf gain, handed to the builder as its linear amplitude squared.
pub const SHELF_GAIN: u32 = 60;
/// `f32[4]` per channel from `+64` — the biquad histories.
pub const SHELF_HISTORY: u32 = 64;
/// `s32` at `+192` — 1 while the shelf is running, 0 once its histories have been cleared.
pub const SHELF_ENGAGED: u32 = 192;
/// `f32[5]` at `+196` — the builder's `[a1, a2, b0, b1, b2] / a0`.
pub const SHELF_COEFFICIENTS: u32 = 196;
/// `f32` at `+216` — the corner the coefficients were built for.
pub const SHELF_CACHED_CORNER: u32 = 216;
/// `f32` at `+220` — the gain they were built for.
pub const SHELF_CACHED_GAIN: u32 = 220;

/// Build the five normalised high-shelf coefficients at `coefficients` (`sub_82B43D78`, **thin**).
///
/// `angle` is `f1` and `gain` is `f2`. With `A = sqrt(gain)` and `alpha = sin(angle) * 0.70710653`
/// this is the RBJ high shelf at slope 1, divided through by `a0`. Written in the lifted order and
/// precision, with two details kept that a tidier version drops: `sqrt(A)` is taken **four separate
/// times** (four `fsqrts` of the same value), and every multiply-add is a scalar `fmadds` or
/// `fnmsubs` — one rounding — which is not the same as the separate multiply and add a reader of the
/// formula would write.
pub fn build_shelf_coefficients<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    coefficients: u32,
    angle: f64,
    gain: f64,
) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // stfd f29,-40(r1)
    let sine_raw = trig.sine(g, angle)?; // bl 0x82f4ded0
    fpscr.disable_flush_mode_unconditional();
    let sine = fp::frsp(sine_raw); // frsp f29,f1
    let cosine_raw = trig.cosine(g, angle)?; // fmr f1,f30 ; bl 0x82f4dfb0
    fpscr.disable_flush_mode_unconditional();

    let a = fp::sqrt_single(gain); // fsqrts f10,f31
    let one = fp::load_single(g, SHELF_ONE)?;
    let alpha_scale = fp::load_single(g, SHELF_ALPHA_SCALE)?;
    let two = fp::load_single(g, SHELF_TWO)?;
    let minus_two = fp::load_single(g, SHELF_MINUS_TWO)?;
    let cosine = fp::frsp(cosine_raw); // frsp f9,f1
    let a_plus = fp::add_single(a, one); // fadds f8,f10,f13
    let a_minus = fp::sub_single(a, one); // fsubs f7,f10,f13
    let alpha = fp::mul_single(sine, alpha_scale); // fmuls f6,f29,f12

    let beta0 = fp::mul_single(fp::sqrt_single(a), alpha); // fsqrts f5 ; fmuls f4,f5,f6
    let a0_base = fp::nmsub_single(a_minus, cosine, a_plus); // fnmsubs f3,f7,f9,f8
    let beta1 = fp::sqrt_single(a); // fsqrts f2,f10
    let a0 = fp::fmadd_single(beta0, two, a0_base); // fmadds f1,f4,f0,f3
    let beta2 = fp::mul_single(beta1, alpha); // fmuls f12,f2,f6
    let a_minus_cos = fp::mul_single(a_minus, cosine); // fmuls f5,f7,f9
    let beta3 = fp::sqrt_single(a); // fsqrts f4,f10
    let inv_a0 = fp::div_single(one, a0); // fdivs f3,f13,f1
    let beta4 = fp::sqrt_single(a); // fsqrts f2,f10
    let b0_mid = fp::fmadd_single(beta2, two, a_minus_cos); // fmadds f1,f12,f0,f5
    let b1_base = fp::fmadd_single(a_plus, cosine, a_minus); // fmadds f13,f8,f9,f7
    let b2_base = fp::fmadd_single(a_minus, cosine, a_plus); // fmadds f12,f7,f9,f8
    let beta5 = fp::mul_single(beta3, alpha); // fmuls f5,f4,f6
    let a1_base = fp::nmsub_single(a_plus, cosine, a_minus); // fnmsubs f4,f8,f9,f7
    let a2_base = fp::nmsub_single(a_minus, cosine, a_plus); // fnmsubs f9,f7,f9,f8
    let beta6 = fp::mul_single(beta4, alpha); // fmuls f7,f2,f6
    let b0_sum = fp::add_single(b0_mid, a_plus); // fadds f6,f1,f8
    let b1_norm = fp::mul_single(b1_base, inv_a0); // fmuls f2,f13,f3
    let b2_sum = fp::nmsub_single(beta5, two, b2_base); // fnmsubs f1,f5,f0,f12
    let a1_norm = fp::mul_single(a1_base, inv_a0); // fmuls f13,f4,f3
    let a2_sum = fp::nmsub_single(beta6, two, a2_base); // fnmsubs f12,f7,f0,f9
    let b0_norm = fp::mul_single(b0_sum, inv_a0); // fmuls f9,f6,f3
    let b1_scaled = fp::mul_single(b1_norm, a); // fmuls f8,f2,f10
    let b2_norm = fp::mul_single(b2_sum, inv_a0); // fmuls f7,f1,f3

    let at = |k: u32| coefficients.wrapping_add(k);
    fp::store_single(g, at(dsp::biquad::COEFF_A1), fp::mul_single(a1_norm, two))?;
    fp::store_single(g, at(dsp::biquad::COEFF_A2), fp::mul_single(a2_sum, inv_a0))?;
    fp::store_single(g, at(dsp::biquad::COEFF_B0), fp::mul_single(b0_norm, a))?;
    fp::store_single(
        g,
        at(dsp::biquad::COEFF_B1),
        fp::mul_single(b1_scaled, minus_two),
    )?;
    fp::store_single(g, at(dsp::biquad::COEFF_B2), fp::mul_single(b2_norm, a))
}

/// `mullw ; rlwinm 2,0,29 ; add` — one channel's float array. `mullw` is the 64-bit product of two
/// sign-extended words, the shift is 32-bit, and the add is 64-bit on zero-extended words; the callee
/// uses only the low half.
fn shelf_channel_address(stride: u16, index: u32, buffer: u32) -> u64 {
    let elements = i64::from(i32::from(stride)) * i64::from(index as i32);
    u64::from((elements as u32) << 2) + u64::from(buffer)
}

/// Run the per-source shelf filter (`sub_82B26740`): `object` is `r3`, `pair` the buffer-descriptor
/// pair in `r4`. Returns 1 on every path.
///
/// The corner is the low-pass's normalised cutoff over the same pool cells ([`normalised_cutoff`],
/// [`CUTOFF_CEILING`], [`CUTOFF_FLOOR`] — one test pins that they are the same addresses). Then:
///
/// - **At or past the ceiling, or at unity gain, the shelf is bypassed.** A NaN corner is unordered
///   and bypasses too; a NaN gain compares unequal to 1.0 and keeps filtering. On the bypass the
///   histories are cleared once — only when the engaged flag is exactly 1 — and the flag drops to 0.
/// - **Otherwise it filters**: the engaged flag is set, the corner is floored, the coefficients are
///   rebuilt only when the (corner, gain) pair moved, every channel runs through the biquad, and the
///   pair's two words are exchanged.
///
/// Both paths publish the cache, and on both the gain is **reloaded** from `+60` for it rather than
/// reused. The channel count is re-read after every channel, and the descriptor fields every
/// iteration, as the original does.
pub fn shelf_stage<T: Trig>(g: &mut Guest, trig: &mut T, object: u32, pair: u32) -> Result<u64> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // stfd f31,-64(r1)
    let format = g.u32(pair.wrapping_add(STREAM_FORMAT))?; // lwz r10,40(r4)
    let mut corner = normalised_cutoff(g, object, format)?;
    let limit = fp::load_single(g, CUTOFF_CEILING)?; // lfs f13,2176(r11)

    let mut gain = 0.0;
    let mut filter = false;
    if corner < limit {
        gain = fp::load_single(g, object.wrapping_add(SHELF_GAIN))?; // lfs f2,60(r3)
        filter = gain != fp::load_single(g, SHELF_ONE)?; // fcmpu ; beq -- unity is a no-op
    }

    if filter {
        if g.u32(object.wrapping_add(SHELF_ENGAGED))? as i32 == 0 {
            g.set_u32(object.wrapping_add(SHELF_ENGAGED), 1)?; // li r10,1 ; stw r10,192(r3)
        }
        let floor_value = fp::load_single(g, CUTOFF_FLOOR)?; // lfs f0,2172(r11)
        if corner < floor_value {
            corner = floor_value; // fmr f31,f0
        }
        let cached_corner = fp::load_single(g, object.wrapping_add(SHELF_CACHED_CORNER))?;
        let unchanged = corner == cached_corner
            && gain == fp::load_single(g, object.wrapping_add(SHELF_CACHED_GAIN))?;
        if !unchanged {
            build_shelf_coefficients(
                g,
                trig,
                object.wrapping_add(SHELF_COEFFICIENTS),
                corner,
                gain,
            )?;
            fpscr.disable_flush_mode_unconditional();
            let published = fp::load_single(g, object.wrapping_add(SHELF_GAIN))?; // reloaded
            fp::store_single(g, object.wrapping_add(SHELF_CACHED_CORNER), corner)?;
            fp::store_single(g, object.wrapping_add(SHELF_CACHED_GAIN), published)?;
        }

        let mut channels = u32::from(g.u8(object.wrapping_add(CHANNEL_COUNT))?); // lbz r11,42(r30)
        let input_desc = g.u32(pair.wrapping_add(STREAM_BUFFER_A))?; // lwz r29,28(r26)
        let output_desc = g.u32(pair.wrapping_add(STREAM_BUFFER_B))?; // lwz r28,32(r26)
        if channels != 0 {
            let coefficients = object.wrapping_add(SHELF_COEFFICIENTS);
            let mut history = object.wrapping_add(SHELF_HISTORY);
            let mut index = 0u32;
            loop {
                // All four descriptor fields are re-read on every iteration.
                let in_stride = g.u16(input_desc.wrapping_add(DESC_STRIDE))?;
                let out_stride = g.u16(output_desc.wrapping_add(DESC_STRIDE))?;
                let in_base = g.u32(input_desc.wrapping_add(DESC_BASE))?;
                let out_base = g.u32(output_desc.wrapping_add(DESC_BASE))?;
                let output = shelf_channel_address(out_stride, index, out_base) as u32;
                let input = shelf_channel_address(in_stride, index, in_base) as u32;
                dsp::biquad::biquad(g, history, output, input, coefficients, BLOCK_FRAMES)?;
                channels = u32::from(g.u8(object.wrapping_add(CHANNEL_COUNT))?); // reloaded
                index += 1;
                history = history.wrapping_add(16);
                if index >= channels {
                    break; // cmplw ; blt
                }
            }
        }
        // loc_82B26858 -- the swap, with both words reloaded.
        let swap_out = g.u32(pair.wrapping_add(STREAM_BUFFER_B))?;
        let swap_in = g.u32(pair.wrapping_add(STREAM_BUFFER_A))?;
        g.set_u32(pair.wrapping_add(STREAM_BUFFER_A), swap_out)?;
        g.set_u32(pair.wrapping_add(STREAM_BUFFER_B), swap_in)?;
        return Ok(1);
    }

    // loc_82B26878 -- bypassed. cmpwi cr6,r11,1: only exactly 1 clears.
    if g.u32(object.wrapping_add(SHELF_ENGAGED))? as i32 == 1 {
        let mut channels = u32::from(g.u8(object.wrapping_add(CHANNEL_COUNT))?);
        if channels != 0 {
            let zero = fp::load_single(g, crate::leaves::ZERO_CELL)?; // lfs f0,23056(r9)
            let mut cursor = object.wrapping_add(SHELF_GAIN); // addi r11,r30,60
            let mut done = 0u32;
            loop {
                fp::store_single(g, cursor.wrapping_add(4), zero)?; // stfs f0,4(r11)
                done += 1;
                fp::store_single(g, cursor.wrapping_add(8), zero)?; // stfs f0,8(r11)
                fp::store_single(g, cursor.wrapping_add(12), zero)?; // stfs f0,12(r11)
                cursor = cursor.wrapping_add(16); // stfsu f0,16(r11)
                fp::store_single(g, cursor, zero)?;
                channels = u32::from(g.u8(object.wrapping_add(CHANNEL_COUNT))?); // reloaded
                if done >= channels {
                    break;
                }
            }
        }
        g.set_u32(object.wrapping_add(SHELF_ENGAGED), 0)?; // stw r11,192(r30)
    }
    let published = fp::load_single(g, object.wrapping_add(SHELF_GAIN))?; // lfs f0,60(r30)
    fp::store_single(g, object.wrapping_add(SHELF_CACHED_CORNER), corner)?;
    fp::store_single(g, object.wrapping_add(SHELF_CACHED_GAIN), published)?;
    Ok(1)
}

// ============================================================ sub_82B2C658: the peaking equaliser

/// `f32` at `+68` — the peak's Q. Clamped for the arithmetic, cached **unclamped**.
pub const PEAK_QUALITY: u32 = 68;
/// `f32[4]` per channel from `+72` — the biquad histories.
pub const PEAK_HISTORY: u32 = 72;
/// `s32` at `+200` — 1 while filtering, 0 once a bypass has cleared the histories.
pub const PEAK_FILTERING: u32 = 200;
/// `f32[5]` at `+204` — `[a1, a2, b0, b1, b2] / a0`.
pub const PEAK_COEFFICIENTS: u32 = 204;
/// `f32` at `+224` — the clamped angle the coefficients were built at.
pub const PEAK_CACHED_WARP: u32 = 224;
/// `f32` at `+228` — the gain they were built at.
pub const PEAK_CACHED_GAIN: u32 = 228;
/// `f32` at `+232` — the Q they were built at, as stored rather than as clamped.
pub const PEAK_CACHED_QUALITY: u32 = 232;
/// `lis -32246 ; lfs -28032` — the lowest Q the arithmetic uses.
pub const QUALITY_FLOOR: u32 = (((-32246i32 as u32) & 0xFFFF) << 16).wrapping_sub(28032);
/// `lis -32246 ; lfs -26900` — the highest.
pub const QUALITY_CEILING: u32 = (((-32246i32 as u32) & 0xFFFF) << 16).wrapping_sub(26900);
const _: () = assert!(QUALITY_FLOOR == 0x8209_9280 && QUALITY_CEILING == 0x8209_96EC);

/// Run the per-channel peaking equaliser over a 256-frame block (`sub_82B2C658`). `object` is `r3`
/// at full width, `stream` the descriptor pair in `r4`. Returns 1 on every path.
///
/// The same family as the shelf and the low-pass, and different from both in ways the tests pin:
///
/// - **The angle is clamped, never bypassed on.** Below the floor it rises to it, above the ceiling it
///   falls to it; only a gain of exactly 1.0 bypasses. A NaN angle is kept as it is, and a NaN gain
///   is "not equal" and filters.
/// - **Q is clamped for the arithmetic but cached as stored.** The cache check compares the raw `+68`
///   against the raw cache, so a Q outside the clamp does not force a rebuild every block.
/// - **The coefficients are the RBJ peak**: `alpha = sin / 2Q`, and with `A = sqrt(gain)` the
///   numerator uses `alpha * A` and the denominator `alpha / A`. `b1` and `a1` are the same product,
///   computed twice.
/// - On the bypass the histories are cleared once — only when the flag reads exactly 1 — and the
///   descriptor pair is **not** swapped.
pub fn peaking_stage<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    object: u64,
    stream: u32,
) -> Result<u64> {
    let obj = object as u32;
    let at = |k: u32| obj.wrapping_add(k);
    let format = g.u32(stream.wrapping_add(STREAM_FORMAT))?; // lwz r10,40(r4)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let mut warp = normalised_cutoff(g, obj, format)?; // fdivs ; fmuls -- two roundings
    let warp_floor = fp::load_single(g, CUTOFF_FLOOR)?; // lfs f0,2172(r11)
    if warp < warp_floor {
        warp = warp_floor; // fmr f29,f0
    } else {
        let warp_ceiling = fp::load_single(g, CUTOFF_CEILING)?; // lfs f0,2176(r11)
        if warp > warp_ceiling {
            warp = warp_ceiling;
        }
    }

    let gain = fp::load_single(g, at(SHELF_GAIN))?; // lfs f28,60(r30)
    let unit = fp::load_single(g, SHELF_ONE)?; // lfs f31,-22460(r11)
    let filtering = g.u32(at(PEAK_FILTERING))? as i32; // lwz r11,200(r30)

    if gain == unit {
        // The first bypassed block clears the histories, once.
        if filtering == 1 {
            let mut channels = u32::from(g.u8(at(CHANNEL_COUNT))?);
            if channels != 0 {
                let zero = fp::load_single(g, crate::leaves::ZERO_CELL)?;
                let mut cursor = at(PEAK_QUALITY); // addi r11,r30,68
                let mut done = 0u32;
                loop {
                    fp::store_single(g, cursor.wrapping_add(4), zero)?;
                    done += 1;
                    fp::store_single(g, cursor.wrapping_add(8), zero)?;
                    fp::store_single(g, cursor.wrapping_add(12), zero)?;
                    cursor = cursor.wrapping_add(16); // stfsu f0,16(r11)
                    fp::store_single(g, cursor, zero)?;
                    channels = u32::from(g.u8(at(CHANNEL_COUNT))?); // reloaded
                    if done >= channels {
                        break;
                    }
                }
            }
            g.set_u32(at(PEAK_FILTERING), 0)?;
        }
        let gain_now = fp::load_single(g, at(SHELF_GAIN))?;
        let quality_now = fp::load_single(g, at(PEAK_QUALITY))?;
        fp::store_single(g, at(PEAK_CACHED_WARP), warp)?;
        fp::store_single(g, at(PEAK_CACHED_GAIN), gain_now)?;
        fp::store_single(g, at(PEAK_CACHED_QUALITY), quality_now)?;
        return Ok(1); // no kernel, no swap
    }

    if filtering == 0 {
        g.set_u32(at(PEAK_FILTERING), 1)?; // tested on the value loaded above
    }
    let mut unchanged = warp == fp::load_single(g, at(PEAK_CACHED_WARP))?;
    if unchanged {
        unchanged = gain == fp::load_single(g, at(PEAK_CACHED_GAIN))?;
    }
    if unchanged {
        let quality_now = fp::load_single(g, at(PEAK_QUALITY))?;
        unchanged = quality_now == fp::load_single(g, at(PEAK_CACHED_QUALITY))?;
    }
    if !unchanged {
        let mut quality = fp::load_single(g, at(PEAK_QUALITY))?; // lfs f30,68(r30)
        let quality_floor = fp::load_single(g, QUALITY_FLOOR)?;
        if quality < quality_floor {
            quality = quality_floor;
        } else {
            let quality_ceiling = fp::load_single(g, QUALITY_CEILING)?;
            if quality > quality_ceiling {
                quality = quality_ceiling;
            }
        }
        let sine_raw = trig.sine(g, warp)?; // fmr f1,f29 ; bl 0x82f4ded0
        fpscr.disable_flush_mode_unconditional();
        let sine = fp::frsp(sine_raw); // frsp f26,f1
        let cosine_raw = trig.cosine(g, warp)?; // bl 0x82f4dfb0
        fpscr.disable_flush_mode_unconditional();
        let amplitude = fp::sqrt_single(gain); // fsqrts f12,f28
        let gain_now = fp::load_single(g, at(SHELF_GAIN))?;
        let quality_now = fp::load_single(g, at(PEAK_QUALITY))?;
        fp::store_single(g, at(PEAK_CACHED_GAIN), gain_now)?; // stfs f11,228(r30)
        fp::store_single(g, at(PEAK_CACHED_QUALITY), quality_now)?; // stfs f10,232(r30)
        let two = fp::load_single(g, SHELF_TWO)?;
        let minus_two = fp::load_single(g, SHELF_MINUS_TWO)?;
        fp::store_single(g, at(PEAK_CACHED_WARP), warp)?; // stfs f29,224(r30)
        let two_q = fp::mul_single(quality, two); // fmuls f9,f30,f0
        let cosine = fp::frsp(cosine_raw); // frsp f8,f1
        let alpha = fp::div_single(sine, two_q); // fdivs f7,f26,f9
        let cos_term = fp::mul_single(cosine, minus_two); // fmuls f6,f8,f13
        let alpha_over_a = fp::div_single(alpha, amplitude); // fdivs f5,f7,f12
        let alpha_times_a = fp::mul_single(amplitude, alpha); // fmuls f4,f12,f7
        let a0 = fp::add_single(alpha_over_a, unit); // fadds f3,f5,f31
        let a2 = fp::sub_single(unit, alpha_over_a); // fsubs f2,f31,f5
        let b0 = fp::add_single(alpha_times_a, unit); // fadds f1,f4,f31
        let b2 = fp::sub_single(unit, alpha_times_a); // fsubs f0,f31,f4
        let inv = fp::div_single(unit, a0); // fdivs f13,f31,f3
        let c = at(PEAK_COEFFICIENTS);
        fp::store_single(g, c, fp::mul_single(inv, cos_term))?; // stfs 204 -- a1
        fp::store_single(g, c + 4, fp::mul_single(a2, inv))?; // stfs 208 -- a2
        fp::store_single(g, c + 8, fp::mul_single(b0, inv))?; // stfs 212 -- b0
        fp::store_single(g, c + 12, fp::mul_single(inv, cos_term))?; // stfs 216 -- b1, again
        fp::store_single(g, c + 16, fp::mul_single(b2, inv))?; // stfs 220 -- b2
    }

    // loc_82B2C834 -- every channel through the biquad, reading A and writing B.
    let mut channels = u32::from(g.u8(at(CHANNEL_COUNT))?);
    let src_desc = g.u32(stream.wrapping_add(STREAM_BUFFER_A))?;
    let dst_desc = g.u32(stream.wrapping_add(STREAM_BUFFER_B))?;
    if channels != 0 {
        let coefficients = (object + u64::from(PEAK_COEFFICIENTS)) as u32;
        let mut history = (object + u64::from(PEAK_HISTORY)) as u32;
        let mut index = 0u32;
        loop {
            let src_stride = g.u16(src_desc.wrapping_add(DESC_STRIDE))?;
            let dst_stride = g.u16(dst_desc.wrapping_add(DESC_STRIDE))?;
            let src_base = g.u32(src_desc.wrapping_add(DESC_BASE))?;
            let dst_base = g.u32(dst_desc.wrapping_add(DESC_BASE))?;
            let src = shelf_channel_address(src_stride, index, src_base) as u32;
            let dst = shelf_channel_address(dst_stride, index, dst_base) as u32;
            dsp::biquad::biquad(g, history, dst, src, coefficients, BLOCK_FRAMES)?;
            channels = u32::from(g.u8(at(CHANNEL_COUNT))?); // reloaded
            index += 1;
            history = history.wrapping_add(16); // on the r3 the kernel left, which it never writes
            if index >= channels {
                break;
            }
        }
    }
    // loc_82B2C89C -- the swap, both loads before both stores.
    let new_a = g.u32(stream.wrapping_add(STREAM_BUFFER_B))?;
    let new_b = g.u32(stream.wrapping_add(STREAM_BUFFER_A))?;
    g.set_u32(stream.wrapping_add(STREAM_BUFFER_A), new_a)?;
    g.set_u32(stream.wrapping_add(STREAM_BUFFER_B), new_b)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathlib::{Unported, tests::Scripted};

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    // Everything else sits above `OBJECT + 1024`, so a test can aim a 1 KB channel write at the
    // object itself without also clobbering the stream it is reading through.
    const STREAM: u32 = BASE + 0x800;
    const FORMAT: u32 = BASE + 0x840;
    const DESC_A: u32 = BASE + 0x880;
    const DESC_B: u32 = BASE + 0x8C0;
    const BUF_A: u32 = BASE + 0x1000;
    const BUF_B: u32 = BASE + 0x3000;
    const COEFFS: u32 = BASE + 0x8000;

    /// A guest with the object, the stream, two descriptors, two 8-channel buffers, and every rodata
    /// cell the three bodies read — each as its **own** four-byte segment, so a body that computed
    /// the wrong address gets an `Err` out of [`Guest`] rather than a plausible neighbouring float.
    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x9000);
        for (cell, value) in [
            (CUTOFF_SCALE, 1.0f32),
            (CUTOFF_FLOOR, 0.01),
            (CUTOFF_CEILING, 1.0),
            (ZERO_SINGLE, 0.0),
            (KERNEL_BIAS, 0.0),
            (HALF_SINGLE, 0.5),
            (ONE_SINGLE, 1.0),
            (MINUS_TWO_SINGLE, -2.0),
            (TWO_SINGLE, 2.0),
        ] {
            g.put(cell, value.to_bits().to_be_bytes().to_vec());
        }
        g.set_u32(STREAM + STREAM_FORMAT, FORMAT).unwrap();
        g.set_u32(STREAM + STREAM_BUFFER_A, DESC_A).unwrap();
        g.set_u32(STREAM + STREAM_BUFFER_B, DESC_B).unwrap();
        g.set_u32(DESC_A + DESC_BASE, BUF_A).unwrap();
        g.set_u16(DESC_A + DESC_STRIDE, 256).unwrap();
        g.set_u32(DESC_B + DESC_BASE, BUF_B).unwrap();
        g.set_u16(DESC_B + DESC_STRIDE, 256).unwrap();
        g
    }

    /// `numerator / rate * scale` with rate and scale both 1.0, so the cutoff *is* the `+52` word.
    fn cutoff(g: &mut Guest, value: f32) {
        g.set_u32(FORMAT + FORMAT_SAMPLE_RATE, 1.0f32.to_bits())
            .unwrap();
        g.set_u32(OBJECT + CUTOFF_INPUT, value.to_bits()).unwrap();
    }

    fn cached(g: &mut Guest, value: f32) {
        g.set_u32(OBJECT + CACHED_CUTOFF, value.to_bits()).unwrap();
    }

    /// A unit-gain pass-through so the stage's output equals its input: `b0 = 1`, everything else 0.
    fn passthrough_coefficients(g: &mut Guest) {
        for k in 0..5u32 {
            g.set_u32(OBJECT + COEFFICIENTS + 4 * k, 0f32.to_bits())
                .unwrap();
        }
        g.set_u32(
            OBJECT + COEFFICIENTS + dsp::biquad::COEFF_B0,
            1.0f32.to_bits(),
        )
        .unwrap();
    }

    fn channels(g: &mut Guest, n: u8) {
        g.set_u8(OBJECT + CHANNEL_COUNT, n).unwrap();
    }

    /// A distinct ramp in each of `n` channels of `BUF_A`, and a poison in `BUF_B`.
    fn input(g: &mut Guest, n: u32) {
        for c in 0..n {
            for i in 0..BLOCK_FRAMES {
                let v = c as f32 * 1000.0 + i as f32;
                g.set_u32(BUF_A + 1024 * c + 4 * i, v.to_bits()).unwrap();
                g.set_u32(BUF_B + 1024 * c + 4 * i, 0x7F7F_7F7F).unwrap();
            }
        }
    }

    fn state_words(g: &Guest, channel: u32) -> Vec<u32> {
        (0..4)
            .map(|k| g.u32(OBJECT + FILTER_STATE + 16 * channel + 4 * k).unwrap())
            .collect()
    }

    fn poison_state(g: &mut Guest, n: u32) {
        for c in 0..n {
            for k in 0..4u32 {
                g.set_u32(OBJECT + FILTER_STATE + 16 * c + 4 * k, 0xDEAD_0000 + k)
                    .unwrap();
            }
        }
    }

    fn coefficients_at(g: &Guest, at: u32) -> Vec<f32> {
        (0..5).map(|k| g.f32(at + 4 * k).unwrap()).collect()
    }

    // ------------------------------------------------------------------ sub_82B43CC0

    /// The five coefficients the RBJ low-pass at Q = 1 gives for this sine and cosine, computed in
    /// the lifted operation order and with the lifted roundings. Shares that order with the port by
    /// construction, so what it checks is which expression lands in which slot, not the rounding —
    /// `the_second_b2_divide_is_not_a_copy_of_b0` and the store-order test carry that half.
    fn model_lowpass(sine: f32, cosine: f32) -> [f32; 5] {
        model_lowpass_with_half(sine, cosine, 0.5)
    }

    fn model_lowpass_with_half(sine: f32, cosine: f32, half: f64) -> [f32; 5] {
        let (s, c) = (sine as f64, cosine as f64);
        let alpha = fp::mul_single(s, half);
        let numerator = fp::sub_single(1.0, c);
        let a0 = fp::add_single(alpha, 1.0);
        let inv = fp::div_single(1.0, a0);
        let two_a0 = fp::mul_single(a0, 2.0);
        [
            fp::mul_single(fp::mul_single(c, -2.0), inv) as f32,
            fp::mul_single(fp::sub_single(1.0, alpha), inv) as f32,
            fp::div_single(numerator, two_a0) as f32,
            fp::mul_single(numerator, inv) as f32,
            fp::div_single(numerator, two_a0) as f32,
        ]
    }

    #[test]
    fn the_coefficients_are_the_rbj_low_pass_at_unit_q() {
        let mut g = guest();
        let mut trig = Scripted {
            sine: 0.5,
            cosine: 0.75,
            ..Default::default()
        };
        build_lowpass_coefficients(&mut g, &mut trig, COEFFS, 0.25).unwrap();

        assert_eq!(coefficients_at(&g, COEFFS), model_lowpass(0.5, 0.75));
        // Both helpers were asked, both with the angle, sine first.
        assert_eq!(trig.asked, vec![('s', 0.25), ('c', 0.25)]);
        // b0 and b2 are the same expression; a1 and b1 are not, so the slots are not interchangeable.
        let c = coefficients_at(&g, COEFFS);
        assert_eq!(c[2], c[4], "b0 and b2 are the same quotient");
        assert_ne!(c[0], c[3]);
    }

    #[test]
    fn the_four_constants_are_read_live_from_the_map() {
        // Not folded in: patch `half` and alpha moves, which moves a0 and therefore four of the five.
        let mut g = guest();
        let mut trig = Scripted {
            sine: 1.0,
            cosine: 0.0,
            ..Default::default()
        };
        build_lowpass_coefficients(&mut g, &mut trig, COEFFS, 0.0).unwrap();
        let before = coefficients_at(&g, COEFFS);

        let mut patched = guest();
        patched.set_u32(HALF_SINGLE, 0.25f32.to_bits()).unwrap();
        let mut t2 = Scripted {
            sine: 1.0,
            cosine: 0.0,
            ..Default::default()
        };
        build_lowpass_coefficients(&mut patched, &mut t2, COEFFS, 0.0).unwrap();
        let after = coefficients_at(&patched, COEFFS);

        assert_ne!(before, after, "the 0.5 cell is loaded, not a literal");
        assert_eq!(before, model_lowpass_with_half(1.0, 0.0, 0.5));
        assert_eq!(
            after,
            model_lowpass_with_half(1.0, 0.0, 0.25),
            "alpha followed the cell"
        );
    }

    #[test]
    fn each_constant_cell_is_read_on_every_call() {
        // One four-byte segment per cell, so dropping any one of the four is an `Err` rather than a
        // wrong number. That is the whole reason the test guest is built that way.
        for missing in [HALF_SINGLE, ONE_SINGLE, MINUS_TWO_SINGLE, TWO_SINGLE] {
            let mut g = guest();
            g.put(missing, Vec::new());
            let mut trig = Scripted {
                sine: 0.5,
                cosine: 0.5,
                ..Default::default()
            };
            assert!(
                build_lowpass_coefficients(&mut g, &mut trig, COEFFS, 0.0).is_err(),
                "cell {missing:#x} is not read"
            );
        }
    }

    #[test]
    fn an_unported_sine_refuses_and_writes_nothing() {
        let mut g = guest();
        for k in 0..5u32 {
            g.set_u32(COEFFS + 4 * k, 0x7F7F_7F7F).unwrap();
        }
        let e = build_lowpass_coefficients(&mut g, &mut Unported, COEFFS, 0.5).unwrap_err();
        assert_eq!(e.address, 0x82F4_DED0, "the sine, by guest address");
        for k in 0..5u32 {
            assert_eq!(
                g.u32(COEFFS + 4 * k).unwrap(),
                0x7F7F_7F7F,
                "word {k} was written"
            );
        }
    }

    #[test]
    fn the_cosines_double_tail_is_discarded_by_the_frsp() {
        // `frsp f11,f1` — a `Trig` that answers with more than 24 mantissa bits must give the same
        // coefficients as its own f32 rounding.
        //
        // The input is chosen so that the tail *survives* the next operation, which is the part a
        // weaker version of this test got wrong. `1 - 2^-25` is exactly half an f32 ulp below 1.0 and
        // rounds to 1.0; `fsubs(1.0, that)` is then `0.0` through the `frsp` and `2^-25` without it.
        // A value whose tail is lost in the following rounding anyway would not distinguish the two.
        //
        // **The sine's `frsp f30,f1` is invisible and this test does not claim otherwise.** Its only
        // consumer is `alpha = fmuls(sine, 0.5)`, and scaling by a power of two commutes with rounding
        // to single — so `(x·0.5)→f32` and `(x→f32)·0.5` agree for every non-denormal `x`. Measured
        // rather than argued: removing that one `frsp` leaves the whole suite green. It is reproduced
        // because the original has it, and it joins the module's "no test can catch this" list. The
        // same pair of facts holds for [`highpass_stage`]'s own two `frsp`s.
        let long = 1.0_f64 - f64::from_bits(0x3E60_0000_0000_0000); // 1 - 2^-25
        assert_ne!(
            long, long as f32 as f64,
            "the input has to distinguish the two"
        );
        assert_eq!(
            long as f32, 1.0,
            "…and it has to round to something the next op treats apart"
        );

        let mut g = guest();
        let mut trig = Scripted {
            sine: long,
            cosine: long,
            ..Default::default()
        };
        build_lowpass_coefficients(&mut g, &mut trig, COEFFS, 0.0).unwrap();

        let mut h = guest();
        let rounded = long as f32 as f64;
        let mut t2 = Scripted {
            sine: rounded,
            cosine: rounded,
            ..Default::default()
        };
        build_lowpass_coefficients(&mut h, &mut t2, COEFFS, 0.0).unwrap();

        assert_eq!(coefficients_at(&g, COEFFS), coefficients_at(&h, COEFFS));
        assert_eq!(
            coefficients_at(&g, COEFFS)[2],
            0.0,
            "b0 came out of `1 - 1.0`, not `1 - long`"
        );
    }

    // ------------------------------------------------------------------ the two stages

    #[test]
    fn the_low_pass_filters_and_swaps_when_the_cutoff_is_under_the_ceiling() {
        let mut g = guest();
        cutoff(&mut g, 0.5);
        cached(&mut g, 0.5); // equal, so no trigonometry is reached at all
        passthrough_coefficients(&mut g);
        channels(&mut g, 2);
        input(&mut g, 3); // three channels of input and poison, two of which are in the count

        // `Unported` is deliberate: the majority path needs no sine, and a test that supplied one
        // would not establish that.
        assert_eq!(
            lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap(),
            1
        );

        // b0 = 1 and everything else 0, so each destination channel is its source channel.
        for c in 0..2u32 {
            for i in [0u32, 1, 255] {
                let want = c as f32 * 1000.0 + i as f32;
                assert_eq!(
                    g.f32(BUF_B + 1024 * c + 4 * i).unwrap(),
                    want,
                    "channel {c} frame {i}"
                );
            }
        }
        // Channel 2 is past the count: its poison has to survive, and its source is non-zero so a
        // stray pass would be visible rather than writing the same zero.
        assert_eq!(
            g.u32(BUF_B + 2048).unwrap(),
            0x7F7F_7F7F,
            "a third channel was filtered"
        );
        // And the descriptor pair is swapped.
        assert_eq!(g.u32(STREAM + STREAM_BUFFER_A).unwrap(), DESC_B);
        assert_eq!(g.u32(STREAM + STREAM_BUFFER_B).unwrap(), DESC_A);
    }

    #[test]
    fn the_low_pass_bypasses_at_the_ceiling_and_leaves_the_pair_alone() {
        // `!(cutoff < ceiling)`, so exactly 1.0 against a 1.0 ceiling bypasses: the test is strict.
        let mut g = guest();
        cutoff(&mut g, 1.0);
        cached(&mut g, 0.5); // below the ceiling, so the history is cleared
        channels(&mut g, 3);
        input(&mut g, 3);
        poison_state(&mut g, 4); // four poisoned slots, three in the count

        assert_eq!(
            lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap(),
            1
        );

        assert_eq!(
            g.u32(BUF_B).unwrap(),
            0x7F7F_7F7F,
            "the output block was written"
        );
        assert_eq!(
            g.u32(STREAM + STREAM_BUFFER_A).unwrap(),
            DESC_A,
            "the pair was swapped"
        );
        assert_eq!(g.u32(STREAM + STREAM_BUFFER_B).unwrap(), DESC_B);
        for c in 0..3u32 {
            assert_eq!(
                state_words(&g, c),
                vec![0u32; 4],
                "channel {c}'s history survived"
            );
        }
        assert_eq!(
            state_words(&g, 3),
            vec![0xDEAD_0000, 0xDEAD_0001, 0xDEAD_0002, 0xDEAD_0003],
            "a fourth slot, past the count, was cleared"
        );
        assert_eq!(
            g.f32(OBJECT + CACHED_CUTOFF).unwrap(),
            1.0,
            "the cache is published either way"
        );
    }

    #[test]
    fn the_bypass_clears_the_history_only_when_the_cache_was_itself_below_the_ceiling() {
        // `bge cr6,0x82b27eb0` — a cached cutoff at or above the ceiling means those words hold no
        // filter state, so they are left as they are. This is the branch a port most easily drops.
        let mut g = guest();
        cutoff(&mut g, 2.0);
        cached(&mut g, 1.0); // at the ceiling, not below it
        channels(&mut g, 1);
        poison_state(&mut g, 1);

        lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();

        assert_eq!(
            state_words(&g, 0),
            vec![0xDEAD_0000, 0xDEAD_0001, 0xDEAD_0002, 0xDEAD_0003]
        );
        assert_eq!(g.f32(OBJECT + CACHED_CUTOFF).unwrap(), 2.0);
    }

    #[test]
    fn the_history_clear_walks_four_words_per_channel_from_plus_fifty_six() {
        // The cursor starts at `+52` and the four stores are `+4`, `+8`, `+12` and then the advanced
        // cursor — so channel i's words are at `+56 + 16i` and the cutoff word at `+52` is **not**
        // one of them. A port that started the cursor at `+56` would clear `+60`…`+72` instead.
        let mut g = guest();
        cutoff(&mut g, 2.0);
        cached(&mut g, 0.5);
        channels(&mut g, 2);
        poison_state(&mut g, 3);

        lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();

        assert_eq!(state_words(&g, 0), vec![0u32; 4]);
        assert_eq!(state_words(&g, 1), vec![0u32; 4]);
        assert_eq!(
            state_words(&g, 2),
            vec![0xDEAD_0000, 0xDEAD_0001, 0xDEAD_0002, 0xDEAD_0003],
            "a third channel is past the count"
        );
        // `+52` is the cursor's *base*, and the first store is at `+4` off it — so the cutoff input
        // word survives the clear. A port that started the cursor at `+56` would clear `+60`…`+72`
        // instead, leaving channel 0's first word above untouched and the third channel's first word
        // cleared.
        assert_eq!(
            g.u32(OBJECT + CUTOFF_INPUT).unwrap(),
            2.0f32.to_bits(),
            "the input word"
        );
        assert_eq!(g.f32(OBJECT + CACHED_CUTOFF).unwrap(), 2.0);
    }

    #[test]
    fn the_low_pass_clamps_up_to_the_floor_and_the_clamp_is_strict() {
        // Below the floor the coefficients are built from the floor, not from the cutoff — and the
        // cache is published as the clamped value, which is how the next call knows not to rebuild.
        let mut g = guest();
        cutoff(&mut g, 0.001); // under a 0.01 floor
        cached(&mut g, 0.0);
        channels(&mut g, 0);
        let mut trig = Scripted {
            sine: 0.25,
            cosine: 0.5,
            ..Default::default()
        };

        lowpass_stage(&mut g, &mut trig, OBJECT as u64, STREAM as u64).unwrap();

        // The angle is the *widened* floor cell, not the decimal 0.01: `lfs` loads a single.
        let floor = 0.01f32 as f64;
        assert_eq!(
            trig.asked,
            vec![('s', floor), ('c', floor)],
            "the floor, not 0.001"
        );
        assert_eq!(g.f32(OBJECT + CACHED_CUTOFF).unwrap(), 0.01);
        assert_eq!(
            coefficients_at(&g, OBJECT + COEFFICIENTS),
            model_lowpass(0.25, 0.5)
        );

        // Exactly on the floor is not clamped, which is the same value here — so the test that the
        // compare is strict has to come from *above* the floor instead.
        let mut h = guest();
        cutoff(&mut h, 0.02);
        cached(&mut h, 0.0);
        channels(&mut h, 0);
        let mut t2 = Scripted {
            sine: 0.25,
            cosine: 0.5,
            ..Default::default()
        };
        lowpass_stage(&mut h, &mut t2, OBJECT as u64, STREAM as u64).unwrap();
        let kept = 0.02f32 as f64;
        assert_eq!(
            t2.asked,
            vec![('s', kept), ('c', kept)],
            "a cutoff above the floor is kept"
        );
    }

    #[test]
    fn an_unchanged_cutoff_skips_the_rebuild_entirely() {
        // `beq cr6,0x82b27ef0`. With `Unported` this is observable as the difference between a
        // completed call and an `Err`, which is the sharpest form the distinction can take.
        let mut g = guest();
        cutoff(&mut g, 0.5);
        cached(&mut g, 0.5);
        channels(&mut g, 0);
        assert!(lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).is_ok());

        let mut h = guest();
        cutoff(&mut h, 0.5);
        cached(&mut h, 0.5000001);
        channels(&mut h, 0);
        let e = lowpass_stage(&mut h, &mut Unported, OBJECT as u64, STREAM as u64).unwrap_err();
        assert_eq!(e.address, 0x82F4_DED0, "a moved cutoff reaches the sine");
    }

    #[test]
    fn a_nan_cutoff_bypasses_both_stages() {
        // Both compares are unordered, so `!(x < c)` and `!(x > f)` are both true for a NaN. The
        // whole point of writing them as negations rather than as `>=` and `<=`.
        for (name, run) in [("low", 0u8), ("high", 1u8)] {
            let mut g = guest();
            g.set_u32(FORMAT + FORMAT_SAMPLE_RATE, 1.0f32.to_bits())
                .unwrap();
            g.set_u32(OBJECT + CUTOFF_INPUT, f32::NAN.to_bits())
                .unwrap();
            cached(&mut g, 0.5);
            channels(&mut g, 1);
            input(&mut g, 1);

            if run == 0 {
                lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
            } else {
                highpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
            }
            assert_eq!(
                g.u32(BUF_B).unwrap(),
                0x7F7F_7F7F,
                "{name}: filtered a NaN cutoff"
            );
            assert_eq!(
                g.u32(STREAM + STREAM_BUFFER_A).unwrap(),
                DESC_A,
                "{name}: swapped"
            );
            assert!(
                g.f32(OBJECT + CACHED_CUTOFF).unwrap().is_nan(),
                "{name}: cache"
            );
        }
    }

    #[test]
    fn the_high_pass_bypasses_at_the_floor_where_the_low_pass_filters() {
        // The two stages disagree about the same cutoff, which is the clearest statement that they
        // are mirrors: 0.005 is under the 0.01 floor, so the high-pass bypasses and the low-pass
        // clamps up and filters.
        let mut high = guest();
        cutoff(&mut high, 0.005);
        cached(&mut high, 0.0);
        channels(&mut high, 1);
        input(&mut high, 1);
        highpass_stage(&mut high, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
        assert_eq!(
            high.u32(BUF_B).unwrap(),
            0x7F7F_7F7F,
            "the high-pass bypassed"
        );
        assert_eq!(high.u32(STREAM + STREAM_BUFFER_A).unwrap(), DESC_A);

        // And a cutoff *between* the two cells is what pins which of them the high-pass tests: 0.5
        // is above the floor and below the ceiling, so it filters. Reading the ceiling there instead
        // would bypass.
        let mut mid = guest();
        cutoff(&mut mid, 0.5);
        cached(&mut mid, 0.5); // equal, so no rebuild and no Trig
        passthrough_coefficients(&mut mid);
        channels(&mut mid, 1);
        input(&mut mid, 1);
        highpass_stage(&mut mid, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
        assert_eq!(
            mid.f32(BUF_B + 4).unwrap(),
            1.0,
            "a mid-band cutoff filters"
        );
        assert_eq!(mid.u32(STREAM + STREAM_BUFFER_A).unwrap(), DESC_B);

        let mut low = guest();
        cutoff(&mut low, 0.005);
        cached(&mut low, 0.01); // the *clamped* value, so no rebuild and no Trig
        passthrough_coefficients(&mut low);
        channels(&mut low, 1);
        input(&mut low, 1);
        lowpass_stage(&mut low, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
        assert_eq!(low.f32(BUF_B + 4).unwrap(), 1.0, "the low-pass filtered");
        assert_eq!(low.u32(STREAM + STREAM_BUFFER_A).unwrap(), DESC_B);
    }

    #[test]
    fn the_high_pass_clamps_down_to_the_ceiling() {
        let mut g = guest();
        cutoff(&mut g, 4.0); // above a 1.0 ceiling
        cached(&mut g, 0.0);
        channels(&mut g, 0);
        let mut trig = Scripted {
            sine: 0.25,
            cosine: 0.5,
            ..Default::default()
        };
        highpass_stage(&mut g, &mut trig, OBJECT as u64, STREAM as u64).unwrap();
        assert_eq!(
            trig.asked,
            vec![('s', 1.0), ('c', 1.0)],
            "the ceiling, not 4.0"
        );
        assert_eq!(g.f32(OBJECT + CACHED_CUTOFF).unwrap(), 1.0);
    }

    #[test]
    fn the_high_pass_coefficients_differ_from_the_low_pass_in_the_numerator_and_in_b1() {
        // One sine and cosine, two bodies, five slots. a1 and a2 must agree exactly; b0 and b2 must
        // use `1 + cos` where the low-pass uses `1 - cos`; and b1 must be the *negation* of
        // `num/a0`. Writing the expected values out by hand rather than through a second model is
        // what makes this a check on the port and not on a shared helper.
        let (s, c) = (0.25f32, 0.5f32);
        let mut g = guest();
        cutoff(&mut g, 0.5);
        cached(&mut g, 0.0);
        channels(&mut g, 0);
        let mut trig = Scripted {
            sine: s as f64,
            cosine: c as f64,
            ..Default::default()
        };
        highpass_stage(&mut g, &mut trig, OBJECT as u64, STREAM as u64).unwrap();
        let high = coefficients_at(&g, OBJECT + COEFFICIENTS);

        let mut h = guest();
        let mut t2 = Scripted {
            sine: s as f64,
            cosine: c as f64,
            ..Default::default()
        };
        build_lowpass_coefficients(&mut h, &mut t2, COEFFS, 0.5).unwrap();
        let low = coefficients_at(&h, COEFFS);

        assert_eq!(high[0], low[0], "a1 = -2cos/a0 in both");
        assert_eq!(high[1], low[1], "a2 = (1-alpha)/a0 in both");

        let alpha = fp::mul_single(s as f64, 0.5);
        let a0 = fp::add_single(alpha, 1.0);
        let inv = fp::div_single(1.0, a0);
        let two_a0 = fp::mul_single(a0, 2.0);
        let numerator = fp::add_single(c as f64, 1.0); // 1 + cos
        assert_eq!(
            high[2],
            fp::div_single(numerator, two_a0) as f32,
            "b0 = (1+cos)/(2 a0)"
        );
        assert_eq!(high[4], high[2], "b2 is the same quotient, computed twice");
        assert_eq!(
            high[3],
            fp::neg_double(fp::mul_single(numerator, inv)) as f32,
            "b1 is negated"
        );
        assert!(
            high[3] < 0.0 && low[3] > 0.0,
            "and the sign is the difference from the low-pass"
        );
    }

    #[test]
    fn the_channel_count_is_reloaded_at_the_bottom_of_the_filter_loop() {
        // `lbz r11,42(r30)` after every kernel call. The object is arranged so that channel 0's
        // *destination* block covers `+42`, and the byte written there is a zero — so a hoisted
        // count would filter all four channels and the reloaded one stops after the first.
        //
        // The destination buffer is the object itself here, which is exactly the aliasing layout the
        // C++ `Windows()` declines. So what this establishes is that the reload survived the
        // transcription, not that the guest agrees with the answer.
        let mut g = guest();
        cutoff(&mut g, 0.5);
        cached(&mut g, 0.5);
        passthrough_coefficients(&mut g);
        channels(&mut g, 4);
        input(&mut g, 4);
        // Point the destination at the object, so filtering channel 0 writes 1 KB from +0 — which
        // includes +42. Frame 10 of the ramp is 10.0f32 = 0x41200000, whose byte at +42 is zero.
        g.set_u32(DESC_B + DESC_BASE, OBJECT).unwrap();
        g.set_u32(OBJECT + 1024, 0x7F7F_7F7F).unwrap(); // channel 1's destination, if it ran

        lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();

        assert_eq!(
            g.u8(OBJECT + CHANNEL_COUNT).unwrap(),
            0,
            "the pass cleared its own count"
        );
        // Channel 1's destination sits past the 1 KB channel 0 covered, and its source is 1000.0.
        assert_eq!(
            g.u32(OBJECT + 1024).unwrap(),
            0x7F7F_7F7F,
            "the loop ran a second channel"
        );
    }

    #[test]
    fn a_zero_channel_count_still_swaps_the_pair_and_rebuilds() {
        let mut g = guest();
        cutoff(&mut g, 0.5);
        cached(&mut g, 0.0);
        channels(&mut g, 0);
        let mut trig = Scripted {
            sine: 0.25,
            cosine: 0.5,
            ..Default::default()
        };
        lowpass_stage(&mut g, &mut trig, OBJECT as u64, STREAM as u64).unwrap();
        assert_eq!(trig.asked.len(), 2, "the rebuild happened");
        assert_eq!(
            g.u32(STREAM + STREAM_BUFFER_A).unwrap(),
            DESC_B,
            "and the pair swapped"
        );
    }

    #[test]
    fn the_state_pointer_advances_sixteen_bytes_per_channel() {
        // Every channel must carry its own four history words. With a coefficient set whose output
        // depends on the history, two channels filtered from *identical* input have to produce
        // identical output — which they only do if each read its own slot rather than sharing one.
        let mut g = guest();
        cutoff(&mut g, 0.5);
        cached(&mut g, 0.5);
        channels(&mut g, 2);
        // b0 = 1, a1 = -1: y[k] = x[k] + y[k-1], a running sum that is extremely sensitive to the
        // carried y1.
        for k in 0..5u32 {
            g.set_u32(OBJECT + COEFFICIENTS + 4 * k, 0f32.to_bits())
                .unwrap();
        }
        g.set_u32(
            OBJECT + COEFFICIENTS + dsp::biquad::COEFF_B0,
            1.0f32.to_bits(),
        )
        .unwrap();
        g.set_u32(
            OBJECT + COEFFICIENTS + dsp::biquad::COEFF_A1,
            (-1.0f32).to_bits(),
        )
        .unwrap();
        for c in 0..2u32 {
            for i in 0..BLOCK_FRAMES {
                g.set_u32(BUF_A + 1024 * c + 4 * i, 1.0f32.to_bits())
                    .unwrap();
            }
        }
        // Channel 0's history is seeded; channel 1's is left at zero.
        g.set_u32(
            OBJECT + FILTER_STATE + dsp::biquad::HISTORY_Y1,
            100.0f32.to_bits(),
        )
        .unwrap();

        lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();

        assert_eq!(
            g.f32(BUF_B).unwrap(),
            101.0,
            "channel 0 saw its own seeded y1"
        );
        assert_eq!(
            g.f32(BUF_B + 1024).unwrap(),
            1.0,
            "channel 1 saw its own zero"
        );
        // And each slot carries its own y[255] back out.
        assert_ne!(state_words(&g, 0), state_words(&g, 1));
    }

    #[test]
    fn the_format_pointer_is_dereferenced_before_anything_is_written() {
        // `lwz r10,40(r4)` then `lfs f13,12(r10)`. A null or unmapped format block is an `Err` here
        // and the original would load through it; the C++ `Windows()` declines the call for exactly
        // that reason, so nothing is known about what the guest does either.
        let mut g = guest();
        g.set_u32(STREAM + STREAM_FORMAT, 0).unwrap();
        cached(&mut g, 9.0);
        channels(&mut g, 1);
        assert!(lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).is_err());
        assert!(highpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).is_err());
        assert_eq!(
            g.f32(OBJECT + CACHED_CUTOFF).unwrap(),
            9.0,
            "a refused call wrote the cache"
        );
    }

    #[test]
    fn the_cutoff_is_two_roundings_and_not_one_fma() {
        // `fdivs` then `fmuls`: the quotient is narrowed to a single before it is scaled. An input
        // where the two forms disagree pins it.
        let (n, r, s) = (1.0f32, 3.0f32, 7.0f32);
        let two = fp::mul_single(fp::div_single(n as f64, r as f64), s as f64);
        let one = fp::mul_single(n as f64 / r as f64, s as f64);
        assert_ne!(two, one, "the distinguishing input has to distinguish");

        let mut g = guest();
        g.set_u32(CUTOFF_SCALE, s.to_bits()).unwrap();
        g.set_u32(CUTOFF_CEILING, 1e9f32.to_bits()).unwrap();
        g.set_u32(FORMAT + FORMAT_SAMPLE_RATE, r.to_bits()).unwrap();
        g.set_u32(OBJECT + CUTOFF_INPUT, n.to_bits()).unwrap();
        cached(&mut g, 0.0);
        channels(&mut g, 0);
        let mut trig = Scripted {
            sine: 0.0,
            cosine: 0.0,
            ..Default::default()
        };
        lowpass_stage(&mut g, &mut trig, OBJECT as u64, STREAM as u64).unwrap();
        assert_eq!(trig.asked, vec![('s', two), ('c', two)]);
    }

    #[test]
    fn both_stages_restore_the_entry_flush_mode() {
        for high in [false, true] {
            let mut g = guest();
            cutoff(&mut g, 0.5);
            cached(&mut g, 0.5);
            passthrough_coefficients(&mut g);
            channels(&mut g, 1);
            input(&mut g, 1);
            let before = crate::vmx::get_mxcsr();
            if high {
                highpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
            } else {
                lowpass_stage(&mut g, &mut Unported, OBJECT as u64, STREAM as u64).unwrap();
            }
            assert_eq!(crate::vmx::get_mxcsr(), before);
        }
    }

    // ------------------------------------------------------------------------------ the shelf

    const SH_BASE: u32 = 0x4000_0000;
    const SH_OBJ: u32 = SH_BASE + 0x100;
    const SH_PAIR: u32 = SH_BASE + 0x400;
    const SH_FORMAT: u32 = SH_BASE + 0x440;
    const SH_IN_DESC: u32 = SH_BASE + 0x480;
    const SH_OUT_DESC: u32 = SH_BASE + 0x4C0;
    const SH_IN: u32 = SH_BASE + 0x1000;
    const SH_OUT: u32 = SH_BASE + 0x3000;
    const SH_COEFFS_SCRATCH: u32 = SH_BASE + 0x800;
    const SH_POISON: u32 = 0xDEAD_BEEF;

    fn shelf_trig() -> crate::mathlib::tests::Scripted {
        crate::mathlib::tests::Scripted {
            sine: 0.3,
            cosine: 0.95,
            ..Default::default()
        }
    }

    fn shelf_guest(channels: u8, nominal: f32, gain: f32) -> Guest {
        let mut g = Guest::single(SH_BASE, 0x6000);
        for (addr, v) in [
            (CUTOFF_SCALE, 1.0f32),
            (CUTOFF_CEILING, 0.45),
            (CUTOFF_FLOOR, 0.001),
            (SHELF_ONE, 1.0),
            (SHELF_ALPHA_SCALE, 0.707_106_53),
            (SHELF_TWO, 2.0),
            (SHELF_MINUS_TWO, -2.0),
            (crate::leaves::ZERO_CELL, 0.0),
            (dsp::biquad::DENORM_BIAS, 0.0),
        ] {
            g.put(addr, v.to_bits().to_be_bytes().to_vec());
        }
        g.set_u8(SH_OBJ + CHANNEL_COUNT, channels).unwrap();
        g.set_u32(SH_OBJ + CUTOFF_INPUT, nominal.to_bits()).unwrap();
        g.set_u32(SH_OBJ + SHELF_GAIN, gain.to_bits()).unwrap();
        g.set_u32(SH_OBJ + SHELF_CACHED_CORNER, (-1.0f32).to_bits())
            .unwrap();
        g.set_u32(SH_OBJ + SHELF_CACHED_GAIN, (-1.0f32).to_bits())
            .unwrap();
        for w in 0..(16 * 8 / 4) {
            g.set_u32(SH_OBJ + SHELF_HISTORY + 4 * w, SH_POISON)
                .unwrap();
        }
        g.set_u32(SH_PAIR + STREAM_FORMAT, SH_FORMAT).unwrap();
        g.set_u32(SH_FORMAT + FORMAT_SAMPLE_RATE, 1.0f32.to_bits())
            .unwrap();
        g.set_u32(SH_PAIR + STREAM_BUFFER_A, SH_IN_DESC).unwrap();
        g.set_u32(SH_PAIR + STREAM_BUFFER_B, SH_OUT_DESC).unwrap();
        for (desc, buf) in [(SH_IN_DESC, SH_IN), (SH_OUT_DESC, SH_OUT)] {
            g.set_u32(desc + DESC_BASE, buf).unwrap();
            g.set_u16(desc + DESC_STRIDE, 256).unwrap();
        }
        for c in 0..u32::from(channels) {
            for i in 0..BLOCK_FRAMES {
                let v = (c as f32 + 1.0) * 0.1 * ((i % 7) as f32 - 3.0);
                g.set_u32(SH_IN + 1024 * c + 4 * i, v.to_bits()).unwrap();
            }
        }
        g
    }

    fn coeff(g: &Guest, at: u32, k: u32) -> f32 {
        g.f32(at + k).unwrap()
    }

    #[test]
    fn a_unity_gain_shelf_is_flat() {
        // A = 1 makes the numerator equal the denominator: b0 = 1, b1 = a1, b2 = a2. That is a
        // property of the RBJ shelf, not of this code, so it checks the arithmetic independently.
        let mut g = shelf_guest(1, 0.1, 1.0);
        let mut trig = shelf_trig();
        build_shelf_coefficients(&mut g, &mut trig, SH_COEFFS_SCRATCH, 0.5, 1.0).unwrap();
        let c = |k| coeff(&g, SH_COEFFS_SCRATCH, k);
        assert!(
            (c(dsp::biquad::COEFF_B0) - 1.0).abs() < 1e-6,
            "b0 = {}",
            c(dsp::biquad::COEFF_B0)
        );
        assert!((c(dsp::biquad::COEFF_B1) - c(dsp::biquad::COEFF_A1)).abs() < 1e-6);
        assert!((c(dsp::biquad::COEFF_B2) - c(dsp::biquad::COEFF_A2)).abs() < 1e-6);
        assert_eq!(
            trig.asked,
            vec![('s', 0.5), ('c', 0.5)],
            "sine then cosine, of one angle"
        );
    }

    #[test]
    fn the_shelf_coefficients_match_the_rbj_formula() {
        // The textbook high shelf at slope 1, in f64, with the same sine, cosine and alpha scale.
        let gain = 4.0f64;
        let mut g = shelf_guest(1, 0.1, gain as f32);
        build_shelf_coefficients(&mut g, &mut shelf_trig(), SH_COEFFS_SCRATCH, 0.5, gain).unwrap();
        let (s, c) = (0.3f64, 0.95f64);
        let a = gain.sqrt();
        let alpha = s * f64::from(0.707_106_53f32);
        let sa = a.sqrt();
        let a0 = (a + 1.0) - (a - 1.0) * c + 2.0 * sa * alpha;
        let want = [
            2.0 * ((a - 1.0) - (a + 1.0) * c) / a0,
            ((a + 1.0) - (a - 1.0) * c - 2.0 * sa * alpha) / a0,
            a * ((a + 1.0) + (a - 1.0) * c + 2.0 * sa * alpha) / a0,
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c) / a0,
            a * ((a + 1.0) + (a - 1.0) * c - 2.0 * sa * alpha) / a0,
        ];
        let keys = [
            dsp::biquad::COEFF_A1,
            dsp::biquad::COEFF_A2,
            dsp::biquad::COEFF_B0,
            dsp::biquad::COEFF_B1,
            dsp::biquad::COEFF_B2,
        ];
        for (k, w) in keys.iter().zip(want) {
            let got = f64::from(coeff(&g, SH_COEFFS_SCRATCH, *k));
            assert!(
                (got - w).abs() <= 1e-5 * w.abs().max(1.0),
                "coefficient +{k}: {got} vs {w}"
            );
        }
    }

    #[test]
    fn at_or_past_the_ceiling_the_shelf_clears_once_and_bypasses() {
        // corner = 0.5 / 1.0 = 0.5, past the 0.45 ceiling. Engaged is exactly 1, so the histories are
        // cleared and the flag drops; the pair is not swapped and the trigonometry is never asked.
        let mut g = shelf_guest(2, 0.5, 4.0);
        g.set_u32(SH_OBJ + SHELF_ENGAGED, 1).unwrap();
        let mut trig = shelf_trig();
        assert_eq!(shelf_stage(&mut g, &mut trig, SH_OBJ, SH_PAIR).unwrap(), 1);
        for w in 0..8u32 {
            assert_eq!(
                g.u32(SH_OBJ + SHELF_HISTORY + 4 * w).unwrap(),
                0,
                "history word {w}"
            );
        }
        assert_eq!(
            g.u32(SH_OBJ + SHELF_HISTORY + 32).unwrap(),
            SH_POISON,
            "two channels only"
        );
        assert_eq!(g.u32(SH_OBJ + SHELF_ENGAGED).unwrap(), 0);
        assert_eq!(
            g.f32(SH_OBJ + SHELF_CACHED_CORNER).unwrap(),
            0.5,
            "the cache is published"
        );
        assert_eq!(g.f32(SH_OBJ + SHELF_CACHED_GAIN).unwrap(), 4.0);
        assert_eq!(
            g.u32(SH_PAIR + STREAM_BUFFER_A).unwrap(),
            SH_IN_DESC,
            "no swap"
        );
        assert!(trig.asked.is_empty());

        // An engaged flag of 2 is not exactly 1: nothing is cleared and the flag stays.
        let mut g = shelf_guest(2, 0.5, 4.0);
        g.set_u32(SH_OBJ + SHELF_ENGAGED, 2).unwrap();
        shelf_stage(&mut g, &mut shelf_trig(), SH_OBJ, SH_PAIR).unwrap();
        assert_eq!(g.u32(SH_OBJ + SHELF_HISTORY).unwrap(), SH_POISON);
        assert_eq!(g.u32(SH_OBJ + SHELF_ENGAGED).unwrap(), 2);
    }

    #[test]
    fn a_unity_gain_bypasses_even_below_the_ceiling() {
        let mut g = shelf_guest(1, 0.1, 1.0);
        let mut trig = shelf_trig();
        shelf_stage(&mut g, &mut trig, SH_OBJ, SH_PAIR).unwrap();
        assert!(trig.asked.is_empty(), "no coefficients built");
        assert_eq!(
            g.u32(SH_OUT).unwrap(),
            0,
            "nothing filtered into the (zeroed) output"
        );
        assert_eq!(
            g.u32(SH_PAIR + STREAM_BUFFER_A).unwrap(),
            SH_IN_DESC,
            "no swap"
        );
    }

    #[test]
    fn the_filter_path_rebuilds_filters_every_channel_and_swaps() {
        let initial = shelf_guest(2, 0.1, 4.0);
        let mut g = initial.clone();
        for w in 0..8u32 {
            g.set_u32(SH_OBJ + SHELF_HISTORY + 4 * w, 0).unwrap();
        }
        let before = g.clone();
        let mut trig = shelf_trig();
        assert_eq!(shelf_stage(&mut g, &mut trig, SH_OBJ, SH_PAIR).unwrap(), 1);

        assert_eq!(
            trig.asked.len(),
            2,
            "the pair (corner, gain) moved, so the coefficients were rebuilt"
        );
        assert_eq!(g.u32(SH_OBJ + SHELF_ENGAGED).unwrap(), 1);
        assert_eq!(g.f32(SH_OBJ + SHELF_CACHED_GAIN).unwrap(), 4.0);
        assert_eq!(
            g.u32(SH_PAIR + STREAM_BUFFER_A).unwrap(),
            SH_OUT_DESC,
            "swapped"
        );
        assert_eq!(g.u32(SH_PAIR + STREAM_BUFFER_B).unwrap(), SH_IN_DESC);

        // Every channel equals the biquad run directly, with the coefficients the stage built and
        // the history block the channel owns. That pins the channel addressing and the offsets.
        let mut h = before;
        for k in 0..5u32 {
            let bits = g.u32(SH_OBJ + SHELF_COEFFICIENTS + 4 * k).unwrap();
            h.set_u32(SH_OBJ + SHELF_COEFFICIENTS + 4 * k, bits)
                .unwrap();
        }
        for ch in 0..2u32 {
            dsp::biquad::biquad(
                &mut h,
                SH_OBJ + SHELF_HISTORY + 16 * ch,
                SH_OUT + 1024 * ch,
                SH_IN + 1024 * ch,
                SH_OBJ + SHELF_COEFFICIENTS,
                BLOCK_FRAMES,
            )
            .unwrap();
        }
        for ch in 0..2u32 {
            for i in [0u32, 1, 100, 255] {
                let at = SH_OUT + 1024 * ch + 4 * i;
                assert_eq!(
                    g.u32(at).unwrap(),
                    h.u32(at).unwrap(),
                    "channel {ch}, sample {i}"
                );
            }
        }
    }

    #[test]
    fn an_unchanged_corner_and_gain_reuse_the_coefficients() {
        let mut g = shelf_guest(1, 0.1, 4.0);
        g.set_u32(SH_OBJ + SHELF_CACHED_CORNER, 0.1f32.to_bits())
            .unwrap();
        g.set_u32(SH_OBJ + SHELF_CACHED_GAIN, 4.0f32.to_bits())
            .unwrap();
        for k in 0..5u32 {
            g.set_u32(SH_OBJ + SHELF_COEFFICIENTS + 4 * k, 0.0f32.to_bits())
                .unwrap();
        }
        let mut trig = shelf_trig();
        shelf_stage(&mut g, &mut trig, SH_OBJ, SH_PAIR).unwrap();
        assert!(trig.asked.is_empty(), "no rebuild");
        assert_eq!(
            g.u32(SH_OBJ + SHELF_COEFFICIENTS + 8).unwrap(),
            0,
            "coefficients untouched"
        );
    }

    #[test]
    fn the_shelf_shares_the_low_pass_pool_cells() {
        assert_eq!(SHELF_ONE, ONE_SINGLE);
        assert_eq!(SHELF_TWO, TWO_SINGLE);
        assert_eq!(SHELF_MINUS_TWO, MINUS_TWO_SINGLE);
        assert_eq!(
            (CUTOFF_CEILING, CUTOFF_FLOOR, CUTOFF_SCALE),
            (0x822F_8E80, 0x822F_8E7C, 0x820B_411C)
        );
    }

    // ------------------------------------------------------------------ the peaking equaliser

    fn peak_guest(channels: u8, nominal: f32, gain: f32, quality: f32) -> Guest {
        let mut g = shelf_guest(channels, nominal, gain);
        for (addr, v) in [(QUALITY_FLOOR, 0.5f32), (QUALITY_CEILING, 20.0)] {
            g.put(addr, v.to_bits().to_be_bytes().to_vec());
        }
        g.set_u32(SH_OBJ + PEAK_QUALITY, quality.to_bits()).unwrap();
        for k in [PEAK_CACHED_WARP, PEAK_CACHED_GAIN, PEAK_CACHED_QUALITY] {
            g.set_u32(SH_OBJ + k, (-1.0f32).to_bits()).unwrap();
        }
        for w in 0..(16 * 8 / 4) {
            g.set_u32(SH_OBJ + PEAK_HISTORY + 4 * w, 0).unwrap();
        }
        g
    }

    fn rbj_peak(sine: f64, cosine: f64, gain: f64, q: f64) -> [f64; 5] {
        let a = gain.sqrt();
        let alpha = sine / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        [
            -2.0 * cosine / a0,
            (1.0 - alpha / a) / a0,
            (1.0 + alpha * a) / a0,
            -2.0 * cosine / a0,
            (1.0 - alpha * a) / a0,
        ]
    }

    fn peak_coefficients(g: &Guest) -> [f64; 5] {
        let mut out = [0.0; 5];
        for (k, v) in out.iter_mut().enumerate() {
            *v = f64::from(g.f32(SH_OBJ + PEAK_COEFFICIENTS + 4 * k as u32).unwrap());
        }
        out
    }

    #[test]
    fn the_peak_coefficients_match_the_rbj_formula_and_b1_equals_a1() {
        let mut g = peak_guest(1, 0.1, 2.0, 1.5);
        let mut trig = shelf_trig();
        assert_eq!(
            peaking_stage(&mut g, &mut trig, u64::from(SH_OBJ), SH_PAIR).unwrap(),
            1
        );
        let got = peak_coefficients(&g);
        for (k, (x, w)) in got.iter().zip(rbj_peak(0.3, 0.95, 2.0, 1.5)).enumerate() {
            assert!(
                (x - w).abs() <= 1e-5 * w.abs().max(1.0),
                "coefficient {k}: {x} vs {w}"
            );
        }
        assert_eq!(
            got[0].to_bits(),
            got[3].to_bits(),
            "b1 and a1 are the same product"
        );
        assert_eq!(trig.asked, vec![('s', 0.1f32 as f64), ('c', 0.1f32 as f64)]);
        assert_eq!(
            g.u32(SH_OBJ + PEAK_FILTERING).unwrap(),
            1,
            "the flag is raised"
        );
        assert_eq!(
            g.u32(SH_PAIR + STREAM_BUFFER_A).unwrap(),
            SH_OUT_DESC,
            "and the pair swapped"
        );
    }

    #[test]
    fn q_is_clamped_for_the_arithmetic_but_cached_as_stored() {
        // Q = 0.1 is below the 0.5 floor: the coefficients use 0.5, the cache keeps 0.1.
        let mut g = peak_guest(1, 0.1, 2.0, 0.1);
        peaking_stage(&mut g, &mut shelf_trig(), u64::from(SH_OBJ), SH_PAIR).unwrap();
        let got = peak_coefficients(&g);
        let want = rbj_peak(0.3, 0.95, 2.0, 0.5);
        assert!((got[2] - want[2]).abs() <= 1e-5, "b0 used the clamped Q");
        assert_eq!(
            g.f32(SH_OBJ + PEAK_CACHED_QUALITY).unwrap(),
            0.1,
            "the cache keeps the raw Q"
        );
    }

    #[test]
    fn an_angle_past_the_ceiling_is_clamped_not_bypassed() {
        // 0.9 / 1.0 is past the 0.45 ceiling. The shelf would bypass here; the peak clamps and filters.
        let mut g = peak_guest(1, 0.9, 2.0, 1.0);
        let mut trig = shelf_trig();
        peaking_stage(&mut g, &mut trig, u64::from(SH_OBJ), SH_PAIR).unwrap();
        assert_eq!(
            trig.asked[0],
            ('s', 0.45f32 as f64),
            "the trig saw the ceiling"
        );
        assert_eq!(g.f32(SH_OBJ + PEAK_CACHED_WARP).unwrap(), 0.45);
        assert_eq!(
            g.u32(SH_PAIR + STREAM_BUFFER_A).unwrap(),
            SH_OUT_DESC,
            "it filtered"
        );
    }

    #[test]
    fn a_unity_gain_bypasses_clears_once_and_does_not_swap() {
        let mut g = peak_guest(2, 0.1, 1.0, 1.0);
        g.set_u32(SH_OBJ + PEAK_FILTERING, 1).unwrap();
        for w in 0..8u32 {
            g.set_u32(SH_OBJ + PEAK_HISTORY + 4 * w, SH_POISON).unwrap();
        }
        let mut trig = shelf_trig();
        assert_eq!(
            peaking_stage(&mut g, &mut trig, u64::from(SH_OBJ), SH_PAIR).unwrap(),
            1
        );
        for w in 0..8u32 {
            assert_eq!(
                g.u32(SH_OBJ + PEAK_HISTORY + 4 * w).unwrap(),
                0,
                "history word {w}"
            );
        }
        assert_eq!(g.u32(SH_OBJ + PEAK_FILTERING).unwrap(), 0);
        assert_eq!(
            g.u32(SH_PAIR + STREAM_BUFFER_A).unwrap(),
            SH_IN_DESC,
            "no swap"
        );
        assert!(trig.asked.is_empty());
        assert_eq!(
            g.f32(SH_OBJ + PEAK_CACHED_GAIN).unwrap(),
            1.0,
            "the caches are published"
        );
    }

    #[test]
    fn the_peak_filters_every_channel_as_the_biquad_does() {
        let mut g = peak_guest(2, 0.1, 2.0, 1.0);
        let before = g.clone();
        peaking_stage(&mut g, &mut shelf_trig(), u64::from(SH_OBJ), SH_PAIR).unwrap();
        let mut h = before;
        for k in 0..5u32 {
            let bits = g.u32(SH_OBJ + PEAK_COEFFICIENTS + 4 * k).unwrap();
            h.set_u32(SH_OBJ + PEAK_COEFFICIENTS + 4 * k, bits).unwrap();
        }
        for ch in 0..2u32 {
            dsp::biquad::biquad(
                &mut h,
                SH_OBJ + PEAK_HISTORY + 16 * ch,
                SH_OUT + 1024 * ch,
                SH_IN + 1024 * ch,
                SH_OBJ + PEAK_COEFFICIENTS,
                BLOCK_FRAMES,
            )
            .unwrap();
        }
        for ch in 0..2u32 {
            for i in [0u32, 7, 128, 255] {
                let at = SH_OUT + 1024 * ch + 4 * i;
                assert_eq!(
                    g.u32(at).unwrap(),
                    h.u32(at).unwrap(),
                    "channel {ch}, sample {i}"
                );
            }
        }
    }

    #[test]
    fn the_biquad_coefficient_layout_is_a1_a2_b0_b1_b2() {
        // Both builders here store raw offsets 0..16 in this order; the kernel's names must agree.
        use dsp::biquad::{COEFF_A1, COEFF_A2, COEFF_B0, COEFF_B1, COEFF_B2};
        assert_eq!(
            [COEFF_A1, COEFF_A2, COEFF_B0, COEFF_B1, COEFF_B2],
            [0, 4, 8, 12, 16]
        );
    }
}
