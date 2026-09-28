//! EA `.mpf` interactive-music **sequencing maps**.
//!
//! A `.mpf` is the companion to a `.mus`: the `.mus` holds audio segments, the `.mpf`
//! says which segment plays next. Nine sections; the top level and sections 4-8 were
//! decoded earlier, sections 0-3 here.
//!
//! Every layout in this module was read out of the lifted guest reader in the `PATHI_`
//! module (`sub_82956DD0` loads the file; `sub_829534F8` resolves a node;
//! `sub_82953588` resolves a script; `sub_82957CA0` walks a node's branch table;
//! `sub_829543E0` resolves a variable; `sub_82957A00` reads the tempo table), then
//! checked against all three retail files. See `docs/xma-transcode.md`.
//!
//! ```text
//! 0x00  "PFDx"
//! 0x04  u8 u8        major.minor -- the loader rejects anything but 5.3
//! 0x06  u16          0xB003 on all three files, unread by the loader
//! 0x08  u32          0
//! 0x0C  u8           map type: the loader keys its four load slots on this
//! 0x0D  u8           section 6 entry count (`.mus` links)
//! 0x0E  u8           unidentified -- no reader found in the module
//! 0x0F  u8           section 2 entry count (scripts)
//! 0x10  u8           0 on all three files
//! 0x11  u8           section 4 record count (variables)
//! 0x12  u16          section 0 entry count (nodes)
//! 0x14  u32[10]      section byte offsets; [0] == 0x48, [9] == file size
//! ```
//!
//! Two encodings recur:
//!
//! * a **word offset** -- a `u16` or `u32` that is multiplied by 4 to give a byte offset
//!   *from the start of the file*. Sections 0, 2, 5 and 6 are tables of these.
//! * a **section pointer** -- the loader adds `offset[i]` to the image base once, into
//!   fields 28+4*i of its runtime object, so section *i* is always reached the same way.

use crate::{Error, Result, be16, be32};

/// `"PFDx"`, the file magic. Formed in the loader as `lis 20550` / `ori 17528`.
pub const MAGIC: [u8; 4] = *b"PFDx";
/// The only version the loader accepts (`sub_82956DD0` compares both bytes).
pub const VERSION: (u8, u8) = (5, 3);
/// Size of the fixed header, and the value of `offset[0]` in every file.
pub const HEADER_SIZE: usize = 0x48;
/// Sections in the file.
pub const SECTION_COUNT: usize = 9;

/// A half-open byte range inside the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
    pub fn contains(&self, at: usize) -> bool {
        at >= self.start && at < self.end
    }
}

/// The fixed `.mpf` header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// `(5, 3)`; any other pair is rejected by the guest loader and by [`Header::parse`].
    pub version: (u8, u8),
    /// `0x06`. `0xB003` on all three files. The loader never reads it.
    pub unknown_06: u16,
    /// `0x0C`. The loader keeps four maps live at once and keys the slot on this byte,
    /// and the music manager passes `0x01000000 << map_type` in its load request.
    /// 0 = world, 1 = game, 2 = ipod on the retail files.
    pub map_type: u8,
    /// `0x0D`. Number of `u32` entries in section 6, bound-checked by the guest.
    pub mus_link_count: u8,
    /// `0x0E`. Unidentified: nothing in the module reads header byte 14.
    pub unknown_0e: u8,
    /// `0x0F`. Number of `u16` entries in section 2 (scripts).
    pub script_count: u8,
    /// `0x10`. Zero on all three files; the count below is read as a byte at `0x11`.
    pub unknown_10: u8,
    /// `0x11`. Number of 20-byte records in section 4 (variables).
    pub variable_count: u8,
    /// `0x12`. Number of `u16` entries in section 0 (nodes).
    pub node_count: u16,
    /// The nine section spans, derived from the ten offsets at `0x14`.
    pub sections: [Span; SECTION_COUNT],
}

