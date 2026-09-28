//! EA `.mus` interactive-music streams.
//!
//! Structurally distinct from the block-chained `.sns` / `.dat` payloads: the length
//! field lives in a **12-byte block header** rather than inline before each chunk, and
//! blocks are grouped into segments that a companion `.mpf` sequences at runtime.
//!
//! ```text
//! block   [u32 flags<<24 | size]   size COUNTS the 12-byte header
//!         [u32 num_samples]
//!         [u32 length_field]       same encoding as eaac::chunk_length
//!         [body ...]
//! ```
//!
//! Flag bit `0x80` marks the last block of a segment. That block is padded out to an
//! alignment boundary, so its body is longer than its declared length.
//!
//! Note the file's segment count at offset 4 is **little-endian** while everything else
//! in the format is big-endian.

use crate::{Error, Result, be32, eaac};

/// Byte size of a block header.
pub const HEADER_SIZE: usize = 12;
/// Flag in the top byte of the size word marking a segment's final block.
pub const FLAG_SEGMENT_END: u32 = 0x80;
/// Segments begin on this boundary. See [`next_segment_start`].
pub const SEGMENT_ALIGN: usize = 0x80;

/// The `.mus` file header.
///
/// Fields established by walking all three `.mus` files on the disc end to end; the
/// slots not named here are carried raw rather than guessed at.
///
/// ```text
/// 0x04  u32 LE   segment_count          the format's one little-endian field
/// 0x28  u32 be   unidentified           distinct per file; NOT the `.mpf` link (checked)
/// 0x30  u32 be   snr_table_offset >> 4
/// 0x34  u32 be   first_block_offset >> 7
/// 0x38  u32 be   8 on all three files
/// ```
///
/// Before this, the SNR table offset and the segment count were passed in by hand and
/// the verified claim was "8 segments". Eight is the constant at `0x38`, not a count:
/// the real totals are 1074, 5725 and 1380.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Number of segments, and of records in the SNR table.
    pub segment_count: u32,
    /// Byte offset of the SNR table: one 16-byte [`crate::eaac::Header`] per segment.
    pub snr_table_offset: usize,
    /// Byte offset of the first segment's first block header.
    pub first_block_offset: usize,
    /// `0x28`, unidentified.
    pub unknown_28: u32,
    /// `0x38`, 8 on every file measured.
    pub unknown_38: u32,
}

impl Header {
    pub const SIZE: usize = 0x40;
    /// Stride of one SNR table record.
    pub const SNR_RECORD_SIZE: usize = 16;

    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            return Err(Error::new(
                0,
                format!("`.mus` is {} bytes, want {}", data.len(), Self::SIZE),
            ));
        }
        let segment_count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let header = Self {
            segment_count,
            snr_table_offset: (be32(data, 0x30)? as usize) << 4,
            first_block_offset: (be32(data, 0x34)? as usize) << 7,
            unknown_28: be32(data, 0x28)?,
            unknown_38: be32(data, 0x38)?,
        };
        if header.segment_count == 0 {
            return Err(Error::new(4, "zero segments"));
        }
        let table_end = header.snr_table_offset + Self::SNR_RECORD_SIZE * segment_count as usize;
        if table_end > data.len() {
            return Err(Error::new(
                0x30,
                format!("SNR table ends at {table_end}, past the file"),
            ));
        }
        if header.first_block_offset + HEADER_SIZE > data.len() {
            return Err(Error::new(0x34, "first block offset is past the file"));
        }
        Ok(header)
    }

    /// The stream header for segment `i`, read from the SNR table.
    pub fn snr(&self, data: &[u8], i: usize) -> Result<eaac::Header> {
        if i >= self.segment_count as usize {
            return Err(Error::new(
                0,
                format!("segment {i} of {}", self.segment_count),
            ));
        }
        eaac::Header::parse(data, self.snr_table_offset + Self::SNR_RECORD_SIZE * i)
    }
}

/// Walk every segment in a `.mus` file.
///
/// Each segment's sample count is checked against the SNR table, which states it
/// independently of the block headers the walk sums -- so agreement is a real
/// cross-check, not a self-consistency one.
pub fn segments(data: &[u8]) -> Result<Vec<Segment>> {
    let header = Header::parse(data)?;
    let mut out = Vec::with_capacity(header.segment_count as usize);
    let mut at = header.first_block_offset;
    for i in 0..header.segment_count as usize {
        let seg = segment(data, at)?;
        let declared = header.snr(data, i)?.num_samples;
        if u64::from(declared) != seg.num_samples() {
            return Err(Error::new(
                seg.blocks[0].offset,
                format!(
                    "segment {i}: blocks sum to {} but the SNR table says {declared}",
                    seg.num_samples()
                ),
            ));
        }
        let end = seg.end();
        out.push(seg);
        match next_segment_start(data, end) {
            Some(n) => at = n,
            None if i + 1 == header.segment_count as usize => break,
            None => {
                return Err(Error::new(
                    end,
                    format!("file ends after segment {i} of {}", header.segment_count),
                ));
            }
        }
    }
    Ok(out)
}

