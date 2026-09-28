//! The two bus mixers: one source's channels into an object's bus blocks (`sub_82B31838`), and a
//! descriptor's channels into consecutive 1 KB runs (`sub_82B305C0`).
//!
//! Ported from `recomp/src/audio_ports/sub_82B31838.inc` (**STATUS: verified**, 933,816 calls in a
//! played session) and `sub_82B305C0.inc` (**STATUS: thin**: verified, but never called in the boot
//! profile; 100,810 comparisons in a played one).
//!
//! [`mix_source`] picks one of three mixes. Modes 1 and 3 of the owner record ramp between the zero
//! cell and a gain through [`routing::downmix`]; any other mode ramps from the gain the buses stand at
//! to the target — or, when those are equal, walks the routing row itself and mixes each route flat
//! with [`dsp::scale::scale_accumulate`]. Either way the buses end at the target gain, which is stored,
//! and each channel's gain cell times the target is published on the object.
//!
//! [`mix_back_channels`] is the simpler relative: the owner record's mode picks a gain ramp (modes 1
//! and 3) or a flat scale for every channel, and each source's last sample is kept.
//!
//! Neither structure is named by `docs/rw_audio_structs.h`, so the offsets stay numbers.

use crate::vmx::Fpscr;
use crate::{Guest, Result, dsp, fp, routing};

// ------------------------------------------------------------------ sub_82B31838: one source

/// `lwz r11,12(r31)` — the record whose byte at +72 picks the mix.
pub const SOURCE_MODE_OWNER: u32 = 12;
/// `lbz r11,72(r11)` — 1, 3, or anything else.
pub const MODE_BYTE: u32 = 72;
/// `lbz r27,41(r31)` — channels to fan out; the routing row.
pub const SOURCE_CHANNELS: u32 = 41;
/// `lfs f0,52(r31)` — the gain this block should end at.
pub const TARGET_GAIN: u32 = 52;
/// `lwz r9,64(r31)` — the first of the bus blocks.
pub const BUS_ARRAY: u32 = 64;
/// `lhz r11,76(r31)` — the byte offset, inside the bus array, of the word each call increments.
pub const BUS_COUNTER: u32 = 76;
/// `lbz r7,78(r31)` — bus blocks; the routing column.
pub const BUS_COUNT: u32 = 78;
/// One single per channel: its gain cell times the target.
pub const CHANNEL_GAINS: u32 = 80;
/// `stfs f0,112(r31)` — the gain the buses were mixed at.
pub const CURRENT_GAIN: u32 = 112;
/// Non-zero snaps the current gain to the target instead of ramping.
pub const GAIN_DIRTY: u32 = 116;
/// `lwz r11,28(r4)` — the command record's source.
pub const COMMAND_SOURCES: u32 = 28;
/// `lwz r11,4(r11)` — the source's first channel buffer.
pub const SOURCE_FIRST: u32 = 4;
/// `lhz r8,14(r11)` — words between one channel and the next.
pub const SOURCE_STRIDE: u32 = 14;
/// `lfs f13,1020(r7)` — each channel's gain cell, its buffer's last single.
pub const SOURCE_GAIN: u32 = 1020;
/// The bus stride and the mix length.
pub const BUS_BYTES: u32 = 1024;
/// `li r6,256` — what the flat mix is told to mix.
pub const MIX_SINGLES: u32 = 256;
/// `stwu r1,-208(r1)`. The two pointer arrays the mixers are handed live in this frame.
pub const MIX_FRAME_BYTES: u32 = 208;
/// `addi r4,r1,80` — the channel pointers.
pub const FRAME_CHANNELS: u32 = 80;
/// `addi r3,r1,112` — the bus pointers.
pub const FRAME_BUSES: u32 = 112;

