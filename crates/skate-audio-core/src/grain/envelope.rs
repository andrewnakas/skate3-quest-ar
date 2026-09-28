//! The board owner's segment envelopes and the push that programs them: what `owner+1028`/`+1032`
//! (the grain speed scale) and `owner+1152`/`+1156` (the frequency-shift offset) are.
//!
//! | function | here |
//! |---|---|
//! | `sub_8248D368`, reset | [`Envelope::reset`] |
//! | `sub_8248D3C0`, append a segment | [`Envelope::add`] |
//! | `sub_8248D498`, append a segment chained to the previous end | [`Envelope::add_chained`] |
//! | `sub_8248D510`, advance by `dt` | [`Envelope::advance`] |
//! | `sub_824C6198`'s push branch (state `+335`) and per-frame advance | [`program_push`] |
//!
//! The object is 124 bytes (`owner+912`, `+1036`, `+1340`): `+0` elapsed, `+4..` durations,
//! `+28..` starts, `+52..` ends, `+76..` chained bytes, `+84..` curve words, `+108` count, `+112`
//! current, `+116` value, `+120` idle byte. So `owner+1028` is the `+912` envelope's value and
//! `+1032` its idle byte; `+1152`/`+1156` the `+1036` envelope's; `+1456`/`+1464` the `+1340` one's.
//! The grain record uses a value only while its idle byte is clear.
//!
//! Every push (`[state+335]`, the push trigger) restarts `+912` from its current value (or 1.0)
//! to a speed-dependent peak, holds, and returns to 1.0; and `+1036` from 0 to a peak shift in Hz,
//! holds, and returns to 0. Each frame `sub_824C6198` advances each envelope that is not idle by
//! the frame's `dt`.

use crate::fp::{
    add_single, div_single, fcfid, fmadd_single, frsp, fsel, mul_single, neg_double, nmsub_single,
    sub_single,
};

use super::board::KMH_PER_MS;

/// `lis -32250 ; lfs 14920`: 0.001, milliseconds to seconds in `sub_8248D3C0`.
pub const MILLISECONDS: f32 = f32::from_bits(0x3A83_126F);
/// `lis -32243 ; lfs 29160`: 0.01, the shortest segment.
pub const SHORTEST_SEGMENT: f32 = f32::from_bits(0x3C23_D70A);

/// The vault values `sub_824C6198` reads (from the `[owner+1500]` truck's grain collection, class
/// `0x7AB23C11B6ADA2DE`, all in `default`; metal overrides `shift_low` to −50).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushTuning {
    /// `0x2D751DEB89BB5E33` (45 km/h): `t = clamp((v − 1) × 3.6 / this, 0, 1)`.
    pub ramp_kmh: f32,
    /// `0xE239B03F0E890686` (1.4) and `0xB87ECDDAAB0F8404` (1.1): the speed-scale peak at t = 0, 1.
    pub scale_low: f32,
    pub scale_high: f32,
    /// `0xC658A7923FC7B99E` (−52) and `0xA15AD56E225ADBA6` (−20): the shift peak in Hz at t = 0, 1.
    pub shift_low: f32,
    pub shift_high: f32,
    /// Milliseconds: scale attack `0xB3D7468820AFC661` (35), hold `0xDAC9DA910EF0316C` (200),
    /// return `0x0C3D5DBC262ED276` (600); shift attack `0x09A5CC79BA2178E7` (30), hold
    /// `0x3206FD96427EA4D2` (200), return `0xDB597F672CA47138` (600).
    pub scale_ms: [i32; 3],
    pub shift_ms: [i32; 3],
}

/// The segment envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub elapsed: f32,
    pub durations: [f32; 6],
    pub starts: [f32; 6],
    pub ends: [f32; 6],
    pub chained: [bool; 6],
    pub curves: [u32; 6],
    pub count: i32,
    pub current: i32,
    pub value: f32,
    pub idle: bool,
}

impl Default for Envelope {
    /// The state `sub_8248D368` leaves.
    fn default() -> Self {
        Envelope {
            elapsed: 0.0,
            durations: [0.0; 6],
            starts: [0.0; 6],
            ends: [0.0; 6],
            chained: [false; 6],
            curves: [0; 6],
            count: 0,
            current: 0,
            value: 0.0,
            idle: true,
        }
    }
}

impl Envelope {
    /// `sub_8248D368`.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `sub_8248D3C0`: append a linear segment `start → end` over `ms`. `false` when five exist
    /// (the original's −1).
    pub fn add(&mut self, start: f32, end: f32, ms: i32) -> bool {
        let n = self.count;
        if n == 5 {
            return false;
        }
        let i = n as usize;
        let mut duration = mul_single(frsp(fcfid(i64::from(ms))), f64::from(MILLISECONDS));
        if !(duration > 0.0) {
            duration = f64::from(SHORTEST_SEGMENT);
        }
        self.durations[i] = duration as f32;
        self.ends[i] = end;
        self.starts[i] = start;
        self.curves[i] = 0;
        self.chained[i] = false;
        self.idle = false;
        if n == 0 {
            self.value = self.starts[0];
        }
        self.count = n + 1;
        true
    }

