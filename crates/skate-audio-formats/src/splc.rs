//! `SPLC` banks — the `.bnk` files in `audiofiles.big`, the sound banks the game's one-shot
//! ("Splice") voices play from.
//!
//! Recovered from the retail loader and the playback path, not guessed: the loader
//! `sub_828DC660` (its `[resource+4] == 2` branch) computes the region bases and hands them to
//! `sub_82974F48`, which registers them and patches the in-file pointers; `sub_82975700` resolves
//! a sample id to a record; `sub_829757D0` walks a record's groups and members; `sub_82975CC8`
//! and `sub_82976020` read the member's fields. Field names below are only those whose use is
//! visible in that code — everything else is carried raw.
//!
//! Big-endian throughout. The loader's arithmetic, with `data` the file's first byte:
//!
//! ```text
//! records    = data + 60                                  36 bytes × [+12]
//! containers = records + 36·[+12]                          72 bytes × [+16]
//!              then 24 bytes × [+20]                       (0 in every bank on the disc)
//! groups     then, per record, [record+7] groups of {12-byte header + 72 bytes × [group+8]}
//! samples    = data + 60 + [+8]                            12 bytes × [+24]
//! stream data= samples + 12·[+24]                          EAAC streams, addressed by the table
//! ```
//!
//! A sample id below `[+12]` names a record directly; at or above it, `id − [+12]` names a
//! container, whose `sub_82976DD8` pick selects one of its record ids (`sub_82975700`).
//!
//! Checked on the owner's `audiofiles.big` (2026-09-19): all 20 `SPLC` banks walk exactly — every
//! record's groups and members land inside the file, the walk ends precisely on the sample table,
//! all 5,973 member sample ids are within the table, every container id is within the record and
//! container range, and all 2,266 table entries point at an EAAC header with a known codec and
//! sample rate.

use crate::{Error, Result, be16, be32, eaac};

/// `"SPLC"`.
pub const MAGIC: &[u8; 4] = b"SPLC";
/// `[+4]`, 3 on every bank on the disc.
pub const VERSION: u32 = 3;
/// The header the regions are measured from (`addi r9,r4,60`).
pub const HEADER_BYTES: usize = 60;
/// `sub_82975700`'s record stride, `sub_82974F48`'s `36·i`.
pub const RECORD_BYTES: usize = 36;
/// `sub_82975700`'s container stride and `sub_829757D0`'s member stride (`72·pick`).
pub const CONTAINER_BYTES: usize = 72;
pub const MEMBER_BYTES: usize = 72;
/// `sub_829757D0`'s group header, before its members.
pub const GROUP_HEADER_BYTES: usize = 12;
/// One sample-table entry.
pub const SAMPLE_BYTES: usize = 12;

/// One 36-byte record: a "sound" with one group per child voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record {
    /// `+4`, the record's own index.
    pub id: u16,
    /// `+7`, the number of groups (`sub_829757D0`'s child count).
    pub children: u8,
    /// `+12` and `+16`, the base and random range of the value `sub_82975A60` stores at the
    /// container's `+88`.
    pub value_base: f32,
    pub value_range: f32,
    /// `+8`, `+20`, `+24`: read by neither the play nor the spatialize path this port covers.
    pub unknown_8: f32,
    pub unknown_20: f32,
    pub unknown_24: f32,
    /// Where the record's first group is, relative to the file.
    pub groups_offset: usize,
}

/// One group of alternative members (`sub_829757D0`: pick one per child).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    /// The first member, relative to the file (`[group+0]`, which the loader patches to
    /// `group + 12`).
    pub members_offset: usize,
    /// `+8`, the number of members.
    pub count: u8,
    /// `+9`, the selection mode `sub_82976DD8` takes.
    pub mode: u8,
}

/// One 72-byte member: a sample with its randomisation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Member {
    /// `+0`, the index into the sample table.
    pub sample: u16,
    /// `+3`, tested against 255 by `sub_82976020`.
    pub flag_3: u8,
    /// `+8` and `+48`: the gain base and its random range (`sub_82975CC8` stores
    /// `base + rand × range` at the voice's `+64`/`+72`).
    pub gain: f32,
    pub gain_range: f32,
    /// `+20` and `+52`: the start delay and its random range (the voice's `+80`; a voice with a
    /// delay is not queued in the frame it is created).
    pub delay: f32,
    pub delay_range: f32,
    /// `+44`: the pitch spread (`sub_82975CC8`'s `+68`; 0 gives the fixed 4.0 branch).
    pub pitch_spread: f32,
    /// `+64`: the play probability — `sub_829757D0` skips the member when `rand` exceeds it.
    pub probability: f32,
    /// `+12` and `+16`: `sub_82976020` compares `+16` against −127.0 and reads `+12`.
    pub unknown_12: f32,
    pub unknown_16: f32,
    /// `+60` (the queued request's byte) and `+68` (chooses the graph's shape in `sub_82976020`).
    pub kind_60: u8,
    pub flag_68: u8,
}