impl Header {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            return Err(Error::new(0, "truncated: shorter than a .mpf header"));
        }
        if data[..4] != MAGIC {
            return Err(Error::new(0, "not a .mpf: magic is not \"PFDx\""));
        }
        let version = (data[4], data[5]);
        if version != VERSION {
            return Err(Error::new(
                4,
                format!("unsupported version {}.{}, want 5.3", version.0, version.1),
            ));
        }
        let mut offsets = [0usize; SECTION_COUNT + 1];
        for (i, slot) in offsets.iter_mut().enumerate() {
            *slot = be32(data, 0x14 + 4 * i)? as usize;
        }
        if offsets[0] != HEADER_SIZE {
            return Err(Error::new(0x14, "offset[0] is not 0x48"));
        }
        for i in 0..SECTION_COUNT {
            if offsets[i] > offsets[i + 1] {
                return Err(Error::new(
                    0x14 + 4 * i,
                    "section offsets are not ascending",
                ));
            }
        }
        if offsets[SECTION_COUNT] != data.len() {
            return Err(Error::new(
                0x14 + 4 * SECTION_COUNT,
                format!(
                    "last offset {:#x} is not the file size {:#x}",
                    offsets[SECTION_COUNT],
                    data.len()
                ),
            ));
        }
        let sections = std::array::from_fn(|i| Span {
            start: offsets[i],
            end: offsets[i + 1],
        });
        Ok(Self {
            version,
            unknown_06: be16(data, 6)?,
            map_type: data[0x0C],
            mus_link_count: data[0x0D],
            unknown_0e: data[0x0E],
            script_count: data[0x0F],
            unknown_10: data[0x10],
            variable_count: data[0x11],
            node_count: be16(data, 0x12)?,
            sections,
        })
    }

    pub fn section(&self, i: usize) -> Span {
        self.sections[i]
    }
}

/// Read a `u16` word offset and turn it into a byte offset (`* 4`, from the file start).
fn word_offset16(data: &[u8], at: usize) -> Result<usize> {
    Ok(be16(data, at)? as usize * 4)
}

// ---------------------------------------------------------------------------
// Section 0 / section 1: the node graph
// ---------------------------------------------------------------------------

/// One branch of a node's transition table: 4 bytes, `{i8 lo, i8 hi, s16 next}`.
///
/// `sub_82957CA0` compares a signed 7-bit control value against `lo..=hi` in order and
/// takes the first entry that contains it; if none does, it takes the entry whose
/// endpoint is nearest. A negative `next` is "no successor".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Branch {
    pub lo: i8,
    pub hi: i8,
    pub next: i16,
}

impl Branch {
    pub fn contains(&self, value: i32) -> bool {
        value >= self.lo as i32 && value <= self.hi as i32
    }
}

/// A section 1 record: 16-byte header plus `branches.len()` 4-byte branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Byte offset of the record in the file.
    pub offset: usize,
    /// Byte length of the record: always `16 + 4 * branches.len()`.
    pub len: usize,
    /// `+0x00` as a signed halfword. `> 0` selects a `.mus` segment (1-based, and also
    /// the 1-based index into section 8). `<= 0` marks a control node; `sub_82957F20`
    /// dispatches 0, -1, -2 and -3 to distinct behaviours.
    pub segment: i16,
    /// `+0x02` bits 11..15: which of the guest's 24 tracks the node belongs to.
    /// Zero in every record of all three retail files.
    pub track: u8,
    /// `+0x02` bits 5..10, a 6-bit field passed to the host's node callback. Unnamed.
    pub field_5_10: u8,
    /// `+0x02` bits 0..4. 0 except on the nodes that also set `field_5_10` to 63.
    pub field_0_4: u8,
    /// The whole of `+0x04`, whose bits 15..19 give the branch count.
    pub word4: u32,
    /// `+0x06` low nibble: the divisor `sub_82957A00` applies to the section 8 rate.
    /// Only 1 and 4 occur; 4 exactly on the nodes that carry a segment.
    pub meter: u8,
    /// `+0x08`. Unidentified; a `u16` index plus a `0x40` flag byte.
    pub word8: u32,
    /// `+0x0C`. Zero except on 8 records across the three files. Bit 31 makes
    /// `sub_82957A00` take its rate from the player instead of the tempo table, and
    /// bits 8..31 are the script id a `-3` control node triggers.
    pub word12: u32,
    pub branches: Vec<Branch>,
}

