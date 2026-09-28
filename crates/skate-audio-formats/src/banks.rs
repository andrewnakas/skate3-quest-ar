//! The sound banks in `audiofiles.big`: `.abk`, `.csi`, `.ems` and `.bnk`.
//!
//! These are the layer above the stream containers. Together they are how a game event
//! reaches a sound, and the parts of that chain this module can prove are:
//!
//! * an [`Ems`] record places one ambient emitter in the world and names the sound it
//!   plays with a 64-bit [`crate::hash::name_id`];
//! * an [`Abk`] bank carries the samples plus an export table of **named ports** --
//!   `..._msg` inputs, `..._snd` outputs, `..._vol` parameters -- each identified by a
//!   `(project_id, name_id)` pair rather than by the string;
//! * a [`Csi`] project is the symbol table those pairs resolve against.
//!
//! Everything here is big-endian. Field names say what was checked; slots whose meaning
//! is not established are carried raw as `unknown_*` rather than guessed at, because a
//! plausible name for an unverified field is exactly the kind of claim this project has
//! had to retract before.
//!
//! What is **not** decoded: how an `.abk`'s ports are wired to its samples (the region
//! between the header and the sample bank), the `.bnk` per-sample parameter block, and
//! the `.csi`'s own graph -- only its symbol table is read here.

use crate::{Error, Result, be16, be32};

// ---------------------------------------------------------------------------
// .abk -- "ABKC", a patch bank
// ---------------------------------------------------------------------------

/// `.abk` magic.
pub const ABK_MAGIC: &[u8; 4] = b"ABKC";
/// Magic of the sample-bank section inside an `.abk`.
pub const SAMPLE_BANK_MAGIC: &[u8; 4] = b"S10A";

/// One named port exported by an [`Abk`].
///
/// The name is stored inline in the bank, but the engine addresses it by the
/// `(project_id, name_id)` pair: `project_id` selects a [`Csi`] project and `name_id`
/// is that project's id for the name. Checked on real data: no name in
/// `audiofiles.big` ever carries two different `name_id`s, and every pair whose
/// project ships a `.csi` in the same archive resolves there to the same string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    /// Offset into the bank of the record this export points at.
    pub record_offset: usize,
    /// Id of the authoring project, matching [`Csi::project_id`].
    pub project_id: u16,
    /// The project's id for this name.
    pub name_id: u16,
    pub name: String,
    /// `+0x00` of the export entry. An offset into the bank's first section.
    pub target: u32,
    /// `+0x08` of the export entry. Only 13 distinct values across all 376 banks; its
    /// top byte correlates with the name's role (0 for `_snd`, 2 for `_msg`), which is
    /// a correlation and not a decoded field.
    pub kind: u32,
}

/// A parsed `.abk` patch bank.
#[derive(Clone, Debug)]
pub struct Abk {
    /// `0x08`. `0x05030001` on 369 of 376 banks, `0x05030002` on the rest.
    pub variant: u32,
    /// `0x18`, start of the [`SAMPLE_BANK_MAGIC`] section. Also stored again at `0x20`.
    pub sample_bank_offset: usize,
    /// `0x24`, and equal to `patch_table_offset - sample_bank_offset` on every bank.
    pub sample_bank_size: u32,
    /// `0x30`, start of a trailing table of offsets into the first section.
    pub patch_table_offset: usize,
    /// `0x38`, start of the export table.
    pub export_table_offset: usize,
    /// Offsets of each sample, relative to [`Abk::sample_bank_offset`].
    pub samples: Vec<u32>,
    /// Offsets into the first section, from the table at [`Abk::patch_table_offset`].
    pub patches: Vec<u32>,
    pub exports: Vec<Export>,
}

