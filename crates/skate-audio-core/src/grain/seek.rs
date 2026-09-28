//! The SndPlayer1 seek-table reader that turns a grain player's start time into a decoder restart
//! point: `sub_82B470D0` and the run-length column decoders under it, the seek branch of
//! `sub_82B33780`, and the frame-offset computation of the play command `sub_82B32DC8` that feeds it.
//!
//! | function | here |
//! |---|---|
//! | `sub_82B46EA0`, one signed varint | [`read_varint`] |
//! | `sub_82B46FB0`, add the next varint to a column's value | [`column_step`] |
//! | `sub_82B47010`, a run-length column's next value | [`column_next`] |
//! | `sub_82B47658`, the first row of four columns | [`first_row`] |
//! | `sub_82B474B8`, column layout 1: walk rows to the target frame | [`seek_layout1`] |
//! | `sub_82B470D0`, dispatch on the table's kind and layout | [`seek`] |
//! | `sub_82B33780`, the seek branch and its slot/record stores | [`apply_seek_fields`] |
//! | `sub_82B32DC8`, `fctiwz(rate × start)` and its admission mask | [`start_frame`] |
//!
//! Column layout 0 (`sub_82B472C0`) and table kind 1 (`sub_82B471D8`) are not used by any
//! `grains.big` member (all 14 are kind 0, layout 1) and are not ported: reaching them is an error
//! naming the address.
//!
//! All state lives in guest memory, as in the original: the column records and the shared byte
//! cursor are on `sub_82B474B8`'s stack frame (`sp − 272`), the result record is the caller's.
//!
//! **The result record** (`out`, 36 bytes): `+0` 0, `+4` side-data pointer (table-relative base plus
//! the accumulated column 1, or 0), `+8` the first sample of the entry the decoder restarts in,
//! `+12` samples to skip after the preroll, `+16` the preroll actually used (`min(target − entry,
//! preroll)`), `+20` the accumulated column 0 (byte offset of the entry in the EAAC stream), `+24`
//! the header's low nibble, `+28` the preroll (u16 widened), `+32` byte: the entry was a key row.
//! By construction `+8 + +16 + +12` is the target frame.

use crate::fp;
use crate::{Error, Guest, Result};

/// `sub_82B474B8`'s frame, `stwu r1,-272(r1)`.
pub const LAYOUT1_FRAME: u32 = 272;
/// `sub_82B33780`'s frame, `stwu r1,-144(r1)`; its result record is at `+80`.
pub const DECODE_FRAME: u32 = 144;
/// Bytes of the result record `sub_82B470D0` fills.
pub const RESULT_BYTES: u32 = 36;

