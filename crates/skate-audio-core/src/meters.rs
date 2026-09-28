//! The meters: per-channel mean-square and peak meters over one block, kept in ring histories
//! (`sub_82B373C8`), and the tick that runs them every so many blocks (`sub_82B376B8`).
//!
//! Ported from `recomp/src/audio_ports/sub_82B373C8.inc` and `sub_82B376B8.inc`, both **STATUS:
//! thin**: verified with zero divergence, on 23 comparable calls in the boot profile and about
//! 10,000 in a played one.
//!
//! Each metered block reads one quarter of every channel's 256 singles: sixteen 16-byte loads, 64
//! bytes apart. Their magnitudes are squared into four accumulators and max-reduced into four more,
//! and **all four accumulators are fed the same vector**, so the sum that comes out is four times the
//! sum of squares. That is the original's arithmetic, reproduced. The vector work runs under the
//! flush mode, with two roundings per multiply-add ([`crate::vmx`] rule 1).
//!
//! The scalar tail turns each channel's lanes into a mean, `sum * scale / frames`, and keeps three
//! running values per channel on the object: the level (a moving sum over the ring of means), the
//! block's peak, and a held peak. On the ring's last block the running sums are published as the
//! levels and cleared. The ring cursor then advances by an actual divide.
//!
//! [`meter_tick`] counts off an interval. When it expires it re-derives the ring length if the
//! stream's rate has changed, clearing both rings, then runs [`meter_block`] and
//! [`crate::layout::expand_layout`] on the same object. The layout expansion's three source blocks
//! are this module's level, peak and held-peak arrays (+272, +304, +336), so group 0's square root
//! is taken over the levels.
//!
//! Nothing in `docs/rw_audio_structs.h` names the object; the offsets are numbers, and the names
//! below describe what the code does with each field.

use core::arch::x86_64::*;

use crate::vmx::{self, Fpscr};
use crate::{Guest, Result, fp, layout, mem};

/// `lbz r28,42(r3)` — channels metered; the red-zone arrays hold eight.
pub const METER_CHANNELS: u32 = 42;
/// `addi r11,r3,236` — the publish loop's cursor, one word before the sums.
pub const METER_SLOT_BASE: u32 = 236;
/// `f32[8]` — the running sum of means.
pub const METER_SUMS: u32 = 240;
/// `f32[8]` — the level.
pub const METER_LEVELS: u32 = 272;
/// `f32[8]` — this block's peak.
pub const METER_PEAKS: u32 = 304;
/// `f32[8]` — the held peak.
pub const METER_HOLDS: u32 = 336;
/// `lfs f0,368(r31)` — the rate the rings were last sized for.
pub const METER_RATE_CACHE: u32 = 368;
/// `lfs f0,372(r31)` — the history length, in seconds.
pub const METER_SECONDS: u32 = 372;
/// `lwz r8,376(r3)` — the ring length, in blocks. Reloaded often.
pub const METER_FRAMES: u32 = 376;
/// `lwz r10,380(r3)` — ticks to skip between meter runs.
pub const METER_INTERVAL: u32 = 380;
/// `lhz r11,384(r3)` — the level ring's byte offset in this object.
pub const METER_LEVEL_RING: u32 = 384;
/// `lhz r11,386(r3)` — the peak ring's.
pub const METER_PEAK_RING: u32 = 386;
/// `lhz r9,388(r3)` — the ring write position.
pub const METER_CURSOR: u32 = 388;
/// `lhz r11,390(r3)` — the interval counter.
pub const METER_TICKS: u32 = 390;
/// The channel descriptor: `lwz r10,4(r4)`, channel 0's singles.
pub const CHANNEL_BASE: u32 = 4;
/// `lhz r11,14(r4)` — singles between channels.
pub const CHANNEL_STRIDE: u32 = 14;
/// The tick's descriptor: `lwz r4,28(r29)`, the channel descriptor [`meter_block`] takes.
pub const TICK_CHANNELS: u32 = 28;
/// `lwz r11,40(r29)` — the format, re-read mid-call; its `+12` is the rate.
pub const TICK_FORMAT: u32 = 40;
/// `lfs f13,12(r11)`.
pub const FORMAT_RATE: u32 = 12;
/// The sums' red-zone array, 16 bytes a channel: `r1 - 304`.
pub const SUM_SCRATCH: u32 = 304;
/// The peaks' red-zone array: `r1 - 176`.
pub const PEAK_SCRATCH: u32 = 176;
/// The deepest red-zone byte [`meter_block`] writes, for a caller seeding its stack.
pub const METER_RED_ZONE: u32 = 320;
/// `stwu r1,-128(r1)` — [`meter_tick`]'s frame; [`meter_block`]'s red zone sits below it.
pub const TICK_FRAME: u32 = 128;
/// One block: 256 singles.
pub const BLOCK_BYTES: u32 = 1024;

