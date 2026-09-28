//! The game-side **input writers** for the controllers the local player's components depend on,
//! transliterated from the lifted game code as pure functions over the values those writers read.
//! Each returns the `(id, word)` pairs the original stores through controller slot 8, in the
//! original's order; apply them with `controller::set` / `AuthoredRuntime::mixmap_set`.
//!
//! | controller | writer | here |
//! |---|---|---|
//! | `60010000` audio state | `sub_824B19C8` (ids 0,1,2,4–11,13,14), `sub_824B2088` (3), `sub_824B23C8` via the bridge (12) | [`state_inputs`], [`listener_facing`], [`player_flag`] |
//! | `60010010`, `60010020` positions (SFXCTL_3DObjPos, vtable `0x822FCFE0`) | `sub_824AEC70` → `sub_824AEB28` → `sub_824AE6E0` (`sub_824AE178`, `sub_824AE418`, acos `sub_82453298`) and `sub_824AEE60` | [`ObjPos::update`] |
//! | listener `*(0x830CFDD4)` | `sub_8248CC08` (camera rows, first player record) | [`Listener::update`] |
//! | the position controllers' `+28/+32/+36` | `sub_824B0C48` binds the state's `+48/80/96` and `+144/160/176` copies of the record | [`PlayerRecord::emitters`] |
//! | `400000E0` Jitter | `sub_824EF378` / `sub_824EF4C8`, vault `sub_824EF0B8` | [`Jitter`] |
//! | `40010010` Contacts | `sub_824B90D8` / `sub_824BA630` (ids 1, 2, 6) | [`Contacts`] |
//! | `40010030` Rail | `sub_824C28B0` (ids 0, 1) | [`rail_inputs`] |
//! | `40010090` OffBoard | `sub_824E9270` (id 0) | [`off_board_input`] |
//! | `400100A0` HandGrabs | `sub_824EC3E0` (id 0) | [`hand_grabs_input`] |
//! | `40010000` SkateBoard | `sub_824C5CA8` (0, 6), `sub_824C6198` (4), `sub_824CA738` (2, 3), `sub_824C7438` (1) | ported in `skate-game`'s `components/board.rs`, not here |
//! | `40000070` Pause | `sub_824E1D00` (ids 0, 1, 2) | [`pause_inputs`], [`Pause`] |
//! | `40000010` Music ids 3, 6 | `sub_824D1208` over the frame record's multiplier bits (`sub_827A2E88`) | [`multiplier_flags`], [`MusicEmphasis`] |
//! | globals | see [`FREE_SKATE_GLOBALS`] | values only |
//!
//! Float work mirrors the lifted forms: scalar `fmuls`/`fsel`/`fctiwz` through [`crate::fp`] under
//! the scalar flush mode, vector work (lengths, dot products, the reciprocal-square-root and
//! reciprocal refinements, the acos) through [`crate::vmx`] under the VMX mode. Vectors are the
//! guest's `[x, y, z, w]` element order.

use core::arch::x86_64::*;

use crate::fp::{self, fctiwz_low_word, frsp, fsel, mul_single};
use crate::vmx::{self, Fpscr};

/// A guest `vector4` in element order.
pub type Vec4 = [f32; 4];

// ---------------------------------------------------------------------- image constants

/// `0x822F9520..0x822F952C`: the speed scales of ids 0, 1, 7 and 8/14 (23592.2, 3932.04, 2359.22,
/// 1685.16). Their bits, verified against the image by the tests.
pub const SPEED_SCALES: [u32; 4] = [0x46B8_507A, 0x4575_C0A3, 0x4513_7395, 0x44D2_A51E];
/// `0x821747FC` 32767.0.
const F32767: u32 = 0x46FF_FE00;
/// `0x8209975C` 0.5, `0x821161AC` 0.7, `0x822F8D58` 0.3 (a double).
const HALF: u32 = 0x3F00_0000;
const SEVEN_TENTHS: u32 = 0x3F33_3333;
const THREE_TENTHS: u64 = 0x3FD3_3333_3333_3333;
/// `0x8209BE90` 0.0001, the angle helpers' minimum 2-D length.
const EPSILON: u32 = 0x38D1_B717;
/// `0x822F8960` 65535.0 and `0x822F8904` 1/2π: the angle scale.
const ANGLE_SCALE: u32 = 0x477F_FF00;
const INV_TWO_PI: u32 = 0x3E22_F983;
/// `0x822F9820..0x822F985C` and `0x822FB840`: `sub_82453298`'s coefficient vectors.
const ACOS_A: [u32; 4] = [0xBD6D_D42D, 0xBED6_5553, 0x3E66_3246, 0x400B_1889];
const ACOS_B: [u32; 4] = [0x3F1D_D7B6, 0x4089_80BD, 0xBF98_3F2F, 0xC0D1_360E];
const ACOS_C: [u32; 4] = [0xBFAF_4418, 0xC08F_6AD9, 0x3FB5_8485, 0x40AF_6AD8];
const ACOS_D: [u32; 4] = [0x4049_0FDB, 0x40C9_0FDB, 0x3EA2_F983, 0x3E22_F983];
const ACOS_ONE_PLUS: u32 = 0x3F80_0001;

/// Vault `Hash_C1831BDB6CB1B1EA` / `Hash_3213338D2D2C817F` (read by `sub_824B19C8`): the listener
/// distance slew rate `0x0395642CA6FC543A` = 100.0 per second and cap `0x204DCC9296DC9400` = 35.0.
pub const DISTANCE_RATE: f32 = 100.0;
pub const DISTANCE_CAP: f32 = 35.0;
/// Vault `Hash_C1831BDB6CB1B1EA` collection `camera`, field `0xA6F853B935E46E5F` = 0.25: the
/// 3DObjPos constructor (`sub_824AE098`) stores it at `+112`, the pull-back of listener frame A.
/// The constructor reads it through the tuning holder's `+60` instance; `sub_8289D5C8` fills that
/// slot from collection `0x55D801EDE7E338B6`, which is `hash64("camera")`.
pub const OBJPOS_PULLBACK: f32 = 0.25;

#[inline]
fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

// ---------------------------------------------------------------------- vector helpers

/// PPC element `i` lives in SSE lane `3 - i` (RexGlue's register layout; `vmx::vmsum3fp`'s mask
/// relies on it).
#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn load(v: Vec4) -> __m128 {
    _mm_set_ps(v[0], v[1], v[2], v[3])
}

#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn lanes(v: __m128) -> Vec4 {
    let mut a = [0f32; 4];
    unsafe { _mm_storeu_ps(a.as_mut_ptr(), v) };
    [a[3], a[2], a[1], a[0]]
}

#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn el(v: __m128, i: usize) -> f32 {
    unsafe { lanes(v)[i] }
}

#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn splat(x: f32) -> __m128 {
    _mm_set1_ps(x)
}

#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn spl(v: __m128, i: usize) -> __m128 {
    unsafe { splat(el(v, i)) }
}

#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn words(w: [u32; 4]) -> __m128 {
    unsafe { load([f(w[0]), f(w[1]), f(w[2]), f(w[3])]) }
}

/// `vsel` of a float by a lane mask.
#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn select(a: __m128, b: __m128, mask: __m128) -> __m128 {
    unsafe {
        _mm_castsi128_ps(vmx::vsel(
            _mm_castps_si128(a),
            _mm_castps_si128(b),
            _mm_castps_si128(mask),
        ))
    }
}

/// The reciprocal square root every site spells out: `vrsqrtefp` then two Newton steps,
/// `x' = x + (x·½)(1 − s·x²)` as `vmulfp`/`vnmsubfp`/`vmaddfp` (two roundings each).
#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn rsqrt2(s: __m128) -> __m128 {
    unsafe {
        let (one, half) = (splat(1.0), splat(0.5));
        let mut x = vmx::vrsqrtefp(s);
        for _ in 0..2 {
            let sq = vmx::vmulfp(x, x);
            let h = vmx::vmulfp(x, half);
            let e = vmx::vnmsubfp(s, sq, one);
            x = vmx::vmaddfp(h, e, x);
        }
        x
    }
}

/// `s · rsqrt(s)`, 0 where `s == 0` (`vcmpeqfp128 0,s ; vsel`).
#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn length_of_square(s: __m128) -> __m128 {
    unsafe {
        let r = vmx::vmulfp(s, rsqrt2(s));
        select(r, splat(0.0), vmx::vcmpeqfp(splat(0.0), s))
    }
}

