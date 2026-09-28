//! The MixMap's shared leaves: the linear→millibel and millibel→linear table walks every stage
//! inlines, the ten input curves, and the three float helpers.
//!
//! Every constant and table is read from guest memory at its image address; nothing is copied into
//! Rust. The two integer conversions are not guest *functions* — the compiler inlined them at every
//! site (`sub_8294F5E8` twice, `sub_82951658`, `sub_82950250` four times, `sub_8294FC88` three
//! times, `sub_82951E00`, `sub_829528A0`, `sub_8294AF70` twice) — but every inlined copy was read
//! and each one's jump table was dumped from the image and checked to map case *c* to the body that
//! subtracts `602 × (c + 1)` (linear→mB) or shifts by *k* (mB→linear). They are factored here once
//! because the copies are the same instruction sequence; each call site passes the default its own
//! copy loads into the result register before the range check.

use crate::fp::{self, fctiwz_low_word, load_single, mul_single, word_to_single};
use crate::{Error, Guest, Result};

/// `lis r11,-32002 ; addi r31,r11,-16464`: the millibel table the linear→mB walk indexes.
pub const LOG_TABLE: u32 = 0x82FD_BFB0;
/// `lis r11,-32002 ; addi r31,r11,-18872` then `addi r10,r31,2404`: the *end* of the descending
/// `32767 × 2^-(m+1)/602` table the mB→linear walk reads backwards from.
pub const POW_TABLE_END: u32 = 0x82FD_BFAC;
/// `lis r10,-32002 ; addi r10,r10,-14416`: the 512-entry input curve.
pub const CURVE_TABLE: u32 = 0x82FD_C7B0;
/// `sub_8294B4D8`: `2^(i/1200)`, i = 0..99 (`addi r11,r8,-12312`).
pub const CENTS_FINE_TABLE: u32 = 0x82FD_CFE8;
/// `sub_8294B4D8`: `2^(i/12)`, i = 0..11 (`addi r8,r5,-12364`).
pub const CENTS_SEMI_TABLE: u32 = 0x82FD_CFB4;

/// 1.0 (`lis -32206` + -22460).
pub const K_ONE: u32 = 0x8231_A844;
/// 0.0 (`lis -32234` + 23056).
pub const K_ZERO: u32 = 0x8216_5A10;
/// 32767.0 (`lis -32233` + 18428).
pub const K_32767: u32 = 0x8217_47FC;
/// −1.0 (`0x821747FC - 26908`), the B stage's "no source" coordinate.
pub const K_NEG_ONE: u32 = 0x8216_DEE0;
/// 1/32767 (`0x822F8600 + 664`).
pub const K_INV_32767: u32 = 0x822F_8898;
/// 4096.0 (`0x822F8600 + 668`).
pub const K_4096: u32 = 0x822F_889C;
/// 2.0 (`lis -32250` + 3152), `sub_8294B4D8`'s octave step.
pub const K_TWO: u32 = 0x8206_0C50;
/// −1.99316 (`0x822F8600 + 1248`), `sub_8294AF70`'s scale above unity.
pub const K_RATIO_ABOVE: u32 = 0x822F_8AE0;
/// +1.99316 (`0x822F8600 + 1252`), `sub_8294AF70`'s scale at or below unity.
pub const K_RATIO_BELOW: u32 = 0x822F_8AE4;
/// 16.666 ms (`0x822F8600 + 1244`), the envelopes' "too short to ramp" threshold.
pub const K_MIN_RAMP_MS: u32 = 0x822F_8ADC;
/// 16.6667 (`0x822F8600 + 2308`): authored envelope times are 60 Hz frames; this makes them ms.
pub const K_FRAMES_TO_MS: u32 = 0x822F_8F04;
/// 1000.0 (`lis -32219` + 28648): dt → ms.
pub const K_MS: u32 = 0x8225_6FE8;
/// 29.97 (`0x822F8600 + 2304`): dt → NTSC frames.
pub const K_FRAMES: u32 = 0x822F_8F00;
/// −0.2 (`lis -32247` + -5508), the B stage's per-evaluation slew toward its ratio.
pub const K_SLEW: u32 = 0x8208_EA7C;
/// 25000.0 (`lis -32239` + 24996), output type 2's scale.
pub const K_25000: u32 = 0x8211_61A4;

/// `divw` as the lifted line guards it: a zero divisor or `INT_MIN / -1` yields 0.
#[inline]
pub(crate) fn divw(a: i32, b: i32) -> i32 {
    if b == 0 || (a == i32::MIN && b == -1) {
        0
    } else {
        a / b
    }
}

/// `mulhw`.
#[inline]
pub(crate) fn mulhw(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 32) as i32
}

