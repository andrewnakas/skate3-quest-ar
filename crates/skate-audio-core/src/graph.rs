//! A voice graph's per-block pass: the node walk `sub_82B44858` and each node's mixer fill
//! `sub_82B444C0`.
//!
//! **Unverified.** Both C++ bodies are gate 1 (every module is reached through a function pointer
//! read from a class table) and the pass is gate 3 too (it profiles itself with `mftb`), so neither
//! was compared. They are transcribed from those bodies. The indirect calls go through
//! [`GraphHost`]; so does the timebase, whose readings only ever land in profiling fields.
//!
//! A module class's function table is `{0, prepare, process}` (`docs/audio-banks.md`, the class
//! table). The pass calls `process(child, pass, flag)` for each child of each node, after the node's
//! mixer fill has asked every source to `prepare(sink, mixer, flag, request)` a length and then to
//! `process(sink, mixer, flag)` it. A declining process falls back to the verified
//! [`advance_and_clear`] (`sub_82B443F8`).

use crate::fp::{fcfid, frsp, load_single, store_single};
use crate::mem::{memcpy, memset};
use crate::mix::advance_and_clear;
use crate::{Guest, Result};

/// The three 276-byte descriptor records the pass publishes (`lis -31993` + 30288).
pub const RECORD_BASE: u32 = 0x8307_7650;
pub const RECORD_STRIDE: u32 = 276;
/// `lis -32234` + 23056: 0.0.
pub const ZERO_SINGLE: u32 = 0x8216_5A10;
/// `0x822F8600 + 440`: the three-tap timing average's multiplier.
pub const TIME_SCALE: u32 = 0x822F_87B8;
/// `lis -32206` + -22460: 1.0, the per-pass scratch float's reset value.
pub const ONE_SINGLE: u32 = 0x8231_A844;
/// The block every fill produces.
pub const BLOCK_FRAMES: u64 = 256;

/// What a graph pass calls indirectly.
pub trait GraphHost {
    /// A class table's `+4`: how many of `request` frames `object` can take for `owner`.
    fn prepare(
        &mut self,
        g: &mut Guest,
        function: u32,
        object: u32,
        owner: u32,
        flag: u32,
        request: u64,
    ) -> Result<u64>;
    /// A class table's `+8`: run `object` for `owner`. Zero means "declined".
    fn process(
        &mut self,
        g: &mut Guest,
        function: u32,
        object: u32,
        owner: u32,
        flag: u32,
    ) -> Result<u64>;
    /// `sub_82B1F7E8`, the timebase. Profiling only; a host with no clock returns 0.
    fn ticks(&mut self) -> u64 {
        0
    }
}

fn word(value: u64) -> i32 {
    value as u32 as i32
}

/// `((index << 2) & ~3) + array`, truncated.
fn float_slot(array: u64, index: u64) -> u32 {
    ((((index as u32) << 2) & 0xFFFF_FFFC) as u64).wrapping_add(array) as u32
}

/// The source cost counter at `+36`: `now + (held - started)`.
fn accumulate(g: &mut Guest, host: &mut dyn GraphHost, sink: u32, started: u64) -> Result<()> {
    let now = host.ticks();
    let held = g.u32(sink + 36)? as u64;
    g.set_u32(
        sink + 36,
        now.wrapping_add(held.wrapping_sub(started)) as u32,
    )
}

