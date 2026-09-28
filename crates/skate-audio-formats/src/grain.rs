//! `.grain` members of `grains.big`: the recordings behind retail's granular board-rolling sound.
//!
//! Recovered from the retail binder `sub_828EC040` (which reads the file in place, exactly as the
//! loader `sub_824C5BF8` → `sub_828DC158` leaves it in memory: the member's bytes, unmodified) and
//! from the seek reader the SndPlayer1 play command reaches through it (`sub_82B32DC8` →
//! `sub_82B33780` → `sub_82B470D0`). Big-endian:
//!
//! ```text
//! +0   u32  header length H          player+52 = [data]; player+64 = data + H (the EAAC stream)
//! +4   f32  duration in seconds       player+56
//! +8   seek table (H - 8 bytes)       player+60 = data + 8; passed as the play command's detail
//! +H   EA Audio Core stream           header + one block (XMA, mono, 44.1 or 48 kHz)
//! ```
//!
//! The seek table, as `sub_82B470D0` reads it:
//!
//! ```text
//! +0   u8   kind          0 = run-length columns (the only kind in grains.big); 1 = sub_82B471D8;
//!                         anything else fails the seek
//! +1   u8   low nibble -> out+24; high nibble = column layout (1 = sub_82B474B8, the only one here;
//!                         0 = sub_82B472C0)
//! +2   u16  preroll samples (384 in every grains.big member): how far before the target the
//!           decoder restarts, clamped at the entry start
//! +4   u32  byte offset of the per-entry side data from the table start (0 = none)
//! +8   the column stream: four interleaved run-length columns of signed varints, read row by
//!      row -- col0 = EAAC byte step, col1 = side-data byte step, col2 = samples in the entry,
//!      col3 = 1 on a key entry. A row whose col2 is negative ends the table.
//! ```
//!
//! In every grain the first row is `{block bytes, side length, num_samples, 1}` -- one entry
//! spanning the whole single-block stream -- so the reader stops in row 0 for any in-range target
//! and the seek resolves to "restart at sample 0, decode `preroll` samples, skip `target −
//! preroll`" (the side data at `+24`, about one byte per 2048-byte XMA packet, is what the hardware
//! decoder uses to find the packet). The rows after it are never reached.
//!
//! The varint and column decoders operate on guest memory in `skate_audio_core::grain::seek`,
//! where they are transliterated from the lifted functions; [`SeekHeader`] here only names the
//! eight header bytes so a loader can validate a member before placing it.
//!
//! Every member of the retail `grains.big` was measured (2026-09-18, all 14): the header length is
//! 112..176 and lands on a codec-3 (XMA) mono EAAC header at 44.1 or 48 kHz whose one block spans
//! the rest of the member; the seek header is `00 10 0180 00000018` in all of them; and the
//! duration float is `num_samples / rate` rounded to single in 11 members and 1 to 3 ulps off it
//! in `concrete_aggregate_soft`, `asphalt_rough_soft` and `concrete_aggregate_hard` -- so the
//! stored float, which is what the player reads, is kept rather than recomputed.

use crate::{Error, Result, be16, be32, eaac, eb};

/// The eight bytes `sub_82B470D0` reads before the column stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekHeader {
    /// `lbz r11,0(r4)` -- 0 for every grain.
    pub kind: u8,
    /// `clrlwi r8,r9,28` -- the low nibble of byte 1, stored at the seek result's `+24`.
    pub low: u8,
    /// `rlwinm r9,r9,28,4,31` -- the high nibble of byte 1, the column layout.
    pub layout: u8,
    /// Bytes 2..3, the result's `+28`.
    pub preroll: u16,
    /// Bytes 4..7, relative to the table; 0 when absent.
    pub side_offset: u32,
}

/// One parsed `.grain` member. Offsets are relative to the member's first byte.
#[derive(Clone, Debug)]
pub struct Grain<'a> {
    pub bytes: &'a [u8],
    pub header_len: u32,
    pub duration: f32,
    pub seek: SeekHeader,
    pub stream: eaac::Header,
}