/// The inlined linear (0..32767) → millibel walk over [`LOG_TABLE`].
///
/// `cntlzw r9,x ; addi r9,r9,-17 ; cmplwi cr6,r9,14 ; bgt default`, then a 15-way jump on the
/// octave *c*: the first six cases index by `(x - 2^(14-c)) >> (5-c)`; the last nine scale `x` up
/// and build the byte offset with an `rlwimi`, reproduced bit for bit. Each subtracts
/// `602 × (c + 1)`. `default` is whatever the site loaded into the result register first
/// (−10000 at every site ported).
pub fn lin_to_mb(g: &Guest, x: u32, default: i32) -> Result<i32> {
    let c = x.leading_zeros().wrapping_sub(17);
    if c > 14 {
        return Ok(default);
    }
    let xi = x as i32;
    let offset: u32 = match c {
        0 => ((xi.wrapping_sub(16384) >> 5) as u32) << 2,
        1 => ((xi.wrapping_sub(8192) >> 4) as u32) << 2,
        2 => ((xi.wrapping_sub(4096) >> 3) as u32) << 2,
        3 => ((xi.wrapping_sub(2048) >> 2) as u32) << 2,
        4 => ((xi.wrapping_sub(1024) >> 1) as u32) << 2,
        5 => (xi.wrapping_sub(512) as u32) << 2,
        _ => {
            // rlwinm r11,x,s,0,31-s ; li r10,L ; addi r9,r11,-K ; rlwimi r10,r9,2,0,29-s
            let s = c - 5;
            const K: [u32; 9] = [511, 509, 505, 497, 481, 449, 385, 257, 1];
            let r11 = x << s;
            let r9 = r11.wrapping_sub(K[(s - 1) as usize]);
            let low = ((1u32 << s) - 1) << 2;
            let mask = !((1u32 << (s + 2)) - 1);
            ((r9 << 2) & mask) | low
        }
    };
    let entry = g.u32(LOG_TABLE.wrapping_add(offset))? as i32;
    Ok(entry.wrapping_sub(602 * (c as i32 + 1)))
}

/// The inlined millibel → linear walk: the register the sites then use (`32767 - r` at the
/// constant builders, the raw `r` at the output writer).
///
/// `mulhw x,0x1B37484B ; srawi 6 ; +sign` is `x / 602` truncated, `divw x,-602` the octave *k*,
/// `rem = -(x - q·602)`; *k* above 15 (unsigned) yields 0, otherwise the table word
/// `POW_TABLE_END - 4·rem` shifted right by *k*.
pub fn mb_to_lin(g: &Guest, x: i32) -> Result<i32> {
    let k = divw(x, -602);
    let t = mulhw(x, 0x1B37_484B) >> 6;
    let q = t.wrapping_add(((t as u32) >> 31) as i32);
    let rem = x.wrapping_sub(q.wrapping_mul(602)).wrapping_neg();
    if k as u32 > 15 {
        return Ok(0);
    }
    let at = POW_TABLE_END.wrapping_sub((rem as u32) << 2);
    Ok((g.u32(at)? as i32) >> k)
}

/// The interpolation fraction both table curves use: `li r8,1023 ; rlwimi r8,x,9,18,21`.
#[inline]
fn curve_fraction(x: i32) -> i32 {
    (((x as u32).rotate_left(9) & 0x3C00) | 1023) as i32
}

/// `sub_8294B668(x, kind)` — the input shaper. Kinds, from the jump table at `0x8294B68C`:
/// 0 table, 1 table(1−x), 2 table², 3 table²(1−x), 4 mirrored table, 5 mirrored(1−x),
/// 6 mirrored², 7 mirrored²(1−x), 8 1−x, 9 x (as `8` of `1−x`). A kind above 9 would jump through
/// the image past the table; every caller checks `kind <= 9` first, so it is an error here.
pub fn curve(g: &Guest, x: i32, kind: u32) -> Result<i32> {
    let inv = 32767i32.wrapping_sub(x); // subfic r3,r3,32767
    match kind {
        0 => {
            let r11 = x >> 6; // srawi. r11,r3,6
            if r11 < 0 || r11 >= 511 {
                return Ok(0);
            }
            let r9 = (r11 as u32) << 2;
            let t = g.u32(CURVE_TABLE.wrapping_add(r9))? as i32;
            if t == 0 {
                return Ok(0);
            }
            let t1 = g.u32(CURVE_TABLE.wrapping_add(4).wrapping_add(r9))? as i32;
            Ok((t1.wrapping_sub(t).wrapping_mul(curve_fraction(x)) >> 15).wrapping_add(t))
        }
        1 => curve(g, inv, 0),
        2 => {
            let y = curve(g, x, 0)?;
            Ok(y.wrapping_mul(y) >> 15)
        }
        3 => curve(g, inv, 2),
        4 => {
            let r10 = 511i32.wrapping_sub(x >> 6);
            let r8 = (r10 as u32) << 2;
            let r5 = g.u32(CURVE_TABLE.wrapping_add(r8))? as i32;
            let r4 = g.u32(CURVE_TABLE.wrapping_add(4).wrapping_add(r8))? as i32;
            let a = 32767i32.wrapping_sub(r5);
            let b = 32767i32.wrapping_sub(r4);
            Ok((b.wrapping_sub(a).wrapping_mul(curve_fraction(x)) >> 15).wrapping_add(a))
        }
        5 => curve(g, inv, 4),
        6 => {
            let y = curve(g, x, 4)?;
            Ok(y.wrapping_mul(y) >> 15)
        }
        7 => curve(g, inv, 6),
        8 => Ok(inv),
        9 => curve(g, inv, 8),
        _ => Err(Error::new(
            0x8294_B668,
            format!("curve kind {kind} jumps past the table"),
        )),
    }
}

