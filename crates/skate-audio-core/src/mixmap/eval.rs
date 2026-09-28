//! The per-evaluation pass, `sub_8294F5E8(host, dt)` (host vtable slot 2), and everything it calls.
//!
//! ```text
//! sub_8294F5E8   timing words; input stage (sub_8294B668 + the linear→mB walk);
//!                product stage (A); then
//!   sub_82951658 B: position/range lookups (sub_8294B668, sub_8294AF70)
//!   sub_82950250 F: envelopes → sub_82951440 (kind 0), sub_82951148 (kind 1),
//!                   sub_82950D40 (kind 3) (sub_8294B5F8)
//!                C: clamped sums; E: output sums
//!   sub_8294FC88 E: the packed int16 output write (sub_8294B4D8)
//! ```
//!
//! The many `sub_8294F6EC`…`sub_8294F870`, `sub_8294F924`…`sub_8294FAA8`, `sub_82951A24`…`sub_82951BA8`,
//! `sub_829502A4`…, `sub_8294FCD4`/`sub_8294FD14`… "functions" in the lifted files are the case
//! labels and loop heads of these bodies (the recompiler emits a function per jump-table target);
//! they are not separate entry points and are handled as the case arms here.
//!
//! Comparisons mirror the branch the original takes, not its mathematical meaning: `ble` is written
//! `!(a > b)`, `bge` `!(a < b)`, so an unordered compare falls the same way it does on the guest.

use super::offsets as H;
use super::tables::{
    K_25000, K_32767, K_FRAMES, K_MIN_RAMP_MS, K_MS, K_NEG_ONE, K_ONE, K_SLEW, K_ZERO,
    cents_to_ratio, curve, curve_float, lin_to_mb, mb_to_lin, ratio_to_mb,
};
use crate::fp::{
    add_single, div_single, fctiwz_low_word, load_single, mul_single, store_single, sub_single,
    word_to_single,
};
use crate::vmx::Fpscr;
use crate::{Error, Guest, Result};

#[inline]
fn rd(g: &Guest, a: u32) -> Result<u32> {
    g.u32(a)
}
#[inline]
fn rdi(g: &Guest, a: u32) -> Result<i32> {
    Ok(g.u32(a)? as i32)
}
#[inline]
fn wr(g: &mut Guest, a: u32, v: u32) -> Result<()> {
    g.set_u32(a, v)
}
#[inline]
fn wri(g: &mut Guest, a: u32, v: i32) -> Result<()> {
    g.set_u32(a, v as u32)
}