/// One 72-byte container: the random pick between records that a sample id at or above the record
/// count names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    /// `+4 + 2k`, the record ids.
    pub ids: Vec<u16>,
    /// `+68`, `+69`: the count and the selection mode.
    pub mode: u8,
}

/// One sample-table entry: an EAAC stream in the bank's stream region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// `+0`, the stream's start relative to the stream region.
    pub offset: u32,
    /// `+4`, its end (the streams are padded, so this is not the next entry's start).
    pub end: u32,
    /// `+8`, a 32-bit id of the sample's name.
    pub hash: u32,
}

/// A parsed `SPLC` bank. Offsets are relative to the file's first byte, so a member's sample can be
/// addressed in guest memory as `bank_base + stream_offset`.
#[derive(Clone, Debug)]
pub struct Splc {
    /// `[+28]`, the bank's own name.
    pub name: String,
    pub records: Vec<Record>,
    pub containers: Vec<Container>,
    pub samples: Vec<Sample>,
    /// Every group, in walk order; a record's are `groups[first..first + children]`.
    pub groups: Vec<Group>,
    /// Where each record's groups start in [`Splc::groups`].
    pub first_group: Vec<usize>,
    /// The stream region (the sample table's end).
    pub streams_offset: usize,
    pub members_total: usize,
}

fn f32_at(data: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(be32(data, at)?))
}

