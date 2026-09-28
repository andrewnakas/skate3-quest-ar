//! The evaluator's shaper slots: ops that turn a position into a sample.
//!
//! Three of them, all transcriptions of `STATUS: verified` bodies: an oscillator reading a
//! quarter-sine table, a multi-segment envelope, and a sampled curve with two interpolation modes.
//! What separates these from `state` is that each reads a **table** — the image's sine table, the
//! object's own segment array, a curve object the cursor points at — rather than a handful of
//! scalar fields, so an index formula rather than a branch is the thing most likely to be wrong.
//! Every index formula below is therefore written as the original's shift-and-add rather than as
//! `base + stride * i`, even where the two agree.

use crate::eval::{
    HALF_SINGLE, ONE_SINGLE, PHASE_TO_UNITS, SINE_NORM, SINE_TABLE, TICK_SCALE_GLOBAL,
    TRIANGLE_GAIN, ZERO_SINGLE,
};
use crate::fp;
use crate::{Guest, Result};

/// Byte offset of the mirror point inside the sine table: quadrants 1 and 3 read backwards from
/// here, so the halfword *at* the mirror is entry 256 and the table's used extent is 514 bytes.
pub const SINE_MIRROR: u32 = 512;

const _: () = assert!(
    crate::eval::SINE_TABLE_BYTES == SINE_MIRROR as usize + 2,
    "the mirror entry"
);

/// `extsw ; std ; lfd ; fcfid ; frsp` on a value already narrowed to `i32`.
#[inline]
fn int_to_single(value: i32) -> f64 {
    fp::word_to_single(value as u32)
}

/// Round half away from zero against the guest's own `0.0f` and `0.5f` cells, then truncate.
///
/// A NaN is not "less than" zero, so it takes the *add* side, exactly as `cr6.lt` would.
#[inline]
fn round_to_word(value: f64, half: f64, zero: f64) -> u32 {
    let adjusted = if value < zero {
        fp::sub_single(value, half)
    } else {
        fp::add_single(value, half)
    };
    fp::fctiwz_low_word(adjusted)
}

/// Slot 28 — `sub_82B1D3D0`. Step one oscillator: wrap its phase, sample the waveform, advance the
/// phase and return the sample as an integer.
///
/// Layout: `u8 waveform` at `+0` (0 quarter-sine table, 1 pulse, 2 ramp, 3 or more triangle),
/// `f32 phase` at `+4`, `s32 period` at `+8` in update ticks, `s32 amplitude` at `+12`.
///
/// A period of zero or less returns 0 and touches nothing. Otherwise the phase advances by
/// `tick_scale / period` and wraps at the guest's `1.0f` cell, and the sample is rounded half away
/// from zero into a word. Writes: `+4` only.
///
/// ## The quarter-sine table
///
/// Waveform 0 scales the phase by the `1024.0f` cell into table units, rounds it to a word, and
/// then splits that word: bits 9:8 pick the quadrant and bits 7:0 the entry. So one cycle is 1024
/// units over four quadrants of 256 entries, read out of a **quarter** table of 257 halfwords at
/// `0x82FD36B8` — quadrant 0 forward from the start, quadrant 1 backward from the mirror at +512,
/// and quadrants 2 and 3 the same two negated.
///
/// Read out of the validated image dump, that table is exactly
///
/// ```text
/// entry[i] = min(65535, floor(65536 * sin(i * pi / 512)))      for i in 0..=256
/// ```
///
/// for all 257 entries with no exceptions — `floor`, not round, and the clamp matters only at
/// entry 256 where the exact value is 65536. The normalisation constant that follows the lookup is
/// `1/65536`, so a full-scale sample returns `amplitude * 65535/65536` rather than `amplitude`.
/// None of that is assumed by this code, which reads the table and both constants live; it is
/// recorded because a test needs to build a table and because an off-by-one in the mirror
/// arithmetic is otherwise invisible.
///
/// ## The wrap loop does not terminate on a NaN or an infinite phase
///
/// `while !(phase < wrap)` is an unordered comparison, so a NaN phase enters the loop and stays in
/// it, subtracting `1.0` from a NaN forever; an infinite phase does the same. The original behaves
/// identically and the C++ port reproduces it deliberately rather than guarding it, so this does
/// too. It is a genuine hang, not a theoretical one: **do not** feed this a phase from an untrusted
/// source, and do not fuzz it without a watchdog.
pub fn op_oscillator(g: &mut Guest, osc: u32) -> Result<u64> {
    let period_word = g.u32(osc + 8)?;
    if !((period_word as i32) > 0) {
        return Ok(0);
    }
    let amplitude_word = g.u32(osc + 12)?;
    let mut phase = fp::load_single(g, osc + 4)?;
    let period_f = fp::word_to_single(period_word);
    let amplitude = fp::word_to_single(amplitude_word);
    let wrap = fp::load_single(g, ONE_SINGLE)?;
    let increment = fp::div_single(fp::load_single(g, TICK_SCALE_GLOBAL)?, period_f);

    // An unordered compare falls INTO the loop and the loop's own test keeps it there. The phase is
    // reloaded from memory each pass, as the original does; the reload is lossless because the
    // value just stored is single-exact.
    if !(phase < wrap) {
        loop {
            phase = fp::sub_single(fp::load_single(g, osc + 4)?, wrap);
            fp::store_single(g, osc + 4, phase)?;
            if phase < wrap {
                break;
            }
        }
    }

    let waveform = g.u8(osc)?;
    phase = fp::load_single(g, osc + 4)?;
    let half = fp::load_single(g, HALF_SINGLE)?;
    let zero = fp::load_single(g, ZERO_SINGLE)?;

    let out = if waveform == 0 {
        let units = fp::mul_single(phase, fp::load_single(g, PHASE_TO_UNITS)?);
        let fixed = round_to_word(units, half, zero);
        // srawi r10,r11,8 ; clrlwi r10,r10,30 — an ARITHMETIC shift, then two bits.
        let quadrant = ((fixed as i32) >> 8) as u32 & 3;
        let offset = (fixed & 0xFF) << 1;
        let sample: i32 = match quadrant {
            0 => g.u16(offset.wrapping_add(SINE_TABLE))? as i32,
            1 => g.u16(SINE_TABLE.wrapping_add(SINE_MIRROR).wrapping_sub(offset))? as i32,
            // neg of a zero-extended halfword, kept in the low word.
            2 => -(g.u16(offset.wrapping_add(SINE_TABLE))? as i32),
            _ => -(g.u16(SINE_TABLE.wrapping_add(SINE_MIRROR).wrapping_sub(offset))? as i32),
        };
        let sample_f = int_to_single(sample);
        fp::mul_single(
            fp::mul_single(sample_f, amplitude),
            fp::load_single(g, SINE_NORM)?,
        )
    } else if waveform == 1 {
        if phase < half { zero } else { amplitude }
    } else if waveform == 2 {
        fp::mul_single(phase, amplitude)
    } else if phase < half {
        fp::mul_single(
            fp::mul_single(phase, amplitude),
            fp::load_single(g, TRIANGLE_GAIN)?,
        )
    } else {
        // `wrap`, not `half`: the register still holds the 1.0 cell loaded at entry, which is why
        // the original reads that address twice.
        fp::mul_single(
            fp::mul_single(fp::sub_single(wrap, phase), amplitude),
            fp::load_single(g, TRIANGLE_GAIN)?,
        )
    };

    // The phase advances on every path, including the pulse's flat sections.
    fp::store_single(g, osc + 4, fp::add_single(phase, increment))?;
    Ok(round_to_word(out, half, zero) as u64)
}