impl Abk {
    /// Byte size of the fixed header. Also the offset of the first section, stated
    /// redundantly at `0x1C` on all 376 banks.
    pub const HEADER_SIZE: usize = 0x5C;

    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(0..4) != Some(&ABK_MAGIC[..]) {
            return Err(Error::new(0, "not an ABKC bank"));
        }
        if data.len() < Self::HEADER_SIZE {
            return Err(Error::new(0, format!("bank is {} bytes", data.len())));
        }
        let variant = be32(data, 0x08)?;
        let declared_size = be32(data, 0x14)? as usize;
        if declared_size != data.len() {
            return Err(Error::new(
                0x14,
                format!(
                    "header says {declared_size} bytes, member is {}",
                    data.len()
                ),
            ));
        }
        let first_section = be32(data, 0x1C)? as usize;
        if first_section != Self::HEADER_SIZE {
            return Err(Error::new(
                0x1C,
                format!("first section at {first_section:#x}"),
            ));
        }
        let sample_bank_offset = be32(data, 0x18)? as usize;
        if be32(data, 0x20)? as usize != sample_bank_offset {
            return Err(Error::new(
                0x20,
                "0x18 and 0x20 disagree on the sample bank",
            ));
        }
        let sample_bank_size = be32(data, 0x24)?;
        let patch_table_offset = be32(data, 0x30)? as usize;
        let export_table_offset = be32(data, 0x38)? as usize;
        if be32(data, 0x34)? as usize != patch_table_offset + 4 {
            return Err(Error::new(0x34, "0x34 is not 0x30 + 4"));
        }
        if sample_bank_size as usize != patch_table_offset - sample_bank_offset {
            return Err(Error::new(0x24, "0x24 is not the sample bank's length"));
        }
        if !(first_section < sample_bank_offset
            && sample_bank_offset < patch_table_offset
            && patch_table_offset < export_table_offset
            && export_table_offset <= data.len())
        {
            return Err(Error::new(0x18, "section offsets are not in order"));
        }

        let samples = read_sample_bank(data, sample_bank_offset)?;
        let patches = read_offset_table(data, patch_table_offset + 4)?;
        let exports = read_exports(data, export_table_offset)?;

        Ok(Self {
            variant,
            sample_bank_offset,
            sample_bank_size,
            patch_table_offset,
            export_table_offset,
            samples,
            patches,
            exports,
        })
    }

    /// An unused slot in the sample table.
    ///
    /// Measured across all 376 banks: 89,431 slots hold 5,168 real offsets and 84,263 of this
    /// value, and **no real offset ever appears after one** — so the table is a fixed-capacity
    /// array with a used prefix, not a sparse map with holes. Adding this to the section base
    /// overflows past 4 GB, which is how it was found: a sample range built from it addressed
    /// `0x10000c17f` and looked like a corrupt bank.
    pub const SAMPLE_ABSENT: u32 = 0xFFFF_FFFF;

    /// How many slots hold a real sample. Equal to the index of the first absent slot.
    pub fn present(&self) -> usize {
        self.samples
            .iter()
            .take_while(|&&o| o != Self::SAMPLE_ABSENT)
            .count()
    }

    /// Byte range of one sample within the whole bank, or `None` for an index past the table or
    /// for an unused slot. The last real sample runs to the end of the sample-bank section.
    pub fn sample_range(&self, i: usize) -> Option<std::ops::Range<usize>> {
        let offset = *self.samples.get(i)?;
        if offset == Self::SAMPLE_ABSENT {
            return None;
        }
        let start = self.sample_bank_offset + offset as usize;
        let end = match self.samples.get(i + 1) {
            // An absent next slot means this is the last real sample, so it runs to the end of
            // the section. Using the sentinel as an end would give a ~4 GB range.
            Some(&next) if next != Self::SAMPLE_ABSENT => self.sample_bank_offset + next as usize,
            _ => self.sample_bank_offset + self.sample_bank_size as usize,
        };
        Some(start..end)
    }
}