/// `|a − b|` over x, y, z: `vsubfp`, `vmsum3fp128`, the length idiom.
#[inline]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn distance(a: Vec4, b: Vec4) -> f32 {
    unsafe {
        let d = vmx::vsubfp(load(a), load(b));
        el(length_of_square(vmx::vmsum3fp(d, d)), 0)
    }
}

/// The bridge's normalisation (`sub_824B0DA8` before `sub_824B2088`): `v · rsqrt(v·v)` with the two
/// Newton steps and **no** zero guard. The caller of [`listener_facing`] passes vectors made this
/// way.
pub fn normalize3(v: Vec4) -> Vec4 {
    let mut fpscr = Fpscr::capture();
    fpscr.enable_flush_mode_unconditional();
    unsafe {
        let x = load(v);
        lanes(vmx::vmulfp(x, rsqrt2(vmx::vmsum3fp(x, x))))
    }
}

/// `vperm128 v,v,v,[0x822FB8B0]` with the mask `00010203 18191A1B 00010203 00010203`:
/// `(x, z, x, x)`, the horizontal plane.
fn horizontal(v: Vec4) -> Vec4 {
    [v[0], v[2], v[0], v[0]]
}

/// `sub_82453298`: the vector acos (argument and result in `v1`), straight-line polynomial with a
/// `vrsqrtefp`-refined `sqrt(1 − |x|)`.
#[target_feature(enable = "sse4.1,fma")]
unsafe fn acos(v1: __m128) -> __m128 {
    unsafe {
        let v60 = vmx::vmulfp(v1, v1);
        let v61 = words([ACOS_ONE_PLUS; 4]);
        let v12 = splat(0.5);
        let v0 = _mm_castsi128_ps(_mm_andnot_si128(
            _mm_set1_epi32(i32::MIN),
            _mm_castps_si128(v1),
        ));
        let (v63, v62) = (words(ACOS_B), words(ACOS_A));
        let v9 = spl(v63, 3);
        let v11 = spl(v62, 3);
        let v58 = vmx::vsubfp(v61, v0);
        let v6 = spl(v63, 2);
        let v8 = spl(v63, 1);
        let v3 = vmx::vnmsubfp(v0, v1, v1);
        let v7 = spl(v62, 2);
        let v61 = words(ACOS_C);
        let v31 = vmx::vmaddfp(v11, v0, v9);
        let v4 = spl(v62, 1);
        let v9 = spl(v63, 0);
        let v10 = vmx::vmulfp(v60, v0);
        let v5 = spl(v62, 0);
        let v63 = words(ACOS_D);
        let v30 = vmx::vmaddfp(v7, v0, v6);
        let v6 = spl(v61, 3);
        let v2 = vmx::vmaddfp(v4, v0, v8);
        let v7 = spl(v61, 2);
        let v8 = spl(v61, 1);
        let v4 = vmx::vmaddfp(v5, v0, v9);
        let v9 = spl(v61, 0);
        let v57 = spl(v63, 0);
        let v13 = vmx::vrsqrtefp(v58);
        let v11 = vmx::vmulfp(v58, v12);
        let v56 = vmx::vmulfp(v57, v12);
        let v5 = vmx::vmaddfp(v31, v0, v6);
        let v6 = vmx::vmaddfp(v30, v0, v7);
        let v7 = vmx::vmaddfp(v2, v0, v8);
        let v9 = vmx::vmaddfp(v4, v0, v9);
        let v0 = vmx::vmulfp(v13, v13);
        let v8 = vmx::vmaddfp(v6, v10, v5);
        let v9 = vmx::vmaddfp(v9, v10, v7);
        let v10 = vmx::vnmsubfp(v11, v0, v12);
        let v0 = vmx::vmulfp(v1, v8);
        let v12 = vmx::vmulfp(v3, v9);
        let v13 = vmx::vmaddfp(v13, v10, v13);
        let v0 = vmx::vmaddfp(v12, v13, v0);
        vmx::vsubfp(v56, v0)
    }
}

fn all(v: Vec4) -> bool {
    v.iter().all(|&b| b.to_bits() != 0)
}

/// The shared body of `sub_824AE178` / `sub_824AE418`: the angle between the horizontal vectors
/// `a` and `b` as `fctiwz(acos(â·b̂) × 65535 × 1/2π)`, or `None` when either is shorter than
/// 0.0001. Returns the angle and the cross term `b̂.x·â.y − b̂.y·â.x` (lane 0) the callers test.
#[target_feature(enable = "sse4.1,fma")]
unsafe fn plane_angle(a: Vec4, b: Vec4) -> Option<(i32, f32)> {
    unsafe {
        let (v63, v62) = (load(a), load(b));
        let eps = splat(f(EPSILON));
        let sa = vmx::vmulfp(v63, v63);
        let la = length_of_square(vmx::vaddfp(spl(sa, 0), spl(sa, 1)));
        let sb = vmx::vmulfp(v62, v62);
        let lb = length_of_square(vmx::vaddfp(spl(sb, 0), spl(sb, 1)));
        if !all(lanes(vmx::vcmpgefp(lb, eps))) || !all(lanes(vmx::vcmpgefp(la, eps))) {
            return None;
        }
        let one = splat(1.0);
        let (rb, ra) = (vmx::vrefp(lb), vmx::vrefp(la));
        let rb1 = vmx::vmaddfp(rb, vmx::vnmsubfp(rb, lb, one), rb);
        let ra1 = vmx::vmaddfp(ra, vmx::vnmsubfp(ra, la, one), ra);
        let rb2 = vmx::vmaddfp(rb1, vmx::vnmsubfp(rb1, lb, one), rb1);
        let ra2 = vmx::vmaddfp(ra1, vmx::vnmsubfp(ra1, la, one), ra1);
        let nb = vmx::vmulfp(rb2, v62); // v126
        let na = vmx::vmulfp(ra2, v63); // v125
        let prod = vmx::vmulfp(nb, na);
        let dot = vmx::vaddfp(spl(prod, 0), spl(prod, 1));
        let clamped = vmx::vminfp(splat(1.0), vmx::vmaxfp(splat(-1.0), dot));
        let angle = vmx::vmulfp(acos(clamped), splat(f(ANGLE_SCALE)));
        let v61 = vmx::vmulfp(spl(nb, 1), spl(na, 0));
        let v60 = vmx::vmulfp(spl(nb, 0), spl(na, 1));
        let scaled = vmx::vmulfp(splat(f(INV_TWO_PI)), angle);
        let cross = el(vmx::vsubfp(v60, v61), 0);
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        let r = fctiwz_low_word(f64::from(el(scaled, 0))) as i32;
        Some((r, cross))
    }
}

// ---------------------------------------------------------------------- 60010000: the audio state

/// What `sub_824B19C8` reads from the audio state (offsets are the state's).
#[derive(Clone, Copy, Debug, Default)]
pub struct StateFields {
    /// `+200` wheels in contact (Collision `wheel_count_0`).
    pub wheel_count_200: u32,
    /// `+208` ground speed (Motion+164).
    pub ground_speed_208: f32,
    /// `+212` |COM velocity|.
    pub com_speed_212: f32,
    /// `+336` brake planted, `+339` manual brake, `+343` trick active.
    pub brake_336: bool,
    pub manual_brake_339: bool,
    pub trick_active_343: bool,
    /// `[[state+16]+72]`: the state's player object is the local one (true in the capture).
    pub local_player_72: bool,
    /// `[*(0x83083C38) + 0x2FCB4] + 16`, the global "G+16" byte: in the capture it rises with each
    /// bail (`+676`) and holds for about 110 frames. Writer not identified (the bail-camera system
    /// around `sub_827AE9C8`/`sub_827F1DD0` reads the same object).
    pub bail_camera_g16: bool,
    /// `+96` (the bridge's copy of `[B+36]+16`) and the listener `[*(0x83083C38)+0x2F078]` `+32`
    /// (`None` when that pointer is null): id 13's distance.
    pub vector_96: Vec4,
    pub listener_32: Option<Vec4>,
}

/// Retail id 12 source, `sub_824B23C8(state)`: `+684` when the state's player is local; otherwise
/// the first group of SFX slot 1 (`sys+660`, its `+16` list) whose `+72` byte is set decides
/// (`group+84 == 0`), and no such group means 1.
pub fn player_flag(local_player_72: bool, state_684: u32, slot1_groups: &[(bool, u32)]) -> u32 {
    if local_player_72 || slot1_groups.is_empty() {
        return state_684;
    }
    for &(flag_72, word_84) in slot1_groups {
        if flag_72 {
            return u32::from(word_84 == 0);
        }
    }
    1
}