    /// `sub_8248D498`: append a segment to `value` over `ms` that starts wherever the previous one
    /// ends. `false` with no previous segment.
    pub fn add_chained(&mut self, value: f32, ms: i32) -> bool {
        if self.count == 0 {
            return false;
        }
        self.add(0.0, value, ms);
        self.chained[(self.count - 1) as usize] = true;
        true
    }

    /// `sub_8248D510`: advance by `dt` seconds. Curve 5 (`sub_8294B668`, the MixMap shaper) is
    /// never set by the board's programming; meeting it is an error.
    pub fn advance(&mut self, dt: f32) -> Result<(), &'static str> {
        if self.idle || self.count == 0 {
            return Ok(());
        }
        let elapsed = add_single(f64::from(self.elapsed), f64::from(dt));
        self.elapsed = elapsed as f32;
        let i = self.current as usize;
        let duration = f64::from(self.durations[i]);
        if elapsed > duration {
            self.value = self.ends[i];
            if self.current < self.count - 1 {
                self.elapsed = sub_single(elapsed, duration) as f32;
                let next = i + 1;
                self.current = next as i32;
                if self.chained[next] {
                    self.starts[next] = self.ends[next - 1];
                }
            } else {
                self.idle = true;
            }
            return Ok(());
        }
        let frac = div_single(elapsed, duration);
        let start = f64::from(self.starts[i]);
        let span = sub_single(f64::from(self.ends[i]), start);
        let value = match self.curves[i] {
            1 => fmadd_single(mul_single(span, frac), frac, start),
            2 => {
                let from_end = sub_single(frac, 1.0);
                fmadd_single(nmsub_single(from_end, from_end, 1.0), span, start)
            }
            5 => return Err("envelope curve 5 (sub_8294B668) is not ported"),
            _ => fmadd_single(span, frac, start),
        };
        self.value = value as f32;
        Ok(())
    }
}

/// `sub_824C6198`'s push branch: `speed` is `[state+208]`, `push` the `[owner+1500]` truck's
/// tuning, `scale` the `+912` envelope and `shift` the `+1036` one.
pub fn program_push(push: &PushTuning, speed: f32, scale: &mut Envelope, shift: &mut Envelope) {
    let one = 1.0f64;
    let start = if !scale.idle { scale.value } else { 1.0 }; // f26
    let over = sub_single(f64::from(speed), one);
    let kmh = mul_single(over, f64::from(KMH_PER_MS));
    let ratio = div_single(kmh, f64::from(push.ramp_kmh));
    let clamped = fsel(neg_double(ratio), 0.0, ratio);
    let t = fsel(sub_single(one, clamped), clamped, one);
    let peak = |low: f32, high: f32| {
        fmadd_single(
            sub_single(f64::from(high), f64::from(low)),
            t,
            f64::from(low),
        ) as f32
    };
    let scale_peak = peak(push.scale_low, push.scale_high);
    scale.reset();
    scale.add(start, scale_peak, push.scale_ms[0]);
    scale.add_chained(scale_peak, push.scale_ms[1]);
    scale.add_chained(1.0, push.scale_ms[2]);
    let shift_peak = peak(push.shift_low, push.shift_high);
    shift.reset();
    shift.add(0.0, shift_peak, push.shift_ms[0]);
    shift.add_chained(shift_peak, push.shift_ms[1]);
    shift.add_chained(0.0, push.shift_ms[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> PushTuning {
        PushTuning {
            ramp_kmh: 45.0,
            scale_low: f32::from_bits(0x3FB3_3333),
            scale_high: f32::from_bits(0x3F8C_CCCD),
            shift_low: -52.0,
            shift_high: -20.0,
            scale_ms: [35, 200, 600],
            shift_ms: [30, 200, 600],
        }
    }

    /// A push at rest: the speed scale rises from 1.0 to 1.4 over 35 ms, holds 200 ms, returns to
    /// 1.0 over 600 ms and goes idle; the shift does the same between 0 and −52 Hz.
    #[test]
    fn a_push_rises_holds_and_returns() {
        let (mut scale, mut shift) = (Envelope::default(), Envelope::default());
        program_push(&tuning(), 0.0, &mut scale, &mut shift);
        assert!(!scale.idle);
        assert_eq!(scale.value, 1.0);
        let dt = 1.0 / 60.0;
        let (mut peak, mut low) = (0f32, 0f32);
        let mut frames = 0;
        while !scale.idle && frames < 200 {
            scale.advance(dt).unwrap();
            shift.advance(dt).unwrap();
            peak = peak.max(scale.value);
            low = low.min(shift.value);
            frames += 1;
        }
        assert_eq!(peak, f32::from_bits(0x3FB3_3333));
        assert_eq!(low, -52.0);
        assert_eq!(scale.value, 1.0);
        assert!((48..=52).contains(&frames), "{frames} frames");
    }

    /// Above 46 km/h + 1 m/s the peaks are the high ends.
    #[test]
    fn fast_pushes_use_the_high_peaks() {
        let (mut scale, mut shift) = (Envelope::default(), Envelope::default());
        program_push(&tuning(), 20.0, &mut scale, &mut shift);
        assert_eq!(scale.ends[0], f32::from_bits(0x3F8C_CCCD));
        assert_eq!(shift.ends[0], -20.0);
        assert!(scale.chained[1] && scale.chained[2]);
    }
}
