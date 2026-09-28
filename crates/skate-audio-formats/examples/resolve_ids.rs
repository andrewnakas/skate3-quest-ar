//! Resolve 64-bit engine name ids against every printable string in a dumped guest image.
//!
//! Code builds these ids as `lis`/`ori`/`rldimi` constants, so a tuning lookup or an object check
//! reads as an opaque number. Hashing each string of the decrypted image with
//! [`hash::name_id`] and matching is how a number becomes a name.
//!
//!     cargo run --release --example resolve_ids -- <image dir> <hex id | =name>...
//!
//! A `=name` argument hashes that name and looks for its id, which is the positive control: a
//! negative result means nothing until a name known to be in the image resolves.

use skate_audio_formats::hash;
use std::collections::HashSet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .ok_or("usage: resolve_ids <image dir> <hex id>...")?;
    let mut wanted = HashSet::new();
    for arg in args {
        if let Some(name) = arg.strip_prefix('=') {
            let id = hash::name_id(name.as_bytes());
            println!("control {name:?} -> {id:016x}");
            wanted.insert(id);
        } else {
            wanted.insert(u64::from_str_radix(arg.trim_start_matches("0x"), 16)?);
        }
    }
    let mut found = HashSet::new();
    let mut strings = 0usize;
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bin") {
            continue;
        }
        let data = std::fs::read(&path)?;
        for run in data.split(|&b| !(0x20..0x7F).contains(&b)) {
            if run.len() < 3 {
                continue;
            }
            // A run may hold several names separated by bytes that happen to be printable, so
            // every suffix that starts a word is tried as well as the whole run.
            for start in 0..run.len() {
                if start > 0 && run[start - 1].is_ascii_alphanumeric() {
                    continue;
                }
                let s = &run[start..];
                strings += 1;
                let id = hash::name_id(s);
                if wanted.contains(&id) && found.insert((id, s.to_vec())) {
                    println!(
                        "{id:016x} = {:?}  ({})",
                        String::from_utf8_lossy(s),
                        path.display()
                    );
                }
            }
        }
    }
    let resolved: HashSet<u64> = found.iter().map(|(id, _)| *id).collect();
    for id in &wanted {
        if !resolved.contains(id) {
            println!("{id:016x} unresolved");
        }
    }
    println!("{strings} candidate strings hashed");
    Ok(())
}