/// `clamp(fctiwz(min(max(v × scale, 0), 32767)), 0, 32767)`: `fmuls ; fneg ; fsel ; fsubs ; fsel ;
/// fctiwz` then the integer clamp, as ids 0, 1, 7, 8, 14 spell it.
fn scaled_speed(v: f32, scale: u32) -> u32 {
    let p = mul_single(f64::from(v), f64::from(f(scale)));
    let lo = fsel(-p, 0.0, p);
    let k = f64::from(f(F32767));
    let hi = fsel(fp::sub_single(k, lo), lo, k);
    (fctiwz_low_word(hi) as i32).clamp(0, 32767) as u32
}

/// `sub_824B19C8(state, dt)`: ids 2, 10, 4, 5, 0, 1, 7, 8, 14, 6, 9, 11, 13 in that order.
/// `distance_784` is the state's `+784` slew (updated in place); `rate`/`cap` are the vault's
/// [`DISTANCE_RATE`]/[`DISTANCE_CAP`].
pub fn state_inputs(
    s: &StateFields,
    distance_784: &mut f32,
    dt: f32,
    rate: f32,
    cap: f32,
) -> Vec<(u32, u32)> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let on = |b: bool| if b { 32767 } else { 0 };
    let mut out = Vec::with_capacity(13);
    out.push((2, on(s.wheel_count_200 == 0)));
    let wheels = match s.wheel_count_200.wrapping_sub(1) {
        0 => 8191,
        1 => 16383,
        2 => 24575,
        3 => 32767,
        _ => 0,
    };
    out.push((10, wheels));
    out.push((4, on(s.brake_336)));
    out.push((5, on(s.manual_brake_339)));
    out.push((0, scaled_speed(s.ground_speed_208, SPEED_SCALES[0])));
    out.push((1, scaled_speed(s.ground_speed_208, SPEED_SCALES[1])));
    out.push((7, scaled_speed(s.ground_speed_208, SPEED_SCALES[2])));
    out.push((8, scaled_speed(s.ground_speed_208, SPEED_SCALES[3])));
    out.push((14, scaled_speed(s.com_speed_212, SPEED_SCALES[3])));
    out.push((6, on(s.trick_active_343)));
    out.push((9, if s.local_player_72 { 0 } else { 32767 }));
    out.push((11, on(s.bail_camera_g16)));
    out.push((13, listener_distance(s, distance_784, dt, rate, cap)));
    out
}

/// Id 13: `|state+96 − listener+32|` capped, slewed at `rate × dt` into `+784`, `/cap × 32767`.
fn listener_distance(s: &StateFields, distance_784: &mut f32, dt: f32, rate: f32, cap: f32) -> u32 {
    let mut d = {
        let mut fpscr = Fpscr::capture();
        fpscr.enable_flush_mode_unconditional();
        unsafe { distance(s.vector_96, s.listener_32.unwrap_or([0.0; 4])) }
    };
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    if d > cap {
        d = cap; // sub_824ADE88 reads the cap again
    }
    let step = mul_single(f64::from(rate), f64::from(dt)) as f32;
    let old = *distance_784;
    let mut new = d;
    if d > old {
        if fp::sub_single(f64::from(d), f64::from(old)) as f32 > step {
            new = fp::add_single(f64::from(step), f64::from(old)) as f32;
        }
    } else if d < old && fp::sub_single(f64::from(old), f64::from(d)) as f32 > step {
        new = fp::sub_single(f64::from(old), f64::from(step)) as f32;
    }
    *distance_784 = new;
    let ratio = fp::div_single(f64::from(new), f64::from(cap));
    let w = fctiwz_low_word(mul_single(ratio, f64::from(f(F32767)))) as i32;
    w.clamp(0, 32767) as u32
}

/// `sub_824B2088(state, a, b)`: id 3, and the state's `+680`. `a` = normalize(record `+496` =
/// `[PhysOut+0]+80`), `b` = normalize(record `+64` = `[PhysOut+0]+128`) ([`normalize3`]),
/// `listener_32` = `*(0x830CFDD4) + 32` (the listener's frame-A direction; see [`Listener`]).
///
/// `+680 = clamp01(f1·0.7 + (a·up·0.3)·f1 + (|b·L|·0.3)·f1)`, `f1 = 1 − (clamp(a·L, −1, 1) + 1)·½`.
pub fn listener_facing(a: Vec4, b: Vec4, listener_32: Vec4) -> (u32, f32) {
    let (dot_al, dot_aup, dot_bl) = {
        let mut fpscr = Fpscr::capture();
        fpscr.enable_flush_mode_unconditional();
        unsafe {
            let l = load(listener_32);
            (
                el(vmx::vmsum3fp(load(a), l), 0),
                el(vmx::vmsum3fp(load(a), load([0.0, 1.0, 0.0, 0.0])), 0),
                el(vmx::vmsum3fp(load(b), l), 0),
            )
        }
    };
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let (one, zero) = (1.0f64, 0.0f64);
    let d = f64::from(dot_al);
    let f5 = fsel(fp::sub_single(-1.0, d), -1.0, d);
    let f3 = fsel(fp::sub_single(one, f5), f5, one);
    let f2 = fp::add_single(f3, one);
    let f1 = fp::nmsub_single(f2, f64::from(f(HALF)), one);
    let f12 = mul_single(f1, f64::from(f(SEVEN_TENTHS)));
    let three = f64::from_bits(THREE_TENTHS);
    let f8 = frsp(f64::from(dot_aup) * three);
    let f6 = fp::fmadd_single(f8, f1, f12);
    let f2b = frsp(fp::abs_double(f64::from(dot_bl)) * three);
    let r = fp::fmadd_single(f2b, f1, f6);
    let lo = fsel(-r, zero, r);
    let v = fsel(fp::sub_single(one, lo), lo, one);
    let w = fctiwz_low_word(mul_single(v, f64::from(f(F32767)))) as i32;
    (w.clamp(0, 32767) as u32, v as f32)
}

// ---------------------------------------------------------------------- positions: SFXCTL_3DObjPos

/// The listener object `*(0x830CFDD4)`, as `sub_824AEB28`/`sub_824AEE60` read it: two
/// position/direction/velocity triples. In the capture the local player's own position controller
/// (`60010010`) reads distance 0 and relative speed 0 against `pos1`/`vel1`, i.e. `pos1` is the
/// point the camera follows (the skater) and `pos0` the camera itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct Listener {
    /// `+0` position (camera), `+32` direction, `+48` velocity.
    pub pos0: Vec4,
    pub dir0: Vec4,
    pub vel0: Vec4,
    /// `+64` position (followed point), `+96` direction, `+112` velocity.
    pub pos1: Vec4,
    pub dir1: Vec4,
    pub vel1: Vec4,
    /// `+16` and `+80`: the previous positions [`Listener::update`] keeps.
    pub prev_pos0_16: Vec4,
    pub prev_pos1_80: Vec4,
}

/// The pointers an owner binds into a 3DObjPos object: `+32` emitter position (the object is
/// active while non-null; `sub_824AEC60` clears it with `+28`), `+36` its velocity (the rates run
/// only when non-null), `+28` a facing vector (id 10 only when non-null).
#[derive(Clone, Copy, Debug, Default)]
pub struct Emitter {
    pub position_32: Option<Vec4>,
    pub velocity_36: Option<Vec4>,
    pub facing_28: Option<Vec4>,
}

/// The 3DObjPos object's own fields (a 128-byte object, ctor `sub_824AE098`).
#[derive(Clone, Copy, Debug)]
pub struct ObjPos {
    /// `+40`/`+44` |listener pos1 − emitter| and its previous value.
    pub dist1_40: f32,
    pub dist1_prev_44: f32,
    /// `+48`/`+52` |listener pos0 − emitter| and its previous value.
    pub dist0_48: f32,
    pub dist0_prev_52: f32,
    /// `+56`: written to id 11 before `sub_824AEB28` zeroes it; set by the owner between frames.
    pub flag_56: u32,
    /// `+64`/`+80` copies of listener pos0/pos1 taken each frame.
    pub pos0_64: Vec4,
    pub pos1_80: Vec4,
    /// `+96`/`+100` signed relative speed against vel1 and its previous value; `+104`/`+108` the
    /// same against vel0. **Uninitialised in retail** until the first active frame (the ctor never
    /// writes them); the replay shows the first activation depends on that memory.
    pub rate1_96: f32,
    pub rate1_prev_100: f32,
    pub rate0_104: f32,
    pub rate0_prev_108: f32,
    /// `+112` the pull-back of frame A ([`OBJPOS_PULLBACK`]).
    pub pullback_112: f32,
}

