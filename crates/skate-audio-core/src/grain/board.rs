//! The board owner's side of the grain players (`SFXObj_SkateBoard`, `.11.cpp`): which grain file a
//! surface plays (`sub_824C8370`, called from the surface router `sub_824C5CA8`), the per-frame
//! record `{gain, pitch, 0, position}` it writes into each player (`sub_824C6BD8`), the slewed
//! intensity at owner `+1160`/`+1164` (`sub_824C8588`), and the per-player bus-chain values it
//! pushes (`sub_824C9058`).
//!
//! Everything here is a pure function of its inputs, with the lifted single-precision rounding at
//! every step. The MixMap outputs the owner reads through its vfuncs 52/56/60/64 are *inputs*
//! (the MixMap port supplies them): `vfunc60(1)`, `vfunc60(2)`, `vfunc56(3)` for the records;
//! `vfunc64(11)`, `vfunc64(12)`, `vfunc52(0)`, `vfunc60(13)`, `vfunc60(21)`, `vfunc60(22)` for the
//! bus chain. Vault values come from class `0x7AB23C11B6ADA2DE` ([`SurfaceTuning`]).
//!
//! Per truck `t` (0, 1) the owner keeps two players, A at `owner+1176+8t` and B at
//! `owner+1180+8t`. Both are bound to the **same** grain file; they differ in `GrainParams`
//! (vault array index 0 → A, 1 → B) and in their record.

use crate::fp::{
    add_single, div_single, fcfid, fmadd_single, frsp, fsel, mul_single, neg_double, sub_single,
};

/// One grain surface choice from `sub_824C8370`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrainChoice {
    /// The vault collection key (`rldimi r31,r9,32,0`).
    pub key: u64,
    /// The collection's grain filename (vault layout `+64`, text) — the `grains.big` member.
    pub member: &'static str,
    /// The owner's grain-data slots for players A and B (`owner + 40 + 4·slot`).
    pub slots: (u32, u32),
}

/// `sub_824C8370`'s default key, returned for surfaces 7 and 8 and anything outside 1..9.
pub const DEFAULT_KEY: u64 = 0xD7ED_BD36_2D7D_2152;

/// `sub_824C8370(owner, surface)`: the grain for a surface; `soft` is `sub_824B23C8 == 1`.
/// `None` for surfaces without grains (7, 8 and outside 1..=9, which keep [`DEFAULT_KEY`] and leave
/// the slots untouched). Surface 9 has no soft variant.
pub fn grain_for_surface(surface: u32, soft: bool) -> Option<GrainChoice> {
    let (key, member, slots) = match (surface, soft) {
        (1, true) => (0x943B_1CB0_5BA9_A6BA, "asphalt_rough_soft.grain", (14, 15)),
        (1, false) => (0x7EB8_015B_4C02_405E, "asphalt_rough_hard.grain", (0, 1)),
        (2, true) => (0xB29F_ADBB_BC2F_39C2, "concrete_rough_soft.grain", (16, 17)),
        (2, false) => (0x0372_1D0F_A99A_03C8, "concrete_rough_hard.grain", (2, 3)),
        (3, true) => (0xDC10_09D5_0E32_7F8F, "asphalt_smooth_soft.grain", (18, 19)),
        (3, false) => (0x7C59_12FC_2DAB_F98C, "asphalt_smooth_hard.grain", (4, 5)),
        (4, true) => (
            0x607A_6BC3_D427_DA49,
            "concrete_smooth_soft.grain",
            (20, 21),
        ),
        (4, false) => (0xFFB5_E3E6_2E0B_4943, "concrete_smooth_hard.grain", (6, 7)),
        (5, true) => (0x382B_1263_6ED9_D8DA, "wood_ramp_soft.grain", (22, 23)),
        (5, false) => (0x7947_A259_F181_FDB4, "wood_ramp_hard.grain", (8, 9)),
        (6, true) => (
            0x863C_58AC_34BA_D599,
            "concrete_aggregate_soft.grain",
            (24, 25),
        ),
        (6, false) => (
            0xB303_AED8_2415_30E2,
            "concrete_aggregate_hard.grain",
            (10, 11),
        ),
        (9, _) => (0x1C9C_52CC_0E1C_D4CF, "metal_smooth_hard.grain", (12, 13)),
        _ => return None,
    };
    Some(GrainChoice { key, member, slots })
}