impl Node {
    /// Header size before the branch table.
    pub const HEADER_SIZE: usize = 16;

    /// Branch count as the guest computes it: `(word_at_4 >> 15) & 0x1F`.
    pub fn branch_count_of(word4: u32) -> usize {
        ((word4 >> 15) & 0x1F) as usize
    }

    fn parse_at(data: &[u8], offset: usize) -> Result<Self> {
        let w0 = be32(data, offset)?;
        let word4 = be32(data, offset + 4)?;
        let n = Self::branch_count_of(word4);
        let len = Self::HEADER_SIZE + 4 * n;
        if offset + len > data.len() {
            return Err(Error::new(offset, "truncated node record"));
        }
        let low = (w0 & 0xFFFF) as u16;
        let mut branches = Vec::with_capacity(n);
        for i in 0..n {
            let at = offset + Self::HEADER_SIZE + 4 * i;
            branches.push(Branch {
                lo: data[at] as i8,
                hi: data[at + 1] as i8,
                next: be16(data, at + 2)? as i16,
            });
        }
        Ok(Self {
            offset,
            len,
            segment: (w0 >> 16) as i16,
            track: ((low >> 11) & 0x1F) as u8,
            field_5_10: ((low >> 5) & 0x3F) as u8,
            field_0_4: (low & 0x1F) as u8,
            word4,
            meter: ((word4 >> 8) & 0xF) as u8,
            word8: be32(data, offset + 8)?,
            word12: be32(data, offset + 12)?,
            branches,
        })
    }

    /// True when the node names a `.mus` segment rather than being a control node.
    pub fn plays_segment(&self) -> bool {
        self.segment > 0
    }
}

/// Section 0: `header.node_count` `u16` word offsets, one per node, in file order.
///
/// The guest bound-checks a node id against `node_count` and reads
/// `image + index[id] * 4`; the table is padded to a 4-byte boundary, so an odd count
/// leaves one trailing `u16`.
pub fn node_index(data: &[u8], header: &Header) -> Result<Vec<usize>> {
    let span = header.section(0);
    let n = header.node_count as usize;
    if span.len() < 2 * n || span.len() - 2 * n > 2 {
        return Err(Error::new(
            span.start,
            format!(
                "section 0 is {} bytes, want {} (+0..2 pad)",
                span.len(),
                2 * n
            ),
        ));
    }
    (0..n)
        .map(|i| word_offset16(data, span.start + 2 * i))
        .collect()
}

/// Parse every section 1 record, in node-id order.
pub fn nodes(data: &[u8], header: &Header) -> Result<Vec<Node>> {
    let index = node_index(data, header)?;
    let span = header.section(1);
    let mut out = Vec::with_capacity(index.len());
    for (id, &at) in index.iter().enumerate() {
        if !span.contains(at) {
            return Err(Error::new(
                at,
                format!("node {id} points outside section 1"),
            ));
        }
        let node = Node::parse_at(data, at)?;
        // The index is dense: a record must end exactly where the next one starts.
        let want_end = index.get(id + 1).copied().unwrap_or(span.end);
        if node.offset + node.len != want_end {
            return Err(Error::new(
                at,
                format!(
                    "node {id} is {} bytes but the index leaves {}",
                    node.len,
                    want_end - node.offset
                ),
            ));
        }
        out.push(node);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Section 2 / section 3: scripts
// ---------------------------------------------------------------------------

/// One 12-byte event of a script.
///
/// `sub_82953660` runs these: it loops over 24 tracks, testing `track_mask` bit by bit,
/// then switches on [`Event::opcode`]. Opcodes run 1..=18; 10, 11 and 12 fall through to
/// the loader's error path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// `+0x00`. Bits 0..23 are a per-track enable mask; bits 28..31 are flags.
    pub track_mask: u32,
    /// `+0x04`. Bits 17..23 are the opcode; bit 16 is set at runtime once the event has
    /// been handled, so it is always clear in the file.
    pub word: u32,
    /// `+0x08`. Operand, decoded per opcode.
    pub payload: u32,
}

impl Event {
    pub const SIZE: usize = 12;
    /// `(word >> 17) & 0x7F`, as `sub_82953660` computes it.
    pub fn opcode(&self) -> u8 {
        ((self.word >> 17) & 0x7F) as u8
    }
}

/// A section 3 record: 20-byte header plus `events.len()` 12-byte events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub offset: usize,
    pub len: usize,
    /// `+0x0C` bits 8..31: the 24-bit id `sub_82953588` matches against.
    pub id: u32,
    pub events: Vec<Event>,
}