impl Default for ObjPos {
    fn default() -> Self {
        Self {
            dist1_40: 0.0,
            dist1_prev_44: 0.0,
            dist0_48: 0.0,
            dist0_prev_52: 0.0,
            flag_56: 0,
            pos0_64: [0.0; 4],
            pos1_80: [0.0; 4],
            rate1_96: 0.0,
            rate1_prev_100: 0.0,
            rate0_104: 0.0,
            rate0_prev_108: 0.0,
            pullback_112: OBJPOS_PULLBACK,
        }
    }
}

/// `-1.0` as the inactive path stores it (`0x8216DEE0`, `stfs`/`lwz` into slot 8).
const MINUS_ONE_BITS: u32 = 0xBF80_0000;

impl ObjPos {
    /// `sub_824AEC70` (vtable slot 9, the per-frame update) for one object. `word15` is the
    /// controller's current input 15 (it is read back with slot 12 and or-ed). Returns the writes
    /// in order.
    pub fn update(
        &mut self,
        listener: &Listener,
        emitter: &Emitter,
        word15: u32,
    ) -> Vec<(u32, u32)> {
        let mut out = Vec::with_capacity(12);
        let Some(position) = emitter.position_32 else {
            out.push((3, 0));
            out.push((1, MINUS_ONE_BITS));
            out.push((2, 0));
            out.push((0, MINUS_ONE_BITS));
            out.push((15, word15 & !1));
            return out;
        };
        let mut w15 = word15 | 1;
        out.push((15, w15));
        out.push((11, self.flag_56));
        self.listener_frames(listener, position, emitter, &mut out);
        if let Some(velocity) = emitter.velocity_36 {
            self.rates(listener, position, velocity, &mut w15, &mut out);
        }
        out
    }

    /// `sub_824AEB28` (slot 13) and `sub_824AE6E0`.
    fn listener_frames(
        &mut self,
        l: &Listener,
        position: Vec4,
        emitter: &Emitter,
        out: &mut Vec<(u32, u32)>,
    ) {
        let mut fpscr = Fpscr::capture();
        fpscr.enable_flush_mode_unconditional();
        self.flag_56 = 0;
        // Frame A: direction perm(pos0 dir), origin perm(pos0) − normalise₂(perm(dir0)) × pull-back.
        let dir_a = horizontal(l.dir0);
        let origin_a = unsafe {
            let d = load(dir_a);
            let sq = vmx::vmulfp(d, d);
            let r = rsqrt2(vmx::vaddfp(spl(sq, 0), spl(sq, 1)));
            let pulled = vmx::vmulfp(vmx::vmulfp(d, r), splat(self.pullback_112));
            lanes(vmx::vsubfp(load(horizontal(l.pos0)), pulled))
        };
        // Frame B: direction perm(dir1), origin perm(pos1).
        let (dir_b, origin_b) = (horizontal(l.dir1), horizontal(l.pos1));
        self.pos0_64 = l.pos0;
        self.pos1_80 = l.pos1;
        let target = horizontal(position);
        let facing = emitter.facing_28.map(horizontal);
        let angle = |dir: Vec4, origin: Vec4| -> u32 {
            unsafe {
                let rel = lanes(vmx::vsubfp(load(target), load(origin)));
                match plane_angle(dir, rel) {
                    None => 0,
                    Some((r, cross)) if cross > 0.0 => (65535 - r) as u32,
                    Some((r, _)) => r as u32,
                }
            }
        };
        let a = angle(dir_a, origin_a);
        out.push((3, a));
        out.push((1, unsafe { distance(self.pos0_64, position) }.to_bits()));
        out.push((2, angle(dir_b, origin_b)));
        out.push((0, unsafe { distance(self.pos1_80, position) }.to_bits()));
        match facing {
            Some(fdir) => {
                let prev = angle(dir_a, origin_a) as i32;
                // sub_824AE418: the angle of the facing against (origin − emitter), flipped the
                // other way, then |that − frame A's angle| (a negative result gains 65536).
                let r = unsafe {
                    let rel = lanes(vmx::vsubfp(load(origin_a), load(target)));
                    match plane_angle(fdir, rel) {
                        None => 0,
                        Some((r, cross)) if 0.0 > cross => 65535 - r,
                        Some((r, _)) => r,
                    }
                };
                let mut d = if r > prev { r - prev } else { prev - r };
                if d < 0 {
                    d += 0x10000;
                }
                out.push((10, d as u32));
            }
            None => {
                out.push((5, 0));
                out.push((6, 0));
                out.push((10, 0));
            }
        }
    }

    /// `sub_824AEE60`: distances, signed relative speeds, the sign-change flags, ids 13 and 14.
    fn rates(
        &mut self,
        l: &Listener,
        position: Vec4,
        velocity: Vec4,
        w15: &mut u32,
        out: &mut Vec<(u32, u32)>,
    ) {
        self.dist1_prev_44 = self.dist1_40;
        self.dist0_prev_52 = self.dist0_48;
        self.rate1_prev_100 = self.rate1_96;
        self.rate0_prev_108 = self.rate0_104;
        let (d1, d0, s1, s0) = {
            let mut fpscr = Fpscr::capture();
            fpscr.enable_flush_mode_unconditional();
            unsafe {
                (
                    distance(l.pos1, position),
                    distance(l.pos0, position),
                    distance(velocity, l.vel1),
                    distance(velocity, l.vel0),
                )
            }
        };
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        self.dist1_40 = d1;
        self.dist0_48 = d0;
        self.rate1_96 = s1;
        self.rate0_104 = s0;
        if self.dist1_prev_44 > d1 {
            self.rate1_96 = mul_single(f64::from(s1), -1.0) as f32;
        }
        if self.dist0_prev_52 > d0 {
            self.rate0_104 = mul_single(f64::from(s0), -1.0) as f32;
        }
        let flips = |p: f32, n: f32| (p < 0.0 && n > 0.0) || (p > 0.0 && n < 0.0);
        if flips(self.rate1_prev_100, self.rate1_96) {
            *w15 |= 0x8000_0000;
            out.push((15, *w15));
        }
        if flips(self.rate0_prev_108, self.rate0_104) {
            *w15 |= 0x4000_0000;
            out.push((15, *w15));
        }
        out.push((13, self.rate1_96.to_bits()));
        out.push((14, self.rate0_104.to_bits()));
    }
}

// ---------------------------------------------------------------------- owners

/// `sub_824C28B0` (Rail, vtable slot 9): id 0 = grinding (`state+341`), id 1 = grind just ended
/// (`+342` last frame's 341, and not 341 now).
pub fn rail_inputs(grinding_341: bool, grinding_prev_342: bool) -> [(u32, u32); 2] {
    [
        (0, if grinding_341 { 32767 } else { 0 }),
        (
            1,
            if grinding_prev_342 && !grinding_341 {
                32767
            } else {
                0
            },
        ),
    ]
}

/// `sub_824E9270` (OffBoard process): id 0 = `state+716` (the skater state is 500, on foot), only
/// while the object's `+28` link exists and its `+52` byte is set (true for the local player).
pub fn off_board_input(on_foot_716: bool) -> (u32, u32) {
    (0, if on_foot_716 { 32767 } else { 0 })
}

/// `sub_824EC3E0` (HandGrabs process): id 0 = the object's `+36` grab byte (set by
/// `sub_824EC488`/`sub_824EC670`/`sub_824EC958` for the local player). Always 0 in the capture.
pub fn hand_grabs_input(grab_36: bool) -> (u32, u32) {
    (0, if grab_36 { 32767 } else { 0 })
}

// ---------------------------------------------------------------------- pause

