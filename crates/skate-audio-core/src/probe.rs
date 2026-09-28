//! A real-worker probe for the recovered final output stages.
//!
//! This owns a small guest image containing the retail-shaped six-channel `Del0` history stage
//! and the recovered output pass. Its renderer feeds a bounded 440 Hz probe through that graph,
//! then hands the resulting interleaved PCM to [`worker::pump_once`]. The probe exists to verify
//! the worker/device bridge; authored rider and board streams enter the same output path later.

use std::f32::consts::TAU;

use crate::{Guest, Result, delay, interleave, output, routing, worker};

const BASE: u32 = 0x4000_0000;
const WORKER: u32 = BASE + 0x0100;
const OUTPUT: u32 = BASE + 0x0400;
const HOLDER: u32 = BASE + 0x0500;
const DESC_A: u32 = BASE + 0x0600;
const DESC_B: u32 = BASE + 0x0640;
const DESC_MIX: u32 = BASE + 0x0680;
const DELAY_STATE: u32 = BASE + 0x0800;
const SOURCE_A: u32 = BASE + 0x2000;
const SOURCE_B: u32 = BASE + 0x4000;
const MIX_PLANES: u32 = BASE + 0x6000;
const HISTORY: u32 = BASE + 0x9000;
const PCM: u32 = BASE + 0x18000;
const STACK: u32 = BASE + 0x2F000;
const CHANNELS: u32 = 6;
const FRAMES: u32 = 256;
const DELAY_SAMPLES: u32 = 720; // final retail `Del0`: 15 ms at 48 kHz
const HISTORY_STRIDE: u32 = 1024;
const SAMPLE_RATE: u32 = 48_000;
const SAMPLES_PER_BLOCK: usize = (worker::BUFFER_BYTES / 4) as usize;

/// Native-rate, six-channel PCM from one submitted Dac buffer.
pub const PCM_CHANNELS: u16 = CHANNELS as u16;
pub const PCM_SAMPLE_RATE: u32 = SAMPLE_RATE;
pub const PCM_FRAMES_PER_BLOCK: u32 = FRAMES;

/// A guest graph and Dac worker that can be pumped by a host audio thread.
pub struct GraphPcmProbe {
    guest: Guest,
    host: ProbeHost,
}

struct ProbeHost {
    rendered_frames: u64,
    submitted: Vec<f32>,
}

impl GraphPcmProbe {
    /// Build the recovered stable final-output path with its measured 15 ms `Del0` delay.
    pub fn new() -> Result<Self> {
        let mut guest = Guest::single(BASE, 0x30000);
        guest.put(0x8306_7000, vec![0; 0x100]);
        guest.put(routing::GAIN_TABLE, vec![0; 0x80]);
        guest.put(output::CLAMP_HIGH, 1.0f32.to_bits().to_be_bytes().to_vec());
        guest.put(
            output::CLAMP_LOW,
            (-1.0f32).to_bits().to_be_bytes().to_vec(),
        );
        guest.put(output::RAMP_ZERO, 0.0f32.to_bits().to_be_bytes().to_vec());
        guest.put(
            output::RAMP_SCALE,
            (1.0f32 / 128.0).to_bits().to_be_bytes().to_vec(),
        );

        guest.set_u8(output::CHANNEL_COUNT_BYTE, CHANNELS as u8)?;
        guest.set_u32(routing::GAIN_TABLE, 1.0f32.to_bits())?;
        guest.set_u8(output::PAIR_TABLE + 10, 0)?;
        guest.set_u8(output::PAIR_TABLE + 11, 5)?;
        for lane in 0..CHANNELS {
            // gain zero (unity), same source and destination lane.
            guest.set_u8(output::OP_TABLE + lane, ((lane as u8) << 3) | lane as u8)?;
        }

        for (descriptor, data) in [
            (DESC_A, SOURCE_A),
            (DESC_B, SOURCE_B),
            (DESC_MIX, MIX_PLANES),
        ] {
            guest.set_u32(descriptor + interleave::PLANE_BASE, data)?;
            guest.set_u16(descriptor + interleave::PLANE_FRAMES, FRAMES as u16)?;
        }
        guest.set_u32(HOLDER + output::DST_DESC, DESC_MIX)?;
        guest.set_u32(OUTPUT + output::DESC_HOLDER, HOLDER)?;
        guest.set_u32(OUTPUT + output::BLOCK_BASE, PCM)?;

        guest.set_u32(DELAY_STATE + delay::BUFFER, HISTORY)?;
        guest.set_u32(DELAY_STATE + delay::REQUEST, DELAY_SAMPLES)?;
        guest.set_u32(DELAY_STATE + delay::STRIDE, HISTORY_STRIDE)?;
        guest.set_u32(DELAY_STATE + delay::CAPACITY, HISTORY_STRIDE)?;
        guest.set_u32(DELAY_STATE + delay::CHANNELS, CHANNELS)?;
        guest.set_u32(OUTPUT + 28, DESC_A)?;
        guest.set_u32(OUTPUT + 32, DESC_B)?;

        guest.set_u32(WORKER + worker::PCM_BASE, PCM)?;
        Ok(Self {
            guest,
            host: ProbeHost {
                rendered_frames: 0,
                submitted: Vec::new(),
            },
        })
    }

