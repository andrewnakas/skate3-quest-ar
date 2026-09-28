//! EA "EB" v3 archives — the package format holding Skate 3's audio.
//!
//! Big-endian, with a hash-sorted entry table and plaintext names stored in entry order:
//!
//! ```text
//! 0x00  "EB" | u16 version (3)
//! 0x04  u32 entry_count
//! 0x08  u32 flags          byte at 0x0A is the offset shift (4 or 6)
//! 0x0C  u32 name_table_offset
//! 0x10  u32 name_table_size
//! 0x1C  u32 total_size     exact byte length of the archive
//! 0x30  entry[entry_count], 16 bytes each:
//!         +0  u32 offset >> shift
//!         +4  u32 compressed_size    0 = stored uncompressed
//!         +8  u32 uncompressed_size
//!         +12 u32 name_hash          table is sorted ascending by this
//! ```
//!
//! Entries with a non-zero compressed size are EA `chunkref` blocks. No audio archive
//! on the disc uses compression, so decompression is not implemented here.

use crate::{Error, Result, be16, be32};

pub const MAGIC: &[u8; 2] = b"EB";
pub const VERSION: u16 = 3;

const HEADER_SIZE: usize = 0x30;
const ENTRY_SIZE: usize = 16;

/// One archive member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub offset: u64,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub name_hash: u32,
    pub name: Option<String>,
}

impl Entry {
    /// True when the member is an EA `chunkref` block rather than stored bytes.
    pub fn is_compressed(&self) -> bool {
        self.compressed_size != 0
    }

    /// Byte range of the member within the archive.
    pub fn range(&self) -> std::ops::Range<usize> {
        let start = self.offset as usize;
        let len = if self.is_compressed() {
            self.compressed_size
        } else {
            self.uncompressed_size
        };
        start..start + len as usize
    }
}

/// A parsed archive directory. Holds no file data.
#[derive(Clone, Debug)]
pub struct Archive {
    pub entries: Vec<Entry>,
    pub total_size: u32,
    pub offset_shift: u8,
}

impl Archive {
    /// Parse the header and entry table. `data` must cover at least the directory;
    /// the whole file is required only to read names.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(0..2) != Some(&MAGIC[..]) {
            return Err(Error::new(0, "not an EB archive"));
        }
        let version = be16(data, 2)?;
        if version != VERSION {
            return Err(Error::new(2, format!("unsupported EB version {version}")));
        }
        let count = be32(data, 4)? as usize;
        let flags = be32(data, 8)?;
        let offset_shift = ((flags >> 8) & 0xFF) as u8;
        if offset_shift != 4 && offset_shift != 6 {
            return Err(Error::new(
                0x0A,
                format!("unexpected offset shift {offset_shift}"),
            ));
        }
        let name_table_offset = be32(data, 0x0C)? as usize;
        let total_size = be32(data, 0x1C)?;

        let table_end = HEADER_SIZE + count * ENTRY_SIZE;
        if table_end > data.len() {
            return Err(Error::new(
                HEADER_SIZE,
                "entry table runs past end of buffer",
            ));
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let at = HEADER_SIZE + i * ENTRY_SIZE;
            entries.push(Entry {
                offset: u64::from(be32(data, at)?) << offset_shift,
                compressed_size: be32(data, at + 4)?,
                uncompressed_size: be32(data, at + 8)?,
                name_hash: be32(data, at + 12)?,
                name: None,
            });
        }

        let mut archive = Self {
            entries,
            total_size,
            offset_shift,
        };
        archive.read_names(data, name_table_offset);
        Ok(archive)
    }

    /// Names are NUL-terminated and stored in entry order behind a 1-2 byte tag, so
    /// they are matched positionally rather than by hash. Absent or truncated names
    /// leave `Entry::name` as `None` rather than failing the parse.
    fn read_names(&mut self, data: &[u8], mut at: usize) {
        for entry in &mut self.entries {
            while at < data.len() && (data[at] < 0x20 || data[at] > 0x7E) {
                at += 1;
            }
            let start = at;
            while at < data.len() && data[at] != 0 {
                at += 1;
            }
            if at > start && at <= data.len() {
                entry.name = std::str::from_utf8(&data[start..at])
                    .ok()
                    .map(str::to_owned);
            }
        }
    }

    /// Check every member lies inside the archive. Returns the first violation.
    pub fn validate(&self) -> Result<()> {
        for entry in &self.entries {
            let end = entry.range().end;
            if end > self.total_size as usize {
                return Err(Error::new(
                    entry.offset as usize,
                    format!(
                        "member ends at {end} but archive is {} bytes",
                        self.total_size
                    ),
                ));
            }
        }
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.name.as_deref() == Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(count: u32, shift: u8) -> Vec<u8> {
        let mut v = vec![0u8; HEADER_SIZE + count as usize * ENTRY_SIZE + 64];
        v[0..2].copy_from_slice(MAGIC);
        v[2..4].copy_from_slice(&VERSION.to_be_bytes());
        v[4..8].copy_from_slice(&count.to_be_bytes());
        v[8..12].copy_from_slice(&((shift as u32) << 8).to_be_bytes());
        let names_at = (HEADER_SIZE + count as usize * ENTRY_SIZE) as u32;
        v[0x0C..0x10].copy_from_slice(&names_at.to_be_bytes());
        let total = v.len() as u32;
        v[0x1C..0x20].copy_from_slice(&total.to_be_bytes());
        v
    }

    #[test]
    fn rejects_a_foreign_magic() {
        assert!(Archive::parse(b"BIG4....").is_err());
    }

    #[test]
    fn rejects_an_unexpected_offset_shift() {
        let v = build(0, 5);
        assert!(Archive::parse(&v).is_err());
    }

    #[test]
    fn applies_the_offset_shift() {
        let mut v = build(1, 4);
        v[HEADER_SIZE..HEADER_SIZE + 4].copy_from_slice(&0x10u32.to_be_bytes());
        v[HEADER_SIZE + 8..HEADER_SIZE + 12].copy_from_slice(&32u32.to_be_bytes());
        let a = Archive::parse(&v).unwrap();
        assert_eq!(a.entries[0].offset, 0x100);
        assert!(!a.entries[0].is_compressed());
    }

    #[test]
    fn reads_names_positionally() {
        let mut v = build(2, 4);
        let names_at = HEADER_SIZE + 2 * ENTRY_SIZE;
        v[names_at..names_at + 10].copy_from_slice(b"\x01first\0\0\0\0");
        v[names_at + 10..names_at + 17].copy_from_slice(b"second\0");
        let a = Archive::parse(&v).unwrap();
        assert_eq!(a.entries[0].name.as_deref(), Some("first"));
        assert_eq!(a.entries[1].name.as_deref(), Some("second"));
        assert!(a.find("second").is_some());
    }

    #[test]
    fn validate_catches_a_member_past_the_end() {
        let mut v = build(1, 4);
        v[HEADER_SIZE..HEADER_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
        v[HEADER_SIZE + 8..HEADER_SIZE + 12].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        let a = Archive::parse(&v).unwrap();
        assert!(a.validate().is_err());
    }
}
