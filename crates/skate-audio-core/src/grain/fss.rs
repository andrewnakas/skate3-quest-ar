//! The two module classes the board's grain chain needs that no voice graph uses:
//! `FrequencyShiftSsb` ("FSS0", descriptor `0x82FCE280`) and `HighShelfIir2` ("HS20", descriptor
//! `0x82FCE9CC`).
//!
//! | function | here |
//! |---|---|
//! | `sub_82B22738`, `FSS0`'s size | [`fss_size`] |
//! | `sub_82B22770`, `FSS0`'s constructor | [`construct_fss`] |
//! | `sub_82B22898`, `FSS0`'s process | [`process_fss`] |
//! | `sub_82473930`, the four-lane cosine it calls beside [`crate::dsp::sine`] | [`cosine4`] |
//! | `sub_82B27F78`, `HS20`'s size | [`HS_SIZE_BYTES`] |
//! | `sub_82B27F80`, `HS20`'s constructor | [`construct_hs`] |
//! | `sub_82B26740`, `HS20`'s process | already ported: [`crate::filters::shelf_stage`] (verified) |
//!
//! **`FrequencyShiftSsb`** is a single-sideband shifter. The input runs through two cascades of
//! two second-order allpass sections each (the crate's verified biquad `sub_82B43AF8`, over
//! coefficient blocks in the class's rodata at `0x82FCE2B0`, `0x82FCE2C4`, `0x82FCE2D8`,
//! `0x82FCE2EC`), a Hilbert pair `I` and `Q`; then per sample `out = I·cos φ − Q·sin φ` with the
//! phase `φ` advancing `2π·shift/rate` per sample from `+260`, and wrapped back into one turn at the
//! end of the block. The shift in Hz is property 0 (slot value `+52`). At 0 Hz the output is the
//! `I` cascade alone -- an allpass, not the input.
//!
//! Its constructor argument's single at `+4` selects an oversampled mode when it is exactly 1.0 (or
//! when there is no argument): a 256-byte-per-channel state and the extra filters `sub_82B42510` /
//! `sub_82B41D58`. The board's chain passes 0.0 (`sub_824C8878`), so only the plain mode is
//! reachable; the oversampled one is refused with an error naming its first unported callee.
//!
//! All float work is done under the flush modes the lifted body sets: the vector steps under
//! `enableFlushModeUnconditional`, the scalar phase arithmetic under `disableFlushMode`. Vector
//! multiply-adds go through [`crate::vmx`], which rounds them twice as the recomp does; scalar
//! `fmadds`/`fnmsubs` are single-rounded (`std::fma` in the lifted scalar code).

use core::arch::x86_64::*;

use crate::dsp::biquad::biquad;
use crate::dsp::sine::sine4_value;
use crate::fp::{
    add_single, div_single, fcfid, fctiwz_low_word, fmadd_single, frsp, load_single, mul_single,
    nmsub_single, store_single, sub_single,
};
use crate::modules::{BASE_VTABLE, ONE, ZERO, copy_class_rows};
use crate::vmx::{self, Fpscr};
use crate::{Error, Guest, Result};

/// Class descriptors.
pub const FSS_DESCRIPTOR: u32 = 0x82FC_E280;
pub const HS_DESCRIPTOR: u32 = 0x82FC_E9CC;
/// Size and constructor functions (class `+4`, `+8`).
pub const FSS_SIZE: u32 = 0x82B2_2738;
pub const FSS_CONSTRUCT: u32 = 0x82B2_2770;
pub const HS_SIZE: u32 = 0x82B2_7F78;
pub const HS_CONSTRUCT: u32 = 0x82B2_7F80;
/// Process functions (the class table's third word).
pub const FSS_PROCESS: u32 = 0x82B2_2898;
pub const HS_PROCESS: u32 = 0x82B2_6740;
/// `li r3,224`.
pub const HS_SIZE_BYTES: u64 = 224;

