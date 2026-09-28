//! The recovered per-block `rw_system` driver (`sub_82B48530`).
//!
//! This module joins the scheduler, deferred-list, and command-ring ports in the order used by
//! the original audio thread.  It deliberately owns no lock: a concrete runtime must call it only
//! from its single audio-thread owner.  The original lock brackets serialize that same ownership
//! against producers; they do not change the guest work performed inside each phase.

use crate::commands::{self, CommandHost, Drain};
use crate::modules;
use crate::patch::Heap;
use crate::scheduler::{self, DeferredReleaseHost, SchedulerHost};
use crate::voices;
use crate::{Guest, Result, fp};

/// `rw_system` fields written by `sub_82B48530`.
pub const FADE_HEAD: u32 = 20;
pub const TIME_COMMANDS: u32 = 232;
pub const TIME_SCHEDULER: u32 = 236;
pub const TIME_OTHER_PHASES: u32 = 244;

/// The fade-list node is embedded at `record + 28`.
pub const FADE_NODE: u32 = 28;
pub const FADE_FLOOR: u32 = 40;
pub const FADE_TRIGGER: u32 = 44;
pub const FADE_VALUE: u32 = 48;

/// Services the guest-only portions of a system block cannot provide themselves.
///
/// The host services required by the recovered system phases.
pub trait FrameHost: SchedulerHost + DeferredReleaseHost + CommandHost {}

impl<T: SchedulerHost + DeferredReleaseHost + CommandHost> FrameHost for T {}

/// Result of one audio-system block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The command-ring snapshot consumed during phase four.
    pub commands: Drain,
}