/// The vault values of one class-`0x7AB23C11B6ADA2DE` collection the grain code reads. Layout
/// fields are at their layout offset; the rest are attribute lookups that fall back to the
/// `default` collection (`sub_82B72420`), and to the zero block at `0x830D0850` if even that fails.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceTuning {
    /// Layout `+0`, the 4×4 matrix `0xA985FBAA9326718D`; the code reads column 1 of each row:
    /// `+4`, `+20`, `+36`, `+52`.
    pub bezier: [f32; 4],
    /// Layout `+64`, the grain filename.
    pub grain: String,
    /// Layout `+68` (`0x4890392C91829954`): max speed in km/h.
    pub max_kmh: f32,
    /// Layout `+72` (`0xCEC749561306022A`): the B player's boost gain.
    pub boost_gain: f32,
    /// Layout `+76` (`0xD380D303C64CF6F8`): the boost speed ramp in km/h (≤ 0 disables the ramp).
    pub boost_kmh: f32,
    /// Layout `+80` (`0x1F459FC797B2C6BA`): A's frequency shift per unit of boost.
    pub shift_boost: f32,
    /// Layout `+84` (`0x5C9AA28695C17004`): the intensity cap in `sub_824C8588`.
    pub intensity_cap: f32,
    /// Layout `+88` (`0x145D8340A9440DA3`): B's base frequency shift.
    pub shift_b: f32,
    /// `0xD18D1174735E5CDE`, `GrainPlayer::GrainParams[2]`: attack, sustain, release, window, drift.
    pub params: [[f32; 5]; 2],
    /// `0x281FF01081475899` / `0x63764B8C7EB9EC9B`: `+1160` rise and fall steps per frame.
    pub rise_step: f32,
    pub fall_step: f32,
    /// `0x6BDC44AE7C3C79D0`: A's gain multiplier while `sub_824CA688`/`sub_824CA6E0` hold.
    pub special_gain: f32,
    /// `0xF62BC5EBD8E5DDE8`: A's frequency shift while those hold.
    pub special_shift: f32,
    /// `0x7FFF3A8AD44809EF`: B's frequency shift per unit of boost.
    pub shift_boost_b: f32,
    /// The push envelopes `sub_824C6198` programs from this collection
    /// ([`crate::grain::envelope`]).
    pub push: super::envelope::PushTuning,
}

/// `0x822F8628`, `0x820641A8`, `0x82063B08`, `0x822F8898`, `0x822F890C`, `0x822F8C64`, `0x82093D8C`
/// as the singles the image holds.
pub const KMH_PER_MS: f32 = f32::from_bits(0x4066_6666); // 3.6
pub const SECOND_OFFSET: f32 = f32::from_bits(0x3DCC_CCCD); // 0.1
pub const THREE: f32 = 3.0;
pub const INV_32767: f32 = f32::from_bits(0x3800_0100);
pub const INV_4096: f32 = f32::from_bits(0x3980_0000);
pub const PAN_SCALE: f32 = f32::from_bits(0x3BB4_00B4); // 360 / 65535
pub const INTENSITY_SCALE: f32 = f32::from_bits(0x3E75_C28F); // 0.24

/// A player's per-frame record, as `sub_824C6BD8` copies it to `player+0..+12`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainRecord {
    pub gain: f32,
    pub pitch: f32,
    pub position: f32,
}

