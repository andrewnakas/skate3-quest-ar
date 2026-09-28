//! The voice classes' kernels, reached by address: a [`GraphHost`] for a voice graph.
//!
//! A module class's function table is `{0, prepare, process}` (`docs/audio-banks.md`, the class
//! table), and [`crate::graph`] calls those through `GraphHost` as `prepare(object, owner, flag,
//! request)` and `process(object, owner, flag)`, the guest's `r3`..`r6`. This routes the ten addresses
//! the voice classes name to their Rust ports, with the registers each port actually reads:
//!
//! | class | prepare | process |
//! |---|---|---|
//! | `Sub0` | — | `sub_82B34E08` → [`mix::flush_accumulator`]`(object, owner)` |
//! | `DCl0` | — | `sub_82B22678` → [`dsp::clip::hard_clip`]`(object, owner)` |
//! | `Del0` | — | stable `sub_82B222D8` path → [`delay::process_stable`]`(object + 100, owner)` |
//! | `PI20` | — | `sub_82B2C658` → [`filters::peaking_stage`]`(object, owner)` |
//! | `Gain` | — | `sub_82B23B50` → [`gains::ramp_channels`]`(object, owner, flag)` |
//! | `HighPassIir2` | — | `sub_82B26568` → [`filters::highpass_stage`]`(object, owner)` |
//! | `LowPassIir2` | — | `sub_82B27E20` → [`filters::lowpass_stage`]`(object, owner)` |
//! | `Pan2D1` | — | `sub_82B29BE0` → [`gains::republish_mix`]`(object, owner, flag, sp)` |
//! | `Rechannel` | `sub_82B2C8E8` → [`leaves::fourth_argument`]`(request)` | `sub_82B2C8F0` → [`mix::refold_rows`]`(object, owner, sp)` |
//! | `Resample` | `sub_82B2DAC8` → [`pitch::advance_pitch`]`(object, owner, request)` | `sub_82B2DBA8` → [`pitch::resample_block`]`(object, owner, sp)` |
//! | `SndPlayer1` | `sub_82B34268` → [`leaves::set_field_460`]`(object, request)` | `sub_82B34278` → [`sndplayer::render_block`]`(object, owner)` |
//! | `Send` | — | `sub_82B31838` → [`bus::mix_source`]`(object, owner, flag, sp)` |
//! | `GainFader` | — | `sub_82B238A8` → [`gains::advance_gain_ramp`]`(object, owner)` |
//!
//! The three voice graph shapes the `msgs1` trace logged use exactly these nine classes
//! (`docs/audio-banks.md`).
//!
//! All but `SndPlayer1`'s process function are verified ports. The routing itself is unverified: which
//! register each port takes is read from its `.inc` body, not observed. An address outside the table
//! is an error naming it.

use crate::graph::GraphHost;
use crate::mathlib::Trig;
use crate::stream::StreamFill;
use crate::{Error, Guest, Result, bus, delay, dsp, filters, gains, leaves, mix, pitch, sndplayer};

/// The voice kernels, with what they need from the host: the trigonometry the filters and the panner
/// call, the stream fill `SndPlayer1` pulls PCM through, and a stack pointer for the ports that build
/// a frame.
pub struct VoiceKernels<'a, T: Trig> {
    pub trig: &'a mut T,
    pub fill: &'a mut dyn StreamFill,
    pub sp: u32,
}

