//! One non-blocking iteration of the Dac worker's double-buffer handoff (`sub_82B21AC0`).
//!
//! Waiting, operating-system critical sections, and the platform device vtable belong to the
//! concrete audio thread.  This port retains the guest-visible part of one ready iteration: fill
//! one of the two 6,144-byte buffers, mark it ready, submit every ready buffer the sink accepts,
//! then advance the fill and submit indices with the original two-store wrap.

use crate::{Guest, Result};

pub const PCM_BASE: u32 = 84;
pub const SUBMIT_INDEX: u32 = 88;
pub const FILL_INDEX: u32 = 92;
pub const DESCRIPTORS: u32 = 100;
pub const DESCRIPTOR_BYTES: u32 = 36;
pub const SLOT_BASE: u32 = 43;
pub const BUFFER_BYTES: u32 = 6144;
pub const BUFFER_COUNT: u32 = 2;

/// Platform work around the worker's guest state.
pub trait WorkerHost {
    /// Run the Dac update that fills the current PCM buffer.  `false` means it produced no audio;
    /// the worker clears that buffer before handing it on, exactly as the retail worker does.
    fn render(&mut self, g: &mut Guest, worker: u32) -> Result<bool>;
    /// The output device's status call.  A result above two prevents another submission this pass.
    fn queued_buffers(&mut self, g: &mut Guest, worker: u32) -> Result<u32>;
    /// Hand a descriptor to the platform sink.  Its corresponding PCM bytes begin at
    /// `PCM_BASE + index * BUFFER_BYTES`.
    fn submit(&mut self, g: &mut Guest, worker: u32, descriptor: u32) -> Result<()>;
}

fn slot(worker: u32, index: u32) -> u32 {
    worker.wrapping_add((index.wrapping_add(SLOT_BASE)) << 2)
}

fn descriptor(worker: u32, index: u32) -> u32 {
    worker.wrapping_add(DESCRIPTORS + index.wrapping_mul(DESCRIPTOR_BYTES))
}

fn pcm(worker: u32, index: u32, base: u32) -> u32 {
    let _ = worker;
    base.wrapping_add(index.wrapping_mul(3) << 11)
}

fn advance(g: &mut Guest, worker: u32, field: u32) -> Result<()> {
    let next = g.u32(worker + field)?.wrapping_add(1);
    g.set_u32(worker + field, next)?;
    if next as i32 == BUFFER_COUNT as i32 {
        g.set_u32(worker + field, 0)?;
    }
    Ok(())
}

/// Fill and, where the output queue allows it, submit one Dac buffer.
///
/// Returns `false` only when the current fill slot was not free.  A successful iteration always
/// leaves it filled, even if the Dac had no voices and the bytes were cleared to silence.
pub fn pump_once<H: WorkerHost + ?Sized>(g: &mut Guest, host: &mut H, worker: u32) -> Result<bool> {
    let fill = g.u32(worker + FILL_INDEX)?;
    if g.u32(slot(worker, fill))? as i32 != 0 {
        return Ok(false);
    }

    if !host.render(g, worker)? {
        let base = g.u32(worker + PCM_BASE)?;
        g.fill(pcm(worker, fill, base), 0, BUFFER_BYTES)?;
    }
    g.set_u32(slot(worker, fill), 1)?;

    while g.u32(slot(worker, g.u32(worker + SUBMIT_INDEX)?))? as i32 == 1 {
        if host.queued_buffers(g, worker)? > 2 {
            break;
        }
        let submit = g.u32(worker + SUBMIT_INDEX)?;
        g.set_u32(slot(worker, submit), 2)?;
        host.submit(g, worker, descriptor(worker, submit))?;
        advance(g, worker, SUBMIT_INDEX)?;
    }
    advance(g, worker, FILL_INDEX)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const WORKER: u32 = BASE;
    const PCM: u32 = BASE + 0x1000;

    #[derive(Default)]
    struct Host {
        renders: u32,
        queued: u32,
        submitted: Vec<u32>,
        audio: bool,
    }

    impl WorkerHost for Host {
        fn render(&mut self, g: &mut Guest, worker: u32) -> Result<bool> {
            self.renders += 1;
            if self.audio {
                let base = g.u32(worker + PCM_BASE)?;
                let fill = g.u32(worker + FILL_INDEX)?;
                g.set_u32(pcm(worker, fill, base), 0x3f80_0000)?;
            }
            Ok(self.audio)
        }

        fn queued_buffers(&mut self, _g: &mut Guest, _worker: u32) -> Result<u32> {
            Ok(self.queued)
        }

        fn submit(&mut self, _g: &mut Guest, _worker: u32, descriptor: u32) -> Result<()> {
            self.submitted.push(descriptor);
            Ok(())
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x5000);
        g.set_u32(WORKER + PCM_BASE, PCM).unwrap();
        g
    }

    #[test]
    fn a_silent_render_clears_then_submits_the_current_buffer() {
        let mut g = guest();
        g.fill(PCM, 0xff, BUFFER_BYTES).unwrap();
        let mut host = Host::default();
        assert!(pump_once(&mut g, &mut host, WORKER).unwrap());
        assert_eq!(host.renders, 1);
        assert_eq!(host.submitted, vec![WORKER + DESCRIPTORS]);
        assert!(
            g.span(PCM, BUFFER_BYTES as usize)
                .unwrap()
                .iter()
                .all(|b| *b == 0)
        );
        assert_eq!(g.u32(slot(WORKER, 0)).unwrap(), 2);
        assert_eq!(g.u32(WORKER + FILL_INDEX).unwrap(), 1);
        assert_eq!(g.u32(WORKER + SUBMIT_INDEX).unwrap(), 1);
    }

    #[test]
    fn a_backed_up_sink_keeps_the_ready_buffer_for_a_later_pass() {
        let mut g = guest();
        let mut host = Host {
            queued: 3,
            audio: true,
            ..Default::default()
        };
        assert!(pump_once(&mut g, &mut host, WORKER).unwrap());
        assert_eq!(host.submitted, Vec::<u32>::new());
        assert_eq!(g.u32(slot(WORKER, 0)).unwrap(), 1);
        assert_eq!(g.u32(WORKER + FILL_INDEX).unwrap(), 1);
    }

    #[test]
    fn a_busy_fill_slot_does_not_call_the_renderer() {
        let mut g = guest();
        g.set_u32(slot(WORKER, 0), 2).unwrap();
        let mut host = Host::default();
        assert!(!pump_once(&mut g, &mut host, WORKER).unwrap());
        assert_eq!(host.renders, 0);
    }
}