impl Splc {
    /// Parse a bank exactly as the loader addresses it, and check that every region it walks stays
    /// inside the file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(..4) != Some(&MAGIC[..]) {
            return Err(Error::new(0, "not an SPLC bank"));
        }
        let version = be32(data, 4)?;
        if version != VERSION {
            return Err(Error::new(4, format!("SPLC version {version}")));
        }
        let table_at = HEADER_BYTES + be32(data, 8)? as usize;
        let record_count = be32(data, 12)? as usize;
        let container_count = be32(data, 16)? as usize;
        let extra_24 = be32(data, 20)? as usize;
        let sample_count = be32(data, 24)? as usize;
        let name_end = data[28..HEADER_BYTES]
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(HEADER_BYTES - 28);
        let name = String::from_utf8_lossy(&data[28..28 + name_end]).into_owned();

        let records_at = HEADER_BYTES;
        let containers_at = records_at + RECORD_BYTES * record_count;
        let mut cursor = containers_at + CONTAINER_BYTES * container_count + 24 * extra_24;
        let streams_offset = table_at + SAMPLE_BYTES * sample_count;
        if streams_offset > data.len() || cursor > data.len() {
            return Err(Error::new(cursor, "SPLC regions exceed the bank"));
        }

        let mut records = Vec::with_capacity(record_count);
        let mut groups = Vec::new();
        let mut first_group = Vec::with_capacity(record_count);
        let mut members_total = 0;
        for i in 0..record_count {
            let at = records_at + RECORD_BYTES * i;
            let children = *data
                .get(at + 7)
                .ok_or_else(|| Error::new(at, "SPLC record out of range"))?;
            first_group.push(groups.len());
            let groups_offset = cursor;
            for _ in 0..children {
                if cursor + GROUP_HEADER_BYTES > data.len() {
                    return Err(Error::new(cursor, "SPLC group header out of range"));
                }
                let count = data[cursor + 8];
                let group = Group {
                    members_offset: cursor + GROUP_HEADER_BYTES,
                    count,
                    mode: data[cursor + 9],
                };
                cursor = group.members_offset + MEMBER_BYTES * usize::from(count);
                if cursor > data.len() {
                    return Err(Error::new(cursor, "SPLC members out of range"));
                }
                members_total += usize::from(count);
                groups.push(group);
            }
            records.push(Record {
                id: be16(data, at + 4)?,
                children,
                unknown_8: f32_at(data, at + 8)?,
                value_base: f32_at(data, at + 12)?,
                value_range: f32_at(data, at + 16)?,
                unknown_20: f32_at(data, at + 20)?,
                unknown_24: f32_at(data, at + 24)?,
                groups_offset,
            });
        }
        // The loader derives the sample table from `[+8]`; the walk must land exactly on it.
        if cursor != table_at {
            return Err(Error::new(
                cursor,
                format!("SPLC groups end at {cursor:#x}, the sample table is at {table_at:#x}"),
            ));
        }

        let mut containers = Vec::with_capacity(container_count);
        for k in 0..container_count {
            let at = containers_at + CONTAINER_BYTES * k;
            let count = *data
                .get(at + 68)
                .ok_or_else(|| Error::new(at, "SPLC container out of range"))?;
            if usize::from(count) > (CONTAINER_BYTES - 4) / 2 {
                return Err(Error::new(at, format!("SPLC container holds {count} ids")));
            }
            let mut ids = Vec::with_capacity(usize::from(count));
            for j in 0..usize::from(count) {
                let id = be16(data, at + 4 + 2 * j)?;
                if usize::from(id) >= record_count + container_count {
                    return Err(Error::new(at, format!("SPLC container id {id}")));
                }
                ids.push(id);
            }
            containers.push(Container {
                ids,
                mode: data[at + 69],
            });
        }

        let mut samples = Vec::with_capacity(sample_count);
        for k in 0..sample_count {
            let at = table_at + SAMPLE_BYTES * k;
            let sample = Sample {
                offset: be32(data, at)?,
                end: be32(data, at + 4)?,
                hash: be32(data, at + 8)?,
            };
            let start = streams_offset + sample.offset as usize;
            if start + 8 > data.len() || streams_offset + sample.end as usize > data.len() {
                return Err(Error::new(at, format!("SPLC sample {k} out of range")));
            }
            samples.push(sample);
        }
        Ok(Self {
            name,
            records,
            containers,
            samples,
            groups,
            first_group,
            streams_offset,
            members_total,
        })
    }

    /// A record's groups.
    pub fn groups_of(&self, record: usize) -> &[Group] {
        let first = self.first_group[record];
        &self.groups[first..first + usize::from(self.records[record].children)]
    }

    /// Member `index` of `group`, read from the bank's bytes.
    pub fn member(&self, data: &[u8], group: &Group, index: u8) -> Result<Member> {
        if index >= group.count {
            return Err(Error::new(index.into(), "SPLC member index out of range"));
        }
        let at = group.members_offset + MEMBER_BYTES * usize::from(index);
        Ok(Member {
            sample: be16(data, at)?,
            flag_3: data[at + 3],
            unknown_12: f32_at(data, at + 12)?,
            unknown_16: f32_at(data, at + 16)?,
            gain: f32_at(data, at + 8)?,
            gain_range: f32_at(data, at + 48)?,
            delay: f32_at(data, at + 20)?,
            delay_range: f32_at(data, at + 52)?,
            pitch_spread: f32_at(data, at + 44)?,
            probability: f32_at(data, at + 64)?,
            kind_60: data[at + 60],
            flag_68: data[at + 68],
        })
    }

    /// The EAAC stream of sample `index`, as an offset into the bank (what the play command takes
    /// as its sample address once the bank is placed in guest memory).
    pub fn stream_offset(&self, index: u16) -> Option<usize> {
        let sample = self.samples.get(usize::from(index))?;
        Some(self.streams_offset + sample.offset as usize)
    }

    /// The stream's EAAC header.
    pub fn stream_header(&self, data: &[u8], index: u16) -> Result<eaac::Header> {
        let at = self
            .stream_offset(index)
            .ok_or_else(|| Error::new(index.into(), "SPLC sample index out of range"))?;
        eaac::Header::parse(data, at)
    }

    /// `sub_82975700`: what a sample id names — a record, or a container to pick from.
    pub fn resolve(&self, id: u16) -> Resolved {
        let records = self.records.len();
        let index = usize::from(id);
        if index < records {
            return Resolved::Record(index);
        }
        let container = index - records;
        if container < self.containers.len() {
            Resolved::Container(container)
        } else {
            // `sub_82975700` clamps an id past both tables to the last record.
            Resolved::Record(records.saturating_sub(1))
        }
    }
}