/// The per-truck inputs of `sub_824C6BD8`'s grain branch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoardInputs {
    /// `[[owner+36]+208]`, the audio state's ground speed (m/s).
    pub speed: f32,
    /// `owner+1028`, applied when byte `owner+1032` is 0: the value of the push envelope at
    /// `owner+912` while it runs ([`crate::grain::envelope`]), `None` while it is idle.
    pub speed_scale: Option<f32>,
    /// MixMap `vfunc60(1)`, `vfunc60(2)` (0..32767) and `vfunc56(3)` (pitch ×4096).
    pub mix_gain_a: i32,
    pub mix_gain_b: i32,
    pub mix_pitch: i32,
    /// `owner+1164` (the slewed intensity) and `owner+1168`.
    pub f1164: f32,
    pub f1168: f32,
    /// `owner+1456` when byte `owner+1464` is set.
    pub f1456: Option<f32>,
    /// Whether `sub_824CA688` or `sub_824CA6E0` held this frame (A takes `special_gain`); see
    /// [`special`].
    pub special: bool,
    /// `owner+1508`, the boost amount; with it the tuning of the owner's `[+1500]` truck's surface
    /// (`+72`, `+76`), which need not be this truck's.
    pub boost: f32,
    pub boost_gain: f32,
    pub boost_kmh: f32,
}

/// `clamp(v × 3.6 / kmh, 0, 1)` as `sub_824C6BD8` spells it: `fdivs`, `fmuls`, and two `fsel`s.
fn speed_ratio(speed: f64, kmh: f64) -> f64 {
    let one = 1.0f64;
    let f13 = div_single(speed, kmh);
    let f8 = mul_single(f13, f64::from(KMH_PER_MS));
    let f7 = fsel(neg_double(f8), 0.0, f8);
    let f6 = sub_single(one, f7);
    fsel(f6, f7, one)
}

/// `sub_824C6BD8`'s grain branch for one truck: the records for players A and B.
pub fn board_records(tuning: &SurfaceTuning, input: &BoardInputs) -> [GrainRecord; 2] {
    let one = 1.0f64;
    let zero = 0.0f64;
    let mut speed = f64::from(input.speed); // f27
    if let Some(scale) = input.speed_scale {
        speed = mul_single(f64::from(scale), speed);
    }
    // Position: a cubic Bezier over t with the vault's control points.
    let t = speed_ratio(speed, f64::from(tuning.max_kmh)); // f4
    let [p3, p2, p1, p0] = tuning.bezier.map(f64::from); // +4, +20, +36, +52
    let f3 = sub_single(one, t);
    let f2 = sub_single(one, f3);
    let f1 = mul_single(p1, f3);
    let f0 = mul_single(f3, f3);
    let f13 = mul_single(f2, f2);
    let f12 = fmadd_single(p2, f2, f1);
    let f11 = mul_single(f0, f3);
    let f8 = mul_single(f13, f2);
    let f7 = mul_single(f12, f2);
    let f6 = mul_single(f8, p3);
    let f5 = mul_single(f7, f3);
    let f4 = fmadd_single(f5, f64::from(THREE), f6);
    let position = fmadd_single(f11, p0, f4); // f29

    // A's gain.
    let mix = frsp(fcfid(i64::from(input.mix_gain_a)));
    let a1164 = f64::from(input.f1164);
    let a1168 = f64::from(input.f1168);
    let larger = fsel(sub_single(a1164, a1168), a1164, a1168);
    let scaled = mul_single(mix, f64::from(INV_32767));
    let mut gain_a = mul_single(sub_single(one, larger), scaled); // f28
    if input.special {
        gain_a = mul_single(f64::from(tuning.special_gain), gain_a);
    }
    if let Some(m) = input.f1456 {
        gain_a = mul_single(f64::from(m), gain_a);
    }
    let pitch = mul_single(frsp(fcfid(i64::from(input.mix_pitch))), f64::from(INV_4096));
    let mut position_b = sub_single(position, f64::from(SECOND_OFFSET));
    if !(position_b >= zero) {
        position_b = zero;
    }

    // B's gain.
    let mix = frsp(fcfid(i64::from(input.mix_gain_b)));
    let mut gain_b = mul_single(a1164, mul_single(mix, f64::from(INV_32767)));
    if let Some(m) = input.f1456 {
        gain_b = mul_single(f64::from(m), gain_b);
    }
    let boost = f64::from(input.boost);
    if boost > zero {
        let ramp_kmh = f64::from(input.boost_kmh);
        let ramp = if ramp_kmh > zero {
            speed_ratio(speed, ramp_kmh)
        } else {
            one
        };
        let f13 = mul_single(boost, ramp);
        let f12 = mul_single(f13, f64::from(input.boost_gain));
        gain_b = fmadd_single(f12, gain_a, gain_b);
        if gain_b > one {
            gain_b = one;
        }
    }
    [
        GrainRecord {
            gain: gain_a as f32,
            pitch: pitch as f32,
            position: position as f32,
        },
        GrainRecord {
            gain: gain_b as f32,
            pitch: pitch as f32,
            position: position_b as f32,
        },
    ]
}