/// The game-side terms SFXObj_Pause's process `sub_824E1D00` tests. `sys` is `*(0x830CFDC4)`.
///
/// * `request`: the duck request (`r29`). Retail sets it when **all** of these hold:
///   `*(0x830CFE24)` and the world `*(0x83083C38)` exist; `sub_824D4EE0(*(0x830CFE24), 1)` — the
///   singleton's flag bit 0, i.e. `([obj+256] & [obj+260] & 1) != 0` under its `+4` lock;
///   `sub_8279E180(world)` is false (the world has no active child through its `+112` vfunc 16);
///   `[*(0x830CFDBC)]+104` is 0; `sys+468` is 0; and `sub_82487ED0()` is false (game-flow state
///   `sys+1196` is not 7, and not 3-with-`[[sub_824AD240()+8]+56]+60 == 1`). The capture holds it
///   for the whole pause-menu pause.
/// * `mode_1064`: `sys+1064 == 1`, a field of the game-mode block `sys+1056…+1072` that
///   `sub_8252E288` fills from a record (the same block holds the mode `sys+1060` the Treatment
///   component's HOM companion and `sub_824898C8` test). It is 1 through free-skate play in the
///   capture: the pause menu there raises id 0. It was not 1 during the capture's start-up, where
///   the same request raised id 2 instead.
/// * `state_6`: `sys+1196 == 6`, which never happens in the capture (id 1 is 0 throughout).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PauseFields {
    pub request: bool,
    pub mode_1064: bool,
    pub state_6: bool,
}

/// SFXObj_Pause (`40000070`, factory row `0x8302D270`, constructor `sub_824E1B30`, vtable
/// `0x822FC1B8`): only the process slot 9 (`sub_824E1D00`) does anything — slot 10 is the stub
/// `0x82B61BB8`, so the class reads no outputs and posts no message.
///
/// The process takes no dt and has no slew: the fade is the MixMap's own. In the capture, the
/// evaluation after id 0 goes to 32767 starts a 175 ms / 12-evaluation ramp that takes every
/// player controller's level outputs to 0 (SkateBoard, Contacts, Wheels, Rail, Cracks, Tricks,
/// Clothing, Treatments, SenseOfSpeed, OffBoard), and clearing it brings them back over 88 ms /
/// 6 evaluations. Through the ported MixMap at a fixed 1/60, SkateBoard's id 21 reaches 0 11 ticks
/// after id 0 is set; id 2 mutes in 2 ticks instead; id 1 changes no level output.
///
/// **Retail ducks and keeps running.** Through the capture's whole pause (evaluations 8895–12843)
/// the bridge still publishes a state every evaluation, the audio state's time scale `+220` stays
/// 1.0 and its `+224` byte stays 0 (both are 1.0 and 0 in all 18553 of the capture's states, so
/// neither is how retail silences a pause), the state's physics values are frozen at their last
/// pre-pause values (ground speed 5.390 throughout), and every component keeps posting its packet
/// redelivery each evaluation (`Class_rolling` twice, `Class_wheels_skid`, `Class_Treatment`,
/// `SenseOfSpeed_wind` once) with 3 posts and 4 releases in those 3949 evaluations. Nothing is
/// skipped and no voice is stopped: the sound goes away only because of this controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pause {
    /// `this+28`, the id-2 latch. The constructor sets it (`li r5,1 ; stb r5,28(r3)`).
    pub armed_28: bool,
}

impl Default for Pause {
    fn default() -> Self {
        Self::retail()
    }
}

impl Pause {
    /// `sub_824E1B30`.
    pub fn retail() -> Self {
        Self { armed_28: true }
    }

    /// One process call: ids 0, 1, 2 in the order `sub_824E1D00` writes them.
    ///
    /// With `mode_1064` the request goes straight to id 0. Without it the first requesting frame
    /// only clears the latch and the following ones hold id 2 (the capture's start-up path, and
    /// its three 1–2 evaluation blips during play). Releasing the request re-arms the latch.
    pub fn process(&mut self, fields: PauseFields) -> [(u32, u32); 3] {
        let mut id0 = 0;
        let mut id2 = 0;
        if fields.request {
            if fields.mode_1064 {
                id0 = 32767;
            } else if self.armed_28 {
                self.armed_28 = false;
            } else {
                id2 = 32767;
            }
        } else {
            self.armed_28 = true;
        }
        [
            (0, id0),
            (1, if fields.state_6 { 32767 } else { 0 }),
            (2, id2),
        ]
    }
}

/// The free-skate Pause inputs: `Pause::process` with retail's free-skate terms
/// (`mode_1064` = true, `state_6` = false), which needs no state because that path never touches
/// the id-2 latch. This is what the capture's pause menu writes — id 0 alone — and what an engine
/// with a menu/pause state should write every frame, paused or not.
pub fn pause_inputs(paused: bool) -> [(u32, u32); 3] {
    Pause::retail().process(PauseFields {
        request: paused,
        mode_1064: true,
        state_6: false,
    })
}

// ---------------------------------------------------------------------- the combo multiplier

/// The multiplier tier bits of the audio frame record's flags word (`*(0x83083C38) + 0x2F0D0`,
/// the record at `+0x2F0B0` word `+32`): exactly one is set while the local player's combo
/// multiplier is at least 1.5.
pub const MULTIPLIER_X1_5: u32 = 0x8000;
pub const MULTIPLIER_X2: u32 = 0x4000;
pub const MULTIPLIER_X3: u32 = 0x2000;

/// `0x82063B08` (3.0), `0x82060C50` (2.0), `0x822249B4` (1.5): the tier thresholds.
const TIER_X3: u32 = 0x4040_0000;
const TIER_X2: u32 = 0x4000_0000;
const TIER_X1_5: u32 = 0x3FC0_0000;

/// `sub_827A2E88` (the audio frame record builder, run for the local player by `sub_827A11B0`):
/// clear bits `0xE000` of the record's `+32`, then — when the player's score object exists —
/// read the multiplier the score module publishes (`[record+60]+56`, stored by `sub_82DA4238`
/// from the combo timer's `+32` at module `+44`: the engine's `Session::combo.multiplier`) and set
///
/// ```text
/// fcmpu m, 3.0 ; blt → ori 0x2000
/// fcmpu m, 2.0 ; blt → ori 0x4000
/// fcmpu m, 1.5 ; blt → ori 0x8000
/// ```
///
/// (a NaN sets `0x2000`, as the unordered compare falls through). Returns the three bits only;
/// the builder's other `+32` bits (`0x1000`, `0x800`, `0x400`, `0x200`, the top five) feed no
/// player-audio reader.
pub fn multiplier_flags(multiplier: Option<f32>) -> u32 {
    let Some(m) = multiplier else { return 0 };
    if !(m < f(TIER_X3)) {
        MULTIPLIER_X3
    } else if !(m < f(TIER_X2)) {
        MULTIPLIER_X2
    } else if !(m < f(TIER_X1_5)) {
        MULTIPLIER_X1_5
    } else {
        0
    }
}

/// `sub_824898C8(sys)`: the "x3" predicate the Music and Tricks writers test first — flag
/// `0x2000`, or `mode_terms` = (`[0x830CFDC4]+932` byte and the byte at `+476`) or game mode
/// `+1060 == 8`. `mode_terms` is false in free skate (the capture's Music id 3 is 0 whenever the
/// Flips emphasis is below the x3 target).
pub fn multiplier_x3(flags: u32, mode_terms: bool) -> bool {
    flags & MULTIPLIER_X3 != 0 || mode_terms
}

/// Music (`40000010`) id 6's targets, the tuning record `C1831BDB6CB1B1EA`/`47EC76B4F9FC79F6`
/// (the one the Flips emphasis also reads): `6EE4718F1A7EB772` = 5000 (flag `0x8000`),
/// `50F6520E2DD3D54C` = 12000 (`0x4000`), `A6AA0C534DEA29E7` = 32767 (`0x2000` or x3).
pub const MUSIC_EMPHASIS_TARGETS: [i32; 3] = [5000, 12000, 32767];
/// Its slew rates per second: down `7C44AE016D99A9EE` (9000.0), up `1666A4A45EC309AE` (3000.0).
pub const MUSIC_EMPHASIS_DOWN: u32 = 0x460C_A000;
pub const MUSIC_EMPHASIS_UP: u32 = 0x453B_8000;

/// The multiplier half of SFXObj_Music's process `sub_824D1208(this, dt)` (vtable `0x822FC4A0`
/// slot 9, first half): id 3 = 32767 while [`multiplier_x3`], and id 6 = the component's `+176`
/// slewed toward the tier's [`MUSIC_EMPHASIS_TARGETS`] entry (0 below x1.5). The process's other
/// ids (0, 1, 2, 4, 5, 7, 8, 9) come from the music player and game state and are not here.
///
/// The emphasis is not music: the local player's SkateBoard (ids 21, 22 — the board grain
/// chain's local levels), Wheels (4), Rail (6), Contacts (1) and Tricks (6, 7, 8) outputs all
/// read Music ids 3 and 6. With the other inputs at their free-skate values, SkateBoard 21/22 go
/// 1267/1835 (x1) → 1287/1865 (x1.5) → 1969/2853 (x2, the capture's values at the x2 plateau) →
/// 28343/32730 (x3), Wheels 4 goes 3046 → 3183 → 7436 → 32730 and Rail 6 1636 → 1649 → 2129 →
/// 20580.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MusicEmphasis {
    /// Component `+176`: the slewed id-6 value (unclamped; the write clamps to 0..=32767).
    pub value_176: i32,
}

