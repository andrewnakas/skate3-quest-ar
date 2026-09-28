//! Skate 3 audio container parsing: EA "EB" archives and EA Audio Core stream headers.
//!
//! Source: the retail Xbox 360 disc, cross-checked against the decompiled loader.
//! Layouts are documented in `docs/rw-audio-core.md`. Everything here is big-endian.
//!
//! This module owns bytes and bounds checks; codec arithmetic belongs elsewhere.
#![forbid(unsafe_code)]

pub mod banks;
pub mod eaac;
pub mod eb;
pub mod grain;
pub mod hash;
pub mod mpf;
pub mod mus;
pub mod splc;

/// A parse failure, with the byte offset it was detected at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub message: String,
}

impl Error {
    pub fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at {:#x}: {}", self.offset, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Read a big-endian u32, bounds-checked.
pub(crate) fn be32(data: &[u8], at: usize) -> Result<u32> {
    data.get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error::new(at, "truncated: wanted 4 bytes"))
}

/// Read a big-endian u16, bounds-checked.
pub(crate) fn be16(data: &[u8], at: usize) -> Result<u16> {
    data.get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| Error::new(at, "truncated: wanted 2 bytes"))
}