/// `S10A | u32 0 | u32 count | u32 offsets[count]`, offsets relative to the tag.
fn read_sample_bank(data: &[u8], at: usize) -> Result<Vec<u32>> {
    if data.get(at..at + 4) != Some(&SAMPLE_BANK_MAGIC[..]) {
        return Err(Error::new(at, "sample bank does not start with S10A"));
    }
    if be32(data, at + 4)? != 0 {
        return Err(Error::new(at + 4, "S10A word at +4 is not zero"));
    }
    let count = be32(data, at + 8)? as usize;
    let mut samples = Vec::with_capacity(count);
    for i in 0..count {
        samples.push(be32(data, at + 12 + 4 * i)?);
    }
    // The table is immediately followed by the first sample, so the first entry pins
    // the whole layout. An empty bank states no first offset at all.
    if let Some(&first) = samples.first() {
        let expected = 12 + 4 * count as u32;
        if first != expected {
            return Err(Error::new(
                at + 12,
                format!("first sample at {first}, want {expected}"),
            ));
        }
    }
    Ok(samples)
}

fn read_offset_table(data: &[u8], at: usize) -> Result<Vec<u32>> {
    let count = be32(data, at)? as usize;
    (0..count).map(|i| be32(data, at + 4 + 4 * i)).collect()
}

/// `u32 count`, then 12 bytes each: `u32 target`, `u32 record_offset`, `u32 kind`.
/// The record is `u16 project_id | u16 name_id | NUL-terminated name`.
fn read_exports(data: &[u8], at: usize) -> Result<Vec<Export>> {
    let count = be32(data, at)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let e = at + 4 + 12 * i;
        let target = be32(data, e)?;
        let record_offset = be32(data, e + 4)? as usize;
        let kind = be32(data, e + 8)?;
        let project_id = be16(data, record_offset)?;
        let name_id = be16(data, record_offset + 2)?;
        let name = read_cstr(data, record_offset + 4)?;
        out.push(Export {
            record_offset,
            project_id,
            name_id,
            name,
            target,
            kind,
        });
    }
    Ok(out)
}

fn read_cstr(data: &[u8], at: usize) -> Result<String> {
    let rest = data
        .get(at..)
        .ok_or_else(|| Error::new(at, "string starts past end"))?;
    let end = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| Error::new(at, "unterminated"))?;
    if end == 0 {
        return Err(Error::new(at, "empty name"));
    }
    std::str::from_utf8(&rest[..end])
        .map(str::to_owned)
        .map_err(|_| Error::new(at, "name is not UTF-8"))
}

// ---------------------------------------------------------------------------
// .csi -- "MOIR", an authoring project's symbol table
// ---------------------------------------------------------------------------

/// `.csi` magic. Read as four bytes; the shipping loader never compares it.
pub const CSI_MAGIC: &[u8; 4] = b"MOIR";

/// One symbol of a [`Csi`] project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// Which of the three tables the symbol came from: 0 and 1 are 12-byte records,
    /// 2 is 16 bytes. What distinguishes the three is not established.
    pub group: u8,
    /// The project's id for this name, as an [`Export`] cites it.
    pub id: u16,
    pub name: String,
    /// `+0x00` of the record.
    pub unknown_a: u32,
    /// Group 2 only: the record's second word, `0x7FFF` on most entries.
    pub unknown_b: u32,
}

/// A parsed `.csi`: the symbol table of one authoring project.
#[derive(Clone, Debug)]
pub struct Csi {
    /// `0x10`. The id `.abk` export records cite; unique across the nine shipped files.
    pub project_id: u16,
    /// `0x0A`, `0x0C`, `0x0E`: the sizes of the three symbol tables.
    pub group_counts: [u16; 3],
    pub symbols: Vec<Symbol>,
    /// Offset the record tables end at, which is also where the string pool begins.
    pub pool_offset: usize,
}

impl Csi {
    /// Offset of the first record table.
    pub const TABLE_OFFSET: usize = 0x28;

    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(0..4) != Some(&CSI_MAGIC[..]) {
            return Err(Error::new(0, "not a MOIR project"));
        }
        let group_counts = [be16(data, 0x0A)?, be16(data, 0x0C)?, be16(data, 0x0E)?];
        let project_id = be16(data, 0x10)?;