/// `sub_8294B5F8(kind, f1)`: the curve over a 0..1 float, back to 0..1.
///
/// `fctiwz(f1 × 32767)` into [`curve`] (0 for a kind above 9), then `float(r) × (1/32767)`.
pub fn curve_float(g: &Guest, kind: u32, f1: f64) -> Result<f64> {
    let r3 = if kind <= 9 {
        let v = fctiwz_low_word(mul_single(f1, load_single(g, K_32767)?)) as i32;
        curve(g, v, kind)?
    } else {
        0
    };
    Ok(mul_single(
        word_to_single(r3 as u32),
        load_single(g, K_INV_32767)?,
    ))
}

/// `sub_8294AF70(f1)`: an amplitude ratio → a scaled millibel integer.
///
/// Above 1.0 it converts `32767 / f1` and scales by −1.99316; otherwise `f1 × 32767` and +1.99316.
pub fn ratio_to_mb(g: &Guest, f1: f64) -> Result<i32> {
    let one = load_single(g, K_ONE)?;
    let k = load_single(g, K_32767)?;
    let (lin, scale) = if !(f1 <= one) {
        // fcmpu f1,f0 ; ble → below
        (fp::div_single(k, f1), K_RATIO_ABOVE)
    } else {
        (mul_single(f1, k), K_RATIO_BELOW)
    };
    let r11 = fctiwz_low_word(lin);
    let r10 = lin_to_mb(g, r11, -10000)?;
    Ok(fctiwz_low_word(mul_single(
        word_to_single(r10 as u32),
        load_single(g, scale)?,
    )) as i32)
}

/// `sub_8294B4D8(cents)`: `2^(cents/1200)` as the guest computes it — whole octaves by repeated
/// `× 2.0`, then the semitone and fine tables (divided through for a negative argument).
pub fn cents_to_ratio(g: &Guest, cents: i32) -> Result<f64> {
    let one = load_single(g, K_ONE)?;
    let two = load_single(g, K_TWO)?;
    let negative = (cents as u32) >> 31; // rlwinm r9,r3,1,31,31
    let mut r3 = cents;
    let mut f0 = one;
    if !(r3 < 1200) {
        let q = (r3 as u32) / 1200; // divwu
        r3 = r3.wrapping_sub(q.wrapping_mul(1200) as i32);
        for _ in 0..q {
            f0 = mul_single(f0, two);
        }
    }
    if !(r3 > -1200) {
        let r10 = (-1200i32).wrapping_sub(r3) as u32;
        let q = r10 / 1200 + 1;
        r3 = r3.wrapping_add(q.wrapping_mul(1200) as i32);
        for _ in 0..q {
            f0 = mul_single(f0, two);
        }
    }
    let hundredth = |v: i32| {
        let t = mulhw(v, 0x51EB_851F) >> 5;
        t.wrapping_add(((t as u32) >> 31) as i32)
    };
    if negative == 0 {
        let semis = divw(r3, 100);
        let fine = r3.wrapping_sub(hundredth(r3).wrapping_mul(100));
        let f13 = load_single(g, CENTS_FINE_TABLE.wrapping_add((fine as u32) << 2))?;
        let f12 = load_single(g, CENTS_SEMI_TABLE.wrapping_add((semis as u32) << 2))?;
        Ok(mul_single(mul_single(f13, f12), f0))
    } else {
        let r9 = r3.wrapping_neg();
        let semis = divw(r9, 100);
        let f13 = load_single(g, CENTS_SEMI_TABLE.wrapping_add((semis as u32) << 2))?;
        let f11 = fp::div_single(one, f13);
        let fine = r9.wrapping_sub(hundredth(r9).wrapping_mul(100));
        let f10 = fp::div_single(f11, f0);
        let f9 = load_single(g, CENTS_FINE_TABLE.wrapping_add((fine as u32) << 2))?;
        let f8 = fp::div_single(one, f9);
        Ok(mul_single(f10, f8))
    }
}