const POOL: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232);
const _: () = assert!(POOL == 0x822F_8600);
/// `lfs f9,1092(r9)` — the summed squares' scale, before the divide.
pub const RMS_SCALE: u32 = POOL + 1092;
/// `lfs f12,2156(r11)` — blocks per hertz.
pub const BLOCKS_PER_HZ: u32 = POOL + 2156;
/// `lfs f12,444(r11)` — the block rate the rings are sized for.
pub const MAX_BLOCK_RATE: u32 = POOL + 444;
/// `lis -32246 ; lfs -26788` — 0.5, the rounding half.
pub const HALF: u32 = (((-32246i32 as u32) & 0xFFFF) << 16).wrapping_sub(26788);
const _: () = assert!(RMS_SCALE == 0x822F_8A44 && BLOCKS_PER_HZ == 0x822F_8E6C);
const _: () = assert!(MAX_BLOCK_RATE == 0x822F_87BC && HALF == 0x8209_975C);

/// Meter one block into the object's running values and ring histories (`sub_82B373C8`).
///
/// `object` is `r3`, `desc` the channel descriptor in `r4`, `sp` is `r1`. There is no frame: the sums
/// and peaks go through two 16-byte-a-channel arrays in the red zone below `sp`, and are read back
/// lane by lane, so those 320 bytes must be mapped. Past eight channels the original walks off its
/// red zone into its caller's frame; that is reproduced, not guarded.
pub fn meter_block(g: &mut Guest, object: u32, desc: u32, sp: u32) -> Result<()> {
    let count = u32::from(g.u8(object.wrapping_add(METER_CHANNELS))?); // lbz r28,42(r3)
    let mut fpscr = Fpscr::capture();
    let sums_at = sp.wrapping_sub(SUM_SCRATCH);
    let peaks_at = sp.wrapping_sub(PEAK_SCRATCH);
    if count != 0 {
        if !vmx::supported() {
            return Err(vmx::unsupported());
        }
        let stride = u32::from(g.u16(desc.wrapping_add(CHANNEL_STRIDE))?); // lhz r11,14(r4)
        let mut row = g.u32(desc.wrapping_add(CHANNEL_BASE))?; // lwz r10,4(r4)
        let row_step = stride.rotate_left(2); // rotlwi r6,r11,2
        let mut scratch = 0u32; // li r8,0
        for _ in 0..count {
            // SAFETY: `vmx::supported()` held above; every access goes through the checked guest.
            unsafe {
                let sign = _mm_set1_epi32(i32::MIN); // vspltisw -1 ; vslw: 0x80000000 each lane
                let zero = _mm_setzero_ps(); // vspltisw128 v59,0
                let (mut sa, mut sb, mut sc, mut sd) = (zero, zero, zero, zero);
                let (mut ma, mut mb, mut mc, mut md) = (zero, zero, zero, zero);
                let mut offset = 0;
                while offset < BLOCK_BYTES {
                    let raw = vmx::lvx128(g, row.wrapping_add(offset))?; // lvx128 v57,r10,r11
                    let mag = _mm_castsi128_ps(_mm_andnot_si128(sign, raw)); // vandc128 v0,v57,v58
                    fpscr.enable_flush_mode_unconditional();
                    sa = vmx::vmaddfp(mag, mag, sa); // vmaddfp v13,v0,v0,v13
                    sb = vmx::vmaddfp(mag, mag, sb);
                    sc = vmx::vmaddfp(mag, mag, sc);
                    sd = vmx::vmaddfp(mag, mag, sd);
                    ma = _mm_max_ps(mag, ma); // vmaxfp128 v63,v0,v63 -- the second operand wins a NaN
                    mb = _mm_max_ps(mag, mb);
                    mc = _mm_max_ps(mag, mc);
                    md = _mm_max_ps(mag, md);
                    offset += 64;
                }
                let sum_cd = _mm_add_ps(sc, sd); // vaddfp128 v56,v11,v10
                let sum_ab = _mm_add_ps(sa, sb); // vaddfp128 v55,v13,v12
                let max_cd = _mm_max_ps(mc, md);
                let max_ab = _mm_max_ps(ma, mb);
                let sums = _mm_add_ps(sum_ab, sum_cd); // vaddfp128 v52,v55,v56
                let peaks = _mm_max_ps(max_ab, max_cd); // vmaxfp128 v51,v53,v54
                row = row.wrapping_add(row_step); // add r10,r6,r10
                vmx::stvx128_ps(g, scratch.wrapping_add(sums_at), sums)?; // stvx128 v52,r8,r11
                vmx::stvx128_ps(g, scratch.wrapping_add(peaks_at), peaks)?; // stvx128 v51,r8,r9
            }
            scratch = scratch.wrapping_add(16); // addi r8,r8,16
        }
    }

    fpscr.disable_flush_mode_unconditional();
    let floor = fp::load_single(g, crate::leaves::ZERO_CELL)?; // lfs f10,23056(r11), on every path
    if count != 0 {
        let scale = fp::load_single(g, RMS_SCALE)?; // lfs f9,1092(r9)
        let history_step = (count << 2) & 0xFFFF_FFFC; // rlwinm r29,r28,2,0,29
        let mut slots = object.wrapping_add(METER_LEVELS); // addi r10,r3,272
        let mut frame_first = 0u32; // li r5,0
        let mut history_row = 0u32; // li r30,0
        for channel in 0..count {
            let frames = g.u32(object.wrapping_add(METER_FRAMES))?; // lwz r8,376(r3)
            let cursor = u32::from(g.u16(object.wrapping_add(METER_CURSOR))?); // lhz r9,388(r3)
            fp::store_single(g, slots.wrapping_add(32), floor)?; // stfs f10,32(r10)
            let (mut sum_even, mut peak, mut sum_odd) = (floor, floor, floor);
            let index = frames.wrapping_mul(channel).wrapping_add(cursor); // mullw ; add r7,r8,r9
            let mut lane = (channel << 4) & 0xFFFF_FFF0; // rlwinm r11,r31,4,0,27
            for _ in 0..2 {
                // Lanes 0 and 1, then 2 and 3: the even ones into one sum, the odd into the other.
                let s = fp::load_single(g, lane.wrapping_add(sums_at))?; // lfsx f8,r11,r9
                let p = fp::load_single(g, lane.wrapping_add(peaks_at))?; // lfsx f13,r11,r8
                sum_even = fp::add_single(s, sum_even); // fadds f11,f8,f11
                if peak < p {
                    peak = p; // fcmpu ; bge ; fmr f0,f13
                }
                let s = fp::load_single(g, lane.wrapping_add(sums_at).wrapping_add(4))?;
                let p = fp::load_single(g, lane.wrapping_add(peaks_at).wrapping_add(4))?;
                sum_odd = fp::add_single(s, sum_odd); // fadds f12,f8,f12
                if peak < p {
                    peak = p;
                }
                lane = lane.wrapping_add(8); // addi r11,r11,8
            }
            let frames_now = g.u32(object.wrapping_add(METER_FRAMES))?; // lwz r9,376(r3)
            let total = fp::add_single(sum_odd, sum_even); // fadds f13,f12,f11
            let level_off = u32::from(g.u16(object.wrapping_add(METER_LEVEL_RING))?);
            let slot = (index << 2) & 0xFFFF_FFFC; // rlwinm r7,r7,2,0,29
            let level_prev = fp::load_single(g, slots)?; // lfs f11,0(r10)
            let level_row = object.wrapping_add(level_off); // add r4,r11,r3
            let peak_off = u32::from(g.u16(object.wrapping_add(METER_PEAK_RING))?);
            let peak_row = object.wrapping_add(peak_off); // add r6,r11,r3
            let frames_f = f64::from(frames_now as i32); // extsw ; std ; lfd ; fcfid
            let weighted = fp::mul_single(total, scale); // fmuls f6,f13,f9
            let history = fp::load_single(g, slot.wrapping_add(level_row))?; // lfsx f5,r7,r4
            let divisor = fp::frsp(frames_f); // frsp f4,f7
            let mean = fp::div_single(weighted, divisor); // fdivs f12,f6,f4
            let delta = fp::sub_single(mean, history); // fsubs f3,f12,f5
            let level = fp::add_single(delta, level_prev); // fadds f2,f3,f11
            fp::store_single(g, slots, level)?; // stfs f2,0(r10)
            if level < floor {
                fp::store_single(g, slots, floor)?; // fcmpu ; bge ; stfs f10,0(r10)
            }
            let sum_prev = fp::load_single(g, slots.wrapping_sub(32))?; // lfs f13,-32(r10)
            fp::store_single(g, slots.wrapping_sub(32), fp::add_single(sum_prev, mean))?;
            if peak < floor {
                // Unreachable while the floor cell holds 0.0: every peak here is a magnitude.
                let slot_peak = fp::load_single(g, slot.wrapping_add(peak_row))?; // lfsx f13,r7,r6
                if !(slot_peak < floor) {
                    fp::store_single(g, slot.wrapping_add(peak_row), peak)?; // stfsx f0,r7,r6
                    let limit = g
                        .u32(object.wrapping_add(METER_FRAMES))?
                        .wrapping_add(frame_first);
                    if frame_first < limit {
                        let mut probe = history_row.wrapping_add(peak_row); // add r9,r30,r6
                        let mut at = frame_first; // mr r11,r5
                        loop {
                            let candidate = fp::load_single(g, probe)?; // lfs f13,0(r9)
                            let held = fp::load_single(g, slots.wrapping_add(32))?; // lfs f11,32(r10)
                            if held < candidate {
                                fp::store_single(g, slots.wrapping_add(32), candidate)?;
                            }
                            let bound = g
                                .u32(object.wrapping_add(METER_FRAMES))?
                                .wrapping_add(frame_first);
                            at = at.wrapping_add(1);
                            probe = probe.wrapping_add(4);
                            if at >= bound {
                                break; // cmplw cr6,r11,r8 ; blt
                            }
                        }
                    }
                }
            } else {
                let held = fp::load_single(g, slots.wrapping_add(64))?; // lfs f13,64(r10)
                fp::store_single(g, slots.wrapping_add(32), peak)?; // stfs f0,32(r10)
                if peak > held {
                    fp::store_single(g, slots.wrapping_add(64), peak)?; // stfs f0,64(r10)
                }
            }
            // loc_82B3762C, on every path.
            fp::store_single(g, slot.wrapping_add(peak_row), peak)?; // stfsx f0,r7,r6
            fp::store_single(g, slot.wrapping_add(level_row), mean)?; // stfsx f12,r7,r4
            slots = slots.wrapping_add(4);
            frame_first = frame_first.wrapping_add(count); // add r5,r5,r28
            history_row = history_row.wrapping_add(history_step); // add r30,r29,r30
        }
    }

    // loc_82B3764C: on the ring's last block, publish each running sum as the level and clear it.
    let frames = g.u32(object.wrapping_add(METER_FRAMES))?;
    let cursor = u32::from(g.u16(object.wrapping_add(METER_CURSOR))?);
    if cursor as i32 == frames.wrapping_sub(1) as i32 && count != 0 {
        let mut cell = object.wrapping_add(METER_SLOT_BASE); // addi r11,r3,236
        for _ in 0..count {
            let sum = fp::load_single(g, cell.wrapping_add(4))?; // lfs f0,4(r11)
            fp::store_single(g, cell.wrapping_add(36), sum)?; // stfs f0,36(r11)
            fp::store_single(g, cell.wrapping_add(4), floor)?; // stfsu f10,4(r11)
            cell = cell.wrapping_add(4);
        }
    }

    // loc_82B37680: (cursor + 1) mod frames, by a divide. The twllei/twlgei guards only log in
    // RexGlue, so a zero ring length divides to a zero quotient and the cursor becomes cursor + 1.
    let cursor_now = u32::from(g.u16(object.wrapping_add(METER_CURSOR))?); // lhz r11,388(r3)
    let frames_now = g.u32(object.wrapping_add(METER_FRAMES))?; // lwz r10,376(r3)
    let wrapped = cursor_now.wrapping_add(1) & 0xFFFF; // addi r9,r11,1 ; clrlwi r8,r9,16
    let (dividend, divisor) = (wrapped as i32, frames_now as i32);
    let quotient = if divisor != 0 && !(dividend == i32::MIN && divisor == -1) {
        (dividend / divisor) as u32
    } else {
        0
    };
    let remainder = wrapped.wrapping_sub(quotient.wrapping_mul(frames_now)); // mullw ; subf
    g.set_u16(object.wrapping_add(METER_CURSOR), remainder as u16)?; // sth r4,388(r3)
    Ok(())
}