/// `sub_82B46EA0`: decode one signed varint at `at`. Returns `(value, length)`; the original stores
/// the value through its `r4` and returns the length.
pub fn read_varint(g: &Guest, at: u32) -> Result<(u32, u32)> {
    let b0 = u32::from(g.u8(at)?);
    let (mut value, sign, len) = if b0 < 192 {
        // rlwinm r11,r10,31,1,31 ; clrlwi r10,r10,31
        (b0 >> 1, b0 & 1, 1)
    } else if b0 < 240 {
        let b1 = u32::from(g.u8(at + 1)?);
        let r9 = (b0 << 8) | b1; // rlwinm r10,r10,8,0,23 ; or
        let r8 = ((r9 as i32) >> 1) as u32; // srawi r8,r9,1
        let r11 = r8 & !0x0000_6000; // rlwinm r11,r8,0,19,16
        (r11.wrapping_add(96), b1 & 1, 2)
    } else if b0 < 252 {
        let b1 = u32::from(g.u8(at + 1)?);
        let b2 = u32::from(g.u8(at + 2)?);
        let r6 = (b0 << 8) | b1;
        let r11 = r6 & !0x0000_F000; // rlwinm r11,r6,0,20,15
        let r8 = (r11 << 8) | b2; // rlwinm r9,r11,8,0,23 ; or
        let r11 = ((r8 as i32) >> 1) as u32; // srawi
        (r11.wrapping_add(6240), b2 & 1, 3)
    } else if b0 < 255 {
        let b1 = u32::from(g.u8(at + 1)?);
        let b2 = u32::from(g.u8(at + 2)?);
        let b3 = u32::from(g.u8(at + 3)?);
        let r8 = (b0 << 16) & 0x0003_0000; // rlwinm r8,r10,16,14,15
        let r11 = (b1 << 8) | b2; // rotlwi r6,r9,8 ; or
        let r9 = b3 & 0xFFFF_FFFE; // rlwinm r9,r5,0,0,30
        let r7 = r11 & 0x0003_FFFF; // clrlwi r7,r11,14
        let r6 = r7 | r8;
        let r5 = r6 << 8; // rlwinm r5,r6,8,0,23
        let r11 = r5 | r9;
        let r9 = ((r11 as i32) >> 1) as u32; // srawi
        (r9.wrapping_add(0x6_0000).wrapping_add(6240), b3 & 1, 4) // addis r11,r9,6 ; addi 6240
    } else {
        // Five bytes: a raw big-endian word, stored without the sign step.
        let v = (u32::from(g.u8(at + 1)?) << 24)
            | (u32::from(g.u8(at + 2)?) << 16)
            | (u32::from(g.u8(at + 3)?) << 8)
            | u32::from(g.u8(at + 4)?);
        return Ok((v, 5));
    };
    if sign != 0 {
        value = 0xFFFF_FFFFu32.wrapping_sub(value); // subfic r11,r11,-1
    }
    Ok((value, len))
}

/// A column record: `+0` the shared cursor cell, `+4` value, `+8` run count, `+12` byte flag.
pub const COLUMN_CURSOR: u32 = 0;
pub const COLUMN_VALUE: u32 = 4;
pub const COLUMN_COUNT: u32 = 8;
pub const COLUMN_FLAG: u32 = 12;

/// `sub_82B46FB0`: read a varint at the cursor, advance the cursor, add it to the column's value.
pub fn column_step(g: &mut Guest, column: u32) -> Result<()> {
    let cell = g.u32(column + COLUMN_CURSOR)?; // lwz r30,0(r3)
    let (value, len) = read_varint(g, g.u32(cell)?)?;
    g.set_u32(cell, g.u32(cell)?.wrapping_add(len))?;
    let sum = g.u32(column + COLUMN_VALUE)?.wrapping_add(value);
    g.set_u32(column + COLUMN_VALUE, sum)
}

/// `sub_82B47010`: the column's next value. A run header `v ≥ 0` holds one delta for `v + 1`
/// rows; `v < 0` reads a fresh delta on each of `1 − v` rows.
pub fn column_next(g: &mut Guest, column: u32) -> Result<u32> {
    if (g.u32(column + COLUMN_COUNT)? as i32) <= 0 {
        let cell = g.u32(column + COLUMN_CURSOR)?;
        let (v, len) = read_varint(g, g.u32(cell)?)?;
        g.set_u32(cell, g.u32(cell)?.wrapping_add(len))?;
        g.set_u32(column + COLUMN_COUNT, v.wrapping_add(1))?;
        g.set_u8(column + COLUMN_FLAG, 1)?;
        if (v as i32) < 0 {
            g.set_u8(column + COLUMN_FLAG, 0)?;
            g.set_u32(column + COLUMN_COUNT, 1u32.wrapping_sub(v))?; // subfic r9,r11,1
        }
        if g.u8(column + COLUMN_FLAG)? != 0 {
            column_step(g, column)?;
        }
    }
    if g.u8(column + COLUMN_FLAG)? == 0 {
        column_step(g, column)?;
    }
    let count = g.u32(column + COLUMN_COUNT)?.wrapping_sub(1);
    g.set_u32(column + COLUMN_COUNT, count)?;
    g.u32(column + COLUMN_VALUE)
}

