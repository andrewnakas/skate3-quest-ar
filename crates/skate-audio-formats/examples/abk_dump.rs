//! Dump one `.abk` patch bank: header offsets, the patch table, the exports, and the undecoded
//! first section as big-endian words with every export target and patch offset marked.
//!
//!     cargo run --release --example abk_dump -- <archive> <member name substring> [max words]

use skate_audio_formats::{banks, eb};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: abk_dump <archive> <member> [max words]")?;
    let wanted = args
        .next()
        .ok_or("usage: abk_dump <archive> <member> [max words]")?;
    let max_words: usize = args.next().map(|s| s.parse()).transpose()?.unwrap_or(2048);
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    let entry = archive
        .entries
        .iter()
        .find(|e| {
            e.name
                .as_deref()
                .is_some_and(|n| n.contains(&wanted) && n.ends_with(".abk"))
        })
        .ok_or("no such .abk member")?;
    let bytes = &data[entry.range()];
    let abk = banks::Abk::parse(bytes)?;
    println!(
        "{}: {} bytes, variant {:#010x}, sample bank {:#x}+{:#x}, patch table {:#x}, exports {:#x}",
        entry.name.as_deref().unwrap_or("?"),
        bytes.len(),
        abk.variant,
        abk.sample_bank_offset,
        abk.sample_bank_size,
        abk.patch_table_offset,
        abk.export_table_offset
    );
    println!(
        "samples present: {} of {}",
        abk.present(),
        abk.samples.len()
    );
    println!("patches ({}): {:x?}", abk.patches.len(), abk.patches);

    let mut marks: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (i, p) in abk.patches.iter().enumerate() {
        marks
            .entry(*p as usize)
            .or_default()
            .push(format!("patch[{i}]"));
    }
    println!("exports ({}):", abk.exports.len());
    for e in &abk.exports {
        println!(
            "  {:32} {:#06x}:{:#06x} kind {:08x} target {:#x}",
            e.name, e.project_id, e.name_id, e.kind, e.target
        );
        marks
            .entry(e.target as usize)
            .or_default()
            .push(format!("export {}", e.name));
    }

    let start = banks::Abk::HEADER_SIZE;
    let end = abk.sample_bank_offset.min(bytes.len());
    println!(
        "first section {start:#x}..{end:#x} ({} bytes):",
        end - start
    );
    for (n, at) in (start..end).step_by(4).enumerate() {
        if n >= max_words {
            println!("  ... truncated at {max_words} words");
            break;
        }
        let w = u32::from_be_bytes(bytes[at..at + 4].try_into()?);
        let f = f32::from_bits(w);
        let as_float = if f.is_finite() && f != 0.0 && (1e-4..1e6).contains(&f.abs()) {
            format!("  f={f}")
        } else {
            String::new()
        };
        let mark = marks
            .get(&at)
            .map(|m| format!("  <-- {}", m.join(", ")))
            .unwrap_or_default();
        println!("  {at:#07x}  {w:08x}{as_float}{mark}");
    }
    Ok(())
}