/// `sub_82B444C0`: fill one 256-frame block for `mixer` from the sources in `sources` (8-byte entries
/// whose first word is a class table) and the sink objects at `stream + 80`. Returns the last status.
pub fn fill_mixer_block(
    g: &mut Guest,
    host: &mut dyn GraphHost,
    mixer: u32,
    sources: u32,
    stream: u32,
) -> Result<u64> {
    let zero = load_single(g, ZERO_SINGLE)?;
    let level_reset = load_single(g, ONE_SINGLE)?;
    let mut level = zero;
    let mut produced: u64 = 0;
    let mut rounds: u64 = 0;
    let mut status: u64;
    let mut channels: u64 = 0;
    let mut direct_block: u64 = 0;
    let mut last_request: u64;

    loop {
        store_single(g, mixer + 56, level_reset)?;
        let last_source = g.u8(stream + 70)? as i32;
        let mut request = BLOCK_FRAMES.wrapping_sub(produced);
        last_request = 0;

        // Descending: negotiate a length with every source.
        for source in (0..=last_source).rev() {
            let started = host.ticks();
            let limit = g.u8(stream + 69)? as i32;
            let descriptor = g.u32((((source as u32) << 3) & 0xFFFF_FFF8).wrapping_add(sources))?;
            let sink = g.u32(((((source + 20) as u32) << 2) & 0xFFFF_FFFC).wrapping_add(stream))?;
            let function = g.u32(descriptor + 4)?;
            let flag = (limit < source) as u32;
            last_request = request;
            let answer = host.prepare(g, function, sink, mixer, flag, request)?;
            request = answer;
            if word(answer) >= BLOCK_FRAMES as i32 {
                request = BLOCK_FRAMES;
            }
            if rounds == 0 {
                g.set_u32(sink + 36, 0)?;
            }
            accumulate(g, host, sink, started)?;
        }
        rounds += 1;

        // Ascending: render, with the fallback when a source declines.
        let mut source: u64 = 0;
        loop {
            let started = host.ticks();
            let limit = g.u8(stream + 69)? as i32;
            let descriptor = g.u32((((source as u32) << 3) & 0xFFFF_FFF8).wrapping_add(sources))?;
            let sink = g.u32((((source as u32 + 20) << 2) & 0xFFFF_FFFC).wrapping_add(stream))?;
            let function = g.u32(descriptor + 8)?;
            let flag = (limit < source as i32) as u32;
            status = host.process(g, function, sink, mixer, flag)?;
            if word(status) == 0 {
                status = advance_and_clear(g, mixer, stream, sink, last_request as u32)? as u64;
                if word(status) == 0 {
                    accumulate(g, host, sink, started)?;
                    break;
                }
            } else if source == 0 {
                store_single(g, stream + 44, zero)?;
            }
            accumulate(g, host, sink, started)?;
            if (source as i32 + 1) > g.u8(stream + 70)? as i32 {
                break;
            }
            source += 1;
        }

        if word(status) != 1 {
            if produced as u32 != 0 {
                let assembly = g.u32(mixer + 36)?;
                if channels != 0 {
                    let tail_bytes = ((BLOCK_FRAMES.wrapping_sub(produced) as u32) << 2) as u64;
                    for channel in 0..channels {
                        let stride = g.u16(assembly + 14)? as u64;
                        let data = g.u32(assembly + 4)? as u64;
                        let index = ((word(stride) as i64 * word(channel) as i64) as u64)
                            .wrapping_add(produced);
                        memset(g, float_slot(data, index), 0, tail_bytes)?;
                    }
                }
                store_single(g, mixer + 52, level)?;
                status = 1;
                g.set_u8(mixer + 60, channels as u8)?;
            }
            break;
        }

        let round_channels = g.u8(mixer + 60)? as u64;
        let round_frames = g.u32(mixer + 48)?;
        let mut mix = true;
        if produced as u32 == 0 && round_frames as i32 == BLOCK_FRAMES as i32 {
            direct_block = 1;
        } else if round_frames as i32 == 0 {
            mix = false;
        }
        if mix {
            if direct_block & 0xFF == 0 {
                let from = g.u32(mixer + 28)?;
                let into = g.u32(mixer + 36)?;
                let round_bytes = ((round_frames << 2) & 0xFFFF_FFFC) as u64;
                for channel in 0..round_channels {
                    let into_stride = g.u16(into + 14)? as u64;
                    let from_stride = g.u16(from + 14)? as u64;
                    let from_data = g.u32(from + 4)? as u64;
                    let into_data = g.u32(into + 4)? as u64;
                    let into_index = ((word(into_stride) as i64 * word(channel) as i64) as u64)
                        .wrapping_add(produced);
                    let from_index = (word(from_stride) as i64 * word(channel) as i64) as u64;
                    memcpy(
                        g,
                        float_slot(into_data, into_index),
                        float_slot(from_data, from_index),
                        round_bytes,
                    )?;
                }
            }
            let format = g.u32(mixer + 40)?;
            let position = f64::from_bits(g.u64(mixer + 16)?);
            channels = round_channels;
            let frames = fcfid(round_frames as i32 as i64);
            level = load_single(g, mixer + 52)?;
            let rounded = frsp(frames);
            let rate = load_single(g, format + 12)?;
            let seconds = frsp(rounded / rate);
            g.set_u64(mixer + 16, (seconds + position).to_bits())?; // fadd: double precision
        }
        produced = (round_frames as u64).wrapping_add(produced);
        if produced as u32 >= BLOCK_FRAMES as u32 {
            break;
        }
    }

    if direct_block & 0xFF == 0 {
        let from = g.u32(mixer + 36)?;
        let into = g.u32(mixer + 32)?;
        if g.u8(mixer + 60)? != 0 {
            let mut channel: u64 = 0;
            loop {
                let from_stride = g.u16(from + 14)? as u64;
                let into_stride = g.u16(into + 14)? as u64;
                let from_data = g.u32(from + 4)? as u64;
                let into_data = g.u32(into + 4)? as u64;
                let from_index = (word(from_stride) as i64 * word(channel) as i64) as u64;
                let into_index = (word(into_stride) as i64 * word(channel) as i64) as u64;
                memcpy(
                    g,
                    float_slot(into_data, into_index),
                    float_slot(from_data, from_index),
                    1024,
                )?;
                if channel + 1 >= g.u8(mixer + 60)? as u64 {
                    break;
                }
                channel += 1;
            }
        }
        let spare = g.u32(mixer + 32)?;
        let ready = g.u32(mixer + 28)?;
        g.set_u32(mixer + 28, spare)?;
        g.set_u32(mixer + 32, ready)?;
    }
    g.set_u32(mixer + 48, BLOCK_FRAMES as u32)?;
    Ok(status)
}