impl Script {
    /// Header size before the event list.
    pub const HEADER_SIZE: usize = 20;

    fn parse_at(data: &[u8], offset: usize) -> Result<Self> {
        let w12 = be32(data, offset + 12)?;
        let n = (w12 & 0xFF) as usize;
        let len = Self::HEADER_SIZE + Event::SIZE * n;
        if offset + len > data.len() {
            return Err(Error::new(offset, "truncated script record"));
        }
        let mut events = Vec::with_capacity(n);
        for i in 0..n {
            let at = offset + Self::HEADER_SIZE + Event::SIZE * i;
            events.push(Event {
                track_mask: be32(data, at)?,
                word: be32(data, at + 4)?,
                payload: be32(data, at + 8)?,
            });
        }
        Ok(Self {
            offset,
            len,
            id: w12 >> 8,
            events,
        })
    }
}

/// Section 2: `header.script_count` `u16` word offsets into section 3.
pub fn script_index(data: &[u8], header: &Header) -> Result<Vec<usize>> {
    let span = header.section(2);
    let n = header.script_count as usize;
    if span.len() < 2 * n || span.len() - 2 * n > 2 {
        return Err(Error::new(
            span.start,
            format!(
                "section 2 is {} bytes, want {} (+0..2 pad)",
                span.len(),
                2 * n
            ),
        ));
    }
    (0..n)
        .map(|i| word_offset16(data, span.start + 2 * i))
        .collect()
}

/// Parse every section 3 record, in section 2 order.
pub fn scripts(data: &[u8], header: &Header) -> Result<Vec<Script>> {
    let index = script_index(data, header)?;
    let span = header.section(3);
    let mut out = Vec::with_capacity(index.len());
    for (i, &at) in index.iter().enumerate() {
        if !span.contains(at) {
            return Err(Error::new(
                at,
                format!("script {i} points outside section 3"),
            ));
        }
        let script = Script::parse_at(data, at)?;
        let want_end = index.get(i + 1).copied().unwrap_or(span.end);
        if script.offset + script.len != want_end {
            return Err(Error::new(
                at,
                format!(
                    "script {i} is {} bytes but the index leaves {}",
                    script.len,
                    want_end - script.offset
                ),
            ));
        }
        out.push(script);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Sections 4-8
// ---------------------------------------------------------------------------

/// A section 4 record: a 16-byte name field plus a `u32`.
///
/// `sub_829543E0` resolves operand mode 2 as `section4[i].value`, bound-checked against
/// the byte at header `0x11`. The name field is a fixed 16 bytes holding a NUL-terminated
/// string; the bytes after the NUL are stale, left over from a longer earlier name in the
/// authoring tool's buffer, so they must be ignored rather than read as a second string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: u32,
    /// The raw 16 name bytes, stale tail included.
    pub raw_name: [u8; 16],
}

impl Variable {
    pub const SIZE: usize = 20;
}

/// Section 4: the interactive-music parameter namespace.
pub fn variables(data: &[u8], header: &Header) -> Result<Vec<Variable>> {
    let span = header.section(4);
    let n = header.variable_count as usize;
    if span.len() != n * Variable::SIZE {
        return Err(Error::new(
            span.start,
            format!(
                "section 4 is {} bytes, want {}",
                span.len(),
                n * Variable::SIZE
            ),
        ));
    }
    (0..n)
        .map(|i| {
            let at = span.start + i * Variable::SIZE;
            let mut raw_name = [0u8; 16];
            raw_name.copy_from_slice(&data[at..at + 16]);
            let end = raw_name.iter().position(|&b| b == 0).unwrap_or(16);
            Ok(Variable {
                name: String::from_utf8_lossy(&raw_name[..end]).into_owned(),
                value: be32(data, at + 16)?,
                raw_name,
            })
        })
        .collect()
}

/// A section 7 record: the link to one companion `.mus`.
///
/// `PATHI_verifymusfile` (`sub_82959B60`) compares `checksum` against the checksum of the
/// opened `.mus` and reports a mismatch by name, which is where the field's meaning comes
/// from rather than from the file's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusLink {
    pub offset: usize,
    /// `+0x08`, the `.mus` content checksum.
    pub checksum: u32,
    /// `+0x0C`, an offset into the `.mus`.
    pub mus_offset: u32,
}

