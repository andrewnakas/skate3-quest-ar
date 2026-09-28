//! Finish one Dac update (`sub_82B219E8`).
//!
//! A worker runs the budgeted pre-pass, then snapshots the live voice array, submits the completed
//! mixer buffers, mirrors the submitted count, and advances the mixer clock.

use crate::{Error, Guest, Result, fp, voices};

pub const SNAP_CLOCK: u32 = 0;
pub const SNAP_VOICES: u32 = 8;
pub const SNAP_RATE: u32 = 12;
pub const SNAP_COUNT: u32 = 20;
pub const SNAP_FLAG: u32 = 22;
pub const CONTEXT_DAC: u32 = 8;
pub const CONTEXT_SINK: u32 = 44;
pub const DAC_CLOCK: u32 = 8;
pub const DAC_VOICES: u32 = 108;
pub const DAC_SUBMITTED: u32 = 228;
pub const DAC_INCREMENT: u32 = 212;
pub const DAC_VOICE_COUNT: u32 = 280;
pub const DAC_LOAD: u32 = 224;
pub const DAC_ELAPSED: u32 = 240;
pub const SINK_SUBMITTED: u32 = 44;

/// `sub_82B3C2D0`'s profile fields.
pub const PROFILE_DAC: u32 = 0;
pub const PROFILE_ACCUMULATED: u32 = 4;
pub const PROFILE_STAMP: u32 = 8;
pub const PROFILE_SMOOTHED: u32 = 16;
pub const PROFILE_HISTORY: u32 = 20;
pub const PROFILE_HISTORY_SLOT: u32 = 28;

/// The image cells used by the Dac pre-pass. Keeping them explicit lets a captured image retain
/// its retail values while focused tests use a bounded guest window.
#[derive(Clone, Copy, Debug)]
pub struct DacBudgetConstants {
    pub profile: u32,
    pub frame_scale: u32,
    pub budget_scale: u32,
    pub budget_limit: u32,
    pub cost_scale: u32,
    pub zero: u32,
}

/// The retail title-update constants for [`budgeted_prepass`].
pub const RETAIL_BUDGET_CONSTANTS: DacBudgetConstants = DacBudgetConstants {
    profile: 0x830B_DEBC,
    frame_scale: 0x822F_87B8,
    budget_scale: 0x822F_8E64,
    budget_limit: 0x820E_D57C,
    cost_scale: 0x820E_D958,
    zero: 0x8216_5A10,
};

/// Dynamic work in `sub_82B3C2D0`.
///
/// The next-voice selector is `sub_82B3C440`, which has no arguments and can leave a different
/// value in the volatile floating-point register holding `cost_scale`. The mutable argument keeps
/// that otherwise invisible ABI effect explicit; a concrete guest dispatcher can replace it with
/// the value left by the original selector.
pub trait DacBudgetHost {
    fn ticks(&mut self) -> u64;
    fn next_retirable_voice(
        &mut self,
        g: &mut Guest,
        dac: u32,
        cost_scale: &mut f64,
    ) -> Result<u32>;
}