/// `sub_8294F5E8(host, mode, dt)`. `mode` is the manager's `+4` word the original passes in `r5`
/// (`sub_8294BAE8` loads `[mgr+4]`); it selects the B stage's record variant.
pub fn evaluate(g: &mut Guest, h: u32, mode: u32, dt: f64) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let old_mode = rd(g, h + H::MODE)?;
    let prev = load_single(g, h + 556)?;
    store_single(g, h + 560, prev)?;
    store_single(g, h + H::DT, dt)?;
    let ms = mul_single(dt, load_single(g, K_MS)?);
    let frames = mul_single(dt, load_single(g, K_FRAMES)?);
    store_single(g, h + 556, frames)?;
    store_single(g, h + H::DT_MS, ms)?;
    wr(g, h + H::PREV_MODE, old_mode)?;
    wr(g, h + H::MODE, mode)?;

    // Input stage: shape each input word with its kind's curve, keep the mB of the result.
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + H::INPUT_CAP)? {
        let e = rd(g, h + 388)?.wrapping_add(16 * i);
        let kind = u32::from(g.u8(e)?) & 0xF;
        let shaped = if kind <= 9 {
            let value = rdi(g, rd(g, e + 4)?)?;
            curve(g, value, kind)?
        } else {
            0
        };
        wri(g, e + 12, shaped)?;
        let mb = lin_to_mb(g, shaped as u32, -10000)?;
        wri(g, e + 8, mb)?;
        i += 1;
    }

    // Product stage (A).
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 464)? {
        let pe = rd(g, h + 400)?.wrapping_add(8 * i);
        let e2 = rd(g, pe + 4)?;
        let def = rd(g, pe)?;
        let input = rd(g, e2)?;
        let depth = rdi(g, def + 12)?;
        let lin = rdi(g, input + 12)?;
        let r10 = 32767i32.wrapping_sub(32767i32.wrapping_sub(lin).wrapping_mul(depth) >> 15);
        let mb = lin_to_mb(g, r10 as u32, -10000)?;
        let r7 = rdi(g, def + 8)?.wrapping_add(mb);
        let mut acc = 32767i32;
        let refs = rd(g, e2 + 4)?;
        if refs != 0 {
            let n = u32::from(g.u8(rd(g, def)? + 4)?);
            for m in 0..n {
                let v = rdi(g, rd(g, refs + 4 * m)?)?;
                acc = v.wrapping_mul(acc) >> 15;
            }
        }
        wri(g, e2 + 8, acc.wrapping_mul(r7) >> 15)?;
        i += 1;
    }

    lookups(g, h)?;
    envelopes(g, h)?;

    // C stage: clamped sums.
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 472)? {
        let e = rd(g, h + 436)?.wrapping_add(8 * i);
        let s = rd(g, e + 4)?;
        let def = rd(g, e)?;
        let refs = rd(g, s)?;
        if refs != 0 {
            wr(g, s + 4, 0)?;
            let n = rd(g, def + 8)? & 0xFF;
            for m in 0..n {
                let v = rd(g, rd(g, refs + 4 * m)?)?;
                let cur = rd(g, s + 4)?;
                wr(g, s + 4, cur.wrapping_add(v))?;
            }
            let w = rd(g, rd(g, def)? + 4)?;
            let max = ((w as i32) >> 16) & 0x7FFF;
            let min = (w | 0xFFFF_0000) as i32;
            if rdi(g, s + 4)? > max {
                wri(g, s + 4, max)?;
            }
            if rdi(g, s + 4)? < min {
                wri(g, s + 4, min)?;
            }
        }
        i += 1;
    }

    // E stage: output sums.
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 476)? {
        let e = rd(g, h + 448)?.wrapping_add(8 * i);
        let st = rd(g, e + 4)?;
        let def = rd(g, e)?;
        let block = rd(g, st + 16)?;
        if rd(g, block + 60)? & 1 == 0 {
            wri(g, st + 8, -10000)?;
        } else {
            let v = u32::from(g.u16(rd(g, def)? + 4)?);
            wr(g, st + 8, v)?;
            if v & 0xFFFF_8000 != 0 {
                wr(g, st + 8, v | 0xFFFF_0000)?;
            }
            let refs = rd(g, st + 4)?;
            if refs != 0 {
                let n = rd(g, def + 8)? & 0xFF;
                for m in 0..n {
                    let add = rd(g, rd(g, refs + 4 * m)?)?;
                    let cur = rd(g, st + 8)?;
                    wr(g, st + 8, add.wrapping_add(cur))?;
                }
            }
        }
        i += 1;
    }

    write_outputs(g, h)
}

// ---------------------------------------------------------------------- B: lookups