/// The inputs of `sub_824C8588`, the owner's slewed intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlewInputs {
    /// `[[owner+36]+96..+108]`, the audio state's vector (lanes 0..2 are used).
    pub vector: [f32; 4],
    /// `[[owner+36]+204]`.
    pub factor: f32,
    /// `[[owner+36]+200]` and byte `+340`: the `owner+1504` latch's inputs.
    pub state_200: i32,
    pub state_340: bool,
    /// The latch as it stood, and `sub_824CA6E0`'s result (evaluated only when the latch is clear).
    pub latch_1504: bool,
    pub ca6e0: bool,
    /// `owner+1160` before the call.
    pub previous: f32,
}

/// `sub_824C8588`'s outputs: the new `+1160`, the returned `|+1160|` (stored to `+1164`) and the
/// latch at `+1504`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slew {
    pub f1160: f32,
    pub f1164: f32,
    pub latch_1504: bool,
}

/// `sub_824C8588`: `min(max(|v| × 0.24, 0), cap) × factor` clamped to `±cap`, zeroed while the
/// latch or `sub_824CA6E0` holds, then slewed toward by at most the rise/fall step.
///
/// `|v|` is the lifted VMX sequence -- `vmsum3fp128`, `vrsqrtefp` and two Newton steps, zero
/// selected for a zero length -- run through the crate's RexGlue lowering.
pub fn slew(tuning: &SurfaceTuning, input: &SlewInputs) -> Slew {
    let length = vector_length(input.vector);
    let cap = f64::from(tuning.intensity_cap);
    let f9 = mul_single(f64::from(length), f64::from(INTENSITY_SCALE));
    let f7 = fsel(neg_double(f9), 0.0, f9);
    let f6 = sub_single(cap, f7);
    let f5 = fsel(f6, f7, cap);
    let mut f31 = mul_single(f5, f64::from(input.factor));
    if f31 > cap {
        f31 = cap;
    }
    let low = mul_single(cap, -1.0);
    if !(f31 >= low) {
        f31 = low;
    }
    let latch = if input.state_340 {
        true
    } else if input.latch_1504 && (input.state_200 == 0 || input.state_200 == 4) {
        false
    } else {
        input.latch_1504
    };
    if latch || input.ca6e0 {
        f31 = 0.0;
    }
    let previous = f64::from(input.previous);
    let step = f64::from(if f31 > previous {
        tuning.rise_step
    } else {
        tuning.fall_step
    });
    if sub_single(f31, previous) > step {
        f31 = add_single(previous, step);
    } else if sub_single(previous, f31) > step {
        f31 = sub_single(previous, step);
    }
    Slew {
        f1160: f31 as f32,
        f1164: (f31 as f32).abs(),
        latch_1504: latch,
    }
}

fn vector_length(v: [f32; 4]) -> f32 {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.enable_flush_mode();
    // SAFETY: the crate's x86_64 path requires SSE4.1, as every VMX port does.
    let length = unsafe { vector_length_vmx(v) };
    drop(fpscr);
    length
}