/// `sub_82B47658`: one value from each of the four columns at `cursor + 4 + 16k` into `out + 4k`.
pub fn first_row(g: &mut Guest, out: u32, cursor: u32) -> Result<()> {
    for k in 0..4 {
        let value = column_next(g, cursor + 4 + 16 * k)?;
        g.set_u32(out + 4 * k, value)?;
    }
    Ok(())
}

/// `sub_82B474B8`: column layout 1. Walk the rows (`col0` byte step, `col1` side step, `col2`
/// samples, `col3` key flag) to the entry holding `target − preroll`, filling `out`. Returns the
/// original's `r3`: 0 when the walk reached `target`, 1 when the table ended first.
pub fn seek_layout1(g: &mut Guest, out: u32, table: u32, target: u32, sp: u32) -> Result<u32> {
    let frame = sp.wrapping_sub(LAYOUT1_FRAME);
    let cursor = frame + 96;
    g.set_u32(cursor, table)?; // stw r4,96(r1)
    for k in 0..4u32 {
        let column = frame + 100 + 16 * k;
        g.set_u32(column + COLUMN_CURSOR, cursor)?;
        g.set_u32(column + COLUMN_VALUE, 0)?;
        g.set_u32(column + COLUMN_COUNT, 0)?;
        g.set_u8(column + COLUMN_FLAG, 0)?;
    }
    let preroll = g.u32(out + 28)?; // lwz r10,28(r3)
    let target = target as i32; // r26
    let r6 = target.wrapping_sub(preroll as i32);
    let floor = r6.max(0); // subfic/addme/and: r6 when positive, else 0 -- r22
    first_row(g, frame + 80, cursor)?;
    let (mut bytes, mut side, mut start) = (0i32, 0i32, 0i32); // r27, r25, r29
    let mut samples = g.u32(frame + 88)? as i32; // r28
    let base = g.u32(out + 4)?; // r21
    if samples < 0 {
        return Ok(1);
    }
    let mut key = g.u32(frame + 92)?; // r3
    let mut side_step = g.u32(frame + 84)?; // r24
    let mut byte_step = g.u32(frame + 80)?; // r23
    loop {
        let covers = start <= floor && floor < samples.wrapping_add(start);
        if covers || key == 1 {
            let side_at = if side_step != 0 {
                (side as u32).wrapping_add(base)
            } else {
                0
            };
            g.set_u32(out + 4, side_at)?;
            let mut used = g.u32(out + 28)? as i32;
            let into = target.wrapping_sub(start);
            g.set_u32(out + 8, start as u32)?;
            if into < used {
                used = into;
            }
            g.set_u32(out + 16, used as u32)?;
            g.set_u32(out + 20, bytes as u32)?;
            g.set_u32(
                out + 12,
                target.wrapping_sub(used).wrapping_sub(start) as u32,
            )?;
            g.set_u8(out + 32, u8::from(key == 1))?; // cntlzw(r3 - 1) >> 5
        }
        start = start.wrapping_add(samples);
        if target < start {
            return Ok(0);
        }
        bytes = bytes.wrapping_add(byte_step as i32);
        side = side.wrapping_add(side_step as i32);
        byte_step = column_next(g, frame + 100)?;
        side_step = column_next(g, frame + 116)?;
        samples = column_next(g, frame + 132)? as i32;
        key = column_next(g, frame + 148)?;
        if samples < 0 {
            return Ok(1);
        }
    }
}