/// Run one recovered `rw_system` block.
///
/// The five phases and each timing-store retain the retail order.  A failed phase leaves later
/// phases untouched, which is intentional: callers get the exact incomplete boundary rather than
/// an output block assembled from partially advanced graph state.
pub fn run_frame<H: FrameHost + ?Sized, M: Heap>(
    g: &mut Guest,
    heap: &mut M,
    host: &mut H,
    system: u32,
    stack: u32,
) -> Result<Frame> {
    let scheduler = system.wrapping_add(scheduler::SYSTEM_SCHEDULER);

    // Phase 1: scheduler bucket zero.
    let phase1_start = host.timebase();
    scheduler::tick_bucket(g, host, scheduler, 0)?;
    let phase1_elapsed = host.timebase().wrapping_sub(phase1_start);

    // Phase 2: clamp and retire the fade-list records.  The successor is loaded before a
    // retirement can unlink the record.
    let phase2_start = host.timebase();
    let mut node = g.u32(system + FADE_HEAD)?;
    while node != 0 {
        let record = node.wrapping_sub(FADE_NODE);
        let next = g.u32(node)?;
        let floor = fp::load_single(g, record + FADE_FLOOR)?;
        let current = fp::load_single(g, record + FADE_VALUE)?;
        if current < floor {
            fp::store_single(g, record + FADE_VALUE, floor)?;
        }
        let trigger = fp::load_single(g, record + FADE_TRIGGER)?;
        let reloaded = fp::load_single(g, record + FADE_VALUE)?;
        if !(trigger < reloaded) {
            voices::retire_object(g, record, 3)?;
        }
        node = next;
    }
    g.set_u32(
        system + TIME_OTHER_PHASES,
        host.timebase().wrapping_sub(phase2_start),
    )?;

    // Phase 3: retire sends, then objects.  The original keeps both list walks under one lock.
    let phase3_start = host.timebase();
    scheduler::drain_deferred_sends(g, system)?;
    scheduler::drain_deferred_releases(g, host, system)?;
    let previous = g.u32(system + TIME_OTHER_PHASES)?;
    g.set_u32(
        system + TIME_OTHER_PHASES,
        host.timebase()
            .wrapping_add(previous.wrapping_sub(phase3_start)),
    )?;

    // Phase 4: a bounded command-ring snapshot.
    let phase4_start = host.timebase();
    let commands = commands::drain(g, heap, host, system, stack)?;
    g.set_u32(
        system + TIME_COMMANDS,
        host.timebase().wrapping_sub(phase4_start),
    )?;

    // Phase 5: scheduler bucket one, followed by exactly two pool-retirement passes.
    let phase5_start = host.timebase();
    scheduler::tick_bucket(g, host, scheduler, 1)?;
    modules::pool_retire_head(g, heap, scheduler)?;
    modules::pool_retire_head(g, heap, scheduler.wrapping_add(scheduler::BUCKET_STRIDE))?;
    g.set_u32(
        system + TIME_SCHEDULER,
        host.timebase()
            .wrapping_add(phase1_elapsed.wrapping_sub(phase5_start)),
    )?;

    Ok(Frame { commands })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device;
    use crate::patch::BumpHeap;

    const BASE: u32 = 0x4000_0000;
    const SYSTEM: u32 = BASE;
    const RING: u32 = BASE + 0x400;
    const TARGET: u32 = BASE + 0x800;
    const FADE: u32 = BASE + 0xa00;
    const STACK: u32 = BASE + 0x1f00;

    struct Host {
        clock: u32,
    }

    impl SchedulerHost for Host {
        fn timebase(&mut self) -> u32 {
            self.clock = self.clock.wrapping_add(1);
            self.clock
        }

        fn process(
            &mut self,
            _g: &mut Guest,
            _function: u32,
            _context: u32,
            _delta: f32,
        ) -> Result<()> {
            Ok(())
        }
    }

    impl DeferredReleaseHost for Host {
        fn requeue_id(&mut self, _g: &mut Guest, _scheduler: u32, _id: u32) -> Result<()> {
            Ok(())
        }

        fn release_deferred(&mut self, _g: &mut Guest, _object: u32) -> Result<()> {
            Ok(())
        }
    }

    impl CommandHost for Host {}

    #[test]
    fn frame_keeps_the_system_phase_order_and_publishes_its_timing() {
        let mut g = Guest::single(BASE, 0x2000);
        g.set_u32(SYSTEM + commands::COMMAND_BUFFER, RING).unwrap();
        g.set_u32(RING, device::COMMAND_PLAYER_FLOAT).unwrap();
        g.set_u32(RING + 4, TARGET).unwrap();
        g.set_u32(RING + 8, 0.75f32.to_bits()).unwrap();
        g.set_u32(SYSTEM + commands::COMMAND_WRITE_OFFSET, 12)
            .unwrap();

        // A non-retiring fade record proves phase two happens before the command phase without
        // needing a handle array for the unrelated retirement path.
        g.set_u32(SYSTEM + FADE_HEAD, FADE + FADE_NODE).unwrap();
        g.set_u32(FADE + FADE_NODE, 0).unwrap();
        g.set_u32(FADE + FADE_FLOOR, 0.25f32.to_bits()).unwrap();
        g.set_u32(FADE + FADE_TRIGGER, 0.20f32.to_bits()).unwrap();
        g.set_u32(FADE + FADE_VALUE, 0.10f32.to_bits()).unwrap();

        let mut heap = BumpHeap {
            next: BASE + 0x1800,
            end: BASE + 0x1e00,
        };
        let mut host = Host { clock: 0 };
        let frame = run_frame(&mut g, &mut heap, &mut host, SYSTEM, STACK).unwrap();

        assert_eq!(frame.commands.records, 1);
        assert_eq!(g.f32(FADE + FADE_VALUE).unwrap(), 0.25);
        assert_eq!(g.f32(TARGET + 56).unwrap(), 0.75);
        assert_eq!(g.u32(SYSTEM + TIME_OTHER_PHASES).unwrap(), 2);
        assert_eq!(g.u32(SYSTEM + TIME_COMMANDS).unwrap(), 1);
        assert_eq!(g.u32(SYSTEM + TIME_SCHEDULER).unwrap(), 2);
    }
}