/// Mix one source's channels into this object's bus blocks for the block (`sub_82B31838`). Returns 1.
///
/// `object` is `r3`, `command` `r4`, `forced` `r5` (only its low byte is tested) and `sp` `r1`. The
/// frame is real guest memory: the channel and bus pointer arrays are built in it and their addresses
/// handed to [`routing::downmix`].
///
/// With no buses the gain is marked dirty and nothing else happens — not even the counter. The
/// counter word is bumped before either pointer array is built. Both arrays are filled from 64-bit
/// accumulators, truncated only at the stores; a channel count above eight runs the channel array
/// into the bus array, which the bus loop then overwrites, exactly as in the original.
pub fn mix_source(g: &mut Guest, object: u32, command: u32, forced: u32, sp: u32) -> Result<u64> {
    let frame = sp.wrapping_sub(MIX_FRAME_BYTES);
    g.set_u32(frame, sp)?; // stwu r1,-208(r1)
    let channel_ptrs = frame.wrapping_add(FRAME_CHANNELS);
    let bus_ptrs = frame.wrapping_add(FRAME_BUSES);
    let mut fpscr = Fpscr::capture();

    // loc_82B31864, when the flag is set or the gain is already dirty: snap to the target.
    if forced & 0xFF != 0 || g.u8(object.wrapping_add(GAIN_DIRTY))? != 0 {
        fpscr.disable_flush_mode_unconditional();
        let target = fp::load_single(g, object.wrapping_add(TARGET_GAIN))?; // lfs f0,52(r31)
        fp::store_single(g, object.wrapping_add(CURRENT_GAIN), target)?; // stfs f0,112(r31)
        g.set_u8(object.wrapping_add(GAIN_DIRTY), 0)?; // stb r11,116(r31)
    }

    // loc_82B31874
    let buses = u32::from(g.u8(object.wrapping_add(BUS_COUNT))?); // lbz r7,78(r31)
    if buses == 0 {
        g.set_u8(object.wrapping_add(GAIN_DIRTY), 1)?; // stb r11,116(r31)
        return Ok(1); // li r3,1
    }
    let bus_array = g.u32(object.wrapping_add(BUS_ARRAY))?; // lwz r9,64(r31)
    let counter = bus_array.wrapping_add(u32::from(g.u16(object.wrapping_add(BUS_COUNTER))?));
    let count = g.u32(counter)?; // lwzx
    g.set_u32(counter, count.wrapping_add(1))?; // addi r10,r10,1 ; stwx
    let channels = u32::from(g.u8(object.wrapping_add(SOURCE_CHANNELS))?); // lbz r27,41(r31)
    let sources = g.u32(command.wrapping_add(COMMAND_SOURCES))?; // lwz r11,28(r4)
    if channels != 0 {
        let stride = u64::from(g.u16(sources.wrapping_add(SOURCE_STRIDE))?) << 2; // rotlwi r8,r8,2
        let mut pointer = u64::from(g.u32(sources.wrapping_add(SOURCE_FIRST))?);
        for i in 0..channels {
            g.set_u32(channel_ptrs.wrapping_add(4 * i), pointer as u32)?; // stwu r11,4(r10)
            pointer = pointer.wrapping_add(stride); // add r11,r8,r11
        }
    }
    let mut pointer = u64::from(bus_array);
    for i in 0..buses {
        g.set_u32(bus_ptrs.wrapping_add(4 * i), pointer as u32)?; // stwu r11,4(r10)
        pointer = pointer.wrapping_add(u64::from(BUS_BYTES)); // addi r11,r11,1024
    }

    // loc_82B318FC
    let owner = g.u32(object.wrapping_add(SOURCE_MODE_OWNER))?; // lwz r11,12(r31)
    let mode = g.u8(owner.wrapping_add(MODE_BYTE))?; // lbz r11,72(r11)
    fpscr.disable_flush_mode_unconditional();
    let zero = crate::leaves::ZERO_CELL;
    match mode {
        1 => {
            let start = fp::load_single(g, zero)?; // lfs f1,23056(r11)
            let current = fp::load_single(g, object.wrapping_add(CURRENT_GAIN))?; // lfs f2,112(r31)
            routing::downmix(
                g,
                bus_ptrs,
                channel_ptrs,
                u64::from(buses),
                channels,
                start,
                current,
            )?;
        }
        3 => {
            let target = fp::load_single(g, object.wrapping_add(TARGET_GAIN))?; // lfs f1,52(r31)
            let start = fp::load_single(g, zero)?; // lfs f2,23056(r11)
            routing::downmix(
                g,
                bus_ptrs,
                channel_ptrs,
                u64::from(buses),
                channels,
                target,
                start,
            )?;
        }
        _ => {
            // loc_82B31954
            let target = fp::load_single(g, object.wrapping_add(TARGET_GAIN))?; // lfs f31,52(r31)
            let current = fp::load_single(g, object.wrapping_add(CURRENT_GAIN))?; // lfs f2,112(r31)
            if target != current {
                routing::downmix(
                    g,
                    bus_ptrs,
                    channel_ptrs,
                    u64::from(buses),
                    channels,
                    target,
                    current,
                )?;
            } else {
                // loc_82B3197C: the downmix loop with the ramp collapsed to one gain per route.
                let row = (8 * channels).wrapping_add(buses).wrapping_sub(9);
                let pair = routing::RANGE_TABLE.wrapping_add(row.wrapping_mul(2));
                let first = u32::from(g.u8(pair)?); // lbzx r30,r11,r10
                if first <= u32::from(g.u8(pair.wrapping_add(1))?) {
                    let mut reached = first;
                    let mut cursor = routing::ROUTE_TABLE.wrapping_add(first).wrapping_sub(1);
                    loop {
                        cursor = cursor.wrapping_add(1); // lbzu r11,1(r28)
                        let entry = u32::from(g.u8(cursor)?);
                        fpscr.disable_flush_mode_unconditional();
                        let gain =
                            fp::load_single(g, routing::GAIN_TABLE + 4 * ((entry >> 6) & 3))?;
                        let source = g.u32(channel_ptrs.wrapping_add(4 * ((entry >> 3) & 7)))?;
                        let bus = g.u32(bus_ptrs.wrapping_add(4 * (entry & 7)))?;
                        let k = fp::mul_single(gain, target); // fmuls f1,f0,f31
                        dsp::scale::scale_accumulate(g, bus, source, MIX_SINGLES, k)?; // bl 0x82b44b20
                        reached += 1; // addi r30,r30,1
                        if reached > u32::from(g.u8(pair.wrapping_add(1))?) {
                            break; // lbz r4,1(r29) ; cmplw ; ble -- reloaded every route
                        }
                    }
                }
            }
        }
    }

    // loc_82B31A00: the buses now stand at the target.
    fpscr.disable_flush_mode_unconditional();
    let target = fp::load_single(g, object.wrapping_add(TARGET_GAIN))?;
    fp::store_single(g, object.wrapping_add(CURRENT_GAIN), target)?; // stfs f0,112(r31)
    // loc_82B31A30 / loc_82B31AB0: four at a time and then a remainder in the original, i ascending
    // in both, the target reloaded for every multiply. The pointers come off the frame and the stores
    // land on the object, so the unrolled form's ordering is not observable.
    for i in 0..channels {
        let channel = g.u32(channel_ptrs.wrapping_add(4 * i))?;
        fpscr.disable_flush_mode_unconditional();
        let scale = fp::load_single(g, object.wrapping_add(TARGET_GAIN))?; // lfs f0,52(r31)
        let cell = fp::load_single(g, channel.wrapping_add(SOURCE_GAIN))?; // lfs f13,1020(r7)
        let at = object.wrapping_add(CHANNEL_GAINS + 4 * i);
        fp::store_single(g, at, fp::mul_single(cell, scale))?; // fmuls f12,f13,f0 ; stfs
    }
    Ok(1) // li r3,1
}