/// `v × rsqrt(|v|²)` with `vrsqrtefp` and two Newton steps, the recomp's two-rounding
/// `vnmsubfp`/`vmaddfp` (`crate::vmx`), as `sub_824C8588` and `sub_824C6198` spell it. Returns the
/// normalised vector and `|v|²`.
#[target_feature(enable = "sse4.1,fma")]
unsafe fn normalise(
    v: std::arch::x86_64::__m128,
) -> (std::arch::x86_64::__m128, std::arch::x86_64::__m128) {
    use crate::vmx;
    use std::arch::x86_64::*;
    unsafe {
        let one = _mm_cvtepi32_ps(_mm_set1_epi32(1));
        let half = _mm_mul_ps(one, _mm_castsi128_ps(_mm_set1_epi32(0x3F00_0000)));
        let squared = vmx::vmsum3fp(v, v);
        let mut r = vmx::vrsqrtefp(squared);
        for _ in 0..2 {
            let r2 = vmx::vmulfp(r, r);
            let h = vmx::vmulfp(r, half);
            let e = vmx::vnmsubfp(squared, r2, one);
            r = vmx::vmaddfp(h, e, r);
        }
        (r, squared)
    }
}

/// The lifted lane arithmetic of `sub_824C8588`'s length, lane 0 returned.
///
/// # Safety
/// Requires SSE4.1 and FMA.
#[target_feature(enable = "sse4.1,fma")]
unsafe fn vector_length_vmx(v: [f32; 4]) -> f32 {
    use crate::vmx;
    use std::arch::x86_64::*;
    unsafe {
        let v61 = _mm_loadu_ps(v.as_ptr());
        let (r, squared) = normalise(v61);
        let zero = _mm_setzero_ps();
        let is_zero = vmx::vcmpeqfp(zero, squared);
        let length = vmx::vmulfp(squared, r);
        let selected = _mm_or_ps(_mm_andnot_ps(is_zero, length), _mm_and_ps(is_zero, zero));
        let mut out = [0f32; 4];
        _mm_storeu_ps(out.as_mut_ptr(), selected);
        out[0]
    }
}

/// `sub_824C6198`'s brake test: `vmsum3(a·rsqrt|a|², b·rsqrt|b|²)`, the cosine between the audio
/// state's `+128` (velocity delta) and `+96` (velocity), with no zero selection (a zero vector
/// gives a non-finite lane, which then compares false).
pub fn normalised_dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.enable_flush_mode();
    // SAFETY: as above.
    let dot = unsafe { normalised_dot_vmx(a, b) };
    drop(fpscr);
    dot
}

#[target_feature(enable = "sse4.1,fma")]
unsafe fn normalised_dot_vmx(a: [f32; 4], b: [f32; 4]) -> f32 {
    use crate::vmx;
    use std::arch::x86_64::*;
    unsafe {
        let va = _mm_loadu_ps(a.as_ptr());
        let vb = _mm_loadu_ps(b.as_ptr());
        let (ra, _) = normalise(va);
        let (rb, _) = normalise(vb);
        let dot = vmx::vmsum3fp(vmx::vmulfp(va, ra), vmx::vmulfp(vb, rb));
        let mut out = [0f32; 4];
        _mm_storeu_ps(out.as_mut_ptr(), dot);
        out[0]
    }
}

/// The values `sub_824C9058` posts for one truck, in the order it posts them. They go to the
/// per-player bus graphs `sub_824C8878` builds (see the module note in `grain`): graph 1 (`Sub0 →
/// HI20 → LI20 → FSS0 → Sen0 → Gai0 → Sen0`) and graph 2 (`Sub0 → Sen0 → Sen0 → Pn21 → Sen0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainValues {
    /// `HI20`/`LI20` property 0 on both players: `vfunc64(12)`, `vfunc64(11)`.
    pub highpass: f32,
    pub lowpass: f32,
    /// `FrequencyShiftSsb` property 0 on A and on B.
    pub shift_a: f32,
    pub shift_b: f32,
    /// `Pn21` property 0 on both: `vfunc52(0) × 360/65535`.
    pub pan: f32,
    /// graph 2's second send (environment) on both: `vfunc60(13) / 32767`.
    pub environment: f32,
    /// graph 2's first send on A and B when the owner is the local player: `vfunc60(21|22) / 32767`.
    pub local: Option<(f32, f32)>,
}