        let mut symbols = Vec::new();
        let mut at = Self::TABLE_OFFSET;
        for (group, &count) in group_counts.iter().enumerate() {
            let stride = if group == 2 { 16 } else { 12 };
            for _ in 0..count {
                let (unknown_a, unknown_b, name_at, id) = if stride == 12 {
                    (
                        be32(data, at)?,
                        0,
                        be32(data, at + 4)? as usize,
                        be16(data, at + 8)?,
                    )
                } else {
                    (
                        be32(data, at)?,
                        be32(data, at + 4)?,
                        be32(data, at + 8)? as usize,
                        be16(data, at + 12)?,
                    )
                };
                symbols.push(Symbol {
                    group: group as u8,
                    id,
                    name: read_cstr(data, name_at)?,
                    unknown_a,
                    unknown_b,
                });
                at += stride;
            }
        }
        Ok(Self {
            project_id,
            group_counts,
            symbols,
            pool_offset: at,
        })
    }

    /// Resolve one of the project's own name ids.
    pub fn name(&self, id: u16) -> Option<&str> {
        self.symbols
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.as_str())
    }
}

// ---------------------------------------------------------------------------
// .ems -- emitter placements
// ---------------------------------------------------------------------------

/// One ambient emitter placed in the world.
///
/// The field roles below are read from the value distributions over all 1,796 records
/// on the disc, except [`Emitter::sound`], which is demonstrated: 48 of the 146 distinct
/// values are [`crate::hash::name_id`] of a real `.abk` bank in the same archive, and
/// two of them also appear as `rldimi`-assembled constants in the shipping code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emitter {
    /// `+0x00`. Small, near-unique within a file.
    pub index: u32,
    /// `+0x04`. Zero on 1,590 of 1,796 records; otherwise 32, 128, 224 and nine others.
    pub flags: u32,
    /// `+0x08`. Three floats; the middle one has by far the narrowest range, which is
    /// what a height ought to look like, so this is read as a world position.
    pub position: [f32; 3],
    /// `+0x14`. Three floats in 1..256, read as an extent because of that range.
    pub extent: [f32; 3],
    /// `+0x20` .. `+0x2C`. Four floats in -1..1, roles unknown.
    pub scalars: [f32; 4],
    /// `+0x30`. The 64-bit name id of the sound this emitter plays.
    pub sound: u64,
    /// `+0x38`. Four floats in 0..1; the last is 1.0 on every record on the disc.
    pub gains: [f32; 4],
}

/// A parsed `.ems`: `u32 count` followed by `count` 72-byte records.
#[derive(Clone, Debug)]
pub struct Ems {
    pub emitters: Vec<Emitter>,
}

impl Ems {
    /// Byte size of one emitter record.
    pub const RECORD_SIZE: usize = 72;

    pub fn parse(data: &[u8]) -> Result<Self> {
        let count = be32(data, 0)? as usize;
        let want = 4 + Self::RECORD_SIZE * count;
        if want != data.len() {
            return Err(Error::new(
                0,
                format!(
                    "{count} emitters need {want} bytes, member is {}",
                    data.len()
                ),
            ));
        }
        let f = |at: usize| -> Result<f32> { Ok(f32::from_bits(be32(data, at)?)) };
        let mut emitters = Vec::with_capacity(count);
        for i in 0..count {
            let r = 4 + Self::RECORD_SIZE * i;
            emitters.push(Emitter {
                index: be32(data, r)?,
                flags: be32(data, r + 4)?,
                position: [f(r + 8)?, f(r + 12)?, f(r + 16)?],
                extent: [f(r + 20)?, f(r + 24)?, f(r + 28)?],
                scalars: [f(r + 32)?, f(r + 36)?, f(r + 40)?, f(r + 44)?],
                sound: (u64::from(be32(data, r + 48)?) << 32) | u64::from(be32(data, r + 52)?),
                gains: [f(r + 56)?, f(r + 60)?, f(r + 64)?, f(r + 68)?],
            });
        }
        Ok(Self { emitters })
    }
}

// ---------------------------------------------------------------------------
// .bnk -- "SPLC", a sample collection
// ---------------------------------------------------------------------------

/// `.bnk` magic.
pub const BNK_MAGIC: &[u8; 4] = b"SPLC";

