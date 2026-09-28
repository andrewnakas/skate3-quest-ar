//! Check the reading of a patch bank's input records against every `.abk` in an archive.
//!
//! Read from the bank installer `sub_82B1DF50` and the listener's instance allocator
//! `sub_82B1D880`:
//! - the header's u16 at `+0x0A` counts input records, starting at the offset stored at `+0x1C`;
//! - a record is 60 bytes plus 4 per entry, where bytes `+36` and `+39` count the entries;
//! - `+4` is the symbol slot an export resolves into;
//! - `+28`/`+30` (u16) are live instances and capacity;
//! - `+44` and `+48` are the instance template's offset and size.
//!
//! If the reading is right, each record's `+4` slot is the target of an export, `+40` points past
//! the records and inside the first section, and every template lies inside the first section. Exports that land elsewhere bind slots inside the patch graph.
//!
//!     cargo run --release --example verify_patch_records -- <archive>

use skate_audio_formats::{banks, eb};

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes(s.try_into().unwrap()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: verify_patch_records <archive>")?;
    let data = std::fs::read(&path)?;
    let (mut banks_seen, mut records, mut failures) = (0, 0, 0);
    let (mut exports_ok, mut exports_total) = (0, 0);
    let mut capacities = std::collections::BTreeMap::<u16, usize>::new();
    let mut per_slot = std::collections::BTreeMap::<usize, usize>::new();
    let mut adjacent = 0;
    let mut kinds = std::collections::BTreeMap::<(bool, u8), usize>::new();
    for e in &eb::Archive::parse(&data)?.entries {
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".abk") {
            continue;
        }
        let Some(b) = data.get(e.range()) else {
            continue;
        };
        let Ok(abk) = banks::Abk::parse(b) else {
            continue;
        };
        banks_seen += 1;
        let count = be16(b, 0x0A).unwrap_or(0) as usize;
        let mut at = be32(b, 0x1C).unwrap_or(0) as usize;
        let mut slots = Vec::new();
        let mut fail = |why: String| {
            failures += 1;
            println!("  FAIL {name}: {why}");
        };
        for r in 0..count {
            let (Some(live), Some(cap), Some(next), Some(tpl), Some(size)) = (
                be16(b, at + 28),
                be16(b, at + 30),
                be32(b, at + 40),
                be32(b, at + 44),
                be32(b, at + 48),
            ) else {
                fail(format!("record {r} at {at:#x} runs off the bank"));
                break;
            };
            records += 1;
            *capacities.entry(cap).or_default() += 1;
            if live != 0 {
                fail(format!(
                    "record {r} at {at:#x} has {live} live instances on disk"
                ));
            }
            if (tpl as usize) + (size as usize) > abk.sample_bank_offset {
                fail(format!(
                    "record {r} template {tpl:#x}+{size:#x} leaves the first section"
                ));
            }
            slots.push(at + 4);
            let entries = b[at + 36] as usize + b[at + 39] as usize;
            let end = at + 60 + 4 * entries;
            // `+40` is copied into each instance (`sub_82B1D880`), so it is a pointer past the
            // records, not "the next region": on most banks it happens to be the very next byte.
            if !(end <= next as usize && (next as usize) < abk.sample_bank_offset) {
                fail(format!("record {r} ends at {end:#x} but +40 is {next:#x}"));
            }
            if next as usize == end {
                adjacent += 1;
            }
            at = end;
        }
        for slot in &slots {
            let n = abk
                .exports
                .iter()
                .filter(|x| x.target as usize == *slot)
                .count();
            *per_slot.entry(n).or_default() += 1;
        }
        for x in &abk.exports {
            exports_total += 1;
            let on_record = slots.contains(&(x.target as usize));
            if on_record {
                exports_ok += 1;
            }
            *kinds.entry((on_record, (x.kind >> 24) as u8)).or_default() += 1;
        }
    }
    println!("{banks_seen} banks, {records} input records, {failures} failures");
    println!("exports landing on a record's +4 slot: {exports_ok} of {exports_total}");
    println!("records whose +40 is the byte after the last record: {adjacent}");
    println!("capacity distribution: {capacities:?}");
    println!("exports per record slot -> records: {per_slot:?}");
    println!("(export lands on a record slot, kind top byte) -> exports: {kinds:?}");
    Ok(())
}