impl MusicEmphasis {
    /// One process call. `flags` from [`multiplier_flags`]; `mode_terms` as in
    /// [`multiplier_x3`]; `dt` the frame time the process gets. Returns ids 3 and 6 in write
    /// order.
    pub fn process(&mut self, flags: u32, mode_terms: bool, dt: f32) -> [(u32, u32); 2] {
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        let x3 = multiplier_x3(flags, mode_terms);
        let id3 = if x3 { 32767 } else { 0 };
        // `fcmpu dt, 0.0 ; ble` skips the slew with r31 = 0 (also for a NaN dt).
        let mut next = 0;
        if dt > 0.0 {
            let target = if x3 {
                MUSIC_EMPHASIS_TARGETS[2]
            } else if flags & MULTIPLIER_X1_5 != 0 {
                MUSIC_EMPHASIS_TARGETS[0]
            } else if flags & MULTIPLIER_X2 != 0 {
                MUSIC_EMPHASIS_TARGETS[1]
            } else {
                0
            };
            // r30 = fctiwz(down × dt), r10 = fctiwz(dt × up).
            let down = fctiwz_low_word(mul_single(f64::from(f(MUSIC_EMPHASIS_DOWN)), f64::from(dt)))
                as i32;
            let up =
                fctiwz_low_word(mul_single(f64::from(dt), f64::from(f(MUSIC_EMPHASIS_UP)))) as i32;
            let current = self.value_176;
            next = target;
            if target < current {
                if current.wrapping_sub(target) > down {
                    next = current.wrapping_sub(down);
                }
            } else if target > current && target.wrapping_sub(current) > up {
                next = current.wrapping_add(up);
            }
        }
        self.value_176 = next;
        [(3, id3), (6, next.clamp(0, 32767) as u32)]
    }
}

// ---------------------------------------------------------------------- globals

/// The global controllers' inputs in the capture after the game started (evaluations 2709–21253,
/// 18545 evaluations): every input id that is ever non-zero, with its non-zero count, its number
/// of changes and its modal value. Real ids are 0–15 (key & 0xF); the capture's input words
/// 16–23 belong to the next controller's block. Each class's writer is its vtable slot 9 process.
///
/// | key | class (writer) | non-zero ids: count / changes / mode | free-skate handling |
/// |---|---|---|---|
/// | `40000000` | Announcer | none | — |
/// | `40000010` | Music (`sub_824D1208`) | 0: 1966/1413/0 · 1: 16579/8/32767 · 2: 18545/0/32767 · 3: 1296/6/0 · 5: 13531/3/32767 · 6: 6305/3499/0 | 3, 6 = [`MusicEmphasis`] (the combo multiplier); 1, 2, 5 constant; 0 = [`FREE_SKATE_MUSIC_VU`] |
/// | `40000020` | Master (`sub_824D5160`) | 1–4: always 32767 · 9: 523/16/0 · 10: 3176/60/0 | 1–4 constant; 9, 10 (voice activity, no player reader) left 0 |
/// | `40000030` | CameraMan | none | — |
/// | `40000050` | Reverb (`sub_824DF468`) | 4: 198/6/0 · 5: 12571/18/32767 · 6: 5776/12/0 | 5 = 32767 (mode); 4 and 6 follow the frame record's `+16` reverb key (`sub_824DE548`; set by `sub_827A2E88` from the world region lookup `sub_82C0EAC0` type 10 at the player's position) — not ported |
/// | `40000060` | NIS (`sub_824E1230`) | 9: 579/6/0 | 0 (id 9 = `sub_82487ED0`: game-flow state `[0x830CFDC4]+1196` = 7, or 3 with a sub-state; not free skate) |
/// | `40000070` | Pause (`sub_824E1D00`) | 0: 3949/2/0 (one pause-menu pause) · 2: 4/6/0 (start-up and three 1–2 evaluation blips) | [`pause_inputs`] every frame |
/// | `40000080` | Speech (`sub_824E2050`) | 1: 61/2/0 · 4: 372/10/0 | 0 |
/// | `40000090` | Bloom | none | — |
/// | `400000A0` | VU (`sub_824EDBE8`) | 0: 17476/9504/32767 | [`FREE_SKATE_MUSIC_VU`] |
/// | `400000B0`–`400000D0` | Challenge, HOM, Menu | none | — |
/// | `400000E0` | Jitter (`sub_824EF378`, generators `sub_824EF4C8`, vault in `sub_824EF0B8`) | 0–4 random every frame | [`Jitter`] |
/// | `400D0000…` | PlayerSpeech | group 1 id 0: 310/8/0 · group 2 id 0: 61/2/0 | 0 |
///
/// The player controllers (`40010000`–`40010090`) read, of the globals: Announcer 0, Music 3, 5,
/// 6, Master 0–6 and 8, CameraMan 0, Reverb 0 and 4, NIS 0–2, 4, 5, 7–10, 13, Pause 0–2, Speech 1,
/// Bloom 0, VU 0–1, Challenge 1, 4–9, 11, HOM 0, 2–4, Menu 8 and Jitter 0–3 — not Master 9/10 or
/// Reverb 5/6. Of the ids that vary in the capture, Music 3/6, Reverb 4, VU 0, NIS 9, Pause 0 and
/// Speech 1 reach them: Music 3/6 ([`MusicEmphasis`]) and Pause 0 ([`pause_inputs`]) are ported;
/// NIS 9 is a game-flow state; Reverb 4 (the world's reverb zone), VU 0 (the output meters) and
/// Speech 1 (61 evaluations, writer not traced) are not.
///
/// **Music on the player's path.** Music ids 3 and 6 are the combo multiplier, ported as
/// [`MusicEmphasis`] (see [`multiplier_flags`]). Id 0 is music playback (the music player's
/// state), and `sub_824EDBE8` writes VU id 0 as a slew-limited level of the audio system's
/// output meters (`[sys+28]+4 → +16`, five channel levels summed, × 0.2 (the double at
/// `0x822F8B00`) × a vault gain from the tuning holder's `+100` instance, ×32767). With no retail
/// music playing, those use [`FREE_SKATE_MUSIC_VU`] — the session's most frequent values, **not a
/// derived state**.
///
/// **The state controller's id 11 (G+16).** Its writer was not found: G is
/// `*(*(0x83083C38) + 0x2FCB4)`, a camera-side object whose `+16` sub-object is read by
/// `sub_827AE9C8`/`sub_827AEF08`/`sub_827AF4E0`/`sub_827F1DD0` (and whose `+24` float the Treatment
/// updater reads); no store to its first byte was found in them or their callees. In the capture it
/// rises on the frame a bail starts (`+676`) and falls 92–270 evaluations later (with the bail's end
/// in two of six cases).
pub const FREE_SKATE_GLOBALS: &[(u32, u32, u32)] = &[
    (0x4000_0010, 1, 32767),
    (0x4000_0010, 2, 32767),
    (0x4000_0010, 5, 32767),
    (0x4000_0020, 1, 32767),
    (0x4000_0020, 2, 32767),
    (0x4000_0020, 3, 32767),
    (0x4000_0020, 4, 32767),
    (0x4000_0050, 5, 32767),
];

/// Music (`40000010`) id 0 and VU (`400000A0`) ids 0, 1 at their most frequent free-skate values
/// in the capture (Music 0 in 89% of evaluations; VU 1 always 0; VU 0 has no dominant value —
/// 32767 in 24% of evaluations, the rest spread over a level, so its entry is only the most
/// frequent). A labelled default for a runtime without retail music, not a retail rule. Music ids
/// 3 and 6 are not here: [`MusicEmphasis`] writes them every frame.
pub const FREE_SKATE_MUSIC_VU: &[(u32, u32, u32)] = &[
    (0x4000_0010, 0, 0),
    (0x4000_00A0, 0, 32767),
    (0x4000_00A0, 1, 0),
];

// ---------------------------------------------------------------------- the listener object

