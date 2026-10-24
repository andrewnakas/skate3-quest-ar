//! Decoded 16-bit PCM as a Bevy audio source: the sound banks of the imported-disc tier and the
//! clean tier's own sounds both play through it.
use bevy::audio::{Decodable, Source};
use bevy::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// Decoded interleaved 16-bit PCM, ready to hand to the mixer.
#[derive(Asset, TypePath, Clone)]
pub struct StreamPcm {
    pub(crate) samples: Arc<Vec<i16>>,
    pub(crate) channels: u16,
    pub(crate) sample_rate: u32,
    /// Frame interval to repeat for an inline EAAC loop. Archive streams without an inline loop
    /// continue to use Bevy's whole-source looping setting.
    pub(crate) loop_range: Option<(usize, usize)>,
}

impl StreamPcm {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        }
    }

    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.sample_rate))
    }

    /// Interleaved 16-bit PCM; `looping` repeats the whole clip inline, forever.
    pub(crate) fn from_i16(samples: Arc<Vec<i16>>, channels: u16, sample_rate: u32, looping: bool) -> Self {
        let frames = if channels == 0 { 0 } else { samples.len() / usize::from(channels) };
        Self {
            samples,
            channels,
            sample_rate,
            loop_range: (looping && frames > 0).then_some((0, frames)),
        }
    }

    /// A short mono sine wave for verifying the host mixer and output device without game data.
    ///
    /// This deliberately lives beside decoded streams so the diagnostic exercises the exact same
    /// Bevy source, mixer and Windows device backend as retail audio does.
    pub(crate) fn tone(hertz: f32, duration: Duration) -> Self {
        const RATE: u32 = 48_000;
        let frames = (duration.as_secs_f64() * f64::from(RATE)).round() as usize;
        let angular_step = std::f32::consts::TAU * hertz / RATE as f32;
        let samples = (0..frames)
            // Keep headroom so the device test is not itself a clipping test.
            .map(|frame| (angular_step * frame as f32).sin() * 0.20 * i16::MAX as f32)
            .map(|sample| sample.round() as i16)
            .collect();
        Self {
            samples: Arc::new(samples),
            channels: 1,
            sample_rate: RATE,
            loop_range: None,
        }
    }
}

/// Plays a [`StreamPcm`] once, converting to the float samples the mixer wants.
pub struct PcmPlayback {
    samples: Arc<Vec<i16>>,
    at: usize,
    channels: u16,
    sample_rate: u32,
    loop_range: Option<(usize, usize)>,
}

impl Iterator for PcmPlayback {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some((start, end)) = self.loop_range {
            let channels = usize::from(self.channels);
            if self.at == end.saturating_mul(channels) {
                self.at = start.saturating_mul(channels);
            }
        }
        let s = *self.samples.get(self.at)?;
        self.at += 1;
        // i16::MIN has no positive counterpart, so dividing by 32768 keeps the full range inside
        // [-1, 1] instead of letting the one extreme sample clip a fraction above it.
        Some(f32::from(s) / 32768.0)
    }
}

impl Source for PcmPlayback {
    fn current_frame_len(&self) -> Option<usize> {
        self.loop_range
            .is_none()
            .then(|| self.samples.len().saturating_sub(self.at))
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        if self.loop_range.is_some() {
            return None;
        }
        let frames = if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        };
        (self.sample_rate != 0)
            .then(|| Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate)))
    }
}

impl Decodable for StreamPcm {
    type DecoderItem = f32;
    type Decoder = PcmPlayback;

    fn decoder(&self) -> PcmPlayback {
        PcmPlayback {
            samples: self.samples.clone(),
            at: 0,
            channels: self.channels,
            sample_rate: self.sample_rate,
            loop_range: self.loop_range,
        }
    }
}