/// `sub_82B470D0`: seek `table` to `target` samples, filling the 36-byte record at `out`. Returns
/// the original's `r3` (0 on success); on failure `+0 +4 +8 +12 +20 +24` are cleared.
pub fn seek(g: &mut Guest, out: u32, table: u32, target: u32, sp: u32) -> Result<u32> {
    let frame = sp.wrapping_sub(128);
    let kind = g.u8(table)? as i8 as i32 as u32; // extsb ; cmplwi
    let status = if kind == 1 {
        return Err(Error::new(
            0x82B4_71D8,
            "seek-table kind 1 (sub_82B471D8) is not used by grains and is not ported",
        ));
    } else if kind > 1 {
        1
    } else {
        let b1 = g.u8(table + 1)?;
        g.set_u32(out, 0)?;
        g.set_u32(out + 24, u32::from(b1 & 0xF))?;
        let layout = b1 >> 4;
        let preroll = g.u16(table + 2)?;
        let offset = g.u32(table + 4)?;
        g.set_u32(out + 28, u32::from(preroll))?;
        g.set_u32(
            out + 4,
            if offset != 0 {
                offset.wrapping_add(table)
            } else {
                0
            },
        )?;
        match layout {
            0 => {
                return Err(Error::new(
                    0x82B4_72C0,
                    "seek-table layout 0 (sub_82B472C0) is not used by grains and is not ported",
                ));
            }
            1 => seek_layout1(g, out, table + 8, target, frame)?,
            _ => 0,
        }
    };
    if status & 0xFF != 0 {
        for k in [0, 4, 8, 12, 20, 24] {
            g.set_u32(out + k, 0)?;
        }
    }
    Ok(status)
}

/// A seek result read back from the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekResult {
    pub status: u32,
    pub side: u32,
    pub entry_start: u32,
    pub skip: u32,
    pub preroll_used: u32,
    pub entry_byte: u32,
    pub low: u32,
    pub key: bool,
}

impl SeekResult {
    pub fn read(g: &Guest, out: u32, status: u32) -> Result<Self> {
        Ok(Self {
            status,
            side: g.u32(out + 4)?,
            entry_start: g.u32(out + 8)?,
            skip: g.u32(out + 12)?,
            preroll_used: g.u32(out + 16)?,
            entry_byte: g.u32(out + 20)?,
            low: g.u32(out + 24)?,
            key: g.u8(out + 32)? != 0,
        })
    }

    /// The first sample the decoder hands on: entry start, then the preroll it decodes and
    /// discards, then the skip.
    pub fn output_frame(&self) -> u32 {
        self.entry_start
            .wrapping_add(self.preroll_used)
            .wrapping_add(self.skip)
    }
}

/// `sub_82B32DC8`'s offset: `fctiwz(slot rate × start seconds)` (a double multiply of the single
/// rate), passed on only when positive, the record kind is not 2 and the slot does not loop
/// (`slot + 24` negative). Otherwise 0, which sends `sub_82B33780` down its no-seek branch.
pub fn start_frame(rate: f32, start_seconds: f64, kind: u8, loop_start: u32) -> i32 {
    let limit = fp::fctiwz_low_word(f64::from(rate) * start_seconds) as i32;
    if limit > 0 && kind != 2 && (loop_start as i32) < 0 {
        limit
    } else {
        0
    }
}