/// `lis -32245 ; lfs 16668`: 2π.
pub const TWO_PI: u32 = 0x820B_411C;
/// `lis -32250 ; lfs 3152` / `lfs 15112`, `lis -32219 ; lfs 29448`: 2, 3, 4.
pub const TWO: u32 = 0x8206_0C50;
pub const THREE: u32 = 0x8206_3B08;
pub const FOUR: u32 = 0x8225_7308;
/// The pool at `0x822F8600`: `+1096` 256.0, `+772` 1/(2π), `+1652` and `+2200` the constructor's
/// two singles (`+268` and the cost).
pub const BLOCK_FRAMES: u32 = 0x822F_8A48;
pub const INV_TWO_PI: u32 = 0x822F_8904;
pub const INITIAL_268: u32 = 0x822F_8C74;
pub const FSS_COST: u32 = 0x822F_8E98;
/// `lis -32208 ; addi -31232 ; lfs 2184`: `HS20`'s cost.
pub const HS_COST: u32 = 0x822F_8E88;
/// The four allpass coefficient blocks (`lis -32003` + −7504, −7484, −7464, −7444).
pub const ALLPASS: [u32; 4] = [0x82FC_E2B0, 0x82FC_E2C4, 0x82FC_E2D8, 0x82FC_E2EC];

/// `stwu r1,-240(r1)`.
pub const PROCESS_FRAME: u32 = 240;

const _: () = {
    const fn lis(hi: i32, lo: i32) -> u32 {
        (((hi & 0xFFFF) << 16) as u32).wrapping_add(lo as u32)
    }
    assert!(TWO_PI == lis(-32245, 16668) && TWO == lis(-32250, 3152));
    assert!(THREE == lis(-32250, 15112) && FOUR == lis(-32219, 29448));
    assert!(BLOCK_FRAMES == lis(-32208, -31232 + 1096) && INV_TWO_PI == lis(-32208, -31232 + 772));
    assert!(INITIAL_268 == lis(-32208, -31232 + 1652) && FSS_COST == lis(-32208, -31232 + 2200));
    assert!(HS_COST == lis(-32208, -31232 + 2184));
    assert!(ALLPASS[0] == lis(-32003, -7504) && ALLPASS[3] == lis(-32003, -7444));
};

/// `sub_82B22738`: 288 bytes when the argument's `+4` is not 1.0, else `296 + 256 × channels`.
pub fn fss_size(g: &Guest, descriptor: u32) -> Result<u64> {
    let arg = g.u32(descriptor)?;
    if arg != 0 {
        let value = load_single(g, arg + 4)?;
        if value != load_single(g, ONE)? {
            return Ok(288);
        }
    }
    let channels = u64::from(g.u8(descriptor + 8)?);
    Ok((channels << 8) + 296)
}

/// `sub_82B22770`: `FSS0`'s constructor.
pub fn construct_fss(g: &mut Guest, instance: u32, arg: u32) -> Result<bool> {
    let zero = load_single(g, ZERO)?;
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
        for k in 0..16u32 {
            store_single(g, instance + 56 + 4 * k, zero)?;
        }
    }
    g.set_u32(instance + 16, instance + 48)?;
    copy_class_rows(g, g.u32(instance + 20)?, instance + 48)?;
    let one = load_single(g, ONE)?;
    let mode = if arg != 0 {
        load_single(g, arg + 4)?
    } else {
        one
    };
    store_single(g, instance + 264, mode)?;
    store_single(g, instance + 260, zero)?;
    store_single(g, instance + 268, load_single(g, INITIAL_268)?)?;
    let cost = load_single(g, FSS_COST)?;
    if load_single(g, instance + 264)? == one {
        return Err(Error::new(
            0x82B4_1CF0,
            "FrequencyShiftSsb's oversampled mode (sub_82B41CF0) is not ported",
        ));
    }
    g.set_u16(instance + 272, 0)?;
    let previous = load_single(g, instance + 32)?;
    let player = g.u32(instance + 12)?;
    let delta = sub_single(cost, previous);
    store_single(g, instance + 28, zero)?;
    let total = load_single(g, player + 40)?;
    store_single(g, player + 40, add_single(delta, total))?;
    store_single(g, instance + 32, cost)?;
    Ok(true)
}

/// `sub_82B27F80`: `HS20`'s constructor.
pub fn construct_hs(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
        let zero = load_single(g, ZERO)?;
        for k in 0..32u32 {
            store_single(g, instance + 64 + 4 * k, zero)?;
        }
    }
    g.set_u32(instance + 16, instance + 48)?;
    copy_class_rows(g, g.u32(instance + 20)?, instance + 48)?;
    let player = g.u32(instance + 12)?;
    g.set_u32(instance + 192, 0)?;
    let cost = load_single(g, HS_COST)?;
    let previous = load_single(g, instance + 32)?;
    let delta = sub_single(cost, previous);
    store_single(g, instance + 216, load_single(g, instance + 52)?)?;
    store_single(g, instance + 220, load_single(g, instance + 60)?)?;
    let total = load_single(g, player + 40)?;
    store_single(g, player + 40, add_single(delta, total))?;
    store_single(g, instance + 32, cost)?;
    Ok(true)
}