impl Listener {
    /// `sub_8248CC08(listener, dt)`, called by `sub_82485190` in the first half before the slot
    /// objects' vfunc 16 (so before every position controller's update).
    ///
    /// * `camera_row2`/`camera_row3`: rows 2 and 3 of the camera's world matrix as
    ///   `sub_827A0C70(camera at *(0x83083C38)+0x2FD20)` returns it — row 3 is the camera position,
    ///   row 2 the **view direction** (the capture's own position controller, the skater in front of
    ///   the camera, reads frame-A angle ≈ 0, which fixes the sign). Row 2 is normalised here.
    /// * `camera_counter_changed`: whether the camera's frame counter (`[*(0x83083C38)+112]`
    ///   vfunc 52, kept at `+128`) changed since the last call; the velocity is only recomputed
    ///   then. For an engine camera that updates every frame, pass `true`.
    /// * `player0`: the audio-state record array's first record (`*(0x83083C38)+0x2F070`, `+8`
    ///   pointer, when its count `+12` is non-zero): its `+0` position, `+16` facing and `+32`
    ///   velocity.
    pub fn update(
        &mut self,
        camera_row2: Vec4,
        camera_row3: Vec4,
        camera_counter_changed: bool,
        dt: f32,
        player0: Option<&PlayerRecord>,
    ) {
        let mut fpscr = Fpscr::capture();
        fpscr.enable_flush_mode_unconditional();
        self.prev_pos0_16 = self.pos0;
        self.pos0 = camera_row3;
        self.dir0 = normalize3(camera_row2);
        if camera_counter_changed {
            let diff = unsafe { lanes(vmx::vsubfp(load(self.pos0), load(self.prev_pos0_16))) };
            if dt == 0.0 {
                self.vel0 = [0.0; 4];
            } else {
                let inv = fp::div_single(1.0, f64::from(dt)) as f32;
                self.vel0 = unsafe { lanes(vmx::vmulfp(load(diff), splat(inv))) };
            }
        }
        if let Some(r) = player0 {
            self.prev_pos1_80 = self.pos1;
            self.pos1 = r.position_0;
            self.dir1 = r.facing_16;
            self.vel1 = r.velocity_32;
        }
    }
}

/// The audio-state record the bridge `sub_827A1B78` fills per player (record = base + 240 + 544 ×
/// index): the vectors the listener and the position controllers use. `B` is the PhysOut bundle
/// (`[[player+1808]]->vfunc92()`).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerRecord {
    /// `+0` = `[B+36]+64` (SystemReckoning position).
    pub position_0: Vec4,
    /// `+16` = `[B+20]+80` (Skeleton).
    pub facing_16: Vec4,
    /// `+32` = `[B+36]+16` (SystemReckoning COM velocity).
    pub velocity_32: Vec4,
    /// `+48` = `[B+0]+144`.
    pub position_48: Vec4,
    /// `+64` = `[B+0]+128`.
    pub facing_64: Vec4,
    /// `+80` = `[B+4]+80` (Motion).
    pub velocity_80: Vec4,
}

impl PlayerRecord {
    /// The two position controllers' bindings, as `sub_824B0C48` (the PlayerPhysics init) makes
    /// them through the 3DObjPos setters `+56` (`sub_82827CD8`, position → `+32`), `+60`
    /// (`sub_82827CD0`, facing → `+28`) and `+64` (`sub_82ABF4E0`, velocity → `+36`), pointed at the
    /// state's copies the bridge refreshes each frame (`state+48/80/96` = record `+0/+16/+32`,
    /// `state+144/160/176` = record `+48/+64/+80`).
    ///
    /// Returns `(60010010, 60010020)`: the state's `+28` child (object id 1) and `+32` child (id 2).
    pub fn emitters(&self) -> (Emitter, Emitter) {
        (
            Emitter {
                position_32: Some(self.position_0),
                velocity_36: Some(self.velocity_32),
                facing_28: Some(self.facing_16),
            },
            Emitter {
                position_32: Some(self.position_48),
                velocity_36: Some(self.velocity_80),
                facing_28: Some(self.facing_64),
            },
        )
    }
}

// ---------------------------------------------------------------------- Jitter (400000E0)

/// One `Sk8::Audio::eJitterParams` channel of SFXObj_Jitter (a 40-byte record at `obj + 40·i`).
#[derive(Clone, Copy, Debug)]
pub struct JitterChannel {
    /// `+32` the collection key (the update runs while it is non-zero).
    pub key: u64,
    /// `+64` field `0x8F956FBAD301AE26`: write the value to the controller.
    pub enabled: bool,
    /// `+68` field `0xE7D491E2EB228F54`: the controller input id.
    pub id: u32,
    /// `+56, +60, +48, +52` field `0xB66AAD957873A8B3` elements 0..3: centre, range, maximum
    /// step, minimum step.
    pub params: [f32; 4],
    /// `+40` the value, `+44` its velocity.
    pub value: f32,
    pub velocity: f32,
}

/// The vault's Jitter channels (class `0x0AB9F005A2C8FBC7`), as `sub_824EF0B8` builds them:
/// `sub_828DA2C8` lists the class's **leaf** collections (keys that are nobody's parent — the
/// set difference of the sorted keys and sorted parents in `sub_828DA0D8`) derived from
/// `default`, in ascending key order; each channel's fields resolve through the parent chain.
/// `(key, enabled, id, params bits)`.
pub const JITTER_VAULT: [(u64, bool, u32, [u32; 4]); 24] = [
    (
        0x02D9546BE518D5A1,
        true,
        4,
        [0x46800000, 0x467FFC00, 0x42C80000, 0x3F800000],
    ),
    (
        0x0DFA1456B4DE6D74,
        false,
        0,
        [0x3F800000, 0x3F000000, 0x3EE66666, 0x3E800000],
    ),
    (
        0x25355B0B6BBB82A7,
        false,
        0,
        [0x3F800000, 0x3F400000, 0x3F3D70A4, 0x3F266666],
    ),
    (
        0x5FBCB3B89B066128,
        true,
        3,
        [0x46800000, 0x467FFC00, 0x455AC000, 0x44BB8000],
    ),
    (
        0x655F9BEE06BE269C,
        false,
        0,
        [0x43C80000, 0x437A0000, 0x437A0000, 0x43480000],
    ),
    (
        0x6AEEEEB9D17C464D,
        true,
        0,
        [0x46800000, 0x467FFC00, 0x46E29000, 0x46A41000],
    ),
    (
        0x7253590E0399A524,
        false,
        0,
        [0x3F800000, 0x3F000000, 0x3EE66666, 0x3E800000],
    ),
    (
        0x7276EC7A5EA8BBDC,
        false,
        0,
        [0x45ABE000, 0x45A41000, 0x457A0000, 0x4708B800],
    ),
    (
        0x816A58ECCCD4112D,
        false,
        0,
        [0x44FA0000, 0x44BB8000, 0x43FA0000, 0x432F0000],
    ),
    (
        0x88008F15F2A4394A,
        false,
        0,
        [0x3FC00000, 0x3F800000, 0x3F733333, 0x3E800000],
    ),
    (
        0x8802BC5475904597,
        false,
        0,
        [0x3FE00000, 0x3FA00000, 0x3F800000, 0x3E800000],
    ),
    (
        0xB5E641CC0B983A3F,
        true,
        1,
        [0x46800000, 0x467FFC00, 0x46EA6000, 0x469C4000],
    ),
    (
        0xC20C6630396E9EDB,
        true,
        5,
        [0x00000000, 0x00000000, 0x00000000, 0x00000000],
    ),
    (
        0xC27373E7FD2DCE47,
        false,
        0,
        [0x3FA00000, 0x3F400000, 0x3F400000, 0x3E800000],
    ),
    (
        0xD4ED2F0BA77ACAB5,
        false,
        0,
        [0x40400000, 0x40300000, 0x40200000, 0x3F000000],
    ),
    (
        0xDB37CAD093CEAC1D,
        false,
        0,
        [0x3F800000, 0x3F400000, 0x3F333333, 0x3F000000],
    ),
    (
        0xE0A49CF4F825146B,
        false,
        0,
        [0x3FC00000, 0x3FA00000, 0x3F9EB852, 0x3F800000],
    ),
    (
        0xE17029CE4388E1E8,
        false,
        0,
        [0x43C80000, 0x43960000, 0x43958000, 0x43470000],
    ),
    (
        0xE58A308607FDB428,
        false,
        0,
        [0x40200000, 0x40100000, 0x40000000, 0x3FC00000],
    ),
    (
        0xE5C4194AB80920EA,
        false,
        0,
        [0x459C4000, 0x455AC000, 0x451C4000, 0x43FA0000],
    ),
    (
        0xEA2ED27D25247927,
        false,
        0,
        [0x40400000, 0x40200000, 0x4019999A, 0x3F800000],
    ),
    (
        0xF88F9621DE824070,
        false,
        0,
        [0x43C80000, 0x43960000, 0x43938000, 0x43160000],
    ),
    (
        0xFA0359BD8FB40469,
        false,
        0,
        [0x40800000, 0x40600000, 0x4059999A, 0x40400000],
    ),
    (
        0xFB7567C027EF051C,
        true,
        2,
        [0x46800000, 0x467FFC00, 0x46F23000, 0x46947000],
    ),
];