/// `sub_82B33780`'s stores for one slot: the seek branch when `frame > 0` and `detail != 0`, the
/// cleared branch otherwise. `slot` and `record` are the 48- and 80-byte ring entries; `sp` is the
/// caller's stack pointer. Returns the seek result on the seek branch.
///
/// The concrete resident-PCM host does not apply these stores (its decoded cursor stands in for
/// the XMA context the original re-arms with them); they are ported so the mapping is on record.
pub fn apply_seek_fields(
    g: &mut Guest,
    slot: u32,
    record: u32,
    detail: u32,
    frame: i32,
    sp: u32,
) -> Result<Option<SeekResult>> {
    if frame > 0 && detail != 0 {
        let out = sp.wrapping_sub(DECODE_FRAME) + 80;
        let status = seek(g, out, detail, frame as u32, sp.wrapping_sub(DECODE_FRAME))?;
        let result = SeekResult::read(g, out, status)?;
        g.set_u32(slot + 36, g.u32(out + 8)?)?;
        g.set_u32(slot + 32, g.u32(out + 12)?)?;
        g.set_u32(record + 60, g.u32(out + 16)?)?;
        g.set_u32(record + 64, g.u32(out + 20)?)?;
        g.set_u32(record + 56, g.u32(out + 4)?)?;
        g.set_u32(record + 68, g.u32(out + 24)?)?;
        g.set_u8(record + 77, g.u8(out + 32)?)?;
        g.set_u32(slot + 28, 0)?;
        g.set_u32(record + 20, g.u32(slot + 36)?)?;
        Ok(Some(result))
    } else {
        g.set_u32(slot + 32, 0)?;
        g.set_u32(record + 60, 0)?;
        g.set_u32(record + 64, 0)?;
        g.set_u32(record + 56, 0)?;
        g.set_u8(record + 77, 1)?;
        g.set_u32(slot + 28, 0)?;
        g.set_u32(slot + 36, 0)?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const TABLE: u32 = BASE + 0x100;
    const OUT: u32 = BASE + 0x800;
    const SP: u32 = BASE + 0x2000;

    fn guest(bytes: &[u8]) -> Guest {
        let mut g = Guest::single(BASE, 0x3000);
        g.set_span(TABLE, bytes).unwrap();
        g
    }

    #[test]
    fn varint_forms() {
        // One byte: 0x10 -> 8, sign clear; 0x13 -> -1 - 9.
        let g = guest(&[0x10, 0x13]);
        assert_eq!(read_varint(&g, TABLE).unwrap(), (8, 1));
        assert_eq!(read_varint(&g, TABLE + 1).unwrap(), ((-10i32) as u32, 1));
        // Two bytes: 0xC0 0x02 -> ((0xC002 >> 1) & ~0x6000) + 96 = 1 + 96.
        let g = guest(&[0xC0, 0x02]);
        assert_eq!(read_varint(&g, TABLE).unwrap(), (97, 2));
        // Three bytes: 0xF0 0x00 0x04 -> ((0x00 << 8 | 4) >> 1) + 6240.
        let g = guest(&[0xF0, 0x00, 0x04]);
        assert_eq!(read_varint(&g, TABLE).unwrap(), (6242, 3));
        // Five bytes: a raw word.
        let g = guest(&[0xFF, 0x12, 0x34, 0x56, 0x78]);
        assert_eq!(read_varint(&g, TABLE).unwrap(), (0x1234_5678, 5));
    }

    /// The first 136 table bytes of `concrete_rough_hard.grain` (member offset 8..0xB0) walked to
    /// every frame: the entry start, the preroll and the skip always add back up to the target, the
    /// entry grows monotonically, and a target past the last entry fails.
    #[test]
    fn real_table_round_trips_every_target() {
        let path = std::env::var_os("SKATE_GRAINS_BIG").map(std::path::PathBuf::from);
        let Some(path) = path.filter(|p| p.is_file()) else {
            eprintln!("SKATE_GRAINS_BIG not set; skipping the retail table walk");
            return;
        };
        let data = std::fs::read(path).unwrap();
        let members = skate_audio_formats::grain::members(&data).unwrap();
        for (name, range) in members {
            let bytes = &data[range];
            let grain = skate_audio_formats::grain::Grain::parse(bytes).unwrap();
            let table = grain.seek_table();
            let mut g = guest(table);
            let frames = grain.stream.num_samples;
            let mut last = 0;
            for target in (1..frames).step_by(997) {
                let status = seek(&mut g, OUT, TABLE, target, SP).unwrap();
                assert_eq!(status, 0, "{name} @ {target}");
                let r = SeekResult::read(&g, OUT, status).unwrap();
                assert_eq!(r.output_frame(), target, "{name} @ {target}");
                assert!(r.entry_start >= last, "{name}: entries go backwards");
                assert!(r.preroll_used <= 384);
                last = r.entry_start;
            }
        }
    }
}