/// `sub_82473930`: four-lane cosine. `x` is `v1` in host lane order (as [`crate::dsp::sine`]).
/// The range reduction is the sine's; the series is `1 + c1·t² + … + c11·t²²` over the rodata
/// vectors at `0x822F97F0`, `0x822F9800` and `0x822F9810`, every `vmaddfp` rounded twice.
pub fn cosine4(g: &Guest, x: [u32; 4]) -> Result<[u32; 4]> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    unsafe { cosine4_impl(g, x) }
}

const COS_A: u32 = 0x822F_9850; // lis -32208 ; addi -26544
const COS_B: u32 = 0x822F_97F0; // addi -26640
const COS_C: u32 = 0x822F_9800; // addi -26624
const COS_D: u32 = 0x822F_9810; // addi -26608

#[target_feature(enable = "sse4.1,fma")]
unsafe fn cosine4_impl(g: &Guest, x: [u32; 4]) -> Result<[u32; 4]> {
    unsafe {
        use crate::vmx::{SPLAT_W0, SPLAT_W1, SPLAT_W2, SPLAT_W3, vspltw128};
        let ps = |v: __m128i| _mm_castsi128_ps(v);
        // vupkd3d128 v61,v63(=0),4 -> host lanes {1.0, 0.0, 3.0, 3.0}; splat of host lane 0.
        let one = _mm_set1_ps(1.0);
        let a = vmx::lvx128(g, COS_A)?;
        let reduce_scale = ps(vspltw128::<{ SPLAT_W3 }>(a)); // v60
        let b = vmx::lvx128(g, COS_B)?;
        let reduce_step = ps(vspltw128::<{ SPLAT_W1 }>(a)); // v0
        let c1 = ps(vspltw128::<{ SPLAT_W1 }>(b)); // v6
        let c = vmx::lvx128(g, COS_C)?;
        let x = _mm_loadu_ps(x.as_ptr() as *const f32);
        let mut fpscr = Fpscr::capture();
        fpscr.enable_flush_mode_unconditional();
        let scaled = vmx::vmulfp(x, reduce_scale); // v59
        let c2 = ps(vspltw128::<{ SPLAT_W2 }>(b)); // v12
        let c3 = ps(vspltw128::<{ SPLAT_W3 }>(b)); // v11
        let d = vmx::lvx128(g, COS_D)?;
        let c4 = ps(vspltw128::<{ SPLAT_W0 }>(c)); // v30
        let c5 = ps(vspltw128::<{ SPLAT_W1 }>(c)); // v31
        let c6 = ps(vspltw128::<{ SPLAT_W2 }>(c)); // v2
        let c7 = ps(vspltw128::<{ SPLAT_W3 }>(c)); // v3
        let c8 = ps(vspltw128::<{ SPLAT_W0 }>(d)); // v5
        let c9 = ps(vspltw128::<{ SPLAT_W1 }>(d)); // v7
        let c10 = ps(vspltw128::<{ SPLAT_W2 }>(d)); // v9
        let c11 = ps(vspltw128::<{ SPLAT_W3 }>(d)); // v10
        let turns = vmx::vrfin(scaled); // v13
        let t = vmx::vnmsubfp(reduce_step, turns, x); // v8
        let t2 = vmx::vmulfp(t, t); // v13
        let t4 = vmx::vmulfp(t2, t2); // v0
        let mut sum = vmx::vmaddfp(c1, t2, one); // v8 = v6*v13 + v4
        let t6 = vmx::vmulfp(t4, t2); // v13
        sum = vmx::vmaddfp(c2, t4, sum); // v8 = v12*v0 + v8
        let t8 = vmx::vmulfp(t4, t4); // v12
        let t10 = vmx::vmulfp(t6, t4); // v0
        let mut acc = vmx::vmaddfp(c3, t6, sum); // v1 = v11*v13 + v8
        let t12 = vmx::vmulfp(t6, t6); // v11
        let t14 = vmx::vmulfp(t8, t6); // v4
        let t16 = vmx::vmulfp(t8, t8); // v6
        let t18 = vmx::vmulfp(t10, t8); // v8
        acc = vmx::vmaddfp(c4, t8, acc); // v1 = v30*v12 + v1
        let t22 = vmx::vmulfp(t12, t10); // v13
        let t20 = vmx::vmulfp(t10, t10); // v12
        acc = vmx::vmaddfp(c5, t10, acc); // v0 = v31*v0 + v1
        acc = vmx::vmaddfp(c6, t12, acc); // v2*v11
        acc = vmx::vmaddfp(c7, t14, acc); // v3*v4
        acc = vmx::vmaddfp(c8, t16, acc); // v5*v6
        acc = vmx::vmaddfp(c9, t18, acc); // v7*v8
        acc = vmx::vmaddfp(c10, t20, acc); // v9*v12
        acc = vmx::vmaddfp(c11, t22, acc); // v1 = v10*v13 + v0
        drop(fpscr);
        let mut out = [0u32; 4];
        _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, _mm_castps_si128(acc));
        Ok(out)
    }
}