// ----------------------------------------------------------- sub_82B305C0: the back channels

/// `lwz r7,12(r3)` — the record whose byte at +72 picks the kernel.
pub const BACK_RECORD: u32 = 12;
/// `lwz r11,52(r3)` — the block holding the counter and the destination runs.
pub const BACK_BUFFER: u32 = 52;
/// `lhz r10,64(r3)` — the counter word's byte offset inside that block.
pub const BACK_COUNTER: u32 = 64;
/// Zero returns 1 with nothing written.
pub const BACK_ENABLED: u32 = 66;
/// The kept last samples: `+4 * (dest_index + 17)`, which is `+68` for destination run 0.
pub const BACK_LAST_SAMPLE: u32 = 68;
/// The first source channel, reloaded every iteration.
pub const BACK_SOURCE_FIRST: u32 = 100;
/// The first destination run, reloaded every iteration.
pub const BACK_DEST_FIRST: u32 = 101;
/// The channel count, reloaded after every store.
pub const BACK_CHANNELS: u32 = 102;
/// `lbz r11,72(r7)` — 1 and 3 ramp, anything else scales.
pub const BACK_RECORD_MODE: u32 = 72;
/// `lwz r30,28(r4)` — the pair's back descriptor.
pub const PAIR_BACK: u32 = 28;
/// `lwz r10,4(r30)` — channel 0's float array.
pub const DESC_BUFFER_BASE: u32 = 4;
/// `lhz r9,14(r30)` — singles between channels.
pub const DESC_CHANNEL_STRIDE: u32 = 14;
/// `lfs f0,1020(r27)` — the source's last single.
pub const BACK_LAST_IN_RUN: u32 = 1020;
/// `lis -32206 ; lfs -22460` — the start gain for mode 1 and for the flat scale.
pub const BACK_GAIN_START: u32 = routing::UNITY_GAIN;
/// `lis -32208 ; addi -31232 ; lfs 1100` — mode 1's ramp step.
pub const BACK_RAMP_STEP_1: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232) + 1100;
/// `lis -32208 ; addi -31232 ; lfs 480` — mode 3's.
pub const BACK_RAMP_STEP_3: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232) + 480;
const _: () = assert!(BACK_RAMP_STEP_1 == 0x822F_8A4C && BACK_RAMP_STEP_3 == 0x822F_87E0);