/// Inputs of [`chain_values`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainInputs {
    pub mix64_11: i32,
    pub mix64_12: i32,
    pub mix52_0: i32,
    pub mix60_13: i32,
    /// `vfunc60(21)`, `vfunc60(22)` when `[[owner+16]+72]` is set.
    pub local: Option<(i32, i32)>,
    /// The `+1504` latch (as updated here) or `sub_824CA6E0`.
    pub special: bool,
    /// `owner+1152` when byte `owner+1156` is 0: the value of the push envelope at `owner+1036`
    /// while it runs ([`crate::grain::envelope`]), `None` while it is idle.
    pub f1152: Option<f32>,
    /// `owner+1508` and the `[+1500]` truck's `+80` and `0x7FFF3A8AD44809EF`.
    pub boost: f32,
    pub boost_shift_a: f32,
    pub boost_shift_b: f32,
}

/// `sub_824C9058`'s values for one truck whose tuning is `tuning`.
pub fn chain_values(tuning: &SurfaceTuning, input: &ChainInputs) -> ChainValues {
    let word = |w: i32| frsp(fcfid(i64::from(w)));
    let mut shift_a = if input.special {
        f64::from(tuning.special_shift)
    } else {
        0.0
    };
    if let Some(v) = input.f1152 {
        shift_a = add_single(f64::from(v), shift_a);
    }
    let boost = f64::from(input.boost);
    if boost > 0.0 {
        shift_a = fmadd_single(f64::from(input.boost_shift_a), boost, shift_a);
    }
    let mut shift_b = f64::from(tuning.shift_b);
    if let Some(v) = input.f1152 {
        shift_b = add_single(f64::from(v), shift_b);
    }
    if boost > 0.0 {
        shift_b = fmadd_single(boost, f64::from(input.boost_shift_b), shift_b);
    }
    ChainValues {
        highpass: word(input.mix64_12) as f32,
        lowpass: word(input.mix64_11) as f32,
        shift_a: shift_a as f32,
        shift_b: shift_b as f32,
        pan: mul_single(word(input.mix52_0), f64::from(PAN_SCALE)) as f32,
        environment: mul_single(word(input.mix60_13), f64::from(INV_32767)) as f32,
        local: input.local.map(|(a, b)| {
            (
                mul_single(word(a), f64::from(INV_32767)) as f32,
                mul_single(word(b), f64::from(INV_32767)) as f32,
            )
        }),
    }
}

/// `sub_824CA688`, the owner's `+1504` latch: set while the audio state's `+340` (balance ≠ 0,
/// i.e. a manual) is set; once latched, cleared when `[state+200]` (wheels in contact) is 0 or 4.
/// Returns the new latch, which is also the function's result.
pub fn manual_latch(latch: bool, state_340: bool, wheels: u32) -> bool {
    if state_340 {
        true
    } else if latch && (wheels == 0 || wheels == 4) {
        false
    } else {
        latch
    }
}

/// `sub_824CA6E0`, the owner's `+1505` latch: set while the audio state's `+372` is set (bit 23 of
/// the airborne trick packet's word `+152`, `rlwinm 9,31,31`, which the bridge `sub_824B0DA8`
/// refreshes only while `+332`, known air); once latched, cleared when both `+615` and `+616` (the
/// feet inside the deck box) are set.
pub fn trick_latch(latch: bool, state_372: bool, state_615: bool, state_616: bool) -> bool {
    if state_372 {
        true
    } else if latch && state_616 && state_615 {
        false
    } else {
        latch
    }
}