/// Slot 14 — `sub_82B1C910`. Step a multi-segment envelope: start it, jump it to a named segment, or
/// count the current segment down and re-arm the next one. Returns the current value, rounded.
///
/// Layout: `u16 mode_offset` at `+0` (a **self-relative** byte offset of the mode word), `u8
/// mode_prev` at `+2`, `u8 index` at `+3`, `f32 timer` at `+4`, `f32 step` at `+8`, `f32 value` at
/// `+12`, `u8 count` at `+16`, `u16 jump` at `+18`, `f32 initial` at `+20`, and the segments from
/// `+24` at a stride of 8: `{ f32 duration, f32 target }`.
///
/// Modes: **1** start or run, **2** hold (the value is left alone), **3** jump or run; anything else
/// forces the value to zero. Start and jump are edge-triggered off `mode_prev`, which the tail
/// rewrites from the mode word's low byte — so the mode word is *re-read after the stores*, which is
/// observable whenever `mode_offset` points inside `+2..+15`, and is therefore reloaded here rather
/// than reused.
///
/// Writes: `+2` always; `+3`, `+4`, `+8` and `+12` depending on the path.
///
/// Two details that a tidier implementation would get wrong. The jump is gated by a **signed
/// halfword** compare of `jump` against the current index, but only `jump`'s low byte indexes the
/// segment array — so a jump field of `0x0105` gates on 261 and lands on segment 5. And when a
/// segment completes, the next segment's slope is derived from the **target just reached**, not from
/// the value word that was stored from it; the two are equal here, and the original's choice is kept
/// so that they stay equal if one of them is ever found not to be.
pub fn op_envelope(g: &mut Guest, env: u32) -> Result<u64> {
    const SEGMENTS: u32 = 24;
    const STRIDE: u32 = 8;
    const TARGET: u32 = 4;

    let mode_offset = g.u16(env)? as u32;
    let mode = g.u32(mode_offset.wrapping_add(env))? as i32;
    // Held in one register all the way to the tail, so it is loaded once here.
    let zero = fp::load_single(g, ZERO_SINGLE)?;
    let mut handled = false;
    let mut clear_value = false;

    // A start: mode 1 seen for the first time.
    if mode == 1 && g.u8(env + 2)? == 0 {
        let initial = fp::load_single(g, env + 20)?;
        g.set_u8(env + 3, 0)?;
        fp::store_single(g, env + 12, initial)?;
        let duration = fp::load_single(g, env + SEGMENTS)?;
        fp::store_single(g, env + 4, duration)?;
        let target = fp::load_single(g, env + SEGMENTS + TARGET)?;
        let slope = fp::div_single(fp::sub_single(target, initial), duration);
        let step = fp::mul_single(slope, fp::load_single(g, TICK_SCALE_GLOBAL)?);
        fp::store_single(g, env + 8, step)?;
        handled = true;
    }

    // A jump: mode 3 seen for the first time, and only forward.
    if !handled && mode == 3 && g.u8(env + 2)? != 3 {
        let jump = g.u16(env + 18)?;
        let current = g.u8(env + 3)? as i32;
        if current < ((jump as i16) as i32) {
            let index = (jump & 0xFF) as u32; // only the low byte indexes
            let value = fp::load_single(g, env + 12)?;
            g.set_u8(env + 3, index as u8)?;
            let segment = env + SEGMENTS + index * STRIDE;
            let duration = fp::load_single(g, segment)?;
            fp::store_single(g, env + 4, duration)?;
            let target = fp::load_single(g, segment + TARGET)?;
            let slope = fp::div_single(fp::sub_single(target, value), duration);
            let step = fp::mul_single(slope, fp::load_single(g, TICK_SCALE_GLOBAL)?);
            fp::store_single(g, env + 8, step)?;
            handled = true;
        }
    }

    if !handled {
        if mode == 1 || mode == 3 {
            let count = g.u8(env + 16)? as u32;
            let index = g.u8(env + 3)? as u32;
            if index < count {
                let timer = fp::load_single(g, env + 4)?;
                let delta = fp::load_single(g, TICK_SCALE_GLOBAL)?;
                let left = fp::sub_single(timer, delta);
                fp::store_single(g, env + 4, left)?;
                // An unordered compare takes the segment-done side.
                if left > zero {
                    let value = fp::load_single(g, env + 12)?;
                    let step = fp::load_single(g, env + 8)?;
                    fp::store_single(g, env + 12, fp::add_single(step, value))?;
                } else {
                    let next = (index + 1) & 0xFF;
                    let reached = fp::load_single(g, env + SEGMENTS + index * STRIDE + TARGET)?;
                    g.set_u8(env + 3, next as u8)?;
                    fp::store_single(g, env + 12, reached)?;
                    if next < count {
                        let segment = env + SEGMENTS + next * STRIDE;
                        let duration = fp::load_single(g, segment)?;
                        fp::store_single(g, env + 4, duration)?;
                        let target = fp::load_single(g, segment + TARGET)?;
                        // From the target just reached, not from the stored value word.
                        let slope = fp::div_single(fp::sub_single(target, reached), duration);
                        fp::store_single(g, env + 8, fp::mul_single(slope, delta))?;
                    } else {
                        clear_value = true;
                    }
                }
            } else {
                clear_value = true;
            }
        } else if mode != 2 {
            clear_value = true;
        }
    }
    if clear_value {
        fp::store_single(g, env + 12, zero)?;
    }

    // The mode word is re-read after the stores above; see the note on the function.
    let mode_again = g.u32(mode_offset.wrapping_add(env))?;
    let value = fp::load_single(g, env + 12)?;
    // The sign test happens BEFORE the byte store and before the half constant is loaded.
    let negative = value < zero;
    g.set_u8(env + 2, (mode_again & 0xFF) as u8)?;
    let half = fp::load_single(g, HALF_SINGLE)?;
    let rounded = if negative {
        fp::sub_single(value, half)
    } else {
        fp::add_single(value, half)
    };
    Ok(fp::fctiwz_low_word(rounded) as u64)
}