impl<T: Trig> GraphHost for VoiceKernels<'_, T> {
    fn prepare(
        &mut self,
        g: &mut Guest,
        function: u32,
        object: u32,
        owner: u32,
        _flag: u32,
        request: u64,
    ) -> Result<u64> {
        match function {
            0x82B2_C8E8 => Ok(leaves::fourth_argument(request)),
            0x82B2_DAC8 => pitch::advance_pitch(g, object, owner, request as u32),
            0x82B3_4268 => leaves::set_field_460(g, object, request as u16),
            other => Err(Error::new(
                other,
                format!("prepare function {other:#010x} is not a voice kernel"),
            )),
        }
    }

    fn process(
        &mut self,
        g: &mut Guest,
        function: u32,
        object: u32,
        owner: u32,
        flag: u32,
    ) -> Result<u64> {
        match function {
            0x82B3_4E08 => mix::flush_accumulator(g, object, owner),
            0x82B2_22D8 => delay::process_stable(
                g,
                object.wrapping_add(100),
                owner,
                u32::from(g.u8(object.wrapping_add(42))?),
            ),
            0x82B2_2678 => dsp::clip::hard_clip(g, object, owner),
            0x82B2_3B50 => Ok(gains::ramp_channels(g, object, owner, flag as u64)?.r3),
            0x82B2_6568 => filters::highpass_stage(g, self.trig, object as u64, owner as u64),
            0x82B2_7E20 => filters::lowpass_stage(g, self.trig, object as u64, owner as u64),
            0x82B2_C658 => filters::peaking_stage(g, self.trig, object as u64, owner),
            0x82B2_9BE0 => gains::republish_mix(g, self.trig, object, owner, flag, self.sp),
            0x82B2_C8F0 => mix::refold_rows(g, object, owner, self.sp),
            0x82B2_DBA8 => pitch::resample_block(g, object, owner, self.sp),
            0x82B3_4278 => sndplayer::render_block(g, self.fill, object, owner),
            0x82B3_1838 => bus::mix_source(g, object, owner, flag, self.sp),
            0x82B2_38A8 => gains::advance_gain_ramp(g, self.trig, object, owner),
            // The grain chain's FrequencyShiftSsb and HighShelfIir2 (`crate::grain::fss`).
            crate::grain::fss::FSS_PROCESS => {
                crate::grain::fss::process_fss(g, object, owner, self.sp)
            }
            crate::grain::fss::HS_PROCESS => filters::shelf_stage(g, self.trig, object, owner),
            other => Err(Error::new(
                other,
                format!("process function {other:#010x} is not a voice kernel"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathlib::Unported;

    const MEM: u32 = 0x5000_0000;

    struct NoFill;
    impl StreamFill for NoFill {
        fn fill(&mut self, _g: &mut Guest, _s: u32, _d: u32, _f: u64) -> Result<u64> {
            panic!("no stream on this path")
        }
    }

    fn host<'a>(trig: &'a mut Unported, fill: &'a mut NoFill) -> VoiceKernels<'a, Unported> {
        VoiceKernels {
            trig,
            fill,
            sp: MEM + 0x1000,
        }
    }

    #[test]
    fn prepare_routes_to_the_ports_with_the_fourth_argument() {
        let mut g = Guest::single(MEM, 0x2000);
        let (mut trig, mut fill) = (Unported, NoFill);
        let mut h = host(&mut trig, &mut fill);
        assert_eq!(
            h.prepare(&mut g, 0x82B2_C8E8, MEM, MEM, 0, 0x1_0000_0100)
                .unwrap(),
            0x1_0000_0100,
            "64-bit passthrough"
        );
        assert_eq!(
            h.prepare(&mut g, 0x82B3_4268, MEM, MEM + 4, 1, 256)
                .unwrap(),
            0
        );
        assert_eq!(g.u16(MEM + 460).unwrap(), 256, "the request, not r4 or r5");
    }

    #[test]
    fn a_stopped_player_process_reaches_the_render() {
        let mut g = Guest::single(MEM, 0x2000);
        g.set_u16(MEM + 460, 4).unwrap(); // block frames, so an empty render returns 0
        g.set_u16(MEM + 464, 0x200).unwrap(); // record ring; its record is in state 0
        let (mut trig, mut fill) = (Unported, NoFill);
        let mut h = host(&mut trig, &mut fill);
        assert_eq!(
            h.process(&mut g, 0x82B3_4278, MEM, MEM + 0x400, 0).unwrap(),
            0
        );
    }

    #[test]
    fn unknown_addresses_are_errors_that_name_them() {
        let mut g = Guest::single(MEM, 0x100);
        let (mut trig, mut fill) = (Unported, NoFill);
        let mut h = host(&mut trig, &mut fill);
        let err = h.process(&mut g, 0x8200_0000, MEM, MEM, 0).unwrap_err();
        assert!(err.message.contains("0x82000000"), "{}", err.message);
        assert!(
            h.prepare(&mut g, 0x82B2_3B50, MEM, MEM, 0, 0).is_err(),
            "Gain has no prepare"
        );
    }
}