/// Run the Dac timing, smoothing, and budgeted-retirement pre-pass (`sub_82B3C2D0`).
///
/// The first two timebase reads, all single-rounded budget arithmetic, and the profile accumulator
/// stores retain their original order. A selected voice is retired through `sub_82B49100` with
/// argument two; selection remains a host boundary because the title's selector is dynamic.
pub fn budgeted_prepass<H: DacBudgetHost + ?Sized>(
    g: &mut Guest,
    host: &mut H,
    constants: DacBudgetConstants,
) -> Result<()> {
    let entry_ticks = host.ticks();
    let work_start = host.ticks();
    let profile = constants.profile;
    let accumulated = g.u32(profile + PROFILE_ACCUMULATED)?;
    let history = fp::add_single(
        fp::load_single(g, profile + PROFILE_HISTORY)?,
        fp::load_single(g, profile + PROFILE_HISTORY + 4)?,
    );
    let last_stamp = g.u32(profile + PROFILE_STAMP)?;
    let slot = g.u32(profile + PROFILE_HISTORY_SLOT)?;
    let carried = u64::from(accumulated).wrapping_sub(u64::from(last_stamp));
    let spent = work_start.wrapping_add(carried) as u32;
    let this_frame = fp::frsp(f64::from(spent));
    let smoothed = fp::mul_single(
        fp::add_single(history, this_frame),
        fp::load_single(g, constants.frame_scale)?,
    );
    fp::store_single(g, profile + PROFILE_SMOOTHED, smoothed)?;
    fp::store_single(
        g,
        profile + PROFILE_HISTORY + ((slot << 2) & !3),
        this_frame,
    )?;
    let slot_after = g.u32(profile + PROFILE_HISTORY_SLOT)?;
    g.set_u32(profile + PROFILE_ACCUMULATED, 0)?;
    // `cntlzw(slot_after) >> 5 & 1`: only zero has 32 leading zeroes, so the two history
    // slots alternate 0 → 1 → 0. Do not replace this with a boolean cast in the other direction.
    g.set_u32(profile + PROFILE_HISTORY_SLOT, u32::from(slot_after == 0))?;

    let work_end = host.ticks();
    let dac = g.u32(profile + PROFILE_DAC)?;
    g.set_u32(profile + PROFILE_STAMP, work_end as u32)?;
    let load = fp::load_single(g, dac + DAC_LOAD)?;
    if load < fp::load_single(g, constants.budget_limit)? {
        let count = u32::from(g.u16(dac + DAC_VOICE_COUNT)?);
        let mut cost_scale = fp::load_single(g, constants.cost_scale)?;
        let mut total = fp::mul_single(smoothed, cost_scale);
        let mut cursor = g.u32(dac + DAC_VOICES)?.wrapping_sub(8);
        for _ in 0..count {
            cursor = cursor.wrapping_add(8);
            total = fp::fmadd_single(fp::load_single(g, g.u32(cursor)?)?, cost_scale, total);
        }
        let target = fp::mul_single(load, fp::load_single(g, constants.budget_scale)?);
        let mut budget = fp::sub_single(total, target);
        let zero = fp::load_single(g, constants.zero)?;
        if budget > zero {
            loop {
                let voice = host.next_retirable_voice(g, dac, &mut cost_scale)?;
                if voice == 0 {
                    break;
                }
                budget = fp::nmsub_single(fp::load_single(g, voice)?, cost_scale, budget);
                voices::retire_object(g, voice, 2)?;
                if !(budget > zero) {
                    break;
                }
                cost_scale = fp::load_single(g, constants.cost_scale)?;
            }
        }
    }

    let tail_start = host.ticks();
    let folded = tail_start.wrapping_add(
        u64::from(g.u32(profile + PROFILE_ACCUMULATED)?)
            .wrapping_sub(u64::from(g.u32(profile + PROFILE_STAMP)?)),
    );
    g.set_u32(profile + PROFILE_ACCUMULATED, folded as u32)?;
    let exit_ticks = host.ticks();
    let dac = g.u32(profile + PROFILE_DAC)?;
    g.set_u32(
        dac + DAC_ELAPSED,
        exit_ticks.wrapping_sub(entry_ticks) as u32,
    )
}

/// Image/global values fixed for a matching title update.
#[derive(Clone, Copy, Debug)]
pub struct DacConstants {
    pub stats: u32,
    pub stats_owner_slot: u32,
    pub profile: u32,
    pub snapshot_rate: f32,
}

/// Platform calls surrounding snapshot publication.
pub trait DacHost {
    /// `sub_82B3C2D0`, the budgeted work immediately before the snapshot.
    fn before_snapshot(&mut self, g: &mut Guest) -> Result<()>;
    /// `sub_82B44858`, submit the completed pass to the host output sink.
    fn submit(&mut self, g: &mut Guest, sink: u32, snapshot: u32) -> Result<u64>;
    /// `sub_82B1F7E8`, recorded in the profile block at `+8`.
    fn ticks(&mut self) -> u32;
}