/// `sub_82B44858`: one pass over the nodes in `params` (`+8` an array of 8-byte entries whose first
/// word is a node, `+20` a u16 count). Returns the last status.
pub fn run_pass(g: &mut Guest, host: &mut dyn GraphHost, pass: u32, params: u32) -> Result<u64> {
    let call_start = host.ticks();
    g.set_u32(pass + 40, params)?;

    // Three descriptor records, the same five fields in three store orders, each reloading the device.
    {
        let device = g.u32(pass + 24)?;
        let byte = g.u32(device + 252)?;
        let source = g.u32(pass)?;
        let record = RECORD_BASE;
        g.set_u32(record, device)?;
        g.set_u16(record + 14, 256)?;
        g.set_u8(record + 16, byte as u8)?;
        g.set_u16(record + 12, 0)?;
        g.set_u32(record + 4, source)?;
        g.set_u32(pass + 28, record)?;
    }
    {
        let device = g.u32(pass + 24)?;
        let byte = g.u32(device + 252)?;
        let source = g.u32(pass + 4)?;
        let record = RECORD_BASE + RECORD_STRIDE;
        g.set_u8(record + 16, byte as u8)?;
        g.set_u32(record, device)?;
        g.set_u16(record + 12, 0)?;
        g.set_u16(record + 14, 256)?;
        g.set_u32(record + 4, source)?;
        g.set_u32(pass + 32, record)?;
    }
    {
        let device = g.u32(pass + 24)?;
        let byte = g.u32(device + 252)?;
        let source = g.u32(pass + 8)?;
        let record = RECORD_BASE + 2 * RECORD_STRIDE;
        g.set_u32(record, device)?;
        g.set_u16(record + 12, 0)?;
        g.set_u16(record + 14, 256)?;
        g.set_u8(record + 16, byte as u8)?;
        g.set_u32(record + 4, source)?;
        g.set_u32(pass + 36, record)?;
    }

    let mut status: u64 = 0;
    if g.u16(g.u32(pass + 40)? + 20)? != 0 {
        let zero = load_single(g, ZERO_SINGLE)?;
        let scale = load_single(g, TIME_SCALE)?;
        let scratch_init = load_single(g, ONE_SINGLE)?;
        let mut offset: u32 = 0;
        let mut index: u64 = 0;
        loop {
            let array = g.u32(g.u32(pass + 40)? + 8)?;
            let node_start = host.ticks();
            let node = g.u32(array.wrapping_add(offset))?;
            let table = g.u32(node + 24)?;
            store_single(g, pass + 56, scratch_init)?;

            let mut child_index: u64;
            if g.u8(node + 70)? < 255 {
                let copy = g.u64(g.u32(pass + 40)?)?;
                g.set_u64(pass + 16, copy)?;
                status = fill_mixer_block(g, host, pass, table, node)?;
                child_index = g.u8(node + 70)? as u64 + 1; // reloaded after the call
            } else {
                child_index = 0;
                status = 1;
            }
            let copy = g.u64(g.u32(pass + 40)?)?;
            g.set_u64(pass + 16, copy)?;

            if word(status) == 1 {
                let mut set_progress = true;
                if (child_index as i32) < g.u8(node + 68)? as i32 {
                    loop {
                        let child_start = host.ticks();
                        let child = g.u32(node + 4 * (child_index as u32 + 20))?;
                        if g.u8(child + 40)? == 0 {
                            let descriptor = g.u32(table.wrapping_add(8 * child_index as u32))?;
                            let progress = g.u8(node + 69)? as i32;
                            let past = (progress < child_index as i32) as u32;
                            let function = g.u32(descriptor + 8)?;
                            status = host.process(g, function, child, pass, past)?;
                            if word(status) == 0 {
                                status =
                                    advance_and_clear(g, pass, node, child, BLOCK_FRAMES as u32)?
                                        as u64;
                                if word(status) == 0 {
                                    break; // untimed, unadvanced
                                }
                            } else if child_index as i32 == 0 {
                                store_single(g, node + 44, zero)?;
                            }
                        }
                        let elapsed = host.ticks().wrapping_sub(child_start);
                        child_index += 1;
                        g.set_u32(child + 36, elapsed as u32)?;
                        if !((child_index as i32) < g.u8(node + 68)? as i32) {
                            break;
                        }
                    }
                    set_progress = word(status) == 1;
                }
                if set_progress {
                    let count = g.u8(node + 68)?;
                    g.set_u8(node + 69, count)?;
                }
            }

            // The node's timing tail.
            let node_elapsed = host.ticks().wrapping_sub(node_start);
            let slot_b = load_single(g, node + 8)?;
            let flag = g.u32(node + 12)?;
            let low = node_elapsed & 0xFFFF_FFFF;
            let slot_a = load_single(g, node + 4)?;
            let history = frsp(slot_b + slot_a);
            let next_flag = flag.wrapping_add(1);
            let as_single = frsp(fcfid(low as i64));
            g.set_u32(node + 60, node_elapsed as u32)?;
            index += 1;
            offset = offset.wrapping_add(8);
            let total = frsp(history + as_single);
            store_single(g, node, frsp(total * scale))?;
            store_single(g, node.wrapping_add(next_flag.wrapping_mul(4)), as_single)?;
            let reloaded = g.u32(node + 12)?;
            g.set_u32(node + 12, (reloaded == 0) as u32)?;
            let count = g.u16(g.u32(pass + 40)? + 20)? as i32;
            if !((index as i32) < count) {
                break;
            }
        }
    }
    let total = host.ticks().wrapping_sub(call_start);
    g.set_u32(pass + 44, total as u32)?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x5000_0000;
    const MIXER: u32 = MEM;
    const SOURCES: u32 = MEM + 0x100;
    const STREAM: u32 = MEM + 0x200;
    const SINK: u32 = MEM + 0x300;
    const TABLE: u32 = MEM + 0x380;
    const READY: u32 = MEM + 0x400;
    const SPARE: u32 = MEM + 0x440;
    const ASSEMBLY: u32 = MEM + 0x480;
    const FORMAT: u32 = MEM + 0x4C0;
    const READY_DATA: u32 = MEM + 0x1000;
    const SPARE_DATA: u32 = MEM + 0x2000;
    const ASSEMBLY_DATA: u32 = MEM + 0x3000;

    /// One source that renders `chunk` frames of a running ramp per call into the mixer's ready
    /// buffer, never more than it was offered.
    struct Chunks {
        chunk: u32,
        offered: u64,
        next: f32,
        processed: Vec<(u32, u32, u32)>,
    }

    impl GraphHost for Chunks {
        fn prepare(
            &mut self,
            _g: &mut Guest,
            _f: u32,
            _o: u32,
            _m: u32,
            _flag: u32,
            request: u64,
        ) -> Result<u64> {
            self.offered = request;
            Ok(request)
        }
        fn process(
            &mut self,
            g: &mut Guest,
            function: u32,
            object: u32,
            owner: u32,
            flag: u32,
        ) -> Result<u64> {
            self.processed.push((function, object, flag));
            if owner != MIXER {
                return Ok(1); // a child module in the pass test
            }
            let frames = self.chunk.min(self.offered as u32);
            let ready = g.u32(MIXER + 28)?;
            let data = g.u32(ready + 4)?;
            for i in 0..frames {
                self.next += 1.0;
                g.set_u32(data + 4 * i, self.next.to_bits())?;
            }
            g.set_u32(MIXER + 48, frames)?;
            g.set_u8(MIXER + 60, 1)?;
            Ok(1)
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x4000);
        g.put(ZERO_SINGLE, vec![0; 4]);
        g.put(TIME_SCALE, (1.0f32 / 3.0).to_bits().to_be_bytes().to_vec());
        g.put(ONE_SINGLE, 1.0f32.to_bits().to_be_bytes().to_vec());
        g.put(RECORD_BASE, vec![0; 3 * RECORD_STRIDE as usize]);
        for (desc, data) in [
            (READY, READY_DATA),
            (SPARE, SPARE_DATA),
            (ASSEMBLY, ASSEMBLY_DATA),
        ] {
            g.set_u32(desc + 4, data).unwrap();
            g.set_u16(desc + 14, 256).unwrap();
        }
        g.set_u32(MIXER + 28, READY).unwrap();
        g.set_u32(MIXER + 32, SPARE).unwrap();
        g.set_u32(MIXER + 36, ASSEMBLY).unwrap();
        g.set_u32(MIXER + 40, FORMAT).unwrap();
        g.set_u32(FORMAT + 12, 48000.0f32.to_bits()).unwrap();
        g.set_u32(SOURCES, TABLE).unwrap(); // source 0's class table
        g.set_u32(TABLE + 4, 0x82B2_DAC8).unwrap();
        g.set_u32(TABLE + 8, 0x82B2_DBA8).unwrap();
        g.set_u32(STREAM + 80, SINK).unwrap(); // source 0's sink object
        g
    }

    #[test]
    fn short_rounds_are_assembled_into_one_block_and_the_pair_swaps() {
        let mut g = guest();
        let mut host = Chunks {
            chunk: 100,
            offered: 0,
            next: 0.0,
            processed: vec![],
        };
        assert_eq!(
            fill_mixer_block(&mut g, &mut host, MIXER, SOURCES, STREAM).unwrap(),
            1
        );
        assert_eq!(host.processed.len(), 3, "100 + 100 + 56 frames");
        assert_eq!(
            g.u32(MIXER + 28).unwrap(),
            SPARE,
            "the assembled block is now the ready one"
        );
        let block: Vec<f32> = (0..256)
            .map(|i| g.f32(SPARE_DATA + 4 * i).unwrap())
            .collect();
        assert_eq!(block, (1..=256).map(|n| n as f32).collect::<Vec<_>>());
        assert_eq!(g.u32(MIXER + 48).unwrap(), 256);
        let position = f64::from_bits(g.u64(MIXER + 16).unwrap());
        assert!((position - 256.0 / 48000.0).abs() < 1e-9, "{position}");
    }

    #[test]
    fn a_full_first_round_is_used_directly_with_no_copy() {
        let mut g = guest();
        let mut host = Chunks {
            chunk: 256,
            offered: 0,
            next: 0.0,
            processed: vec![],
        };
        fill_mixer_block(&mut g, &mut host, MIXER, SOURCES, STREAM).unwrap();
        assert_eq!(host.processed.len(), 1);
        assert_eq!(
            g.u32(MIXER + 28).unwrap(),
            READY,
            "no swap on the direct path"
        );
        assert_eq!(g.f32(READY_DATA + 4 * 255).unwrap(), 256.0);
    }

    #[test]
    fn the_pass_runs_each_child_with_the_past_progress_flag_and_records_progress() {
        let mut g = guest();
        let (pass, params, node, child_a, child_b, class_a) = (
            MEM + 0x600,
            MEM + 0x700,
            MEM + 0x800,
            MEM + 0x900,
            MEM + 0x940,
            MEM + 0x980,
        );
        g.set_u32(pass + 24, MEM + 0xA00).unwrap(); // device
        g.set_u32(params + 8, MEM + 0xB00).unwrap();
        g.set_u16(params + 20, 1).unwrap();
        g.set_u32(MEM + 0xB00, node).unwrap();
        g.set_u8(node + 70, 255).unwrap(); // no mixer fill: straight to the children
        g.set_u8(node + 68, 2).unwrap();
        g.set_u8(node + 69, 0).unwrap();
        let children = MEM + 0xC00; // the node's 8-byte child table, one class table per child
        g.set_u32(node + 24, children).unwrap();
        g.set_u32(children, TABLE).unwrap(); // child 0: TABLE's class, process 0x82B2DBA8
        g.set_u32(children + 8, class_a).unwrap(); // child 1: its own class
        g.set_u32(class_a + 8, 0x82B2_7E20).unwrap();
        g.set_u32(node + 80, child_a).unwrap();
        g.set_u32(node + 84, child_b).unwrap();
        let mut host = Chunks {
            chunk: 0,
            offered: 0,
            next: 0.0,
            processed: vec![],
        };
        assert_eq!(run_pass(&mut g, &mut host, pass, params).unwrap(), 1);
        assert_eq!(
            host.processed,
            vec![(0x82B2_DBA8, child_a, 0), (0x82B2_7E20, child_b, 1)]
        );
        assert_eq!(g.u8(node + 69).unwrap(), 2, "progress recorded");
        assert_eq!(g.u32(pass + 28).unwrap(), RECORD_BASE);
        assert_eq!(g.u16(RECORD_BASE + 14).unwrap(), 256);
        assert_eq!(g.u32(node + 12).unwrap(), 1, "the timing slot alternates");
    }
}