/// What [`Splc::resolve`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    Record(usize),
    Container(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal bank: one record with one group of two members, one container, two samples.
    fn bank() -> Vec<u8> {
        let mut data = vec![0u8; HEADER_BYTES];
        data[..4].copy_from_slice(MAGIC);
        data[4..8].copy_from_slice(&VERSION.to_be_bytes());
        data[12..16].copy_from_slice(&1u32.to_be_bytes()); // one record
        data[16..20].copy_from_slice(&1u32.to_be_bytes()); // one container
        data[24..28].copy_from_slice(&2u32.to_be_bytes()); // two samples
        data[28..32].copy_from_slice(b"one\0");
        // record
        let mut record = vec![0u8; RECORD_BYTES];
        record[4..6].copy_from_slice(&0u16.to_be_bytes());
        record[7] = 1; // one group
        record[12..16].copy_from_slice(&1.0f32.to_be_bytes());
        record[16..20].copy_from_slice(&0.5f32.to_be_bytes());
        data.extend_from_slice(&record);
        // container: two ids, mode 2
        let mut container = vec![0u8; CONTAINER_BYTES];
        container[4..6].copy_from_slice(&0u16.to_be_bytes());
        container[6..8].copy_from_slice(&0u16.to_be_bytes());
        container[68] = 2;
        container[69] = 2;
        data.extend_from_slice(&container);
        // group of two members
        let mut group = vec![0u8; GROUP_HEADER_BYTES];
        group[8] = 2;
        group[9] = 1;
        data.extend_from_slice(&group);
        for (sample, gain) in [(0u16, 1.0f32), (1, 0.5)] {
            let mut member = vec![0u8; MEMBER_BYTES];
            member[..2].copy_from_slice(&sample.to_be_bytes());
            member[8..12].copy_from_slice(&gain.to_be_bytes());
            member[64..68].copy_from_slice(&1.0f32.to_be_bytes());
            data.extend_from_slice(&member);
        }
        let table_at = data.len();
        data[8..12].copy_from_slice(&((table_at - HEADER_BYTES) as u32).to_be_bytes());
        // sample table, then two eight-byte "streams"
        for (offset, end) in [(0u32, 8u32), (8, 16)] {
            data.extend_from_slice(&offset.to_be_bytes());
            data.extend_from_slice(&end.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
        }
        data.extend_from_slice(&[0; 16]);
        data
    }

    #[test]
    fn parses_records_groups_members_containers_and_samples() {
        let data = bank();
        let splc = Splc::parse(&data).unwrap();
        assert_eq!(splc.name, "one");
        assert_eq!(splc.records.len(), 1);
        assert_eq!(splc.records[0].children, 1);
        assert_eq!(splc.records[0].value_base, 1.0);
        assert_eq!(splc.records[0].value_range, 0.5);
        assert_eq!(splc.members_total, 2);
        let groups = splc.groups_of(0);
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].count, groups[0].mode), (2, 1));
        let first = splc.member(&data, &groups[0], 0).unwrap();
        assert_eq!((first.sample, first.gain, first.probability), (0, 1.0, 1.0));
        assert_eq!(splc.member(&data, &groups[0], 1).unwrap().sample, 1);
        assert!(splc.member(&data, &groups[0], 2).is_err());
        assert_eq!(splc.containers.len(), 1);
        assert_eq!(splc.containers[0].mode, 2);
        assert_eq!(splc.containers[0].ids, vec![0, 0]);
        assert_eq!(splc.samples.len(), 2);
        assert_eq!(splc.stream_offset(1), Some(splc.streams_offset + 8));
        // ids below the record count are records; the container follows them.
        assert_eq!(splc.resolve(0), Resolved::Record(0));
        assert_eq!(splc.resolve(1), Resolved::Container(0));
        assert_eq!(splc.resolve(9), Resolved::Record(0));
    }

    /// Every `SPLC` bank in the owner's `audiofiles.big`: the walk must land on the sample table,
    /// every member's sample id must be in the table, and every entry must point at an EAAC header.
    #[test]
    #[ignore = "needs the installed assets"]
    fn parses_every_bank_on_the_disc() {
        let path = std::path::Path::new(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets\private\stock\data\audio\audiofiles.big",
        );
        let Ok(archive) = std::fs::read(path) else { return };
        let parsed = crate::eb::Archive::parse(&archive).unwrap();
        let (mut banks, mut samples, mut members) = (0, 0, 0);
        for entry in &parsed.entries {
            let Some(name) = entry.name.as_deref() else { continue };
            if entry.is_compressed() || !name.to_ascii_lowercase().ends_with(".bnk") {
                continue;
            }
            let data = &archive[entry.range()];
            if data.get(..4) != Some(&MAGIC[..]) {
                continue;
            }
            let splc = Splc::parse(data).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            banks += 1;
            samples += splc.samples.len();
            members += splc.members_total;
            for index in 0..splc.samples.len() as u16 {
                splc.stream_header(data, index)
                    .unwrap_or_else(|e| panic!("{name} sample {index}: {e:?}"));
            }
            for record in 0..splc.records.len() {
                for group in splc.groups_of(record) {
                    for k in 0..group.count {
                        let member = splc.member(data, group, k).unwrap();
                        assert!(
                            usize::from(member.sample) < splc.samples.len(),
                            "{name}: member sample {} of {}",
                            member.sample,
                            splc.samples.len()
                        );
                    }
                }
            }
        }
        println!("SPLC banks {banks}, samples {samples}, members {members}");
        assert!(banks >= 20, "only {banks} SPLC banks parsed");
    }

    #[test]
    fn rejects_a_bank_whose_walk_misses_the_sample_table() {
        let mut data = bank();
        // One more group than the record declares leaves the walk short of the table.
        data[HEADER_BYTES + 7] = 2;
        assert!(Splc::parse(&data).is_err());
        let mut data = bank();
        data[..4].copy_from_slice(b"ABKC");
        assert!(Splc::parse(&data).is_err());
    }
}