/// One block of a music segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub offset: usize,
    /// Stride to the next block, including this header.
    pub size: u32,
    pub flags: u8,
    pub num_samples: u32,
    /// Declared payload length; never larger than `size - HEADER_SIZE`.
    pub data_len: u32,
}

impl Block {
    pub fn is_segment_end(&self) -> bool {
        u32::from(self.flags) & FLAG_SEGMENT_END != 0
    }

    /// Byte range of the decodable payload, excluding any alignment padding.
    pub fn data_range(&self) -> std::ops::Range<usize> {
        let start = self.offset + HEADER_SIZE;
        start..start + self.data_len as usize
    }

    pub fn parse(data: &[u8], at: usize) -> Result<Self> {
        let word = be32(data, at)?;
        let size = word & 0x00FF_FFFF;
        if (size as usize) <= HEADER_SIZE {
            return Err(Error::new(
                at,
                format!("block size {size} does not advance"),
            ));
        }
        if at + size as usize > data.len() {
            return Err(Error::new(at, format!("block size {size} runs past end")));
        }
        let field = be32(data, at + 8)?;
        let data_len = eaac::chunk_length(field)
            .ok_or_else(|| Error::new(at + 8, format!("field {field} is not a valid length")))?;
        let body = size - HEADER_SIZE as u32;
        if data_len > body {
            return Err(Error::new(
                at + 8,
                format!("declared length {data_len} exceeds body {body}"),
            ));
        }
        Ok(Self {
            offset: at,
            size,
            flags: (word >> 24) as u8,
            num_samples: be32(data, at + 4)?,
            data_len,
        })
    }
}

/// A run of blocks ending at a `FLAG_SEGMENT_END`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub blocks: Vec<Block>,
}

impl Segment {
    pub fn num_samples(&self) -> u64 {
        self.blocks.iter().map(|b| u64::from(b.num_samples)).sum()
    }

    /// Offset just past the final block, before any inter-segment padding.
    pub fn end(&self) -> usize {
        self.blocks.last().map_or(0, |b| b.offset + b.size as usize)
    }
}

/// Read one segment starting at `at`.
pub fn segment(data: &[u8], at: usize) -> Result<Segment> {
    let mut blocks = Vec::new();
    let mut pos = at;
    loop {
        let block = Block::parse(data, pos)?;
        pos += block.size as usize;
        let last = block.is_segment_end();
        blocks.push(block);
        if last {
            return Ok(Segment { blocks });
        }
        if blocks.len() > 4096 {
            return Err(Error::new(at, "no segment-end flag within 4096 blocks"));
        }
    }
}