impl<'a> Grain<'a> {
    /// Parse a member's bytes exactly as the binder addresses them.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let header_len = be32(bytes, 0)?;
        let duration = f32::from_bits(be32(bytes, 4)?);
        let h = header_len as usize;
        if h < 16 || h > bytes.len() {
            return Err(Error::new(
                0,
                format!("grain header length {h} is out of range"),
            ));
        }
        let byte1 = *bytes
            .get(9)
            .ok_or_else(|| Error::new(9, "truncated seek table"))?;
        let seek = SeekHeader {
            kind: bytes[8],
            low: byte1 & 0xF,
            layout: byte1 >> 4,
            preroll: be16(bytes, 10)?,
            side_offset: be32(bytes, 12)?,
        };
        let stream = eaac::Header::parse(bytes, h)?;
        Ok(Self {
            bytes,
            header_len,
            duration,
            seek,
            stream,
        })
    }

    /// The EAAC stream (header at offset 0), what `ffmpeg::decode_resident` takes.
    pub fn stream_bytes(&self) -> &'a [u8] {
        &self.bytes[self.header_len as usize..]
    }

    /// The seek table bytes, `data + 8 .. data + H`.
    pub fn seek_table(&self) -> &'a [u8] {
        &self.bytes[8..self.header_len as usize]
    }
}

/// The named `.grain` members of an EB archive, in entry order.
pub fn members(archive: &[u8]) -> Result<Vec<(String, std::ops::Range<usize>)>> {
    let parsed = eb::Archive::parse(archive)?;
    let mut out = Vec::new();
    for entry in &parsed.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".grain") {
            continue;
        }
        if entry.is_compressed() {
            return Err(Error::new(
                entry.offset as usize,
                format!("{name}: compressed grain members are unsupported"),
            ));
        }
        let range = entry.range();
        if range.end > archive.len() {
            return Err(Error::new(
                range.start,
                format!("{name}: member out of bounds"),
            ));
        }
        out.push((name.to_owned(), range));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first 40 bytes of `concrete_rough_hard.grain`, verbatim, and the EAAC header at its
    /// header length (0xB0), spliced after padding so the parse walks the real offsets.
    fn concrete_rough_hard_head() -> Vec<u8> {
        let mut bytes = vec![0u8; 0xB0 + 16];
        let head = [
            0x00, 0x00, 0x00, 0xB0, 0x41, 0xAE, 0xBC, 0x39, // H = 176, 21.8418 s
            0x00, 0x10, 0x01, 0x80, 0x00, 0x00, 0x00,
            0x18, // kind 0, layout 1, preroll 384, +24
        ];
        bytes[..16].copy_from_slice(&head);
        // 0300AC44 000EB29C 0003F727 000EB29C: codec 3, mono, 44.1 kHz, 963228 samples, one block.
        bytes[0xB0..0xB0 + 16].copy_from_slice(&[
            0x03, 0x00, 0xAC, 0x44, 0x00, 0x0E, 0xB2, 0x9C, 0x00, 0x03, 0xF7, 0x27, 0x00, 0x0E,
            0xB2, 0x9C,
        ]);
        bytes
    }

    #[test]
    fn parses_the_binder_fields() {
        let bytes = concrete_rough_hard_head();
        let grain = Grain::parse(&bytes).unwrap();
        assert_eq!(grain.header_len, 0xB0);
        assert_eq!(grain.duration.to_bits(), 0x41AE_BC39);
        assert_eq!(
            grain.seek,
            SeekHeader {
                kind: 0,
                low: 0,
                layout: 1,
                preroll: 384,
                side_offset: 24,
            }
        );
        assert_eq!(grain.stream.sample_rate, 44_100);
        assert_eq!(grain.stream.num_samples, 963_228);
        assert_eq!(grain.stream.channels(), 1);
        // For this member the duration word is num_samples / rate rounded to single.
        assert_eq!(
            ((963_228f64 / 44_100.0) as f32).to_bits(),
            grain.duration.to_bits()
        );
    }

    #[test]
    fn rejects_a_header_length_past_the_member() {
        let mut bytes = concrete_rough_hard_head();
        bytes[3] = 0xFF;
        bytes[2] = 0x10;
        assert!(Grain::parse(&bytes).is_err());
    }
}