/// [`BoardInputs::special`] as `sub_824C6BD8` evaluates it: `sub_824CA688() || sub_824CA6E0()`,
/// the second only when the first is false (so its latch is not updated otherwise). Returns
/// `(special, latch_1504, latch_1505)`.
#[allow(clippy::too_many_arguments)]
pub fn special(
    latch_1504: bool,
    latch_1505: bool,
    state_340: bool,
    wheels: u32,
    state_372: bool,
    state_615: bool,
    state_616: bool,
) -> (bool, bool, bool) {
    let manual = manual_latch(latch_1504, state_340, wheels);
    if manual {
        return (true, manual, latch_1505);
    }
    let trick = trick_latch(latch_1505, state_372, state_615, state_616);
    (trick, manual, trick)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `concrete_rough_hard` from the vault (`Hash_03721D0FA99A03C8`, parent `default`).
    pub fn concrete_rough_hard() -> SurfaceTuning {
        SurfaceTuning {
            bezier: [
                f32::from_bits(0x3F80_0000),
                f32::from_bits(0x3F69_EE58),
                f32::from_bits(0x3D0D_3DCB),
                0.0,
            ],
            grain: "concrete_rough_hard.grain".into(),
            max_kmh: 60.0,
            boost_gain: 2.0,
            boost_kmh: 10.0,
            shift_boost: -100.0,
            intensity_cap: f32::from_bits(0x3F19_999A),
            shift_b: -10.0,
            params: [[0.1, 0.2, 0.1, 1.6, 0.05], [0.2, 0.1, 0.2, 1.5, 0.05]],
            rise_step: f32::from_bits(0x3D75_C28F),
            fall_step: f32::from_bits(0x3D75_C28F),
            special_gain: f32::from_bits(0x3F26_6666),
            special_shift: 150.0,
            shift_boost_b: 50.0,
            push: crate::grain::envelope::PushTuning {
                ramp_kmh: 45.0,
                scale_low: f32::from_bits(0x3FB3_3333),
                scale_high: f32::from_bits(0x3F8C_CCCD),
                shift_low: -52.0,
                shift_high: -20.0,
                scale_ms: [35, 200, 600],
                shift_ms: [30, 200, 600],
            },
        }
    }

    #[test]
    fn surface_table_matches_the_vault_filenames() {
        assert_eq!(
            grain_for_surface(2, false).unwrap().member,
            "concrete_rough_hard.grain"
        );
        assert_eq!(grain_for_surface(2, true).unwrap().slots, (16, 17));
        assert_eq!(grain_for_surface(9, true), grain_for_surface(9, false));
        for surface in [7, 8, 10, 13, 14, 0] {
            assert!(grain_for_surface(surface, false).is_none());
        }
    }

    #[test]
    fn position_endpoints_follow_the_control_points() {
        let tuning = concrete_rough_hard();
        let input = BoardInputs {
            speed: 0.0,
            speed_scale: None,
            mix_gain_a: 32767,
            mix_gain_b: 16000,
            mix_pitch: 4096,
            f1164: 0.0,
            f1168: 0.0,
            f1456: None,
            special: false,
            boost: 0.0,
            boost_gain: 2.0,
            boost_kmh: 10.0,
        };
        let [a, b] = board_records(&tuning, &input);
        assert_eq!(a.position, 0.0);
        assert_eq!(b.position, 0.0, "clamped at zero");
        assert_eq!(a.pitch, 1.0);
        assert_eq!(a.gain, mul_single(32767.0, f64::from(INV_32767)) as f32);
        // Above max speed t = 1 and the position is the last control point.
        let [a, b] = board_records(
            &tuning,
            &BoardInputs {
                speed: 40.0,
                ..input
            },
        );
        assert_eq!(a.position, 1.0);
        assert_eq!(b.position, f32::from_bits(0x3F66_6666));
    }

    #[test]
    fn slew_is_limited_to_the_step() {
        let tuning = concrete_rough_hard();
        let s = slew(
            &tuning,
            &SlewInputs {
                vector: [3.0, 4.0, 0.0, 0.0],
                factor: 1.0,
                state_200: 1,
                state_340: false,
                latch_1504: false,
                ca6e0: false,
                previous: 0.0,
            },
        );
        assert_eq!(s.f1160, f32::from_bits(0x3D75_C28F));
        assert_eq!(s.f1164, s.f1160);
        let zero = slew(
            &tuning,
            &SlewInputs {
                vector: [0.0; 4],
                factor: 1.0,
                state_200: 1,
                state_340: false,
                latch_1504: false,
                ca6e0: false,
                previous: 0.0,
            },
        );
        assert_eq!(zero.f1160, 0.0);
    }
}