/// `sub_82951658(host)`.
fn lookups(g: &mut Guest, h: u32) -> Result<()> {
    if rd(g, h + H::MODE)? != rd(g, h + H::PREV_MODE)? {
        let mut i = 0u32;
        while (i as i32) < rdi(g, h + 356)? {
            let def = rd(g, h + 420)?.wrapping_add(20 * i);
            let rec = rd(g, def)?;
            let first = rec + 4;
            let n = u32::from(g.u8(rec)?) & 0xF;
            let mut want = rdi(g, h + H::MODE)?;
            loop {
                let mut found = false;
                for m in 0..n {
                    let v = first + 24 * m;
                    if (u32::from(g.u8(v)?) & 0xF) as i32 == want {
                        wr(g, def + 4, v)?;
                        wr(g, def + 12, rd(g, h + H::PREV_MODE)?)?;
                        wr(g, def + 8, rd(g, h + H::MODE)?)?;
                        wr(g, def + 16, 0)?;
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
                if want == 0 {
                    // The original branches back with r8 = 0 forever.
                    return Err(Error::new(
                        0x8295_16F8,
                        format!("lookup record at {rec:#010x} has no variant 0: the guest spins"),
                    ));
                }
                want = 0;
            }
            i += 1;
        }
    }

    let zero = load_single(g, K_ZERO)?;
    let k32767 = load_single(g, K_32767)?;
    let slew = load_single(g, K_SLEW)?;
    let neg_one = load_single(g, K_NEG_ONE)?;
    let one = load_single(g, K_ONE)?;
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 468)? {
        let e = rd(g, h + 416)?.wrapping_add(8 * i);
        let st = rd(g, e + 4)?;
        let blk = rd(g, st + 4)?;
        if rd(g, blk + 60)? & 1 == 0 {
            wri(g, st + 12, -10000)?;
            wr(g, st + 16, 0)?;
            wr(g, st + 8, 0)?;
            wr(g, st + 20, 0)?;
            i += 1;
            continue;
        }
        let def = rd(g, e)?;
        let v = rd(g, def + 4)?;
        let r7 = rdi(g, v)?;
        let r10 = rdi(g, v + 4)?;
        let x_sel = ((r7 >> 12) as u32) & 0xF;
        let i_sel = ((r7 >> 8) as u32) & 0xF;
        let mut f10 = match x_sel {
            0 => load_single(g, blk + 4)?,
            1 => load_single(g, blk)?,
            _ => neg_one,
        };
        let mut f9 = f10;
        let r11 = match i_sel {
            0 => rd(g, blk + 12)?,
            1 => rd(g, blk + 8)?,
            _ => 0,
        };
        wr(g, st + 8, r11)?;
        let q = (((r11 as i32) >> 14) as u32) & 3;
        let (shift, r30, fo) = match q {
            0 => (28, r11 as i32, [32u32, 36, 40, 44]),
            1 => (16, (r11 as i32).wrapping_sub(16384), [40, 44, 48, 52]),
            2 => (24, (r11 as i32).wrapping_sub(32768), [48, 52, 56, 60]),
            _ => (20, (r11 as i32).wrapping_sub(49152), [56, 60, 32, 36]),
        };
        let kind = ((r10 >> shift) as u32) & 0xF;
        let f0 = load_single(g, st + fo[0])?;
        let f12 = load_single(g, st + fo[1])?;
        let f13 = load_single(g, st + fo[2])?;
        let f11 = load_single(g, st + fo[3])?;
        if f10 > f12 && f10 > f11 {
            wri(g, st + 12, -10000)?;
            wr(g, st + 16, 0)?;
            wr(g, st + 20, 0)?;
            i += 1;
            continue;
        }
        if !(f10 >= f0) {
            f10 = f0;
        }
        if !(f9 >= f13) {
            f9 = f13;
        }
        if !(f10 <= f12) {
            f10 = f12;
        }
        if !(f9 <= f11) {
            f9 = f11;
        }
        let a = div_single(sub_single(f10, f0), sub_single(f12, f0));
        let b = div_single(sub_single(f9, f13), sub_single(f11, f13));
        let r3 = fctiwz_low_word(mul_single(a, k32767)) as i32;
        let r28 = fctiwz_low_word(mul_single(b, k32767)) as i32;
        let r29 = if kind <= 9 { curve(g, r3, kind)? } else { 0 };
        let r3b = if r30 != 0 {
            if kind <= 9 { curve(g, r28, kind)? } else { 0 }
        } else {
            32767
        };
        let r11b = r30 << 1;
        let r7b = 32767i32.wrapping_sub(r11b);
        let lin = (r7b.wrapping_mul(r29) >> 15).wrapping_add(r11b.wrapping_mul(r3b) >> 15);
        wri(g, st + 16, lin)?;
        let mb = lin_to_mb(g, rd(g, st + 16)?, -10000)?;
        wri(g, st + 12, mb)?;
        let low = rd(g, v + 4)? & 0xFFFF;
        if low == 0 {
            i += 1;
            continue;
        }
        let f0 = word_to_single(low);
        let flags = rd(g, blk + 60)?;
        let (f8, level) = if x_sel == 1 {
            let f8 = load_single(g, blk)?;
            if flags & 0x8000_0000 == 0 {
                let mut span = add_single(load_single(g, blk + 52)?, f0);
                if !(span > zero) {
                    span = f0;
                }
                let r = ratio_to_mb(g, div_single(f0, span))?;
                (f8, word_to_single(r as u32))
            } else {
                wr(g, blk + 60, flags & 0x7FFF_FFFF)?;
                (f8, zero)
            }
        } else {
            let f8 = load_single(g, blk + 4)?;
            if flags & 0x4000_0000 == 0 {
                let mut span = add_single(load_single(g, blk + 56)?, f0);
                if !(span > zero) {
                    span = f0;
                }
                let r = ratio_to_mb(g, div_single(f0, span))?;
                (f8, word_to_single(r as u32))
            } else {
                wr(g, blk + 60, flags & 0xBFFF_FFFF)?;
                (f8, zero)
            }
        };
        if load_single(g, st + 28)? == zero {
            store_single(g, st + 28, one)?;
        }
        let last = load_single(g, st + 24)?;
        let delta = if f8 > last {
            sub_single(f8, last)
        } else {
            sub_single(last, f8)
        };
        store_single(g, st + 28, delta)?;
        store_single(g, st + 24, f8)?;
        let r8 = rdi(g, st + 20)?;
        let step = fctiwz_low_word(mul_single(
            sub_single(level, word_to_single(r8 as u32)),
            slew,
        )) as i32;
        wri(g, st + 20, r8.wrapping_sub(step))?;
        i += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------- F: envelopes

/// The envelope state (`st`, 48 bytes): `+0` state, `+4` elapsed ms, `+8` start level,
/// `+12` start time, `+16` trigger pointer, `+20` references, `+24` mB out, `+28` level,
/// `+32/+36/+40/+44` attack / decay / hold / release ms.
struct Env {
    st: u32,
    trig: bool,
    hold: bool,
    retrig: bool,
    zero: f64,
    one: f64,
    min_ramp: f64,
}

impl Env {
    fn ld(&self, g: &Guest, off: u32) -> Result<f64> {
        load_single(g, self.st + off)
    }
    fn sf(&self, g: &mut Guest, off: u32, v: f64) -> Result<()> {
        store_single(g, self.st + off, v)
    }
    fn w(&self, g: &Guest, off: u32) -> Result<i32> {
        rdi(g, self.st + off)
    }
    fn sw(&self, g: &mut Guest, off: u32, v: i32) -> Result<()> {
        wri(g, self.st + off, v)
    }
    /// Every body's "past the end" reset.
    fn reset(&self, g: &mut Guest) -> Result<()> {
        self.sf(g, 4, self.zero)?;
        self.sf(g, 12, self.zero)?;
        self.sw(g, 8, 0)?;
        self.sw(g, 0, 0)?;
        self.sw(g, 28, 0)?;
        self.sw(g, 24, 0)
    }
    /// Enter `state` with the timer at zero and a start level.
    fn enter(&self, g: &mut Guest, state: i32, level: i32) -> Result<()> {
        self.sf(g, 4, self.zero)?;
        self.sw(g, 0, state)?;
        self.sf(g, 12, self.zero)?;
        self.sw(g, 8, level)
    }
    /// Jump into `state` part-way through `to` (ms), in proportion to what is left of `from`:
    /// `t = (from - elapsed) / from × to`; a `from` under 16.666 ms starts `to` at its end.
    /// Every site stores the new state before its 16.666 ms test.
    fn carry(&self, g: &mut Guest, state: i32, from: f64, to_off: u32) -> Result<()> {
        self.sw(g, 0, state)?;
        if from < self.min_ramp {
            let level = self.w(g, 28)?;
            let to = self.ld(g, to_off)?;
            self.sf(g, 12, to)?;
            self.sf(g, 4, to)?;
            self.sw(g, 8, level)
        } else {
            let elapsed = self.ld(g, 4)?;
            let level = self.w(g, 28)?;
            let rem = sub_single(from, elapsed);
            let to = self.ld(g, to_off)?;
            self.sw(g, 8, level)?;
            let t = mul_single(div_single(rem, from), to);
            self.sf(g, 12, t)?;
            self.sf(g, 4, t)
        }
    }
    /// The attack ramp: `level = start + curve(t) × (32767 - start)`.
    fn attack(&self, g: &mut Guest, curve_kind: u32, span: f64, elapsed: f64) -> Result<()> {
        let start_t = self.ld(g, 12)?;
        let span = sub_single(span, start_t);
        let mut t = sub_single(elapsed, start_t);
        if span > self.zero {
            t = div_single(t, span);
        }
        let start = self.w(g, 8)?;
        let f1 = curve_float(g, curve_kind, t)?;
        let v = fctiwz_low_word(mul_single(
            f1,
            word_to_single(32767i32.wrapping_sub(start) as u32),
        )) as i32;
        self.sw(g, 28, v.wrapping_add(start))
    }
    /// The normalised position in a ramp that started at `+12`.
    fn progress(&self, g: &Guest, span: f64, elapsed: f64) -> Result<f64> {
        let start_t = self.ld(g, 12)?;
        let span = sub_single(span, start_t);
        let mut t = sub_single(elapsed, start_t);
        if span > self.zero {
            t = div_single(t, span);
        }
        Ok(t)
    }
}

/// `sub_82950250(host)`.
fn envelopes(g: &mut Guest, h: u32) -> Result<()> {
    let zero = load_single(g, K_ZERO)?;
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 480)? {
        let e = rd(g, h + 404)?.wrapping_add(8 * i);
        let st = rd(g, e + 4)?;
        let def = rd(g, e)?;
        i += 1;
        if rd(g, st)? == 0 && rd(g, rd(g, st + 16)?)? == 0 {
            store_single(g, st + 4, zero)?;
            wr(g, st + 8, 0)?;
            store_single(g, st + 12, zero)?;
            wr(g, st, 0)?;
            wr(g, st + 28, 0)?;
            wr(g, st + 24, 0)?;
            if (rdi(g, rd(g, def)?)? >> 9) & 1 != 0 {
                wri(g, st + 24, -10000)?;
            }
            continue;
        }
        let elapsed = add_single(load_single(g, h + H::DT_MS)?, load_single(g, st + 4)?);
        store_single(g, st + 4, elapsed)?;
        let rec = rd(g, def)?;
        match u32::from(g.u8(rec)?) & 0xF {
            0 => envelope_ar(g, e)?,
            1 => envelope_ahr(g, e)?,
            3 => envelope_adsr(g, e)?,
            _ => {}
        }
        let rec = rd(g, def)?;
        let mut half = rd(g, rec + 4)? & 0xFFFF;
        if half & 0x8000 != 0 {
            half |= 0xFFFF_0000;
        }
        let flag = ((rdi(g, rec)? >> 9) & 1) != 0;
        if !flag {
            let d8 = rdi(g, def + 8)?;
            let level = rdi(g, st + 28)?;
            if (half as i32) > 0 {
                let x = (level.wrapping_mul(d8) >> 15)
                    .wrapping_sub(d8)
                    .wrapping_add(32767);
                let mb = lin_to_mb(g, x as u32, -10000)?;
                wri(g, st + 24, rdi(g, def + 4)?.wrapping_add(mb))?;
            } else {
                let x = 32767i32.wrapping_sub(level.wrapping_mul(d8) >> 15);
                wri(g, st + 24, lin_to_mb(g, x as u32, -10000)?)?;
            }
        } else {
            let mb = lin_to_mb(g, rd(g, st + 28)?, -10000)?;
            wri(g, st + 24, mb)?;
        }
        let refs = rd(g, st + 20)?;
        if refs == 0 {
            continue;
        }
        let mut acc = 32767i32;
        let n = u32::from(g.u8(rd(g, def)? + 4)?);
        for m in 0..n {
            acc = rdi(g, rd(g, refs + 4 * m)?)?.wrapping_mul(acc) >> 15;
        }
        if !flag {
            let v = rdi(g, st + 24)?;
            wri(g, st + 24, v.wrapping_mul(acc) >> 15)?;
        } else {
            let lin = mb_to_lin(g, rdi(g, st + 24)?)?;
            let mb = lin_to_mb(g, (lin.wrapping_mul(acc) >> 15) as u32, -10000)?;
            wri(g, st + 24, mb)?;
        }
    }
    Ok(())
}

fn env_for(g: &Guest, e: u32) -> Result<(Env, u32)> {
    let def = rd(g, e)?;
    let st = rd(g, e + 4)?;
    let rec = rd(g, def)?;
    let rec0 = rdi(g, rec)?;
    Ok((
        Env {
            st,
            trig: rd(g, rd(g, st + 16)?)? != 0,
            hold: (rec0 >> 8) & 1 != 0,
            retrig: (rec0 >> 10) & 1 != 0,
            zero: load_single(g, K_ZERO)?,
            one: load_single(g, K_ONE)?,
            min_ramp: load_single(g, K_MIN_RAMP_MS)?,
        },
        rec,
    ))
}

fn nibble12(g: &Guest, at: u32) -> Result<u32> {
    Ok(((rdi(g, at)? >> 12) as u32) & 0xF)
}

/// `sub_82951440` — kind 0: attack, then release (a retrigger restarts the attack part-way).
fn envelope_ar(g: &mut Guest, e: u32) -> Result<()> {
    let (env, rec) = env_for(g, e)?;
    let attack = nibble12(g, rec + 12)?;
    let release = nibble12(g, rec + 20)?;
    loop {
        let s = env.w(g, 0)? as u32;
        if s < 1 {
            env.sw(g, 0, 1)?;
        }
        if s <= 1 {
            let span = env.ld(g, 32)?;
            let elapsed = env.ld(g, 4)?;
            if elapsed < span {
                if span == env.zero {
                    return env.sw(g, 28, 32767);
                }
                return env.attack(g, attack, span, elapsed);
            }
            env.enter(g, 4, 32767)?;
            continue;
        }
        let span = env.ld(g, 44)?;
        let elapsed = env.ld(g, 4)?;
        if !(elapsed < span) {
            return env.reset(g);
        }
        if !env.trig || !env.retrig {
            let t = env.progress(g, span, elapsed)?;
            let start = env.w(g, 8)?;
            let f1 = curve_float(g, release, sub_single(env.one, t))?;
            let v = fctiwz_low_word(mul_single(f1, word_to_single(start as u32))) as i32;
            return env.sw(g, 28, start.wrapping_sub(v));
        }
        env.carry(g, 1, span, 32)?;
    }
}

/// The release ramp of kinds 1 and 3: `level = start - (1 - curve(1 - t)) × start`.
fn release_ramp(g: &mut Guest, env: &Env, curve_kind: u32, span: f64, elapsed: f64) -> Result<()> {
    let t = env.progress(g, span, elapsed)?;
    let start = env.w(g, 8)?;
    let f1 = curve_float(g, curve_kind, sub_single(env.one, t))?;
    let v = fctiwz_low_word(mul_single(
        sub_single(env.one, f1),
        word_to_single(start as u32),
    )) as i32;
    env.sw(g, 28, start.wrapping_sub(v))
}

/// `sub_82951148` — kind 1: attack, hold (state 3), release.
fn envelope_ahr(g: &mut Guest, e: u32) -> Result<()> {
    let (env, rec) = env_for(g, e)?;
    let attack = nibble12(g, rec + 12)?;
    let release = nibble12(g, rec + 20)?;
    loop {
        let s = env.w(g, 0)? as u32;
        if s < 1 {
            env.sw(g, 0, 1)?;
        }
        if s <= 1 {
            if env.hold && !env.trig {
                let span = env.ld(g, 32)?;
                env.carry(g, 4, span, 44)?;
                continue;
            }
            let span = env.ld(g, 32)?;
            let elapsed = env.ld(g, 4)?;
            if elapsed < span {
                if span == env.zero {
                    return env.sw(g, 28, 32767);
                }
                return env.attack(g, attack, span, elapsed);
            }
            env.enter(g, 3, 32767)?;
            continue;
        }
        if s == 3 {
            if !env.trig && env.hold {
                env.enter(g, 4, 32767)?;
                continue;
            }
            if !env.hold && env.ld(g, 4)? > env.ld(g, 40)? {
                env.enter(g, 4, 32767)?;
                continue;
            }
            env.sw(g, 28, 32767)?;
            if env.hold {
                env.sf(g, 4, env.zero)?;
            }
            return Ok(());
        }
        let span = env.ld(g, 44)?;
        let elapsed = env.ld(g, 4)?;
        if !(elapsed < span) {
            return env.reset(g);
        }
        if !env.trig || !env.retrig {
            return release_ramp(g, &env, release, span, elapsed);
        }
        env.carry(g, 1, span, 32)?;
    }
}

/// `sub_82950D40` — kind 3: attack, decay to the sustain level (`[rec+20] >> 16`), sustain,
/// release.
fn envelope_adsr(g: &mut Guest, e: u32) -> Result<()> {
    let (env, rec) = env_for(g, e)?;
    let attack = nibble12(g, rec + 12)?;
    let decay = nibble12(g, rec + 16)?;
    let release = nibble12(g, rec + 20)?;
    let sustain = rdi(g, rec + 20)? >> 16;
    loop {
        let s = env.w(g, 0)? as u32;
        if s > 3 {
            let span = env.ld(g, 44)?;
            let elapsed = env.ld(g, 4)?;
            if !(elapsed < span) {
                return env.reset(g);
            }
            if !env.trig || !env.retrig {
                return release_ramp(g, &env, release, span, elapsed);
            }
            env.carry(g, 1, span, 32)?;
            continue;
        }
        if s == 0 {
            env.sw(g, 0, 1)?;
        }
        if s <= 1 {
            if env.hold && !env.trig {
                let span = env.ld(g, 32)?;
                env.carry(g, 4, span, 44)?;
                continue;
            }
            let span = env.ld(g, 32)?;
            let elapsed = env.ld(g, 4)?;
            if elapsed < span {
                if !(span > env.min_ramp) {
                    return env.sw(g, 28, 32767);
                }
                return env.attack(g, attack, span, elapsed);
            }
            env.enter(g, 2, 32767)?;
            continue;
        }
        if s == 2 {
            if !env.trig && env.hold {
                // No short-ramp test on this edge: straight proportional carry into release.
                let span = env.ld(g, 36)?;
                let level = env.w(g, 28)?;
                let elapsed = env.ld(g, 4)?;
                env.sw(g, 0, 4)?;
                let rem = sub_single(span, elapsed);
                let to = env.ld(g, 44)?;
                env.sw(g, 8, level)?;
                let t = mul_single(div_single(rem, span), to);
                env.sf(g, 12, t)?;
                env.sf(g, 4, t)?;
                continue;
            }
            let in_decay = if env.hold && !env.trig {
                true
            } else {
                !(env.ld(g, 4)? > env.ld(g, 36)?)
            };
            if in_decay {
                let span = env.ld(g, 36)?;
                let start_t = env.ld(g, 12)?;
                let len = sub_single(span, start_t);
                let mut t = sub_single(env.ld(g, 4)?, env.ld(g, 12)?);
                if len > env.zero {
                    t = div_single(t, len);
                }
                let start = env.w(g, 8)?;
                let f1 = curve_float(g, decay, sub_single(env.one, t))?;
                let span_level = sustain.wrapping_sub(start);
                let v = fctiwz_low_word(mul_single(
                    sub_single(env.one, f1),
                    word_to_single(span_level as u32),
                )) as i32;
                return env.sw(g, 28, v.wrapping_add(start));
            }
            env.enter(g, 3, sustain)?;
            continue;
        }
        // s == 3: sustain.
        if !env.trig && env.hold {
            env.enter(g, 4, sustain)?;
            continue;
        }
        if !env.hold && env.ld(g, 4)? > env.ld(g, 40)? {
            env.enter(g, 4, sustain)?;
            continue;
        }
        env.sw(g, 28, sustain)?;
        if env.hold {
            env.sf(g, 4, env.zero)?;
        }
        return Ok(());
    }
}

// ---------------------------------------------------------------------- E: the output write

/// `[blk + 4·(id>>1)]` with the half `id & 1` selects replaced by `value`.
fn write_half(g: &mut Guest, block: u32, id: u32, value: i32) -> Result<()> {
    let at = block.wrapping_add((((id as i32) >> 1) as u32) << 2);
    let keep = 0xFFFFu32 << ((id.wrapping_sub(1) << 4) & 0x10);
    let shift = (id << 4) & 0x10;
    let w = rd(g, at)?;
    wr(g, at, ((value as u32 & 0xFFFF) << shift) | (w & keep))
}

/// Volume: clamp to −10000..0 mB (the `addic/subfze/and` idiom is `min(x, 0)`), then mB → linear.
fn volume(g: &Guest, x: i32) -> Result<i32> {
    // `if x < -10000 { -10000 } else { x.min(0) }` is the literal reading, but rustc 1.98.1 / LLVM
    // 22.1.8 miscompiles exactly that shape at -O (it folds to `x.min(0)`; reproduced standalone).
    // `clamp` is the same function for these bounds and compiles correctly.
    let clamped = x.clamp(-10000, 0);
    mb_to_lin(g, clamped)
}

/// `sub_8294FC88(host)`. Each output's descriptor (`def+12`, from the section's G table) is a word
/// whose top byte's low nibble is the type and low 5 bits the count, then one word per output:
/// bits 26–30 the destination id, 21–25 the special-reference index, 0–15 a signed offset, bit 31
/// "raw lookup value".
///
/// | type | normal | via a special (B) reference |
/// |---|---|---|
/// | 0, 4 | volume: clamp −10000..0, mB→linear | same, plus the lookup's mB (`st+12`) |
/// | 1 | clamp −4800..2400 | plus the lookup's slewed value (`st+20`); above 2400 → 2400, below −4800 → **0** |
/// | 2 | clamp −10000..0, `sub_8294B4D8` × 25000 | clamp −10000..0 (no conversion) |
/// | 3, >4 | clamp 0..25000 | the sum unchanged, no offset |
///
/// A disabled block (word 15 bit 0 clear) gets only its first output written: type 1 → 0,
/// type 2 → 25000, otherwise −10000.
fn write_outputs(g: &mut Guest, h: u32) -> Result<()> {
    let k25000 = load_single(g, K_25000)?;
    let mut i = 0u32;
    while (i as i32) < rdi(g, h + 476)? {
        let e = rd(g, h + 448)?.wrapping_add(8 * i);
        i += 1;
        let def = rd(g, e)?;
        let st = rd(g, e + 4)?;
        let desc = rd(g, def + 12)?;
        let head = rd(g, desc)?;
        let block = rd(g, st + 16)?;
        let kind = (((head as i32) >> 24) as u32) & 0xF;
        let count = head & 0x1F;
        if rd(g, block + 60)? & 1 == 0 {
            let value = match kind {
                1 => 0,
                2 => 25000,
                _ => -10000,
            };
            let d = rd(g, desc + 4)?;
            let id = (((d as i32) >> 26) as u32) & 0x1F;
            write_half(g, block, id, value)?;
            continue;
        }
        for n in 0..count {
            let d = rd(g, desc + 4 + 4 * n)?;
            let id = (((d as i32) >> 26) as u32) & 0x1F;
            let source = (((d as i32) >> 21) as u32) & 0x1F;
            let offset = i32::from(d as u16 as i16);
            let specials = u32::from(g.u16(def + 8)?) & 0x1F;
            let sum = rdi(g, st + 8)?;
            let value = if (specials as i32) > 0 && (source as i32) < specials as i32 {
                let entry = rd(g, rd(g, st + 12)?.wrapping_add(source << 2))?;
                if d & 0x8000_0000 != 0 {
                    (rd(g, rd(g, entry + 4)? + 8)? & 0xFFFF) as i32
                } else {
                    match kind {
                        0 | 4 => {
                            let lookup = rdi(g, rd(g, entry + 4)? + 12)?;
                            volume(g, lookup.wrapping_add(sum).wrapping_add(offset))?
                        }
                        1 => {
                            let slewed = rdi(g, rd(g, entry + 4)? + 20)?;
                            let v = slewed.wrapping_add(sum).wrapping_add(offset);
                            if v > 2400 {
                                2400
                            } else if v < -4800 {
                                0
                            } else {
                                v
                            }
                        }
                        2 => {
                            let v = sum.wrapping_add(offset);
                            if v < -10000 {
                                -10000
                            } else if v > 0 {
                                0
                            } else {
                                v
                            }
                        }
                        _ => sum,
                    }
                }
            } else {
                let v = sum.wrapping_add(offset);
                match kind {
                    0 | 4 => volume(g, v)?,
                    1 => v.clamp(-4800, 2400),
                    2 => {
                        // Same clamp as `volume` (see the miscompile note there).
                        let r = cents_to_ratio(g, v.clamp(-10000, 0))?;
                        fctiwz_low_word(mul_single(r, k25000)) as i32
                    }
                    _ => {
                        if v < 0 {
                            0
                        } else if v > 25000 {
                            25000
                        } else {
                            v
                        }
                    }
                }
            };
            write_half(g, block, id, value)?;
        }
    }
    Ok(())
}
