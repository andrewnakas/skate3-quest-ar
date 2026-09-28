//! Check the two fixup lists the bank installer `sub_82B1DF50` applies, on every `.abk`.
//!
//! At `+0x30` the header points at `{u32 count, u32 offsets[count]}` whose words the installer turns
//! into code references (`TABLE[word] - address - 4`), and at `+0x34` at a second such list whose
//! words it rebases by the bank's address. If that reading is right, every code-reference word is
//! an opcode index below 40 and every rebased word is an offset inside the bank.
//!
//!     cargo run --release --example verify_bank_fixups -- <archive>

use skate_audio_formats::{banks, eb};
use std::collections::BTreeMap;

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: verify_bank_fixups <archive>")?;
    let data = std::fs::read(&path)?;
    let (mut banks_seen, mut failures) = (0, 0);
    let mut code_counts = BTreeMap::<u32, usize>::new();
    let (mut rebases, mut rebase_in_bank) = (0usize, 0usize);
    let mut rebase_targets = BTreeMap::<&'static str, usize>::new();
    for e in &eb::Archive::parse(&data)?.entries {
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        if !name.ends_with(".abk") {
            continue;
        }
        let b = &data[e.range()];
        let Ok(abk) = banks::Abk::parse(b) else {
            continue;
        };
        banks_seen += 1;
        let code_at = be32(b, 0x30).unwrap() as usize;
        let rebase_at = be32(b, 0x34).unwrap() as usize;
        let code = be32(b, code_at).unwrap_or(u32::MAX);
        *code_counts.entry(code).or_default() += 1;
        for i in 0..code.min(10_000) as usize {
            let off = be32(b, code_at + 4 + 4 * i).unwrap_or(u32::MAX) as usize;
            match be32(b, off) {
                Some(w) if w < 40 => {}
                other => {
                    failures += 1;
                    println!("  FAIL {name}: code reference {i} at {off:#x} holds {other:x?}");
                }
            }
        }
        let n = be32(b, rebase_at).unwrap_or(0) as usize;
        for i in 0..n {
            let off = be32(b, rebase_at + 4 + 4 * i).unwrap_or(u32::MAX) as usize;
            rebases += 1;
            match be32(b, off) {
                Some(w) if (w as usize) < b.len() => {
                    rebase_in_bank += 1;
                    let region = if (w as usize) < abk.sample_bank_offset {
                        "first section"
                    } else if (w as usize) < abk.patch_table_offset {
                        "sample bank"
                    } else {
                        "tables"
                    };
                    *rebase_targets.entry(region).or_default() += 1;
                }
                other => {
                    failures += 1;
                    if failures <= 20 {
                        println!("  FAIL {name}: rebase {i} at {off:#x} holds {other:x?}");
                    }
                }
            }
            if off >= abk.sample_bank_offset {
                failures += 1;
                println!("  FAIL {name}: rebase {i} patches {off:#x}, outside the first section");
            }
        }
    }
    println!("{banks_seen} banks, {failures} failures");
    println!("code-reference counts -> banks: {code_counts:?}");
    println!(
        "rebased words: {rebases}, {rebase_in_bank} inside their bank, by target region {rebase_targets:?}"
    );
    Ok(())
}
