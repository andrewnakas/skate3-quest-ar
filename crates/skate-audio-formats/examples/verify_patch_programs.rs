//! Walk every input record's evaluator program in every `.abk`, and check it against the reading of
//! the interpreter `sub_82B1E290` and the instance allocator `sub_82B1D880`.
//!
//! The allocator copies the record's template (`+0x2C`, `+0x30` bytes) into a new instance and links
//! `instance + 8` onto the interpreter's list as a node `{+0 next, +8 program, +12 operand block}`,
//! where the program is the record's `+0x28` and the operand block is `instance + 24`. A program is
//! a stream of `{u8 opcode, u8 pairs, u16 unread, pairs x {u32 src, u32 dst}, u32 block_advance}`
//! ended by opcode 255. The op at each record gets the current block pointer, and the pairs store
//! or copy words at block offsets; then the block pointer moves by `block_advance`.
//!
//! If that is right, every opcode is below 40, every stream ends inside the first section, and
//! the block pointer never leaves the template's operand area.
//!
//!     cargo run --release --example verify_patch_programs -- <archive> [bank substring...]

use skate_audio_formats::{banks, eb};
use std::collections::BTreeMap;

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes(s.try_into().unwrap()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: verify_patch_programs <archive> [bank...]")?;
    let focus: Vec<String> = args.collect();
    let data = std::fs::read(&path)?;
    let (mut programs, mut ops_total, mut failures) = (0usize, 0usize, 0usize);
    let mut hist: BTreeMap<u8, usize> = BTreeMap::new();
    let mut focus_hist: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    let mut max_ops = 0usize;
    let mut shapes = BTreeMap::<(Vec<u8>, Vec<u8>), usize>::new();
    let mut unused_tail = BTreeMap::<i64, usize>::new();
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
        let focused = focus.iter().any(|f| name.contains(f.as_str()));
        let count = be16(b, 0x0A).unwrap_or(0) as usize;
        let mut rec = be32(b, 0x1C).unwrap_or(0) as usize;
        for r in 0..count {
            let program = be32(b, rec + 40).unwrap_or(0) as usize;
            let size = be32(b, rec + 48).unwrap_or(0) as i64;
            let block_area = size - 24;
            let mut at = program;
            let mut block: i64 = 0;
            let mut n = 0usize;
            let mut seq: Vec<u8> = Vec::new();
            let mut fail = |why: String| {
                failures += 1;
                if failures <= 25 {
                    println!("  FAIL {name} record {r}: {why}");
                }
            };
            programs += 1;
            loop {
                if at >= abk.sample_bank_offset {
                    fail(format!("program runs past the first section at {at:#x}"));
                    break;
                }
                let op = b[at];
                if op == 255 {
                    break;
                }
                if op >= 40 {
                    fail(format!("opcode {op} at {at:#x}"));
                    break;
                }
                let pairs = b[at + 1] as usize;
                for p in 0..pairs {
                    let src = be32(b, at + 4 + 8 * p).unwrap_or(0) as i32 as i64;
                    let dst = be32(b, at + 8 + 8 * p).unwrap_or(0) as i32 as i64;
                    // Offsets are relative to the current block and may point back at earlier ones.
                    let outside = |o: i64| block + o < 0 || block + o + 4 > block_area;
                    if outside(dst) || (src != -1 && outside(src)) {
                        fail(format!(
                            "op {op} pair {p} src {src} dst {dst} at block {block} outside {block_area}"
                        ));
                    }
                }
                let advance = be32(b, at + 4 + 8 * pairs).unwrap_or(0) as i32 as i64;
                block += advance;
                if block < 0 || block > block_area {
                    fail(format!(
                        "block pointer {block} leaves the operand area {block_area} after op {op}"
                    ));
                    break;
                }
                *hist.entry(op).or_default() += 1;
                seq.push(op);
                if focused {
                    let v = focus_hist.entry(op).or_default();
                    let short = name.trim_end_matches(".abk").to_string();
                    if !v.contains(&short) {
                        v.push(short);
                    }
                }
                n += 1;
                at += 8 + 8 * pairs;
            }
            ops_total += n;
            let head: Vec<u8> = seq.iter().take(4).copied().collect();
            let tail: Vec<u8> = seq.iter().rev().take(2).rev().copied().collect();
            *shapes.entry((head, tail)).or_default() += 1;
            max_ops = max_ops.max(n);
            *unused_tail
                .entry((block_area - block).signum())
                .or_default() += 1;
            let entries = b[rec + 36] as usize + b[rec + 39] as usize;
            rec += 60 + 4 * entries;
        }
    }
    println!("{programs} programs, {ops_total} ops (longest {max_ops}), {failures} failures");
    println!(
        "block pointer at the end vs the operand area (-1 past, 0 exactly, 1 short): {unused_tail:?}"
    );
    println!("opcode histogram: {hist:?}");
    let mut common: Vec<_> = shapes.iter().collect();
    common.sort_by(|a, b| b.1.cmp(a.1));
    println!("most common (first four ops, last two ops):");
    for ((h, t), c) in common.iter().take(6) {
        println!("  {c:4} x first {h:?} last {t:?}");
    }
    if !focus.is_empty() {
        println!("opcodes used by the focused banks:");
        for (op, names) in &focus_hist {
            println!("  {op:2}: {}", names.join(", "));
        }
    }
    Ok(())
}