/// `mullw r7,r8,r9 ; rlwinm r11,r7,2,0,29 ; add r27,r11,r10` — a 64-bit product of two sign-extended
/// words, a 32-bit shift, a 64-bit add.
fn channel_address(stride: u64, index: u32, base: u64) -> u64 {
    let elements = i64::from(stride as u32 as i32) * i64::from(index as i32);
    u64::from((elements as u32) << 2).wrapping_add(base)
}

/// Mix the pair's back channels into consecutive 1 KB runs of the object's block (`sub_82B305C0`).
/// Returns 1. `object` is `r3`, `pair` `r4`.
///
/// When enabled, the block's counter word is bumped first, and only then are the record, the
/// descriptor and the mode read. Modes 1 and 3 run the gain ramp from a pool gain by a pool step;
/// anything else scales flat at the pool's unity cell. The gains are loaded once, before the loop:
/// the original passes whatever the previous kernel left in `f1`/`f2`, which is the same value
/// because neither kernel writes them. Each source's last single is kept at `+4 * (dest + 17)`.
pub fn mix_back_channels(g: &mut Guest, object: u32, pair: u32) -> Result<u64> {
    if g.u8(object.wrapping_add(BACK_ENABLED))? == 0 {
        return Ok(1); // beq cr6,0x82b30784
    }
    let buffer = u64::from(g.u32(object.wrapping_add(BACK_BUFFER))?); // lwz r11,52(r3)
    let counter_offset = u32::from(g.u16(object.wrapping_add(BACK_COUNTER))?); // lhz r10,64(r3)
    let first_run = u64::from(g.u8(object.wrapping_add(BACK_DEST_FIRST))?) << 10; // rotlwi r9,r9,10
    let counter = counter_offset.wrapping_add(buffer as u32);
    let count = g.u32(counter)?; // lwzx r8,r10,r11
    g.set_u32(counter, count.wrapping_add(1))?; // stwx r8,r10,r11
    let mut dst = first_run.wrapping_add(buffer); // add r28,r9,r11
    let record = g.u32(object.wrapping_add(BACK_RECORD))?; // lwz r7,12(r3)
    let desc = g.u32(pair.wrapping_add(PAIR_BACK))?; // lwz r30,28(r4)
    let mode = g.u8(record.wrapping_add(BACK_RECORD_MODE))?; // lbz r11,72(r7)
    let ramp = mode == 1 || mode == 3;
    if g.u8(object.wrapping_add(BACK_CHANNELS))? == 0 {
        return Ok(1); // on all three paths
    }
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let (gain, step) = match mode {
        1 => (
            fp::load_single(g, BACK_GAIN_START)?,
            fp::load_single(g, BACK_RAMP_STEP_1)?,
        ),
        3 => (
            fp::load_single(g, crate::leaves::ZERO_CELL)?,
            fp::load_single(g, BACK_RAMP_STEP_3)?,
        ),
        _ => (fp::load_single(g, BACK_GAIN_START)?, 0.0),
    };
    let mut channel = 0u64;
    loop {
        let first = u64::from(g.u8(object.wrapping_add(BACK_SOURCE_FIRST))?);
        let source_index = first.wrapping_add(channel); // lbz r11,100(r31) ; add r8
        let stride = u64::from(g.u16(desc.wrapping_add(DESC_CHANNEL_STRIDE))?); // lhz r9,14(r30)
        let source_base = u64::from(g.u32(desc.wrapping_add(DESC_BUFFER_BASE))?); // lwz r10,4(r30)
        let src = channel_address(stride, source_index as u32, source_base);
        if ramp {
            dsp::gain_ramp::gain_ramp_accumulate(g, dst as u32, src as u32, gain, step)?; // bl 0x82b44d18
        } else {
            dsp::scale::scale_accumulate(g, dst as u32, src as u32, 256, gain)?; // li r6,256 ; bl 0x82b44b20
        }
        let dest = u64::from(g.u8(object.wrapping_add(BACK_DEST_FIRST))?);
        let dest_index = dest.wrapping_add(channel); // lbz r11,101(r31) ; add r11,r11,r29
        fpscr.disable_flush_mode_unconditional();
        let last = fp::load_single(g, (src as u32).wrapping_add(BACK_LAST_IN_RUN))?; // lfs f0,1020(r27)
        dst = dst.wrapping_add(1024); // addi r28,r28,1024
        channel = channel.wrapping_add(1); // addi r29,r29,1
        let slot = (dest_index.wrapping_add(17) as u32) << 2; // addi r6,r11,17 ; rlwinm r5,r6,2,0,29
        fp::store_single(g, slot.wrapping_add(object), last)?; // stfsx f0,r5,r31
        if (channel as u32) >= u32::from(g.u8(object.wrapping_add(BACK_CHANNELS))?) {
            break; // lbz r4,102(r31) ; cmplw ; blt -- reloaded after the store
        }
    }
    Ok(1) // li r3,1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;
    use dsp::gain_ramp::{LANE2_SCALE, LANE3_SCALE, RAMP_SPAN, SCALE, SCALE_STEP, STEP_SCALE};

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE;
    const COMMAND: u32 = BASE + 0x200;
    const SOURCES: u32 = BASE + 0x300;
    const OWNER: u32 = BASE + 0x400;
    const REF_ARRAYS: u32 = BASE + 0x800;
    const CHANNELS_AT: u32 = BASE + 0x1000;
    const BUSES_AT: u32 = BASE + 0x4000;
    const COUNTER: u32 = BUSES_AT + 0x2000;
    const SP: u32 = BASE + 0xF000;

    /// Every pool either mixer reaches: the gain-ramp kernel's, the downmix's, the routing tables and
    /// the back mixer's gains and steps.
    fn guest() -> Guest {
        let mut g = Guest::from_segments(vec![
            Segment {
                base: BASE,
                bytes: vec![0u8; 0x10000],
            },
            Segment {
                base: 0x8206_0000,
                bytes: vec![0u8; 0x4000],
            },
            Segment {
                base: 0x8216_5000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8225_7000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x820E_D000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x822F_8000,
                bytes: vec![0u8; 0x1000],
            },
            Segment {
                base: 0x8231_A000,
                bytes: vec![0u8; 0x2000],
            },
        ]);
        let f = |g: &mut Guest, at: u32, v: f32| g.set_u32(at, v.to_bits()).unwrap();
        f(&mut g, STEP_SCALE, 4.0);
        f(&mut g, RAMP_SPAN, 64.0);
        f(&mut g, LANE2_SCALE, 2.0);
        f(&mut g, LANE3_SCALE, 3.0);
        for group in 1..8u32 {
            for k in 0..4 {
                f(&mut g, SCALE[group as usize] + 4 * k, group as f32);
            }
        }
        for k in 0..4 {
            f(&mut g, SCALE_STEP + 4 * k, 8.0);
        }
        f(&mut g, crate::leaves::ZERO_CELL, 0.0);
        f(&mut g, routing::DOWNMIX_RAMP_RATE, 1.0 / 65.0);
        f(&mut g, BACK_GAIN_START, 1.0);
        f(&mut g, BACK_RAMP_STEP_1, 1.0 / 64.0);
        f(&mut g, BACK_RAMP_STEP_3, 1.0 / 128.0);
        for (i, v) in [1.0f32, 0.707, 0.5, 0.25].iter().enumerate() {
            f(&mut g, routing::GAIN_TABLE + 4 * i as u32, *v);
        }
        // Eight channel buffers whose last single is the gain cell, and eight buses at 0.5.
        for c in 0..8u32 {
            for i in 0..256u32 {
                f(
                    &mut g,
                    CHANNELS_AT + 1024 * c + 4 * i,
                    (c + 1) as f32 * 0.1 + i as f32 * 0.001,
                );
                f(&mut g, BUSES_AT + 1024 * c + 4 * i, 0.5);
            }
        }
        g
    }

    fn words(g: &Guest, at: u32, n: u32) -> Vec<u32> {
        (0..n).map(|i| g.u32(at + 4 * i).unwrap()).collect()
    }

    /// Two channels into two buses: source 0 into bus 1 at gain index 2, source 1 into bus 0 at 0.
    fn source_object(g: &mut Guest, buses: u8, mode: u8, target: f32, current: f32) {
        g.set_u32(OBJECT + SOURCE_MODE_OWNER, OWNER).unwrap();
        g.set_u8(OWNER + MODE_BYTE, mode).unwrap();
        g.set_u8(OBJECT + SOURCE_CHANNELS, 2).unwrap();
        g.set_u8(OBJECT + BUS_COUNT, buses).unwrap();
        g.set_u32(OBJECT + TARGET_GAIN, target.to_bits()).unwrap();
        g.set_u32(OBJECT + CURRENT_GAIN, current.to_bits()).unwrap();
        g.set_u32(OBJECT + BUS_ARRAY, BUSES_AT).unwrap();
        g.set_u16(OBJECT + BUS_COUNTER, (COUNTER - BUSES_AT) as u16)
            .unwrap();
        g.set_u32(COMMAND + COMMAND_SOURCES, SOURCES).unwrap();
        g.set_u32(SOURCES + SOURCE_FIRST, CHANNELS_AT).unwrap();
        g.set_u16(SOURCES + SOURCE_STRIDE, 256).unwrap();
        g.set_u8(routing::RANGE_TABLE + 18, 3).unwrap(); // 2 * (8*2 + 2 - 9)
        g.set_u8(routing::RANGE_TABLE + 19, 4).unwrap();
        g.set_u8(routing::ROUTE_TABLE + 3, 0x81).unwrap(); // gain 2, source 0, bus 1
        g.set_u8(routing::ROUTE_TABLE + 4, 0x08).unwrap(); // gain 0, source 1, bus 0
    }

    /// The two flat mixes the routing row names, made directly.
    fn flat_reference(h: &mut Guest, target: f32) {
        for (bus, chan, gain) in [(1u32, 0u32, 0.5f64), (0, 1, 1.0)] {
            let k = fp::mul_single(gain, f64::from(target));
            dsp::scale::scale_accumulate(
                h,
                BUSES_AT + 1024 * bus,
                CHANNELS_AT + 1024 * chan,
                256,
                k,
            )
            .unwrap();
        }
    }

    /// The downmix, made directly with pointer arrays of its own.
    fn downmix_reference(h: &mut Guest, target: f64, current: f64) {
        for i in 0..2u32 {
            h.set_u32(REF_ARRAYS + 4 * i, BUSES_AT + 1024 * i).unwrap();
            h.set_u32(REF_ARRAYS + 0x40 + 4 * i, CHANNELS_AT + 1024 * i)
                .unwrap();
        }
        routing::downmix(h, REF_ARRAYS, REF_ARRAYS + 0x40, 2, 2, target, current).unwrap();
    }

    fn same_buses(g: &Guest, h: &Guest) {
        for b in 0..2 {
            assert_eq!(
                words(g, BUSES_AT + 1024 * b, 256),
                words(h, BUSES_AT + 1024 * b, 256),
                "bus {b}"
            );
        }
    }

    #[test]
    fn an_unmoved_gain_mixes_each_route_flat_at_its_table_gain() {
        let mut g = guest();
        source_object(&mut g, 2, 0, 0.8, 0.8);
        let mut h = g.clone();
        assert_eq!(mix_source(&mut g, OBJECT, COMMAND, 0, SP).unwrap(), 1);
        flat_reference(&mut h, 0.8);
        same_buses(&g, &h);
        assert_ne!(
            g.u32(BUSES_AT).unwrap(),
            0.5f32.to_bits(),
            "bus 0 was mixed into"
        );
        assert_eq!(g.u32(COUNTER).unwrap(), 1, "the counter");
        for c in 0..2u32 {
            let cell = g.f32(CHANNELS_AT + 1024 * c + SOURCE_GAIN).unwrap();
            assert_eq!(
                g.f32(OBJECT + CHANNEL_GAINS + 4 * c).unwrap(),
                cell * 0.8,
                "channel {c}'s gain"
            );
        }
        assert_eq!(g.u32(SP - MIX_FRAME_BYTES).unwrap(), SP, "the back chain");
    }

    #[test]
    fn a_moved_gain_runs_the_downmix_from_the_current_gain_to_the_target() {
        let mut g = guest();
        source_object(&mut g, 2, 0, 0.8, 0.2);
        let mut h = g.clone();
        mix_source(&mut g, OBJECT, COMMAND, 0, SP).unwrap();
        downmix_reference(&mut h, f64::from(0.8f32), f64::from(0.2f32));
        same_buses(&g, &h);
        assert_eq!(
            g.f32(OBJECT + CURRENT_GAIN).unwrap(),
            0.8,
            "the buses now stand at the target"
        );
    }

    #[test]
    fn forcing_snaps_the_gain_so_the_mix_is_flat() {
        let mut g = guest();
        source_object(&mut g, 2, 0, 0.8, 0.2);
        let mut h = g.clone();
        mix_source(&mut g, OBJECT, COMMAND, 0x100 | 1, SP).unwrap();
        flat_reference(&mut h, 0.8);
        same_buses(&g, &h);
        assert_eq!(g.u8(OBJECT + GAIN_DIRTY).unwrap(), 0);
    }

    #[test]
    fn only_the_low_byte_of_the_flag_counts() {
        let mut g = guest();
        source_object(&mut g, 2, 0, 0.8, 0.2);
        let mut h = g.clone();
        mix_source(&mut g, OBJECT, COMMAND, 0x100, SP).unwrap();
        downmix_reference(&mut h, f64::from(0.8f32), f64::from(0.2f32));
        same_buses(&g, &h);
    }

    #[test]
    fn mode_one_ramps_from_the_zero_cell_toward_the_current_gain() {
        let mut g = guest();
        source_object(&mut g, 2, 1, 0.8, 0.2);
        let mut h = g.clone();
        mix_source(&mut g, OBJECT, COMMAND, 0, SP).unwrap();
        downmix_reference(&mut h, 0.0, f64::from(0.2f32));
        same_buses(&g, &h);
    }

    #[test]
    fn no_buses_marks_the_gain_dirty_and_touches_nothing_else() {
        let mut g = guest();
        source_object(&mut g, 0, 0, 0.8, 0.2);
        assert_eq!(mix_source(&mut g, OBJECT, COMMAND, 0, SP).unwrap(), 1);
        assert_eq!(g.u8(OBJECT + GAIN_DIRTY).unwrap(), 1);
        assert_eq!(g.u32(COUNTER).unwrap(), 0, "the counter is not bumped");
        assert_eq!(g.f32(OBJECT + CURRENT_GAIN).unwrap(), 0.2);
    }

    // ------------------------------------------------------------------ sub_82B305C0

    const BACK: u32 = BASE + 0x100;
    const PAIR: u32 = BASE + 0x500;
    const DESC: u32 = BASE + 0x600;
    const RECORD: u32 = BASE + 0x680;
    const BLOCK: u32 = BASE + 0x8000;

    /// Two channels: sources 1 and 2 into runs 2 and 3 of the block.
    fn back_object(g: &mut Guest, mode: u8, enabled: u8) {
        g.set_u8(BACK + BACK_ENABLED, enabled).unwrap();
        g.set_u32(BACK + BACK_BUFFER, BLOCK).unwrap();
        g.set_u16(BACK + BACK_COUNTER, 0x1800).unwrap();
        g.set_u8(BACK + BACK_SOURCE_FIRST, 1).unwrap();
        g.set_u8(BACK + BACK_DEST_FIRST, 2).unwrap();
        g.set_u8(BACK + BACK_CHANNELS, 2).unwrap();
        g.set_u32(BACK + BACK_RECORD, RECORD).unwrap();
        g.set_u8(RECORD + BACK_RECORD_MODE, mode).unwrap();
        g.set_u32(PAIR + PAIR_BACK, DESC).unwrap();
        g.set_u32(DESC + DESC_BUFFER_BASE, CHANNELS_AT).unwrap();
        g.set_u16(DESC + DESC_CHANNEL_STRIDE, 256).unwrap();
        for i in 0..512u32 {
            g.set_u32(BLOCK + 2048 + 4 * i, 0.25f32.to_bits()).unwrap();
        }
    }

    fn back_reference(h: &mut Guest, ramp: Option<(f64, f64)>) {
        for ch in 0..2u32 {
            let (dst, src) = (BLOCK + 2048 + 1024 * ch, CHANNELS_AT + 1024 * (1 + ch));
            match ramp {
                Some((gain, step)) => {
                    dsp::gain_ramp::gain_ramp_accumulate(h, dst, src, gain, step).unwrap();
                }
                None => dsp::scale::scale_accumulate(h, dst, src, 256, 1.0).unwrap(),
            }
        }
    }

    fn same_runs(g: &Guest, h: &Guest) {
        assert_eq!(words(g, BLOCK + 2048, 512), words(h, BLOCK + 2048, 512));
        // Word 100 rather than word 0: mode 3's ramp starts from a zero gain.
        assert_ne!(
            g.u32(BLOCK + 2048 + 400).unwrap(),
            0.25f32.to_bits(),
            "run 2 was mixed into"
        );
    }

    #[test]
    fn a_disabled_object_writes_nothing() {
        let mut g = guest();
        back_object(&mut g, 0, 0);
        assert_eq!(mix_back_channels(&mut g, BACK, PAIR).unwrap(), 1);
        assert_eq!(g.u32(BLOCK + 0x1800).unwrap(), 0);
        assert_eq!(g.u32(BLOCK + 2048).unwrap(), 0.25f32.to_bits());
    }

    #[test]
    fn other_modes_scale_flat_and_keep_each_last_sample() {
        let mut g = guest();
        back_object(&mut g, 0, 1);
        let mut h = g.clone();
        assert_eq!(mix_back_channels(&mut g, BACK, PAIR).unwrap(), 1);
        back_reference(&mut h, None);
        same_runs(&g, &h);
        assert_eq!(g.u32(BLOCK + 0x1800).unwrap(), 1, "the counter");
        for ch in 0..2u32 {
            let last = g
                .f32(CHANNELS_AT + 1024 * (1 + ch) + BACK_LAST_IN_RUN)
                .unwrap();
            assert_eq!(
                g.f32(BACK + 4 * (2 + ch + 17)).unwrap(),
                last,
                "channel {ch}'s last sample"
            );
        }
    }

    #[test]
    fn modes_one_and_three_ramp_from_their_pool_gains() {
        for (mode, gain, step) in [(1u8, 1.0f32, 1.0f32 / 64.0), (3, 0.0, 1.0 / 128.0)] {
            let mut g = guest();
            back_object(&mut g, mode, 1);
            let mut h = g.clone();
            mix_back_channels(&mut g, BACK, PAIR).unwrap();
            back_reference(&mut h, Some((f64::from(gain), f64::from(step))));
            same_runs(&g, &h);
        }
    }
}