/// The ring byte count: `round(seconds * max_rate)` blocks, channels wide, four bytes each, with a
/// one-entry-per-channel floor when that comes out zero.
fn ring_bytes(g: &Guest, object: u32, seconds: f64, half: f64) -> Result<u32> {
    let max_rate = fp::load_single(g, MAX_BLOCK_RATE)?; // lfs f12,444(r11)
    let channels = u32::from(g.u8(object.wrapping_add(METER_CHANNELS))?); // lbz r11,42(r31)
    let capacity = fp::fmadd_single(seconds, max_rate, half); // fmadds f0,f0,f12,f13
    let blocks = fp::fctidz(capacity) as u32; // fctidz ; stfd ; lwz r10,84(r1)
    let product = i64::from(blocks as i32) * i64::from(channels as i32); // mullw
    let bytes = ((product as u32) << 2) & 0xFFFF_FFFC; // rlwinm r30,r9,2,0,29
    Ok(if bytes != 0 {
        bytes
    } else {
        (channels << 2) & 0xFFFF_FFFC
    })
}

/// One metering tick (`sub_82B376B8`). Returns 1.
///
/// `object` is `r3` at full width (the ring clears address `offset + r3` with a 64-bit add), `desc`
/// is `r4`, `sp` is `r1`. Until the interval expires the tick only counts. Once it does, a changed
/// rate — or a NaN one, which never compares equal — recomputes the ring length as
/// `round(rate * blocks_per_hz * seconds)` through one fused multiply-add, floors it at 1, and clears
/// both rings for `ring_bytes`. The format pointer is re-read after the rate cache is stored, and the
/// peak ring's offset after the level ring is cleared. Then the counter restarts at 1 and the meters
/// and the layout expansion run, [`meter_block`] with its red zone below this function's 128-byte
/// frame.
pub fn meter_tick(g: &mut Guest, object: u64, desc: u32, sp: u32) -> Result<u64> {
    let obj = object as u32;
    let frame = sp.wrapping_sub(TICK_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-128(r1)
    let interval = g.u32(obj.wrapping_add(METER_INTERVAL))?; // lwz r10,380(r3)
    let ticks = u32::from(g.u16(obj.wrapping_add(METER_TICKS))?); // lhz r11,390(r3)
    if (ticks as i32) < (interval as i32) {
        g.set_u16(obj.wrapping_add(METER_TICKS), ticks.wrapping_add(1) as u16)?; // sth r11,390(r3)
        return Ok(1);
    }
    let format = g.u32(desc.wrapping_add(TICK_FORMAT))?; // lwz r11,40(r29)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let cached = fp::load_single(g, obj.wrapping_add(METER_RATE_CACHE))?; // lfs f0,368(r31)
    let rate = fp::load_single(g, format.wrapping_add(FORMAT_RATE))?; // lfs f13,12(r11)
    if cached != rate {
        let seconds = fp::load_single(g, obj.wrapping_add(METER_SECONDS))?; // lfs f0,372(r31)
        let rate_again = fp::load_single(g, format.wrapping_add(FORMAT_RATE))?; // lfs f13,12(r10)
        fp::store_single(g, obj.wrapping_add(METER_RATE_CACHE), rate_again)?; // stfs f13,368(r31)
        let format_again = g.u32(desc.wrapping_add(TICK_FORMAT))?; // lwz r8,40(r29) -- re-read
        let half = fp::load_single(g, HALF)?;
        let blocks_per_hz = fp::load_single(g, BLOCKS_PER_HZ)?;
        let rate_now = fp::load_single(g, format_again.wrapping_add(FORMAT_RATE))?; // lfs f11,12(r8)
        let blocks_per_second = fp::mul_single(rate_now, blocks_per_hz); // fmuls f10,f11,f12
        let frames = fp::fctiwz_low_word(fp::fmadd_single(blocks_per_second, seconds, half));
        g.set_u32(obj.wrapping_add(METER_FRAMES), frames)?; // stw r7,376(r31)
        if (frames as i32) <= 0 {
            g.set_u32(obj.wrapping_add(METER_FRAMES), 1)?; // stw r28,376(r31)
        }
        let bytes = u64::from(ring_bytes(g, obj, seconds, half)?);
        let level_off = u64::from(g.u16(obj.wrapping_add(METER_LEVEL_RING))?); // lhz r11,384(r31)
        mem::memset(g, level_off.wrapping_add(object) as u32, 0, bytes)?; // bl 0x82ee5e80
        let peak_off = u64::from(g.u16(obj.wrapping_add(METER_PEAK_RING))?); // after the first clear
        mem::memset(g, peak_off.wrapping_add(object) as u32, 0, bytes)?; // bl 0x82ee5e80
        g.set_u16(obj.wrapping_add(METER_CURSOR), 0)?; // sth r11,388(r31)
    }
    drop(fpscr);
    let channels = g.u32(desc.wrapping_add(TICK_CHANNELS))?; // lwz r4,28(r29)
    g.set_u16(obj.wrapping_add(METER_TICKS), 1)?; // sth r28,390(r31)
    meter_block(g, obj, channels, frame)?; // bl 0x82b373c8
    layout::expand_layout(g, obj)?; // bl 0x82b370e8, r3 still the object
    Ok(1) // li r3,1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const DESC: u32 = BASE + 0x2000;
    const TICK_DESC: u32 = BASE + 0x2100;
    const FORMAT: u32 = BASE + 0x2200;
    const CHANNELS_AT: u32 = BASE + 0x4000;
    const SP: u32 = BASE + 0x8000;
    const LEVEL_RING: u32 = 0x400;
    const PEAK_RING: u32 = 0xC00;

    /// Scale 1/64, so the summed squares of a constant block divide back to a clean mean; half 0.5;
    /// 1/256 blocks per hertz and a 200-block ring rate.
    fn guest() -> Guest {
        let mut g = Guest::from_segments(vec![
            Segment {
                base: BASE,
                bytes: vec![0u8; 0x10000],
            },
            Segment {
                base: 0x8209_9000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8216_5000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x822F_8000,
                bytes: vec![0u8; 0x1000],
            },
        ]);
        let f = |g: &mut Guest, at: u32, v: f32| g.set_u32(at, v.to_bits()).unwrap();
        f(&mut g, RMS_SCALE, 1.0 / 64.0);
        f(&mut g, HALF, 0.5);
        f(&mut g, BLOCKS_PER_HZ, 1.0 / 256.0);
        f(&mut g, MAX_BLOCK_RATE, 200.0);
        f(&mut g, crate::leaves::ZERO_CELL, 0.0);
        g.set_u32(DESC + CHANNEL_BASE, CHANNELS_AT).unwrap();
        g.set_u16(DESC + CHANNEL_STRIDE, 256).unwrap();
        g.set_u8(OBJECT + METER_CHANNELS, 2).unwrap();
        g.set_u32(OBJECT + METER_FRAMES, 4).unwrap();
        g.set_u16(OBJECT + METER_LEVEL_RING, LEVEL_RING as u16)
            .unwrap();
        g.set_u16(OBJECT + METER_PEAK_RING, PEAK_RING as u16)
            .unwrap();
        g
    }

    /// Channel `c` holds `value` in the quarter the meter reads, and `noise` everywhere else.
    fn channel(g: &mut Guest, c: u32, value: f32, noise: f32) {
        for i in 0..256u32 {
            let v = if (i * 4) % 64 < 16 { value } else { noise };
            g.set_u32(CHANNELS_AT + 1024 * c + 4 * i, v.to_bits())
                .unwrap();
        }
    }

    fn at(g: &Guest, off: u32) -> f32 {
        g.f32(OBJECT + off).unwrap()
    }

    #[test]
    fn a_constant_block_meters_to_its_mean_square_and_its_magnitude() {
        let mut g = guest();
        channel(&mut g, 0, 0.5, 100.0);
        channel(&mut g, 1, -0.25, -100.0);
        meter_block(&mut g, OBJECT, DESC, SP).unwrap();
        // Channel 0: each lane's four accumulators sum sixteen 0.25s, so the lanes hold 16.0, the
        // two passes total 64, and 64 * 1/64 over a four-block ring is a mean of 0.25.
        assert_eq!(at(&g, METER_LEVELS), 0.25, "the level, from a zero history");
        assert_eq!(at(&g, METER_SUMS), 0.25, "the running sum");
        assert_eq!(at(&g, METER_PEAKS), 0.5);
        assert_eq!(at(&g, METER_HOLDS), 0.5, "a higher peak is held");
        // Channel 1: the sign is cleared before squaring, and the unread three quarters never count.
        assert_eq!(at(&g, METER_LEVELS + 4), 0.0625);
        assert_eq!(at(&g, METER_PEAKS + 4), 0.25);
        // The ring histories, at frames * channel + cursor.
        assert_eq!(g.f32(OBJECT + LEVEL_RING).unwrap(), 0.25);
        assert_eq!(
            g.f32(OBJECT + PEAK_RING + 16).unwrap(),
            0.25,
            "channel 1's entry, four blocks on"
        );
        assert_eq!(g.u16(OBJECT + METER_CURSOR).unwrap(), 1);
    }

    #[test]
    fn the_level_moves_by_the_change_from_the_mean_it_replaces() {
        let mut g = guest();
        channel(&mut g, 0, 0.5, 0.0);
        g.set_u32(OBJECT + METER_LEVELS, 1.0f32.to_bits()).unwrap();
        g.set_u32(OBJECT + LEVEL_RING, 0.125f32.to_bits()).unwrap(); // the mean four blocks ago
        g.set_u32(OBJECT + METER_HOLDS, 2.0f32.to_bits()).unwrap();
        meter_block(&mut g, OBJECT, DESC, SP).unwrap();
        assert_eq!(at(&g, METER_LEVELS), 1.125, "1.0 + (0.25 - 0.125)");
        assert_eq!(at(&g, METER_HOLDS), 2.0, "a lower peak leaves the hold");
    }

    #[test]
    fn the_rings_last_block_publishes_the_sums_and_clears_them() {
        let mut g = guest();
        channel(&mut g, 0, 0.5, 0.0);
        g.set_u16(OBJECT + METER_CURSOR, 3).unwrap();
        g.set_u32(OBJECT + METER_SUMS, 0.5f32.to_bits()).unwrap();
        meter_block(&mut g, OBJECT, DESC, SP).unwrap();
        assert_eq!(
            at(&g, METER_LEVELS),
            0.75,
            "0.5 already summed, plus this block's 0.25"
        );
        assert_eq!(at(&g, METER_SUMS), 0.0);
        assert_eq!(
            g.u16(OBJECT + METER_CURSOR).unwrap(),
            0,
            "and the cursor wraps"
        );
    }

    #[test]
    fn with_no_channels_only_the_cursor_moves() {
        let mut g = guest();
        g.set_u8(OBJECT + METER_CHANNELS, 0).unwrap();
        g.set_u16(OBJECT + METER_CURSOR, 2).unwrap();
        meter_block(&mut g, OBJECT, DESC, SP).unwrap();
        assert_eq!(g.u16(OBJECT + METER_CURSOR).unwrap(), 3);
        assert_eq!(at(&g, METER_PEAKS), 0.0);
    }

    /// `poison` fills both rings, to show how much of each a resize clears; left zero otherwise,
    /// because a poisoned history is a huge float and would drive every level to the floor.
    fn tick_guest(interval: u32, ticks: u16, cached: f32, poison: bool) -> Guest {
        let mut g = guest();
        channel(&mut g, 0, 0.5, 0.0);
        channel(&mut g, 1, 0.25, 0.0);
        g.set_u32(OBJECT + METER_INTERVAL, interval).unwrap();
        g.set_u16(OBJECT + METER_TICKS, ticks).unwrap();
        g.set_u32(OBJECT + METER_RATE_CACHE, cached.to_bits())
            .unwrap();
        g.set_u32(OBJECT + METER_SECONDS, 1.0f32.to_bits()).unwrap();
        g.set_u32(TICK_DESC + TICK_CHANNELS, DESC).unwrap();
        g.set_u32(TICK_DESC + TICK_FORMAT, FORMAT).unwrap();
        g.set_u32(FORMAT + FORMAT_RATE, 48_000.0f32.to_bits())
            .unwrap();
        g.set_u8(OBJECT + layout::LAYOUT, 2).unwrap();
        for i in 0..if poison { 0x680u32 / 4 } else { 0 } {
            g.set_u32(OBJECT + LEVEL_RING + 4 * i, 0x7777_7777).unwrap();
            g.set_u32(OBJECT + PEAK_RING + 4 * i, 0x7777_7777).unwrap();
        }
        g
    }

    #[test]
    fn before_the_interval_the_tick_only_counts() {
        let mut g = tick_guest(5, 2, 48_000.0, false);
        let before = g.clone();
        assert_eq!(
            meter_tick(&mut g, u64::from(OBJECT), TICK_DESC, SP).unwrap(),
            1
        );
        assert_eq!(g.u16(OBJECT + METER_TICKS).unwrap(), 3);
        assert_eq!(
            g.u32(OBJECT + METER_PEAKS).unwrap(),
            before.u32(OBJECT + METER_PEAKS).unwrap()
        );
    }

    #[test]
    fn an_expired_interval_runs_the_meters_then_the_layout() {
        let mut g = tick_guest(5, 5, 48_000.0, false);
        let mut h = g.clone();
        meter_tick(&mut g, u64::from(OBJECT), TICK_DESC, SP).unwrap();
        h.set_u16(OBJECT + METER_TICKS, 1).unwrap();
        meter_block(&mut h, OBJECT, DESC, SP - TICK_FRAME).unwrap();
        layout::expand_layout(&mut h, OBJECT).unwrap();
        for off in (0..0x400).step_by(4) {
            assert_eq!(
                g.u32(OBJECT + off).unwrap(),
                h.u32(OBJECT + off).unwrap(),
                "+{off}"
            );
        }
        assert_eq!(
            g.u32(OBJECT + layout::group_dest(0) + 4).unwrap(),
            0.5f32.to_bits(),
            "sqrt of the 0.25 level"
        );
    }

    #[test]
    fn a_changed_rate_resizes_and_clears_both_rings() {
        let mut g = tick_guest(0, 0, 44_100.0, true);
        meter_tick(&mut g, u64::from(OBJECT), TICK_DESC, SP).unwrap();
        // 48000 / 256 = 187.5 blocks a second; one second plus the half, truncated, is 188.
        assert_eq!(g.u32(OBJECT + METER_FRAMES).unwrap(), 188);
        assert_eq!(g.f32(OBJECT + METER_RATE_CACHE).unwrap(), 48_000.0);
        // round(1.0 * 200) blocks, two channels wide: 1,600 bytes of each ring cleared.
        assert_eq!(g.u32(OBJECT + LEVEL_RING + 1596).unwrap(), 0);
        assert_eq!(g.u32(OBJECT + LEVEL_RING + 1600).unwrap(), 0x7777_7777);
        assert_eq!(g.u32(OBJECT + PEAK_RING + 1600).unwrap(), 0x7777_7777);
        // The meters then ran from cursor 0 over the fresh ring, dividing by the new length: channel
        // 0's summed squares scale to 1.0 and channel 1's to 0.25, each over 188 blocks.
        assert_eq!(g.f32(OBJECT + LEVEL_RING).unwrap(), 1.0f32 / 188.0);
        assert_eq!(
            g.f32(OBJECT + LEVEL_RING + 4 * 188).unwrap(),
            0.25f32 / 188.0,
            "channel 1, one ring on"
        );
        assert_eq!(g.u16(OBJECT + METER_CURSOR).unwrap(), 1);
    }
}