/// Find the next segment's first block header, given the previous segment's end.
///
/// Segments are separated by padding of varying width, so this was originally a scan:
/// step forward four bytes at a time until a block header parses. That worked on
/// `Game_Stream.mus` and `World_Stream.mus` by luck -- their segments happen to end
/// 4-aligned -- and failed on `Ipod_Stream.mus` after the very first segment, whose
/// ends land on every residue mod 4. Anchored at an unaligned offset and stepping by
/// four, the scan can only ever visit offsets congruent to `from`, so it never saw a
/// real header again.
///
/// There is no need to search. **Segments begin on a [`SEGMENT_ALIGN`] boundary**, so
/// the next one is computed. Measured over all 8,179 segments of the three `.mus`
/// files on the disc: every transition is exactly this rounding, and walking with it
/// consumes `Game_Stream` and `World_Stream` to the last byte.
pub fn next_segment_start(data: &[u8], from: usize) -> Option<usize> {
    let at = (from + SEGMENT_ALIGN - 1) & !(SEGMENT_ALIGN - 1);
    (at + HEADER_SIZE <= data.len()).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(size: u32, samples: u32, len: u32, flags: u8) -> Vec<u8> {
        let mut v = ((u32::from(flags) << 24) | size).to_be_bytes().to_vec();
        v.extend_from_slice(&samples.to_be_bytes());
        v.extend_from_slice(&(4 * len + eaac::LENGTH_BIAS).to_be_bytes());
        v.resize(size as usize, 0);
        v
    }

    #[test]
    fn parses_a_block_header() {
        let raw = block(2043, 4736, 2031, 0);
        let b = Block::parse(&raw, 0).unwrap();
        assert_eq!(b.size, 2043);
        assert_eq!(b.num_samples, 4736);
        assert_eq!(b.data_len, 2031);
        assert!(!b.is_segment_end());
        assert_eq!(b.data_range(), 12..2043);
    }

    #[test]
    fn honours_the_segment_end_flag() {
        let raw = block(1971, 4384, 1911, 0x80);
        let b = Block::parse(&raw, 0).unwrap();
        assert!(b.is_segment_end());
        // the final block is padded: body is 1959 but only 1911 bytes are data
        assert_eq!(b.data_len, 1911);
        assert_eq!(b.size as usize - HEADER_SIZE, 1959);
    }

    #[test]
    fn rejects_a_length_longer_than_the_body() {
        let raw = block(100, 512, 4000, 0);
        assert!(Block::parse(&raw, 0).is_err());
    }

    /// Build a minimal 0x40-byte file header.
    fn file_header(count: u32, snr: usize, first: usize) -> Vec<u8> {
        let mut v = vec![0u8; Header::SIZE];
        v[4..8].copy_from_slice(&count.to_le_bytes());
        v[0x30..0x34].copy_from_slice(&((snr >> 4) as u32).to_be_bytes());
        v[0x34..0x38].copy_from_slice(&((first >> 7) as u32).to_be_bytes());
        v[0x38..0x3C].copy_from_slice(&8u32.to_be_bytes());
        v
    }

    #[test]
    fn parses_the_file_header() {
        // Game_Stream.mus's actual header values.
        let mut v = file_header(1074, 0x7600, 0xB980);
        v[0x28..0x2C].copy_from_slice(&0x455E_FC90u32.to_be_bytes());
        v.resize(0x20000, 0);
        let h = Header::parse(&v).unwrap();
        assert_eq!(h.segment_count, 1074);
        assert_eq!(h.snr_table_offset, 0x7600);
        assert_eq!(h.first_block_offset, 0xB980);
        assert_eq!(h.unknown_28, 0x455E_FC90);
        assert_eq!(h.unknown_38, 8);
    }

    #[test]
    fn rejects_a_snr_table_that_runs_past_the_file() {
        let mut v = file_header(1074, 0x7600, 0xB980);
        v.resize(0x8000, 0); // table needs 0x7600 + 1074*16 = 0xB920
        assert!(Header::parse(&v).is_err());
    }

    /// Regression: the old scan stepped four bytes at a time from the previous
    /// segment's end, so from an unaligned anchor it could only ever visit offsets
    /// congruent to that anchor mod 4 -- and never found another header.
    /// `Ipod_Stream.mus`'s segment ends land on every residue; these are its first
    /// four, with the boundaries the walk must produce.
    #[test]
    fn next_segment_start_rounds_an_unaligned_end_up_to_the_boundary() {
        let data = vec![0u8; 0x200000];
        // Every one of these is unaligned, so the old scan could not reach the header.
        for (end, want) in [
            (0x3DCBDusize, 0x3DD00usize),
            (0x9726F, 0x97280),
            (0xC47D2, 0xC4800),
            (0xF0322, 0xF0380),
            (0x11A5F1, 0x11A600),
        ] {
            assert_ne!(end % 4, 0, "{end:#x} must be unaligned for this to bite");
            assert_eq!(next_segment_start(&data, end), Some(want), "end {end:#x}");
        }
        // 0x6B404 is the counter-case: 4-aligned, so the old scan handled it. The
        // rounding must still be right.
        assert_eq!(next_segment_start(&data, 0x6B404), Some(0x6B480));
    }

    #[test]
    fn next_segment_start_leaves_an_aligned_end_alone() {
        let data = vec![0u8; 0x1000];
        assert_eq!(next_segment_start(&data, 0x80), Some(0x80));
        assert_eq!(next_segment_start(&data, 0x81), Some(0x100));
    }

    #[test]
    fn next_segment_start_stops_at_the_end_of_the_file() {
        let data = vec![0u8; 0x100];
        assert_eq!(next_segment_start(&data, 0x81), None);
    }

    #[test]
    fn walks_a_segment_to_its_flag() {
        let mut raw = block(60, 512, 40, 0);
        raw.extend(block(60, 512, 40, 0));
        raw.extend(block(60, 256, 40, 0x80));
        let seg = segment(&raw, 0).unwrap();
        assert_eq!(seg.blocks.len(), 3);
        assert_eq!(seg.num_samples(), 1280);
        assert_eq!(seg.end(), 180);
    }
}