impl MusLink {
    pub const SIZE: usize = 20;
}

/// Section 5: one `u32` word offset, which points at section 6.
pub fn section6_pointer(data: &[u8], header: &Header) -> Result<usize> {
    let span = header.section(5);
    if span.len() != 4 {
        return Err(Error::new(span.start, "section 5 is not one u32"));
    }
    Ok(be32(data, span.start)? as usize * 4)
}

/// Sections 6 and 7: `header.mus_link_count` word offsets, each naming a 20-byte record.
pub fn mus_links(data: &[u8], header: &Header) -> Result<Vec<MusLink>> {
    let span = header.section(6);
    let n = header.mus_link_count as usize;
    if span.len() != 4 * n {
        return Err(Error::new(
            span.start,
            format!("section 6 is {} bytes, want {}", span.len(), 4 * n),
        ));
    }
    let s7 = header.section(7);
    (0..n)
        .map(|i| {
            let at = be32(data, span.start + 4 * i)? as usize * 4;
            if !s7.contains(at) || at + MusLink::SIZE > s7.end {
                return Err(Error::new(
                    at,
                    format!("section 6 entry {i} misses section 7"),
                ));
            }
            Ok(MusLink {
                offset: at,
                checksum: be32(data, at + 8)?,
                mus_offset: be32(data, at + 12)?,
            })
        })
        .collect()
}

/// A section 8 record: 8 bytes, one per `.mus` segment.
///
/// `sub_82957A00` reads record `node.segment - 1` and converts its `rate` to a float,
/// divides it by the node's [`Node::meter`] nibble, and uses it as the denominator that
/// turns a tick position into a time. `position` ascends strictly but is unidentified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tempo {
    pub position: u32,
    pub rate: u32,
}

impl Tempo {
    pub const SIZE: usize = 8;
}

/// Section 8, the per-segment tempo table.
pub fn tempo_table(data: &[u8], header: &Header) -> Result<Vec<Tempo>> {
    let span = header.section(8);
    if span.len() % Tempo::SIZE != 0 {
        return Err(Error::new(
            span.start,
            "section 8 is not a multiple of 8 bytes",
        ));
    }
    (0..span.len() / Tempo::SIZE)
        .map(|i| {
            let at = span.start + i * Tempo::SIZE;
            Ok(Tempo {
                position: be32(data, at)?,
                rate: be32(data, at + 4)?,
            })
        })
        .collect()
}