fn lanes(v: __m128) -> [u32; 4] {
    let mut out = [0u32; 4];
    unsafe { _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, _mm_castps_si128(v)) };
    out
}

fn vector(x: [u32; 4]) -> __m128 {
    unsafe { _mm_loadu_ps(x.as_ptr() as *const f32) }
}

/// `sub_82B22898`: process one 256-frame block of `object` over the buffer-descriptor pair `pair`
/// (`r4`); `sp` is the caller's stack pointer. Returns 1.
pub fn process_fss(g: &mut Guest, object: u32, pair: u32, sp: u32) -> Result<u64> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let one = load_single(g, ONE)?;
    let two_pi = load_single(g, TWO_PI)?; // f31
    if load_single(g, object + 264)? == one {
        return Err(Error::new(
            0x82B4_2510,
            "FrequencyShiftSsb's oversampled mode (sub_82B42510 / sub_82B41D58) is not ported",
        ));
    }
    let system = g.u32(object + 8)?;
    let input_desc = g.u32(pair + 28)?;
    let output_desc = g.u32(pair + 32)?;
    let root = g.u32(system)?;
    let scratch = g.u32(root + 32)?; // r26
    let base = scratch.wrapping_sub(3072); // r27
    g.set_u32(root + 32, base)?;
    let in_phase = base + 1024; // r31
    let quadrature = in_phase + 1024; // r30
    let input = g.u32(input_desc + 4)?; // r24
    biquad(g, object + 56, base, input, ALLPASS[0], 256)?;
    biquad(g, object + 72, in_phase, base, ALLPASS[1], 256)?;
    biquad(g, object + 88, base, input, ALLPASS[2], 256)?;
    biquad(g, object + 104, quadrature, base, ALLPASS[3], 256)?;

    fpscr.disable_flush_mode_unconditional();
    let format = g.u32(pair + 40)?;
    let shift = load_single(g, object + 52)?;
    let rate = load_single(g, format + 12)?;
    let per_sample = div_single(shift, rate); // f9
    let two = load_single(g, TWO)?;
    let three = load_single(g, THREE)?;
    let four = load_single(g, FOUR)?;
    let step = mul_single(per_sample, two_pi); // f29
    let phase = load_single(g, object + 260)?; // f30
    let output = g.u32(output_desc + 4)?; // r25
    let frame = sp.wrapping_sub(PROCESS_FRAME);
    store_single(g, frame + 80, phase)?;
    store_single(g, frame + 88, fmadd_single(step, two, phase))?;
    store_single(g, frame + 92, fmadd_single(step, three, phase))?;
    store_single(g, frame + 84, add_single(phase, step))?;
    let mut phases = unsafe { vmx::lvx128_ps(g, frame + 80)? }; // v126
    let advance = mul_single(step, four); // f5
    for k in 0..4 {
        store_single(g, frame + 80 + 4 * k, advance)?;
    }
    let mut increment = None; // v127, loaded in the first group
    for k in 0..64u32 {
        let sine = vector(sine4_value(g, lanes(phases))?);
        fpscr.enable_flush_mode_unconditional();
        let q = unsafe { vmx::lvx128_ps(g, quadrature + 16 * k)? };
        let q_sin = unsafe { vmx::vmulfp(q, sine) }; // v125
        let cosine = vector(cosine4(g, lanes(phases))?);
        fpscr.enable_flush_mode_unconditional();
        let i = unsafe { vmx::lvx128_ps(g, in_phase + 16 * k)? };
        let i_cos = unsafe { vmx::vmulfp(i, cosine) };
        if increment.is_none() {
            increment = Some(unsafe { vmx::lvx128_ps(g, frame + 80)? });
        }
        if k < 63 {
            phases = unsafe { vmx::vaddfp(phases, increment.unwrap()) };
        }
        let out = unsafe { vmx::vsubfp(i_cos, q_sin) };
        unsafe { vmx::stvx128_ps(g, output + 16 * k, out)? };
    }

    fpscr.disable_flush_mode();
    let frames = load_single(g, BLOCK_FRAMES)?;
    let end = fmadd_single(step, frames, phase); // f4
    let inverse = load_single(g, INV_TWO_PI)?;
    let turns = fctiwz_low_word(mul_single(end, inverse)) as i32;
    let whole = frsp(fcfid(i64::from(turns)));
    store_single(g, object + 260, nmsub_single(whole, two_pi, end))?;
    g.set_u32(root + 32, scratch)?;
    let a = g.u32(pair + 32)?;
    let b = g.u32(pair + 28)?;
    g.set_u32(pair + 32, b)?;
    g.set_u32(pair + 28, a)?;
    store_single(g, object + 268, load_single(g, object + 52)?)?;
    drop(fpscr);
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COSINE_WORDS: [(u32, [u32; 4]); 4] = [
        (COS_A, [0x4049_0FDB, 0x40C9_0FDB, 0x3EA2_F983, 0x3E22_F983]),
        (COS_B, [0; 4]),
        (COS_C, [0; 4]),
        (COS_D, [0; 4]),
    ];

    /// The image's cosine and allpass rodata, read from the dumped image when present.
    fn image() -> Option<Guest> {
        let dir = std::env::var_os("SKATE_AUDIO_IMAGE")?;
        let mut g = Guest::default();
        for page in [0x822F_u32, 0x82FC, 0x820B, 0x8206, 0x8225, 0x8231, 0x8216] {
            let path = std::path::Path::new(&dir).join(format!("g_{page:04X}.bin"));
            g.put(page << 16, std::fs::read(path).ok()?);
        }
        Some(g)
    }

    #[test]
    fn cosine_against_f64() {
        let _ = COSINE_WORDS;
        let Some(g) = image() else {
            eprintln!("SKATE_AUDIO_IMAGE not set; skipping");
            return;
        };
        for x in [
            0.0f32, 0.1, 0.5, 1.0, -1.0, 1.5707964, 3.1415927, 6.0, 10.0, -20.0,
        ] {
            let got = f32::from_bits(cosine4(&g, [x.to_bits(); 4]).unwrap()[0]) as f64;
            let two_pi = f32::from_bits(0x40C9_0FDB) as f64;
            let turns = (x * f32::from_bits(0x3E22_F983)).round_ties_even() as f64;
            let want = (x as f64 - two_pi * turns).cos();
            assert!((got - want).abs() < 1e-6, "cos({x}) = {got}, want {want}");
        }
    }

    /// At 0 Hz the block is the in-phase allpass cascade: out = I·cos 0 − Q·sin 0 = I exactly (cos
    /// 0 is exactly 1.0 through the series, sin 0 exactly 0). A 1 kHz shift of a tone moves its
    /// energy: correlation with a shifted reference is high.
    #[test]
    fn zero_shift_is_the_in_phase_cascade() {
        let Some(mut g) = image() else {
            eprintln!("SKATE_AUDIO_IMAGE not set; skipping");
            return;
        };
        const MEM: u32 = 0x4000_0000;
        g.put(MEM, vec![0; 0x10000]);
        let object = MEM + 0x100;
        let pair = MEM + 0x400;
        let system = MEM + 0x500;
        let root = MEM + 0x600;
        let input_desc = MEM + 0x700;
        let output_desc = MEM + 0x740;
        let format = MEM + 0x780;
        let input = MEM + 0x1000;
        let output = MEM + 0x1400;
        let sp = MEM + 0xF000;
        g.set_u32(object + 8, system).unwrap();
        g.set_u32(system, root).unwrap();
        g.set_u32(root + 32, MEM + 0x8000).unwrap();
        g.set_u32(pair + 28, input_desc).unwrap();
        g.set_u32(pair + 32, output_desc).unwrap();
        g.set_u32(pair + 40, format).unwrap();
        g.set_u32(input_desc + 4, input).unwrap();
        g.set_u32(output_desc + 4, output).unwrap();
        store_single(&mut g, format + 12, 48_000.0).unwrap();
        store_single(&mut g, object + 264, 0.0).unwrap();
        for i in 0..256u32 {
            let s = ((i as f32) * 0.05).sin() * 0.5;
            g.set_u32(input + 4 * i, s.to_bits()).unwrap();
        }
        process_fss(&mut g, object, pair, sp).unwrap();
        // The descriptors swapped and the scratch pointer restored.
        assert_eq!(g.u32(pair + 28).unwrap(), output_desc);
        assert_eq!(g.u32(root + 32).unwrap(), MEM + 0x8000);
        // Reference: the two cascaded biquads over the same input.
        let mut r = g.clone();
        for k in 0..16 {
            r.set_u32(object + 56 + 4 * k, 0).unwrap();
        }
        let tmp = MEM + 0x2000;
        let i_out = MEM + 0x2400;
        biquad(&mut r, object + 56, tmp, input, ALLPASS[0], 256).unwrap();
        biquad(&mut r, object + 72, i_out, tmp, ALLPASS[1], 256).unwrap();
        for i in 0..256u32 {
            assert_eq!(
                g.u32(output + 4 * i).unwrap(),
                r.u32(i_out + 4 * i).unwrap(),
                "sample {i}"
            );
        }
        assert_eq!(g.f32(object + 260).unwrap(), 0.0);
    }

    /// A 2 kHz tone shifted by +500 Hz over several blocks comes out as a single sideband: the
    /// DFT at one of 1.5 / 2.5 kHz dominates the other by more than 30 dB, and the level is kept.
    #[test]
    fn a_shift_moves_a_tone_to_one_sideband() {
        let Some(mut g) = image() else {
            eprintln!("SKATE_AUDIO_IMAGE not set; skipping");
            return;
        };
        const MEM: u32 = 0x4000_0000;
        g.put(MEM, vec![0; 0x10000]);
        let (object, pair, system, root) = (MEM + 0x100, MEM + 0x400, MEM + 0x500, MEM + 0x600);
        let (desc_a, desc_b, format) = (MEM + 0x700, MEM + 0x740, MEM + 0x780);
        let (buf_a, buf_b) = (MEM + 0x1000, MEM + 0x1400);
        g.set_u32(object + 8, system).unwrap();
        g.set_u32(system, root).unwrap();
        g.set_u32(root + 32, MEM + 0x8000).unwrap();
        g.set_u32(pair + 40, format).unwrap();
        g.set_u32(desc_a + 4, buf_a).unwrap();
        g.set_u32(desc_b + 4, buf_b).unwrap();
        store_single(&mut g, format + 12, 48_000.0).unwrap();
        store_single(&mut g, object + 264, 0.0).unwrap();
        store_single(&mut g, object + 52, 500.0).unwrap();
        let mut out = Vec::new();
        for block in 0..16u32 {
            g.set_u32(pair + 28, desc_a).unwrap();
            g.set_u32(pair + 32, desc_b).unwrap();
            for i in 0..256u32 {
                let n = (block * 256 + i) as f64;
                let s = (2.0 * std::f64::consts::PI * 2000.0 * n / 48_000.0).cos() * 0.5;
                g.set_u32(buf_a + 4 * i, (s as f32).to_bits()).unwrap();
            }
            process_fss(&mut g, object, pair, MEM + 0xF000).unwrap();
            for i in 0..256u32 {
                out.push(f64::from(g.f32(buf_b + 4 * i).unwrap()));
            }
        }
        let tail = &out[1024..];
        let power = |f: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, x) in tail.iter().enumerate() {
                let w = 2.0 * std::f64::consts::PI * f * n as f64 / 48_000.0;
                re += x * w.cos();
                im += x * w.sin();
            }
            (re * re + im * im).sqrt() / tail.len() as f64
        };
        let (up, down) = (power(2500.0), power(1500.0));
        let ratio = 20.0 * (up.max(down) / up.min(down)).log10();
        assert!(ratio > 30.0, "sidebands 2.5 kHz {up}, 1.5 kHz {down}");
        assert!((up.max(down) - 0.25).abs() < 0.02, "level {}", up.max(down));
    }
}