/// `0x82063A48` 0.001: the random step's unit.
const JITTER_UNIT: u32 = 0x3A83_126F;

/// SFXObj_Jitter's channels (`obj+1232` = their count).
#[derive(Clone, Debug)]
pub struct Jitter {
    pub channels: Vec<JitterChannel>,
}

impl Jitter {
    /// `sub_824EF0B8`: every channel starts at its centre with zero velocity.
    pub fn retail() -> Self {
        let channels = JITTER_VAULT
            .iter()
            .map(|&(key, enabled, id, p)| {
                let params = [f(p[0]), f(p[1]), f(p[2]), f(p[3])];
                JitterChannel {
                    key,
                    enabled,
                    id,
                    params,
                    value: params[0],
                    velocity: 0.0,
                }
            })
            .collect();
        Self { channels }
    }

    /// `sub_824EF378` (SFXObj_Jitter's process, vtable `0x822FC248` slot 9): advance every channel
    /// with one draw of the shared generator (`sub_82A8AF10`, [`crate::grain::rng`], state in guest
    /// memory — the same one the grain player and every other caller advance), then write the
    /// enabled ones as `clamp(fctiwz(clamp(value, 0, 32767)), 0, 32767)`.
    pub fn process(&mut self, g: &mut crate::Guest) -> crate::Result<Vec<(u32, u32)>> {
        let mut out = Vec::new();
        for ch in &mut self.channels {
            if ch.key != 0 {
                let r = crate::grain::rng::next(g)?;
                ch.step(r);
            }
            if ch.enabled {
                let mut fpscr = Fpscr::capture();
                fpscr.disable_flush_mode_unconditional();
                let v = f64::from(ch.value);
                let lo = fsel(-v, 0.0, v);
                let k = f64::from(f(F32767));
                let hi = fsel(fp::sub_single(k, lo), lo, k);
                out.push((ch.id, (fctiwz_low_word(hi) as i32).clamp(0, 32767) as u32));
            }
        }
        Ok(out)
    }
}

impl JitterChannel {
    /// `sub_824EF4C8`: a bounded random walk. `r mod 2001 − 1000` scales `(max − min) × 0.001`,
    /// pushed away from zero by the minimum step, added to the velocity (clamped to ±max), the
    /// velocity added to the value, bouncing off `centre ± range`, then the value clamped there.
    pub fn step(&mut self, r: u32) {
        let mut fpscr = Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        let [p0, p1, p2, p3] = self.params.map(f64::from);
        // mulhwu 0x0603538B ; subf ; srwi 1 ; add ; srwi 10 ; mulli 2001: r mod 2001.
        let hi = ((u64::from(r) * 0x0603_538B) >> 32) as u32;
        let t = (r.wrapping_sub(hi) >> 1).wrapping_add(hi);
        let q = t >> 10;
        let s = (r.wrapping_sub(q.wrapping_mul(2001)) as i32).wrapping_sub(1000);
        let span = fp::sub_single(p2, p3);
        let unit = mul_single(fp::word_to_single(s as u32), f64::from(f(JITTER_UNIT)));
        let d = mul_single(span, unit);
        let push = if d < 0.0 {
            fp::sub_single(d, p3)
        } else {
            fp::add_single(p3, d)
        };
        let vel = fp::add_single(push, f64::from(self.velocity));
        let neg = -p2;
        let lo = fsel(fp::sub_single(neg, vel), neg, vel);
        let v = fsel(fp::sub_single(p2, lo), lo, p2);
        self.velocity = v as f32;
        let upper = fp::add_single(p0, p1);
        let mut x = fp::add_single(v, f64::from(self.value));
        if x > upper {
            let over = fp::sub_single(x, upper);
            self.velocity = mul_single(v, -1.0) as f32;
            x = fp::sub_single(upper, over);
        } else {
            let lower = fp::sub_single(p0, p1);
            if x < lower {
                let under = fp::sub_single(lower, x);
                self.velocity = mul_single(v, -1.0) as f32;
                x = fp::add_single(under, lower);
            }
        }
        let lower = fp::sub_single(p0, p1);
        let a = fsel(fp::sub_single(lower, x), lower, x);
        self.value = fsel(fp::sub_single(upper, a), a, upper) as f32;
    }
}

// ---------------------------------------------------------------------- Contacts (40010010)

/// Materials (the state's per-wheel `+620` values, 0..142) whose AudioSurface collection (key from
/// the image table `0x8302D6E8 + 8 + 16·m`, class `0xD40CB4C0FFE45676`) has field
/// `0x1EBF9D2EB0DD56BA` (layout `+124`, a bool) set through its parent chain. Material 94's key is
/// 0 (no collection: false).
pub const LANDING_FLAG_MATERIALS: &[u32] = &[
    7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 46, 47, 48, 49, 50, 54, 55, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 81, 83, 84, 87, 88, 90, 93, 114, 115, 116, 117, 118, 119, 120, 121, 122,
];

/// What SFXObj_Contacts's inputs read from the audio state.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContactsFields {
    /// `+332` airborne, `+341` grinding.
    pub airborne_332: bool,
    pub grinding_341: bool,
    /// `+464..+467` per-wheel contact bytes (`[B+40]` bytes 68–71).
    pub wheel_contact_464: [bool; 4],
    /// `+448..+460` per-wheel words (the landing class: 1 → 16000, 2 → 32767).
    pub wheel_word_448: [i32; 4],
    /// `+620..+632` per-wheel material (143 = none).
    pub wheel_material_620: [u32; 4],
    /// `[[obj+28]+72]`: the local player.
    pub local_player: bool,
}

/// SFXObj_Contacts's MixMap inputs (only ids 1, 2 and 6 reach any output — the dependency walk of
/// `examples/mixmap_dump.rs`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Contacts {
    /// `+120`: last frame's `+332`.
    pub was_airborne_120: bool,
}

impl Contacts {
    /// `sub_824B90D8` (the process): ids 0, 1 and 6 cleared, then on a landing (was airborne, now
    /// neither airborne nor grinding) `sub_824BA630`: id 1 = 32767; id 6 = 32767 for the local
    /// player when the first contacting wheel with a material (≠ 143) is on a
    /// [`LANDING_FLAG_MATERIALS`] surface; id 2 from the contacting wheels' largest `+448` word
    /// (1 → 16000, 2 → 32767, else 0; 0 with no wheel in contact). Id 2 is written only on a
    /// landing and holds in between.
    ///
    /// Not ported (their inputs reach no output): id 0 (`sub_824B9CC8`, pops), 3–5
    /// (`sub_824B86E0`), 7–8 (`sub_824BC188`), 9 (`sub_824C0AB8`), and `sub_824BA630`'s sound posts.
    pub fn process(&mut self, s: &ContactsFields) -> Vec<(u32, u32)> {
        let mut out = vec![(0, 0), (1, 0), (6, 0)];
        if self.was_airborne_120 && !s.airborne_332 && !s.grinding_341 {
            out.push((1, 32767));
            let mut material = 143u32;
            for i in 0..4 {
                if s.wheel_contact_464[i] {
                    material = s.wheel_material_620[i];
                    if material != 143 {
                        break;
                    }
                }
            }
            if material < 143 && s.local_player && LANDING_FLAG_MATERIALS.contains(&material) {
                out.push((6, 32767));
            }
            let (mut best, mut count) = (0i32, 0);
            for i in 0..4 {
                if s.wheel_contact_464[i] {
                    count += 1;
                    if s.wheel_word_448[i] > best {
                        best = s.wheel_word_448[i];
                    }
                }
            }
            let id2 = match (count, best) {
                (0, _) => 0,
                (_, 1) => 16000,
                (_, 2) => 32767,
                _ => 0,
            };
            out.push((2, id2));
        }
        self.was_airborne_120 = s.airborne_332;
        out
    }
}

#[cfg(test)]
mod tests;
