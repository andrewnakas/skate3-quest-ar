//! Original continuous exhaust synthesizer. No recorded asset is used.
use bevy::{audio::Source, prelude::*};
use std::time::Duration;

/// Which exhaust character to synthesize. `Generic` is the original four-cylinder
/// harmonic stack the kart uses; `FourStrokeSingle` is a thumper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Profile {
    #[default]
    Generic,
    FourStrokeSingle,
}
impl Profile {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "generic" => Some(Self::Generic),
            "four_stroke_single" => Some(Self::FourStrokeSingle),
            _ => None,
        }
    }
}

#[derive(Asset, TypePath)]
pub struct Engine(pub Profile);
pub struct Exhaust {
    profile: Profile,
    phase: f32,
    noise: u32,
    filtered: f32,
}
impl Iterator for Exhaust {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.phase = (self.phase + 80. / 48000.) % 1.;
        self.noise = self.noise.wrapping_mul(1664525).wrapping_add(1013904223);
        let noise = self.noise as f32 / u32::MAX as f32 * 2. - 1.;
        let p = self.phase * std::f32::consts::TAU;
        Some(match self.profile {
            Profile::Generic => {
                self.filtered += 0.12 * (noise - self.filtered);
                let pulse = p.sin() * 0.48
                    + (p * 2.).sin() * 0.24
                    + (p * 3.).sin() * 0.12
                    + (p * 5.).sin() * 0.06;
                (pulse + self.filtered * 0.18) * 0.65
            }
            // A single cylinder fires once every two crank revolutions, so the
            // waveform is one hard asymmetric event per cycle with a long gap,
            // not a harmonic stack. The sharp attack and exponential decay are
            // what make it read as a thumper rather than a drone; the wider
            // noise band is the intake and the knobbies' mechanical clatter.
            Profile::FourStrokeSingle => {
                // Brighter than the generic drone for the intake and knobby clatter,
                // but not so fast that the noise alone can step the output audibly.
                self.filtered += 0.18 * (noise - self.filtered);
                // Fast attack into a long decay. The attack has to be finite:
                // decaying straight off a vertical edge leaves a step at the
                // phase wrap, which is a click and aliases when pitched up.
                const ATTACK: f32 = 0.06;
                let env = if self.phase < ATTACK {
                    0.5 * (1. - (std::f32::consts::PI * self.phase / ATTACK).cos())
                } else {
                    (-(self.phase - ATTACK) * 9.).exp()
                };
                // A little overswing behind the pulse gives the exhaust its bark
                // instead of a bare click, and the body tone fills the gap
                // between strokes without closing it.
                let body = p.sin() * 0.07 + (p * 2.).sin() * 0.04;
                let pulse = env * 0.85 - env * env * 0.25 + body * (1. - env * 0.6);
                // Breathing noise rides with the pulse, so it is loudest on the
                // power stroke rather than sitting under it as a constant hiss.
                (pulse + self.filtered * (0.06 + env * 0.18)) * 0.62
            }
        })
    }
}
impl Source for Exhaust {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        48000
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
impl Decodable for Engine {
    type DecoderItem = f32;
    type Decoder = Exhaust;
    fn decoder(&self) -> Exhaust {
        Exhaust {
            profile: self.0,
            phase: 0.,
            noise: 17,
            filtered: 0.,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhaust_is_bounded_continuous_and_not_silent() {
        for profile in [Profile::Generic, Profile::FourStrokeSingle] {
            let mut source = Engine(profile).decoder();
            let mut last = 0.;
            let mut energy = 0.;
            for _ in 0..48000 {
                let sample = source.next().unwrap();
                assert!(sample.is_finite() && sample.abs() < 0.8, "{profile:?}");
                assert!((sample - last).abs() < 0.1, "{profile:?} stepped by {}", sample - last);
                energy += sample * sample;
                last = sample;
            }
            assert!(energy > 100., "{profile:?} was silent");
        }
    }

    /// The thumper's identity is the gap: one loud event per cycle and a long
    /// quiet decay after it, where a harmonic stack spreads its energy evenly.
    /// Crest factor (peak over RMS) is the standard way to say that, and it does
    /// not care where in the cycle the measurement window happens to start.
    #[test]
    fn the_single_fires_once_per_cycle_and_the_generic_does_not() {
        let crest = |profile| {
            let mut source = Engine(profile).decoder();
            let (mut peak, mut energy) = (0f32, 0f32);
            let n = (48000. / 80.) as usize * 4;
            for _ in 0..n {
                let s = source.next().unwrap();
                peak = peak.max(s.abs());
                energy += s * s;
            }
            peak / (energy / n as f32).sqrt()
        };
        let single = crest(Profile::FourStrokeSingle);
        let generic = crest(Profile::Generic);
        // A sine sits at 1.41; the generic stack is near that, the single well above.
        assert!(single > 3., "single crest factor {single} is not peaky enough");
        assert!(
            single > generic * 1.8,
            "single {single} should be far peakier than generic {generic}"
        );
    }

    #[test]
    fn profile_names_round_trip_and_reject_junk() {
        assert_eq!(Profile::parse("generic"), Some(Profile::Generic));
        assert_eq!(
            Profile::parse("four_stroke_single"),
            Some(Profile::FourStrokeSingle)
        );
        assert_eq!(Profile::parse("two_stroke"), None);
    }
}