/// Everything in one file, parsed.
#[derive(Clone, Debug)]
pub struct Map {
    pub header: Header,
    pub nodes: Vec<Node>,
    pub scripts: Vec<Script>,
    pub variables: Vec<Variable>,
    pub mus_links: Vec<MusLink>,
    pub tempo: Vec<Tempo>,
}

impl Map {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let header = Header::parse(data)?;
        Ok(Self {
            nodes: nodes(data, &header)?,
            scripts: scripts(data, &header)?,
            variables: variables(data, &header)?,
            mus_links: mus_links(data, &header)?,
            tempo: tempo_table(data, &header)?,
            header,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The branch count is the guest's own expression, not a fit to the files. These are
    /// the four `(word4 >> 15) & 0x1F` values the retail files actually produce.
    #[test]
    fn branch_count_matches_the_guest_expression() {
        assert_eq!(Node::branch_count_of(0x0000_0101), 0);
        assert_eq!(Node::branch_count_of(0x0000_8101), 1);
        assert_eq!(Node::branch_count_of(0x0001_0401), 2);
        assert_eq!(Node::branch_count_of(0x0003_1101), 6);
    }

    #[test]
    fn opcode_is_bits_17_to_23() {
        // 0x0008_F000 and 0x0010_B000 are real section 3 event words from game.mpf.
        assert_eq!(
            Event {
                track_mask: 0,
                word: 0x0008_F000,
                payload: 0
            }
            .opcode(),
            4
        );
        assert_eq!(
            Event {
                track_mask: 0,
                word: 0x0010_B000,
                payload: 0
            }
            .opcode(),
            8
        );
        assert_eq!(
            Event {
                track_mask: 0,
                word: 0x001C_B000,
                payload: 0
            }
            .opcode(),
            14
        );
    }

    #[test]
    fn branch_endpoints_are_signed() {
        // world.mpf's dominant pair splits at -1: [-1, 75] then [75, 127].
        let b = Branch {
            lo: -1,
            hi: 75,
            next: 3,
        };
        assert!(b.contains(-1));
        assert!(b.contains(0));
        assert!(!b.contains(76));
    }

    /// A hand-built file, enough to exercise the walkers. This proves the code agrees
    /// with itself; `examples/verify_mpf.rs` is what proves it agrees with the format.
    fn minimal() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&MAGIC);
        f.extend_from_slice(&[5, 3, 0xB0, 0x03]);
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&[1, 1, 0, 1]); // map_type, mus_links, ?, scripts
        f.extend_from_slice(&[0, 1]); // ?, variables
        f.extend_from_slice(&2u16.to_be_bytes()); // nodes
        let s0 = HEADER_SIZE; //  2 nodes -> 4 bytes
        let s1 = s0 + 4; //  node 0: 16+4, node 1: 16
        let s2 = s1 + 20 + 16;
        let s3 = s2 + 2 + 2; //  one u16 + pad
        let s4 = s3 + 20 + 12; //  one script, one event
        let s5 = s4 + 20;
        let s6 = s5 + 4;
        let s7 = s6 + 4;
        let s8 = s7 + 20;
        let end = s8 + 8;
        for o in [s0, s1, s2, s3, s4, s5, s6, s7, s8, end] {
            f.extend_from_slice(&(o as u32).to_be_bytes());
        }
        f.resize(HEADER_SIZE, 0); // 0x3C..0x48 is zero in every real file
        assert_eq!(f.len(), HEADER_SIZE);
        // section 0
        f.extend_from_slice(&((s1 / 4) as u16).to_be_bytes());
        f.extend_from_slice(&(((s1 + 20) / 4) as u16).to_be_bytes());
        // section 1, node 0: segment 1, one branch to node 1
        f.extend_from_slice(&0x0001_0020u32.to_be_bytes());
        f.extend_from_slice(&0x0000_8401u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&[0x00, 0x7F, 0x00, 0x01]);
        // node 1: control node, no branches
        f.extend_from_slice(&0xFFFF_0020u32.to_be_bytes());
        f.extend_from_slice(&0x0000_0101u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        // section 2
        f.extend_from_slice(&((s3 / 4) as u16).to_be_bytes());
        f.extend_from_slice(&0u16.to_be_bytes());
        // section 3: one script, id 0x123456, one event
        f.extend_from_slice(&[0; 12]);
        f.extend_from_slice(&0x1234_5601u32.to_be_bytes());
        f.extend_from_slice(&0u32.to_be_bytes());
        f.extend_from_slice(&0x1200_0001u32.to_be_bytes());
        f.extend_from_slice(&0x0008_F000u32.to_be_bytes());
        f.extend_from_slice(&0xFFFF_FF80u32.to_be_bytes());
        // section 4
        let mut name = [0u8; 16];
        name[..6].copy_from_slice(b"chaser");
        name[7..10].copy_from_slice(b"ers"); // stale tail, as the real files have
        f.extend_from_slice(&name);
        f.extend_from_slice(&7u32.to_be_bytes());
        // section 5 -> section 6 -> section 7
        f.extend_from_slice(&((s6 / 4) as u32).to_be_bytes());
        f.extend_from_slice(&((s7 / 4) as u32).to_be_bytes());
        f.extend_from_slice(&[0; 8]);
        f.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        f.extend_from_slice(&0x0000_1000u32.to_be_bytes());
        f.extend_from_slice(&[0; 4]);
        // section 8
        f.extend_from_slice(&371u32.to_be_bytes());
        f.extend_from_slice(&1600u32.to_be_bytes());
        assert_eq!(f.len(), end);
        f
    }