/// Execute the close-out part of one Dac update.
pub fn finish_update<H: DacHost + ?Sized>(
    g: &mut Guest,
    host: &mut H,
    context: u32,
    constants: DacConstants,
) -> Result<u64> {
    host.before_snapshot(g)?;
    let dac_for_clock = g.u32(context + CONTEXT_DAC)?;
    if dac_for_clock == 0 {
        return Err(Error::new(
            context + CONTEXT_DAC,
            "Dac update has no Dac object",
        ));
    }
    g.set_u64(
        constants.stats + SNAP_CLOCK,
        g.u64(dac_for_clock + DAC_CLOCK)?,
    )?;

    // Reloading this pointer is deliberate: earlier stores can alias the update context.
    let dac_for_voices = g.u32(context + CONTEXT_DAC)?;
    let owner = g.u32(constants.stats_owner_slot)?;
    if owner == 0 {
        return Err(Error::new(
            constants.stats_owner_slot,
            "Dac stats owner is null",
        ));
    }
    g.set_u32(
        constants.stats + SNAP_RATE,
        constants.snapshot_rate.to_bits(),
    )?;
    g.set_u32(
        constants.stats + SNAP_VOICES,
        g.u32(dac_for_voices + DAC_VOICES)?,
    )?;
    let dac_for_count = g.u32(context + CONTEXT_DAC)?;
    g.set_u16(
        constants.stats + SNAP_COUNT,
        g.u16(dac_for_count + DAC_VOICE_COUNT)?,
    )?;
    g.set_u8(
        constants.stats + SNAP_FLAG,
        u8::from(g.u8(owner + 293)? != 0),
    )?;

    let result = host.submit(g, g.u32(context + CONTEXT_SINK)?, constants.stats)?;
    let sink = g.u32(context + CONTEXT_SINK)?;
    let dac_for_submit = g.u32(context + CONTEXT_DAC)?;
    g.set_u32(
        dac_for_submit + DAC_SUBMITTED,
        g.u32(sink + SINK_SUBMITTED)?,
    )?;

    let dac = g.u32(context + CONTEXT_DAC)?;
    let clock = f64::from_bits(g.u64(dac + DAC_CLOCK)?);
    let increment = f64::from(g.f32(dac + DAC_INCREMENT)?);
    g.set_u64(dac + DAC_CLOCK, (clock + increment).to_bits())?;
    g.set_u32(constants.profile + 8, host.ticks())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: u32 = 0x4000_0000;
    const CONTEXT: u32 = BASE;
    const DAC: u32 = BASE + 0x400;
    const SINK: u32 = BASE + 0x800;
    const STATS: u32 = BASE + 0xC00;
    const OWNER_SLOT: u32 = BASE + 0xD00;
    const OWNER: u32 = BASE + 0xE00;
    const PROFILE: u32 = BASE + 0xF00;
    const FRAME_SCALE: u32 = BASE + 0x1100;
    const BUDGET_SCALE: u32 = BASE + 0x1104;
    const BUDGET_LIMIT: u32 = BASE + 0x1108;
    const COST_SCALE: u32 = BASE + 0x110C;
    const ZERO: u32 = BASE + 0x1110;

    fn budget_constants() -> DacBudgetConstants {
        DacBudgetConstants {
            profile: PROFILE,
            frame_scale: FRAME_SCALE,
            budget_scale: BUDGET_SCALE,
            budget_limit: BUDGET_LIMIT,
            cost_scale: COST_SCALE,
            zero: ZERO,
        }
    }

    struct Host {
        prepasses: u32,
        seen: Option<(u32, u32)>,
    }
    impl DacHost for Host {
        fn before_snapshot(&mut self, _g: &mut Guest) -> Result<()> {
            self.prepasses += 1;
            Ok(())
        }
        fn submit(&mut self, g: &mut Guest, sink: u32, snapshot: u32) -> Result<u64> {
            self.seen = Some((sink, snapshot));
            g.set_u32(sink + SINK_SUBMITTED, 77)?;
            Ok(9)
        }
        fn ticks(&mut self) -> u32 {
            1234
        }
    }

    struct BudgetHost {
        ticks: std::collections::VecDeque<u64>,
        selected: u32,
    }

    impl DacBudgetHost for BudgetHost {
        fn ticks(&mut self) -> u64 {
            self.ticks.pop_front().expect("the pre-pass has five ticks")
        }

        fn next_retirable_voice(
            &mut self,
            _g: &mut Guest,
            _dac: u32,
            _cost_scale: &mut f64,
        ) -> Result<u32> {
            self.selected += 1;
            Ok(0)
        }
    }

    #[test]
    fn budget_prepass_smooths_time_resets_its_window_and_queries_retirement() {
        let mut g = Guest::single(BASE, 0x2000);
        g.set_u32(PROFILE + PROFILE_DAC, DAC).unwrap();
        g.set_u32(PROFILE + PROFILE_ACCUMULATED, 100).unwrap();
        g.set_u32(PROFILE + PROFILE_STAMP, 90).unwrap();
        g.set_u32(PROFILE + PROFILE_HISTORY, 2.0f32.to_bits())
            .unwrap();
        g.set_u32(PROFILE + PROFILE_HISTORY + 4, 4.0f32.to_bits())
            .unwrap();
        g.set_u32(PROFILE + PROFILE_HISTORY_SLOT, 0).unwrap();
        g.set_u32(DAC + DAC_LOAD, 0.5f32.to_bits()).unwrap();
        g.set_u16(DAC + DAC_VOICE_COUNT, 0).unwrap();
        for (at, value) in [
            (FRAME_SCALE, 0.5),
            (BUDGET_SCALE, 1.0),
            (BUDGET_LIMIT, 1.0),
            (COST_SCALE, 1.0),
            (ZERO, 0.0),
        ] {
            g.set_u32(at, (value as f32).to_bits()).unwrap();
        }
        let mut host = BudgetHost {
            ticks: [100, 110, 120, 130, 140].into(),
            selected: 0,
        };

        budgeted_prepass(&mut g, &mut host, budget_constants()).unwrap();

        assert_eq!(g.f32(PROFILE + PROFILE_SMOOTHED).unwrap(), 63.0);
        assert_eq!(g.f32(PROFILE + PROFILE_HISTORY).unwrap(), 120.0);
        assert_eq!(g.f32(PROFILE + PROFILE_HISTORY + 4).unwrap(), 4.0);
        assert_eq!(g.u32(PROFILE + PROFILE_HISTORY_SLOT).unwrap(), 1);
        assert_eq!(g.u32(PROFILE + PROFILE_STAMP).unwrap(), 120);
        assert_eq!(g.u32(PROFILE + PROFILE_ACCUMULATED).unwrap(), 10);
        assert_eq!(g.u32(DAC + DAC_ELAPSED).unwrap(), 40);
        assert_eq!(
            host.selected, 1,
            "a positive surplus asks the recovered selector"
        );
    }

    #[test]
    fn snapshots_submits_and_advances_the_clock() {
        let mut g = Guest::single(BASE, 0x2000);
        g.set_u32(CONTEXT + CONTEXT_DAC, DAC).unwrap();
        g.set_u32(CONTEXT + CONTEXT_SINK, SINK).unwrap();
        g.set_u64(DAC + DAC_CLOCK, 10.0f64.to_bits()).unwrap();
        g.set_u32(DAC + DAC_VOICES, BASE + 0x1800).unwrap();
        g.set_u16(DAC + DAC_VOICE_COUNT, 4).unwrap();
        g.set_u32(DAC + DAC_INCREMENT, 0.5f32.to_bits()).unwrap();
        g.set_u32(OWNER_SLOT, OWNER).unwrap();
        g.set_u8(OWNER + 293, 1).unwrap();
        let constants = DacConstants {
            stats: STATS,
            stats_owner_slot: OWNER_SLOT,
            profile: PROFILE,
            snapshot_rate: 48_000.0,
        };
        let mut host = Host {
            prepasses: 0,
            seen: None,
        };
        assert_eq!(
            finish_update(&mut g, &mut host, CONTEXT, constants).unwrap(),
            9
        );
        assert_eq!(host.prepasses, 1);
        assert_eq!(host.seen, Some((SINK, STATS)));
        assert_eq!(f64::from_bits(g.u64(STATS + SNAP_CLOCK).unwrap()), 10.0);
        assert_eq!(g.u32(STATS + SNAP_VOICES).unwrap(), BASE + 0x1800);
        assert_eq!(g.f32(STATS + SNAP_RATE).unwrap(), 48_000.0);
        assert_eq!(g.u16(STATS + SNAP_COUNT).unwrap(), 4);
        assert_eq!(g.u8(STATS + SNAP_FLAG).unwrap(), 1);
        assert_eq!(g.u32(DAC + DAC_SUBMITTED).unwrap(), 77);
        assert_eq!(f64::from_bits(g.u64(DAC + DAC_CLOCK).unwrap()), 10.5);
        assert_eq!(g.u32(PROFILE + 8).unwrap(), 1234);
    }
}