    /// Render and submit one 256-frame block. The returned samples are six-channel interleaved
    /// PCM exactly as the worker submitted them.
    pub fn pump_once(&mut self) -> Result<Vec<f32>> {
        self.host.submitted.clear();
        worker::pump_once(&mut self.guest, &mut self.host, WORKER)?;
        Ok(std::mem::take(&mut self.host.submitted))
    }
}

impl worker::WorkerHost for ProbeHost {
    fn render(&mut self, g: &mut Guest, worker_at: u32) -> Result<bool> {
        let input_desc = g.u32(OUTPUT + 28)?;
        let input = g.u32(input_desc + interleave::PLANE_BASE)?;
        let stride = u32::from(g.u16(input_desc + interleave::PLANE_FRAMES)?);
        for channel in 0..CHANNELS {
            let plane = input.wrapping_add(channel.wrapping_mul(stride).wrapping_mul(4));
            for frame in 0..FRAMES {
                let phase = (self.rendered_frames + u64::from(frame)) as f32 / SAMPLE_RATE as f32;
                // Keep every native output lane active: a stereo device can then downmix this probe
                // while a six-channel device retains the final renderer's layout.
                g.set_u32(
                    plane + frame * 4,
                    (phase * TAU * 440.0).sin().mul_add(0.12, 0.0).to_bits(),
                )?;
            }
        }
        self.rendered_frames += u64::from(FRAMES);

        delay::process_stable(g, DELAY_STATE, OUTPUT, CHANNELS)?;
        g.set_u32(HOLDER + output::SRC_DESC, g.u32(OUTPUT + 28)?)?;
        g.set_u32(
            OUTPUT + output::BLOCK_INDEX,
            g.u32(worker_at + worker::FILL_INDEX)?,
        )?;
        output::output_pass(g, OUTPUT, STACK)?;
        Ok(true)
    }

    fn queued_buffers(&mut self, _g: &mut Guest, _worker: u32) -> Result<u32> {
        // `LivePcm::push` copies synchronously, so it has accepted the preceding block already.
        Ok(0)
    }

    fn submit(&mut self, g: &mut Guest, worker_at: u32, descriptor: u32) -> Result<()> {
        let index =
            descriptor.wrapping_sub(worker_at + worker::DESCRIPTORS) / worker::DESCRIPTOR_BYTES;
        let pcm = g
            .u32(worker_at + worker::PCM_BASE)?
            .wrapping_add(index.wrapping_mul(worker::BUFFER_BYTES));
        self.submitted.reserve(SAMPLES_PER_BLOCK);
        for sample in 0..SAMPLES_PER_BLOCK as u32 {
            self.submitted.push(g.f32(pcm + sample * 4)?);
        }
        // The host has copied this block, equivalent to an immediate completed-buffer callback.
        g.set_u32(worker_at + (index + worker::SLOT_BASE) * 4, 0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_submits_interleaved_graph_pcm_after_the_retail_delay() {
        let mut probe = GraphPcmProbe::new().unwrap();
        let first = probe.pump_once().unwrap();
        let second = probe.pump_once().unwrap();
        let third = probe.pump_once().unwrap();
        let fourth = probe.pump_once().unwrap();
        assert_eq!(first.len(), SAMPLES_PER_BLOCK);
        assert!(first.iter().all(|sample| *sample == 0.0));
        assert!(second.iter().all(|sample| *sample == 0.0));
        assert!(third.iter().any(|sample| *sample != 0.0));
        assert!(fourth.iter().any(|sample| *sample != 0.0));
    }
}