    #[test]
    fn walks_a_minimal_map() {
        let f = minimal();
        let map = Map::parse(&f).expect("parse");
        assert_eq!(map.header.node_count, 2);
        assert_eq!(map.nodes.len(), 2);
        assert_eq!(map.nodes[0].segment, 1);
        assert_eq!(map.nodes[0].meter, 4);
        assert_eq!(
            map.nodes[0].branches,
            vec![Branch {
                lo: 0,
                hi: 0x7F,
                next: 1
            }]
        );
        assert_eq!(map.nodes[1].segment, -1);
        assert!(map.nodes[1].branches.is_empty());
        assert_eq!(map.scripts.len(), 1);
        assert_eq!(map.scripts[0].id, 0x123456);
        assert_eq!(map.scripts[0].events.len(), 1);
        assert_eq!(map.scripts[0].events[0].opcode(), 4);
        assert_eq!(map.variables[0].name, "chaser");
        assert_eq!(map.variables[0].value, 7);
        assert_eq!(map.mus_links[0].checksum, 0xDEAD_BEEF);
        assert_eq!(
            map.tempo,
            vec![Tempo {
                position: 371,
                rate: 1600
            }]
        );
        assert_eq!(
            section6_pointer(&f, &map.header).unwrap(),
            map.header.section(6).start
        );
    }

    #[test]
    fn rejects_a_bad_magic() {
        let mut f = minimal();
        f[0] = b'X';
        assert!(Header::parse(&f).is_err());
    }

    #[test]
    fn rejects_a_wrong_version() {
        let mut f = minimal();
        f[5] = 4;
        assert!(Header::parse(&f).is_err());
    }

    #[test]
    fn rejects_a_last_offset_that_is_not_the_file_size() {
        let mut f = minimal();
        let n = f.len();
        f.push(0);
        assert!(Header::parse(&f).is_err());
        f.truncate(n);
        assert!(Header::parse(&f).is_ok());
    }

    #[test]
    fn rejects_a_node_record_that_does_not_fill_its_index_slot() {
        let mut f = minimal();
        // Clear the 0x8000 bit of node 0's word4, dropping its declared branch count to
        // zero while the index still leaves 20 bytes for it.
        let s1 = Header::parse(&f).unwrap().section(1).start;
        f[s1 + 6] &= 0x7F;
        assert!(nodes(&f, &Header::parse(&f).unwrap()).is_err());
    }
}