/// One entry of a [`Bnk`]'s 36-byte table. Only the index is established; the five
/// floats that follow it are carried raw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BnkEntry {
    /// `+0x00`. Runs 0, 1, 2 ... on every bank measured.
    pub index: u16,
    /// `+0x02`. Unidentified.
    pub unknown: u16,
    /// `+0x04`. Five floats; the last is in the hundreds-to-thousands range on every
    /// bank, which is the right shape for a sample rate but is not checked against one.
    pub values: [f32; 5],
}

/// A parsed `.bnk`. Header and entry table only -- the per-sample parameter block and
/// the sample payload at [`Bnk::payload_offset`] are not decoded.
#[derive(Clone, Debug)]
pub struct Bnk {
    /// `0x04`. 3 on every bank on the disc.
    pub version: u32,
    /// `0x08`. Start of a region this module does not parse.
    pub payload_offset: usize,
    /// `0x10`, `0x18`. Unidentified counts.
    pub unknown_10: u32,
    pub unknown_18: u32,
    /// `0x1C`. May be empty: one bank on the disc has a zero-length name.
    pub name: String,
    pub entries: Vec<BnkEntry>,
}

impl Bnk {
    /// Offset of the entry table.
    pub const TABLE_OFFSET: usize = 0x40;
    /// Stride of one entry.
    pub const ENTRY_SIZE: usize = 36;

    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(0..4) != Some(&BNK_MAGIC[..]) {
            return Err(Error::new(0, "not an SPLC bank"));
        }
        let version = be32(data, 0x04)?;
        let payload_offset = be32(data, 0x08)? as usize;
        let count = be32(data, 0x0C)? as usize;
        let unknown_10 = be32(data, 0x10)?;
        if be32(data, 0x14)? != 0 {
            return Err(Error::new(0x14, "0x14 is not zero"));
        }
        let unknown_18 = be32(data, 0x18)?;
        if payload_offset >= data.len() {
            return Err(Error::new(0x08, "payload offset is past the end"));
        }
        if Self::TABLE_OFFSET + Self::ENTRY_SIZE * count > payload_offset {
            return Err(Error::new(0x0C, "entry table would run into the payload"));
        }
        let name = read_cstr(data, 0x1C).unwrap_or_default();

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = Self::TABLE_OFFSET + Self::ENTRY_SIZE * i;
            let index = be16(data, e)?;
            if usize::from(index) != i {
                return Err(Error::new(e, format!("entry {i} is numbered {index}")));
            }
            let mut values = [0f32; 5];
            for (j, v) in values.iter_mut().enumerate() {
                *v = f32::from_bits(be32(data, e + 4 + 4 * j)?);
            }
            entries.push(BnkEntry {
                index,
                unknown: be16(data, e + 2)?,
                values,
            });
        }
        Ok(Self {
            version,
            payload_offset,
            unknown_10,
            unknown_18,
            name,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_slot_has_no_range_and_does_not_end_the_one_before_it() {
        // Measured on real data: 84,263 of 89,431 slots hold 0xFFFFFFFF and no real offset ever
        // follows one. Treating the sentinel as an offset builds a range that overflows past 4 GB;
        // treating it as the NEXT offset gives a ~4 GB length. Both looked like corrupt banks.
        let abk = Abk {
            variant: 0x0503_0001,
            sample_bank_offset: 0x100,
            sample_bank_size: 0x400,
            patch_table_offset: 0x500,
            export_table_offset: 0x600,
            samples: vec![12, 0x200, Abk::SAMPLE_ABSENT, Abk::SAMPLE_ABSENT],
            patches: Vec::new(),
            exports: Vec::new(),
        };
        assert_eq!(abk.present(), 2);
        assert_eq!(abk.sample_range(0), Some(0x10C..0x300));
        // The last real sample runs to the end of the section, not to the sentinel.
        assert_eq!(abk.sample_range(1), Some(0x300..0x500));
        assert_eq!(abk.sample_range(2), None);
        assert_eq!(abk.sample_range(9), None);
    }

    fn abk_fixture() -> Vec<u8> {
        // Header, then S10A with one sample, then the patch table, then one export.
        let mut v = vec![0u8; 0x5C + 4]; // header plus a four-byte first section
        v[0..4].copy_from_slice(ABK_MAGIC);
        let sample_bank = v.len();
        let s10a = {
            let mut s = Vec::new();
            s.extend_from_slice(SAMPLE_BANK_MAGIC);
            s.extend_from_slice(&0u32.to_be_bytes());
            s.extend_from_slice(&1u32.to_be_bytes());
            s.extend_from_slice(&16u32.to_be_bytes()); // 12 + 4 * 1
            s.extend_from_slice(&[0xAA; 8]);
            s
        };
        v.extend_from_slice(&s10a);
        let patch_table = v.len();
        v.extend_from_slice(&0u32.to_be_bytes()); // 0x30 word
        v.extend_from_slice(&1u32.to_be_bytes()); // count at 0x34
        v.extend_from_slice(&0x5Cu32.to_be_bytes());
        let export_table = v.len();
        v.extend_from_slice(&1u32.to_be_bytes());
        let record = export_table + 4 + 12;
        v.extend_from_slice(&0x60u32.to_be_bytes());
        v.extend_from_slice(&(record as u32).to_be_bytes());
        v.extend_from_slice(&0x0141_4B58u32.to_be_bytes());
        v.extend_from_slice(&0x5C48u16.to_be_bytes());
        v.extend_from_slice(&0x09BAu16.to_be_bytes());
        v.extend_from_slice(b"a_msg\0\0\0");

        let len = v.len() as u32;
        v[0x14..0x18].copy_from_slice(&len.to_be_bytes());
        v[0x18..0x1C].copy_from_slice(&(sample_bank as u32).to_be_bytes());
        v[0x1C..0x20].copy_from_slice(&0x5Cu32.to_be_bytes());
        v[0x20..0x24].copy_from_slice(&(sample_bank as u32).to_be_bytes());
        v[0x24..0x28].copy_from_slice(&((patch_table - sample_bank) as u32).to_be_bytes());
        v[0x30..0x34].copy_from_slice(&(patch_table as u32).to_be_bytes());
        v[0x34..0x38].copy_from_slice(&(patch_table as u32 + 4).to_be_bytes());
        v[0x38..0x3C].copy_from_slice(&(export_table as u32).to_be_bytes());
        v
    }

    #[test]
    fn abk_reads_samples_and_exports() {
        let v = abk_fixture();
        let bank = Abk::parse(&v).unwrap();
        assert_eq!(bank.samples, vec![16]);
        assert_eq!(bank.patches, vec![0x5C]);
        assert_eq!(bank.exports.len(), 1);
        let e = &bank.exports[0];
        assert_eq!(
            (e.project_id, e.name_id, e.name.as_str()),
            (0x5C48, 0x09BA, "a_msg")
        );
        assert_eq!(
            bank.sample_range(0),
            Some(bank.sample_bank_offset + 16..bank.patch_table_offset)
        );
    }

    #[test]
    fn abk_rejects_a_size_that_disagrees_with_the_header() {
        let mut v = abk_fixture();
        v.push(0);
        assert!(Abk::parse(&v).is_err());
    }

    #[test]
    fn abk_rejects_a_moved_sample_bank() {
        let mut v = abk_fixture();
        v[0x20..0x24].copy_from_slice(&0x64u32.to_be_bytes());
        assert!(Abk::parse(&v).is_err());
    }

    #[test]
    fn abk_rejects_a_foreign_magic() {
        let mut v = abk_fixture();
        v[0..4].copy_from_slice(b"SPLC");
        assert!(Abk::parse(&v).is_err());
    }

    fn csi_fixture() -> Vec<u8> {
        let mut v = vec![0u8; Csi::TABLE_OFFSET];
        v[0..4].copy_from_slice(CSI_MAGIC);
        v[0x0A..0x0C].copy_from_slice(&1u16.to_be_bytes());
        v[0x0C..0x0E].copy_from_slice(&0u16.to_be_bytes());
        v[0x0E..0x10].copy_from_slice(&1u16.to_be_bytes());
        v[0x10..0x12].copy_from_slice(&0x5C48u16.to_be_bytes());
        let pool = Csi::TABLE_OFFSET + 12 + 16;
        // group 0, 12 bytes
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&(pool as u32).to_be_bytes());
        v.extend_from_slice(&0x09BAu16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        // group 2, 16 bytes
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&0x7FFFu32.to_be_bytes());
        v.extend_from_slice(&((pool + 6) as u32).to_be_bytes());
        v.extend_from_slice(&0x00BCu16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(v.len(), pool);
        v.extend_from_slice(b"a_msg\0b_vol\0");
        v
    }

    #[test]
    fn csi_reads_both_record_sizes() {
        let p = Csi::parse(&csi_fixture()).unwrap();
        assert_eq!(p.project_id, 0x5C48);
        assert_eq!(p.group_counts, [1, 0, 1]);
        assert_eq!(p.symbols.len(), 2);
        assert_eq!(p.name(0x09BA), Some("a_msg"));
        assert_eq!(p.name(0x00BC), Some("b_vol"));
        assert_eq!(p.symbols[1].unknown_b, 0x7FFF);
        assert_eq!(p.pool_offset, Csi::TABLE_OFFSET + 12 + 16);
    }

    #[test]
    fn csi_rejects_a_foreign_magic() {
        let mut v = csi_fixture();
        v[0..4].copy_from_slice(b"ABKC");
        assert!(Csi::parse(&v).is_err());
    }

    #[test]
    fn ems_requires_the_count_to_match_the_length() {
        let mut v = 1u32.to_be_bytes().to_vec();
        v.extend_from_slice(&[0u8; Ems::RECORD_SIZE]);
        v[4 + 48..4 + 56].copy_from_slice(&0x91C4_1263_C312_C84Bu64.to_be_bytes());
        let e = Ems::parse(&v).unwrap();
        assert_eq!(e.emitters.len(), 1);
        assert_eq!(
            e.emitters[0].sound,
            crate::hash::name_id_str("trees_rustle")
        );
        v.push(0);
        assert!(Ems::parse(&v).is_err());
    }

    #[test]
    fn ems_accepts_an_empty_set() {
        assert!(Ems::parse(&0u32.to_be_bytes()).unwrap().emitters.is_empty());
    }

    fn bnk_fixture() -> Vec<u8> {
        let mut v = vec![0u8; Bnk::TABLE_OFFSET];
        v[0..4].copy_from_slice(BNK_MAGIC);
        v[0x04..0x08].copy_from_slice(&3u32.to_be_bytes());
        v[0x0C..0x10].copy_from_slice(&2u32.to_be_bytes());
        v[0x1C..0x22].copy_from_slice(b"name\0\0");
        v.extend(std::iter::repeat_n(0u8, Bnk::ENTRY_SIZE * 2));
        v[Bnk::TABLE_OFFSET..Bnk::TABLE_OFFSET + 2].copy_from_slice(&0u16.to_be_bytes());
        let second = Bnk::TABLE_OFFSET + Bnk::ENTRY_SIZE;
        v[second..second + 2].copy_from_slice(&1u16.to_be_bytes());
        let payload = v.len() as u32;
        v[0x08..0x0C].copy_from_slice(&payload.to_be_bytes());
        v.push(0xEE);
        v
    }

    #[test]
    fn bnk_reads_its_entry_table() {
        let b = Bnk::parse(&bnk_fixture()).unwrap();
        assert_eq!(b.version, 3);
        assert_eq!(b.name, "name");
        assert_eq!(b.entries.len(), 2);
        assert_eq!(b.entries[1].index, 1);
    }

    #[test]
    fn bnk_rejects_out_of_order_entries() {
        let mut v = bnk_fixture();
        let second = Bnk::TABLE_OFFSET + Bnk::ENTRY_SIZE;
        v[second..second + 2].copy_from_slice(&7u16.to_be_bytes());
        assert!(Bnk::parse(&v).is_err());
    }
}