/// Slot 15 — `sub_82B1CAD8`. Evaluate a sampled curve at a cursor's position, nearest or linear, and
/// cache the result.
///
/// Cursor layout: `u32 curve` at `+0`, `s32 last_pos` at `+4`, `u32 value` at `+8`, `s32 pos` at
/// `+12`. Curve layout: `u8 type` at `+0` (2 = `s16` samples, 1 = `s8`, anything else `s32`), `u16
/// count` at `+2`, `s32 start` at `+4`, `s32 end` at `+8`, `f32 scale` at `+12`, samples from `+16`.
///
/// A position equal to the cached one short-circuits: no store, no float work. Otherwise the
/// position is clamped into `[start, end]` and `index = pos - start`. A scale of **exactly 1.0**
/// selects nearest-sample mode, where the index *is* the sample index and there is no bounds check
/// at all; any other scale interpolates linearly between the samples either side of `index * scale`,
/// with the upper neighbour clamped to `count - 1`.
///
/// Writes: `+4` and `+8`.
///
/// **Neither path is fully guarded, deliberately.** Nearest mode does no bounds check at all, and
/// linear mode clamps only the *upper* neighbour — `i0` is used as computed, so a scaled index past
/// the samples reads past the array. Both are what the original does. In C++ that read went wherever
/// the guest map allowed; here it is either a valid byte of some other segment or an `Error`, and
/// neither matches what hardware would have done. The `Windows()` predicate does not try to bound it
/// either, and the recorded vectors never exercised it. This was mis-transcribed once, as "the curve
/// flattens at its last sample", and caught by a test written to that wrong expectation.
///
/// The two arms of the final branch differ in the original only in whether `r3` comes from the word
/// just stored or from a reload of it — the same address, so the same value. Written as one path.
pub fn op_curve(g: &mut Guest, cursor: u32) -> Result<u64> {
    const CURVE_TYPE: u32 = 0;
    const CURVE_COUNT: u32 = 2;
    const CURVE_START: u32 = 4;
    const CURVE_END: u32 = 8;
    const CURVE_SCALE: u32 = 12;
    const CURVE_SAMPLES: u32 = 16;

    let pos = g.u32(cursor + 12)?;
    let last = g.u32(cursor + 4)?;
    if (pos as i32) == (last as i32) {
        return Ok(g.u32(cursor + 8)? as u64);
    }

    // The curve pointer is loaded BEFORE the store to +4; the curve's own fields after it.
    let curve = g.u32(cursor)?;
    g.set_u32(cursor + 4, pos)?;
    let start = g.u32(curve + CURVE_START)?;
    let end = g.u32(curve + CURVE_END)?;
    let clamped = if (pos as i32) < (start as i32) {
        start
    } else if (pos as i32) > (end as i32) {
        end
    } else {
        pos
    };

    let scale = fp::load_single(g, curve + CURVE_SCALE)?;
    // A 64-bit subtract of two zero-extended words, but only the low 32 bits are ever used.
    let index = clamped.wrapping_sub(start);
    let one = fp::load_single(g, ONE_SINGLE)?;

    // Nearest sample: scale 1.0 means position units are sample units. No bounds check.
    if scale == one {
        let value = match g.u8(curve + CURVE_TYPE)? {
            2 => (g.u16(curve.wrapping_add((index.wrapping_add(8)) << 1))? as i16) as i32 as u32,
            1 => (g.u8(curve.wrapping_add(index).wrapping_add(CURVE_SAMPLES))? as i8) as i32 as u32,
            _ => g.u32(curve.wrapping_add((index.wrapping_add(4)) << 2))?,
        };
        g.set_u32(cursor + 8, value)?;
        return Ok(value as u64);
    }

    // Linear interpolation between the two samples around index * scale.
    let fpos = int_to_single(index as i32);
    let half = fp::load_single(g, HALF_SINGLE)?;
    let zero = fp::load_single(g, ZERO_SINGLE)?;
    let scaled = fp::mul_single(fpos, scale);
    let biased = fp::sub_single(scaled, half);
    let i0 = round_to_word(biased, half, zero);

    let count = g.u16(curve + CURVE_COUNT)? as u32;
    let fi0 = int_to_single(i0 as i32);
    let frac = fp::sub_single(scaled, fi0);
    let mut i1 = i0.wrapping_add(1);
    if !((i1 as i32) < (count as i32)) {
        i1 = count.wrapping_sub(1);
    }

    // Each sample is converted through the red zone with fcfid then frsp, so the narrowing is
    // reproduced. Order within each arm follows the lifted register assignments.
    let (sample0, sample1) = match g.u8(curve + CURVE_TYPE)? {
        2 => {
            let a = (g.u16(curve.wrapping_add((i0.wrapping_add(8)) << 1))? as i16) as i32;
            let b = (g.u16(curve.wrapping_add((i1.wrapping_add(8)) << 1))? as i16) as i32;
            (int_to_single(a), int_to_single(b))
        }
        1 => {
            let b = (g.u8(curve.wrapping_add(i1).wrapping_add(CURVE_SAMPLES))? as i8) as i32;
            let a = (g.u8(curve.wrapping_add(i0).wrapping_add(CURVE_SAMPLES))? as i8) as i32;
            (int_to_single(a), int_to_single(b))
        }
        _ => {
            let b = g.u32(curve.wrapping_add((i1.wrapping_add(4)) << 2))? as i32;
            let a = g.u32(curve.wrapping_add((i0.wrapping_add(4)) << 2))? as i32;
            (int_to_single(a), int_to_single(b))
        }
    };

    let delta = fp::sub_single(sample1, sample0);
    let mixed = fp::fmadd_single(delta, frac, sample0);
    let value = round_to_word(mixed, half, zero);
    g.set_u32(cursor + 8, value)?;
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::SINE_TABLE_BYTES;
    use crate::eval::testutil::*;

    /// The quarter-sine table as the image holds it, built from the formula the dump was shown to
    /// satisfy for all 257 entries.
    fn sine_table_bytes() -> Vec<u8> {
        let mut out = Vec::with_capacity(SINE_TABLE_BYTES);
        for i in 0..=256u32 {
            let exact = 65536.0 * (i as f64 * std::f64::consts::PI / 512.0).sin();
            let entry = (exact.floor() as u64).min(65535) as u16;
            out.extend_from_slice(&entry.to_be_bytes());
        }
        out
    }

    /// An oscillator with the image's own constants mapped, `tick_scale` set so that one call
    /// advances the phase by `1/period`.
    fn oscillator(g: &mut Guest, waveform: u8, phase: f32, period: i32, amplitude: i32) {
        put_rodata(g);
        g.put(ONE_SINGLE, 1.0f32.to_bits().to_be_bytes().to_vec());
        g.put(PHASE_TO_UNITS, 1024.0f32.to_bits().to_be_bytes().to_vec());
        g.put(
            SINE_NORM,
            (1.0f32 / 65536.0).to_bits().to_be_bytes().to_vec(),
        );
        g.put(TRIANGLE_GAIN, 2.0f32.to_bits().to_be_bytes().to_vec());
        g.put(SINE_TABLE, sine_table_bytes());
        g.set_u32(TICK_SCALE_GLOBAL, 1.0f32.to_bits()).unwrap();
        g.set_u8(BLOCK, waveform).unwrap();
        g.set_u32(BLOCK + 4, phase.to_bits()).unwrap();
        g.set_u32(BLOCK + 8, period as u32).unwrap();
        g.set_u32(BLOCK + 12, amplitude as u32).unwrap();
    }

    #[test]
    fn a_non_positive_period_returns_zero_and_writes_nothing() {
        let mut g = block_guest();
        oscillator(&mut g, 0, 0.3, 0, 65536);
        assert_eq!(op_oscillator(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 0.3, "the phase was not advanced");
        g.set_u32(BLOCK + 8, (-4i32) as u32).unwrap();
        assert_eq!(op_oscillator(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 0.3);
    }

    #[test]
    fn the_sine_quadrants_hit_their_exact_table_entries() {
        // Amplitude 65536 cancels the 1/65536 normalisation, so the returned value IS the table
        // entry — which makes an error in the quadrant or mirror arithmetic a wrong integer rather
        // than a rounding difference. The entry for 45 degrees is floor(65536 * sin(pi/4)) = 46340.
        let cases: [(f32, u32); 8] = [
            (0.0, 0),
            (0.125, 46340),
            (0.25, 65535),
            (0.375, 46340),
            (0.5, 0),
            (0.625, (-46340i32) as u32),
            (0.75, (-65535i32) as u32),
            (0.875, (-46340i32) as u32),
        ];
        for (phase, expect) in cases {
            let mut g = block_guest();
            oscillator(&mut g, 0, phase, 1024, 65536);
            assert_eq!(
                op_oscillator(&mut g, BLOCK).unwrap(),
                expect as u64,
                "phase {phase} should sample {expect:#x}"
            );
        }
    }

    #[test]
    fn the_sine_waveform_tracks_a_real_sine_over_a_whole_cycle() {
        // An independent shape check: nothing here re-implements the quadrant split, so a mirrored
        // or sign-flipped quadrant shows up as a gross error rather than a rounding one. The
        // tolerance covers the table's 1/1024-cycle index quantisation, whose worst case is half a
        // step of the steepest slope: 2*pi*amplitude/2048.
        let amplitude = 100_000i32;
        let tolerance = 2.0 * std::f64::consts::PI * amplitude as f64 / 2048.0 + 2.0;
        for k in 0..64 {
            let phase = k as f32 / 64.0;
            let mut g = block_guest();
            oscillator(&mut g, 0, phase, 1024, amplitude);
            let got = op_oscillator(&mut g, BLOCK).unwrap() as u32 as i32 as f64;
            let want = amplitude as f64 * (2.0 * std::f64::consts::PI * phase as f64).sin();
            assert!(
                (got - want).abs() <= tolerance,
                "phase {phase}: got {got}, sine says {want}, tolerance {tolerance}"
            );
        }
    }

    #[test]
    fn the_three_non_table_waveforms_are_what_their_names_say() {
        // Pulse: zero below the half point, amplitude at or above it.
        for (phase, expect) in [(0.0f32, 0u64), (0.4999, 0), (0.5, 1000), (0.9, 1000)] {
            let mut g = block_guest();
            oscillator(&mut g, 1, phase, 1024, 1000);
            assert_eq!(
                op_oscillator(&mut g, BLOCK).unwrap(),
                expect,
                "pulse at {phase}"
            );
        }

        // Ramp: phase * amplitude, rounded half away from zero.
        for (phase, expect) in [(0.0f32, 0u64), (0.25, 250), (0.5, 500), (0.999, 999)] {
            let mut g = block_guest();
            oscillator(&mut g, 2, phase, 1024, 1000);
            assert_eq!(
                op_oscillator(&mut g, BLOCK).unwrap(),
                expect,
                "ramp at {phase}"
            );
        }

        // Triangle: up at twice the ramp's slope, then down. Waveform 3 and 7 take the same path.
        for waveform in [3u8, 7, 255] {
            for (phase, expect) in [
                (0.0f32, 0u64),
                (0.25, 500),
                (0.5, 1000),
                (0.75, 500),
                (0.9, 200),
            ] {
                let mut g = block_guest();
                oscillator(&mut g, waveform, phase, 1024, 1000);
                assert_eq!(
                    op_oscillator(&mut g, BLOCK).unwrap(),
                    expect,
                    "triangle {waveform} at {phase}"
                );
            }
        }
    }

    #[test]
    fn the_phase_advances_by_tick_over_period_on_every_path() {
        for waveform in [0u8, 1, 2, 3] {
            let mut g = block_guest();
            oscillator(&mut g, waveform, 0.5, 4, 1000);
            op_oscillator(&mut g, BLOCK).unwrap();
            assert_eq!(
                g.f32(BLOCK + 4).unwrap(),
                0.75,
                "waveform {waveform} advanced by 1/4"
            );
        }
    }

    #[test]
    fn a_phase_past_the_wrap_is_reduced_before_it_is_sampled() {
        let mut g = block_guest();
        // 2.25 needs two subtractions of 1.0 to come back inside, and the sample must be taken
        // from 0.25, not from 2.25.
        oscillator(&mut g, 0, 2.25, 1024, 65536);
        assert_eq!(
            op_oscillator(&mut g, BLOCK).unwrap(),
            65535,
            "sampled at the wrapped phase"
        );
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 0.25 + 1.0 / 1024.0);

        // Exactly at the wrap: the compare is `<`, so 1.0 wraps to 0.0 rather than staying.
        let mut g = block_guest();
        oscillator(&mut g, 2, 1.0, 1024, 1000);
        assert_eq!(op_oscillator(&mut g, BLOCK).unwrap(), 0, "1.0 became 0.0");
    }

    /// An envelope whose mode word sits at `+0x60`, with `segments` as `{duration, target}` pairs.
    fn envelope(g: &mut Guest, mode: u32, initial: f32, segments: &[(f32, f32)]) {
        put_rodata(g);
        g.put(ONE_SINGLE, 1.0f32.to_bits().to_be_bytes().to_vec());
        // One tick per call keeps the durations readable as call counts.
        g.set_u32(TICK_SCALE_GLOBAL, 1.0f32.to_bits()).unwrap();
        g.set_u16(BLOCK, 0x60).unwrap();
        g.set_u32(BLOCK + 0x60, mode).unwrap();
        g.set_u8(BLOCK + 2, 0).unwrap();
        g.set_u8(BLOCK + 3, 0).unwrap();
        g.set_u32(BLOCK + 4, 0).unwrap();
        g.set_u32(BLOCK + 8, 0).unwrap();
        g.set_u32(BLOCK + 12, 0).unwrap();
        g.set_u8(BLOCK + 16, segments.len() as u8).unwrap();
        g.set_u16(BLOCK + 18, 0).unwrap();
        g.set_u32(BLOCK + 20, initial.to_bits()).unwrap();
        for (i, (duration, target)) in segments.iter().enumerate() {
            let at = BLOCK + 24 + 8 * i as u32;
            g.set_u32(at, duration.to_bits()).unwrap();
            g.set_u32(at + 4, target.to_bits()).unwrap();
        }
    }

    #[test]
    fn the_envelope_starts_once_then_walks_its_segments() {
        let mut g = block_guest();
        // Two segments: 0 -> 100 over four ticks, then 100 -> 50 over two.
        envelope(&mut g, 1, 0.0, &[(4.0, 100.0), (2.0, 50.0)]);

        // The start arms segment 0: the value is the initial and the step is (100-0)/4.
        assert_eq!(op_envelope(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.u8(BLOCK + 3).unwrap(), 0);
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 4.0, "the timer was armed");
        assert_eq!(g.f32(BLOCK + 8).unwrap(), 25.0, "the step");
        assert_eq!(
            g.u8(BLOCK + 2).unwrap(),
            1,
            "the mode byte was latched, so no second start"
        );

        // Four runs: the timer counts down and the value climbs by a step each time.
        for (call, expect) in [(1, 25u64), (2, 50), (3, 75)].iter() {
            assert_eq!(op_envelope(&mut g, BLOCK).unwrap(), *expect, "run {call}");
        }
        // The fourth tick takes the timer to zero, which ends the segment: the value snaps to the
        // target and segment 1 is armed with a step derived from it.
        assert_eq!(op_envelope(&mut g, BLOCK).unwrap(), 100);
        assert_eq!(g.u8(BLOCK + 3).unwrap(), 1, "advanced to segment 1");
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 2.0);
        assert_eq!(g.f32(BLOCK + 8).unwrap(), -25.0, "(50 - 100) / 2");

        assert_eq!(op_envelope(&mut g, BLOCK).unwrap(), 75);
        // Segment 1 ends: the value snaps to 50, the index advances past the count, and the value
        // is then forced to zero — which is what the call returns.
        assert_eq!(op_envelope(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(g.u8(BLOCK + 3).unwrap(), 2, "past the last segment");
        assert_eq!(g.f32(BLOCK + 12).unwrap(), 0.0, "and the value was cleared");
    }

    #[test]
    fn mode_two_holds_the_value_and_an_unknown_mode_clears_it() {
        let mut g = block_guest();
        envelope(&mut g, 1, 10.0, &[(4.0, 100.0)]);
        op_envelope(&mut g, BLOCK).unwrap(); // start
        op_envelope(&mut g, BLOCK).unwrap(); // one run

        let held = g.f32(BLOCK + 12).unwrap();
        assert!(held > 10.0, "the value moved: {held}");

        g.set_u32(BLOCK + 0x60, 2).unwrap();
        op_envelope(&mut g, BLOCK).unwrap();
        assert_eq!(g.f32(BLOCK + 12).unwrap(), held, "mode 2 holds");
        assert_eq!(
            g.f32(BLOCK + 4).unwrap(),
            3.0,
            "and does not count down either"
        );

        g.set_u32(BLOCK + 0x60, 9).unwrap();
        assert_eq!(op_envelope(&mut g, BLOCK).unwrap(), 0);
        assert_eq!(
            g.f32(BLOCK + 12).unwrap(),
            0.0,
            "an unknown mode clears the value"
        );
    }

    #[test]
    fn the_jump_is_gated_signed_on_a_halfword_but_indexes_by_its_low_byte() {
        let mut g = block_guest();
        envelope(&mut g, 1, 0.0, &[(4.0, 10.0), (4.0, 20.0), (4.0, 30.0)]);
        op_envelope(&mut g, BLOCK).unwrap(); // start, index 0

        // A jump field of 0x0102 gates on 258 (> index 0) and lands on segment 2.
        g.set_u16(BLOCK + 18, 0x0102).unwrap();
        g.set_u32(BLOCK + 0x60, 3).unwrap();
        op_envelope(&mut g, BLOCK).unwrap();
        assert_eq!(
            g.u8(BLOCK + 3).unwrap(),
            2,
            "the low byte chose the segment"
        );
        assert_eq!(g.f32(BLOCK + 4).unwrap(), 4.0);
        assert_eq!(g.f32(BLOCK + 8).unwrap(), 7.5, "(30 - 0) / 4");

        // A negative halfword can never gate a jump, whatever its low byte says.
        let mut g = block_guest();
        envelope(&mut g, 1, 0.0, &[(4.0, 10.0), (4.0, 20.0), (4.0, 30.0)]);
        op_envelope(&mut g, BLOCK).unwrap();
        g.set_u16(BLOCK + 18, 0xFF02).unwrap(); // -254 as a signed halfword
        g.set_u32(BLOCK + 0x60, 3).unwrap();
        op_envelope(&mut g, BLOCK).unwrap();
        assert_eq!(
            g.u8(BLOCK + 3).unwrap(),
            0,
            "no jump; it ran segment 0 instead"
        );
    }

    #[test]
    fn the_mode_word_is_reloaded_after_the_stores_when_it_aliases_them() {
        // The one case where reusing the mode read at entry would diverge: put the mode word ON the
        // value word at +12, so the start path's own store rewrites it. The byte latched into +2
        // must come from the reload, which sees the initial value's bits, not from the 1 that was
        // read at entry.
        let mut g = block_guest();
        envelope(&mut g, 1, 0.0, &[(4.0, 100.0)]);
        g.set_u16(BLOCK, 12).unwrap(); // the mode word IS the value word
        // A value word of 1 reads as mode 1 (and as a denormal float, which is the point: this
        // object is being used two ways at once).
        g.set_u32(BLOCK + 12, 1).unwrap();
        g.set_u32(BLOCK + 20, 260.0f32.to_bits()).unwrap(); // bits 0x43820000, low byte 0x00
        g.set_u32(BLOCK + 24, 4.0f32.to_bits()).unwrap();

        op_envelope(&mut g, BLOCK).unwrap();
        assert_eq!(
            g.f32(BLOCK + 12).unwrap(),
            260.0,
            "the start stored the initial value"
        );
        assert_eq!(
            g.u8(BLOCK + 2).unwrap(),
            0x00,
            "the latched byte came from the reloaded word, not from the mode 1 read at entry"
        );
    }

    /// A cursor at `BLOCK` pointing at a curve at `BLOCK + 0x80`.
    fn curve(g: &mut Guest, kind: u8, scale: f32, start: i32, end: i32, samples: &[i32]) -> u32 {
        put_rodata(g);
        g.put(ONE_SINGLE, 1.0f32.to_bits().to_be_bytes().to_vec());
        let curve = BLOCK + 0x80;
        g.set_u32(BLOCK, curve).unwrap();
        g.set_u32(BLOCK + 4, i32::MIN as u32).unwrap(); // a last_pos no test uses
        g.set_u32(BLOCK + 8, 0).unwrap();
        g.set_u8(curve, kind).unwrap();
        g.set_u16(curve + 2, samples.len() as u16).unwrap();
        g.set_u32(curve + 4, start as u32).unwrap();
        g.set_u32(curve + 8, end as u32).unwrap();
        g.set_u32(curve + 12, scale.to_bits()).unwrap();
        for (i, s) in samples.iter().enumerate() {
            match kind {
                2 => g.set_u16(curve + 16 + 2 * i as u32, *s as u16).unwrap(),
                1 => g.set_u8(curve + 16 + i as u32, *s as u8).unwrap(),
                _ => g.set_u32(curve + 16 + 4 * i as u32, *s as u32).unwrap(),
            }
        }
        curve
    }

    #[test]
    fn nearest_mode_indexes_each_sample_width_at_the_right_stride() {
        // Scale exactly 1.0 selects nearest mode; start 10 so the index is pos - 10.
        for kind in [0u8, 1, 2] {
            let mut g = block_guest();
            curve(&mut g, kind, 1.0, 10, 13, &[-2, 7, -9, 40]);
            for (pos, expect) in [(10i32, -2i32), (11, 7), (12, -9), (13, 40)] {
                g.set_u32(BLOCK + 12, pos as u32).unwrap();
                assert_eq!(
                    op_curve(&mut g, BLOCK).unwrap(),
                    expect as u32 as u64,
                    "kind {kind} at {pos}"
                );
                assert_eq!(
                    g.u32(BLOCK + 4).unwrap(),
                    pos as u32,
                    "the position was cached"
                );
            }
            // Out of range clamps to the ends rather than reading past the array.
            g.set_u32(BLOCK + 12, 5).unwrap();
            assert_eq!(op_curve(&mut g, BLOCK).unwrap(), (-2i32) as u32 as u64);
            g.set_u32(BLOCK + 12, 99).unwrap();
            assert_eq!(op_curve(&mut g, BLOCK).unwrap(), 40);
        }
    }

    #[test]
    fn a_repeated_position_short_circuits_without_touching_the_curve() {
        let mut g = block_guest();
        let c = curve(&mut g, 0, 1.0, 0, 3, &[5, 6, 7, 8]);
        g.set_u32(BLOCK + 12, 2).unwrap();
        assert_eq!(op_curve(&mut g, BLOCK).unwrap(), 7);
        // Change the sample under it: a second call at the same position must not notice.
        g.set_u32(c + 16 + 8, 999).unwrap();
        assert_eq!(op_curve(&mut g, BLOCK).unwrap(), 7, "served from the cache");
        // A different position does notice.
        g.set_u32(BLOCK + 12, 1).unwrap();
        assert_eq!(op_curve(&mut g, BLOCK).unwrap(), 6);
        g.set_u32(BLOCK + 12, 2).unwrap();
        assert_eq!(op_curve(&mut g, BLOCK).unwrap(), 999);
    }

    #[test]
    fn linear_mode_interpolates_between_the_neighbours_and_clamps_the_upper_one() {
        // Scale 0.5: two position units per sample, so odd positions land halfway.
        let mut g = block_guest();
        curve(&mut g, 0, 0.5, 0, 8, &[0, 100, 200, 300, 400]);
        let expect = [
            (0i32, 0u32),
            (1, 50),
            (2, 100),
            (3, 150),
            (4, 200),
            (5, 250),
            (6, 300),
            (7, 350),
            (8, 400),
        ];
        for (pos, want) in expect {
            g.set_u32(BLOCK + 12, pos as u32).unwrap();
            assert_eq!(op_curve(&mut g, BLOCK).unwrap(), want as u64, "at {pos}");
        }

        // Past the last sample only the UPPER neighbour is clamped, to count - 1. The lower one is
        // not bounds-checked at all, so the interpolation reads past the array — which is what the
        // original does and is worth pinning, because the obvious reading (that the curve flattens
        // at its last sample) is wrong and this test was written expecting it.
        let mut g = block_guest();
        let c = curve(&mut g, 0, 0.5, 0, 20, &[0, 100, 200]);
        // Position 10 gives index 10, scaled 5.0, so i0 is 5: two entries past the three samples.
        g.set_u32(c + 16 + 4 * 5, 4242).unwrap();
        g.set_u32(BLOCK + 12, 10).unwrap();
        assert_eq!(
            op_curve(&mut g, BLOCK).unwrap(),
            4242,
            "the lower neighbour was read out of bounds, as the original reads it"
        );
    }

    #[test]
    fn the_upper_neighbour_clamp_is_observable_when_the_fraction_is_not_zero() {
        // The clamp only shows up where the two neighbours differ AND the fraction is non-zero.
        // Position 5 at scale 0.5 gives i0 = 2 and frac = 0.5, so i1 would be 3 — one past the
        // three samples. Clamped to 2, both neighbours are the same sample and the answer is 200;
        // unclamped it would read the poison word and return 600.
        let mut g = block_guest();
        let c = curve(&mut g, 0, 0.5, 0, 20, &[0, 100, 200]);
        g.set_u32(c + 16 + 4 * 3, 1000).unwrap();
        g.set_u32(BLOCK + 12, 5).unwrap();
        assert_eq!(
            op_curve(&mut g, BLOCK).unwrap(),
            200,
            "the upper neighbour clamped to 2"
        );
    }

    #[test]
    fn only_a_scale_of_exactly_one_selects_nearest_mode() {
        // A scale above 1.0 interpolates like any other: position 1 at scale 2.0 lands on sample
        // index 2, not on sample index 1 the way nearest mode would.
        let mut g = block_guest();
        curve(&mut g, 0, 2.0, 0, 4, &[0, 100, 200, 300, 400]);
        g.set_u32(BLOCK + 12, 1).unwrap();
        assert_eq!(
            op_curve(&mut g, BLOCK).unwrap(),
            200,
            "interpolated, not nearest"
        );

        // And a scale below 1.0 does too, which the interpolation tests already cover; asserted
        // here as the pair, so a test that tightened `==` into `>=` or `<=` fails either way.
        let mut g = block_guest();
        curve(&mut g, 0, 0.5, 0, 4, &[0, 100, 200, 300, 400]);
        g.set_u32(BLOCK + 12, 1).unwrap();
        assert_eq!(op_curve(&mut g, BLOCK).unwrap(), 50);
    }

    #[test]
    fn linear_mode_sign_extends_narrow_samples() {
        // A -128 byte sample must interpolate as -128, not as 128.
        let mut g = block_guest();
        curve(&mut g, 1, 0.5, 0, 4, &[-128, 0, 127]);
        for (pos, want) in [(0i32, -128i32), (1, -64), (2, 0), (3, 64), (4, 127)] {
            g.set_u32(BLOCK + 12, pos as u32).unwrap();
            assert_eq!(
                op_curve(&mut g, BLOCK).unwrap(),
                want as u32 as u64,
                "byte curve at {pos}"
            );
        }

        // And a -32768 halfword the same.
        let mut g = block_guest();
        curve(&mut g, 2, 0.5, 0, 2, &[-32768, 32767]);
        g.set_u32(BLOCK + 12, 1).unwrap();
        // (-32768 + 32767) / 2 = -0.5, which rounds away from zero to -1.
        assert_eq!(op_curve(&mut g, BLOCK).unwrap(), (-1i32) as u32 as u64);
    }
}
