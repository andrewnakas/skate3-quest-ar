//! Walk every record of every real `.mpf` and check the readings against the file.
//!
//! Each check below is one the *file* can fail: a count the header states against the
//! bytes a section actually holds, a record length against the length the guest's own
//! expression predicts, an index against the section it must land in. Nothing here is a
//! round trip through this crate's own writer, because there isn't one -- a wrong format
//! assumption survives that kind of test unharmed.
//!
//! The strongest check is the last: section 8's record count against the companion
//! `.mus`'s segment count, read by an independent parser (`mus.rs`) from a different
//! file. That one cannot be satisfied by a self-consistent misreading.
//!
//!     cargo run --release --example verify_mpf -- <file.mpf> [...]
//!
//! With no arguments it takes the three retail maps from the tree's data directory.

use skate_audio_formats::{mpf, mus};

const DEFAULT_DIR: &str = "/home/nakas/Documents/skate3/freeskate/runtime/game/data/audio/music";

struct Report {
    checks: usize,
    failures: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Self {
            checks: 0,
            failures: Vec::new(),
        }
    }
    /// Record one check. `what` is printed on failure only.
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.checks += 1;
        if !ok {
            self.failures.push(what());
        }
    }
}

fn companion_mus(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let mut name = String::new();
    let mut chars = stem.chars();
    name.push(chars.next()?.to_ascii_uppercase());
    name.extend(chars);
    name.push_str("_Stream.mus");
    let candidate = path.with_file_name(name);
    candidate.exists().then_some(candidate)
}

fn verify(path: &std::path::Path, r: &mut Report) -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    let map = mpf::Map::parse(&data)?;
    let h = &map.header;

    println!("{}", path.display());
    println!("  size                {} ({:#x})", data.len(), data.len());
    println!(
        "  version {}.{}   0x06 {:#06x}   map type {}   0x0E {}   0x10 {}",
        h.version.0, h.version.1, h.unknown_06, h.map_type, h.unknown_0e, h.unknown_10
    );
    for (i, s) in h.sections.iter().enumerate() {
        println!(
            "  section {i}          {:#08x}..{:#08x}  {:>6} bytes",
            s.start,
            s.end,
            s.len()
        );
    }

    // -- header counts against the sections they describe ---------------------
    let s0 = h.section(0);
    let pad0 = s0.len() - 2 * h.node_count as usize;
    r.check(pad0 <= 2, || {
        format!("section 0 pad is {pad0} bytes, want 0 or 2")
    });
    let s2 = h.section(2);
    let pad2 = s2.len() - 2 * h.script_count as usize;
    r.check(pad2 <= 2, || {
        format!("section 2 pad is {pad2} bytes, want 0 or 2")
    });
    r.check(h.section(4).len() == 20 * h.variable_count as usize, || {
        format!(
            "section 4 is {} bytes for {} variables",
            h.section(4).len(),
            h.variable_count
        )
    });
    r.check(h.section(5).len() == 4, || {
        "section 5 is not one u32".into()
    });
    r.check(h.section(6).len() == 4 * h.mus_link_count as usize, || {
        format!(
            "section 6 is {} bytes for {} links",
            h.section(6).len(),
            h.mus_link_count
        )
    });
    r.check(h.section(7).len() == 20 * h.mus_link_count as usize, || {
        format!(
            "section 7 is {} bytes for {} links",
            h.section(7).len(),
            h.mus_link_count
        )
    });
    r.check(h.section(8).len() % 8 == 0, || {
        "section 8 is not a multiple of 8".into()
    });
    println!(
        "  counts: nodes {}(+{} pad) scripts {}(+{} pad) variables {} mus links {} tempo {}",
        h.node_count,
        pad0,
        h.script_count,
        pad2,
        h.variable_count,
        h.mus_link_count,
        map.tempo.len()
    );

    // -- section 0 -> section 1 -----------------------------------------------
    let index = mpf::node_index(&data, h)?;
    r.check(index.first() == Some(&h.section(1).start), || {
        format!(
            "section 0's first entry is {:#x}, not section 1's start",
            index[0]
        )
    });
    let ascending = index.windows(2).all(|w| w[0] < w[1]);
    r.check(ascending, || "section 0 is not strictly ascending".into());
    // `mpf::nodes` already required each record to fill its index slot exactly, so
    // reaching here means the whole of section 1 is accounted for. Re-state it as a sum.
    let covered: usize = map.nodes.iter().map(|n| n.len).sum();
    r.check(covered == h.section(1).len(), || {
        format!(
            "nodes cover {covered} bytes of section 1's {}",
            h.section(1).len()
        )
    });
    // Every branch target is a node id or an explicit "none".
    let mut branches = 0usize;
    let mut dead_ends = 0usize;
    let mut contiguous = 0usize;
    let mut with_branches = 0usize;
    for (id, n) in map.nodes.iter().enumerate() {
        for b in &n.branches {
            branches += 1;
            if b.next < 0 {
                dead_ends += 1;
            } else {
                r.check((b.next as usize) < map.nodes.len(), || {
                    format!(
                        "node {id} branches to {}, past {} nodes",
                        b.next,
                        map.nodes.len()
                    )
                });
            }
        }
        if !n.branches.is_empty() {
            with_branches += 1;
            if n.branches.windows(2).all(|w| w[0].hi == w[1].lo) {
                contiguous += 1;
            }
        }
    }
    println!(
        "  nodes               {} records, {} branches ({} to no successor)",
        map.nodes.len(),
        branches,
        dead_ends
    );
    println!("  branch ranges       {contiguous}/{with_branches} records have contiguous lo..hi");

    // -- node.segment against the tempo table ---------------------------------
    let mut seen = vec![false; map.tempo.len()];
    let (mut playable, mut control) = (0usize, 0usize);
    for (id, n) in map.nodes.iter().enumerate() {
        if n.plays_segment() {
            playable += 1;
            let i = n.segment as usize;
            r.check(i <= map.tempo.len(), || {
                format!(
                    "node {id} names segment {i}, past {} tempo records",
                    map.tempo.len()
                )
            });
            if i <= map.tempo.len() {
                seen[i - 1] = true;
            }
            r.check(n.meter == 4, || {
                format!("node {id} plays a segment but meter is {}", n.meter)
            });
        } else {
            control += 1;
        }
    }
    let covered_segments = seen.iter().filter(|s| **s).count();
    r.check(covered_segments == map.tempo.len(), || {
        format!(
            "nodes reference {covered_segments} of {} segments",
            map.tempo.len()
        )
    });
    println!(
        "  segment references  {playable} playable nodes, {control} control nodes; \
         they cover {covered_segments}/{} tempo records",
        map.tempo.len()
    );
    let ascending = map.tempo.windows(2).all(|w| w[0].position < w[1].position);
    r.check(ascending, || {
        "section 8 position is not strictly ascending".into()
    });

    // -- section 2 -> section 3 -----------------------------------------------
    let sindex = mpf::script_index(&data, h)?;
    r.check(sindex.first() == Some(&h.section(3).start), || {
        format!(
            "section 2's first entry is {:#x}, not section 3's start",
            sindex[0]
        )
    });
    let covered: usize = map.scripts.iter().map(|s| s.len).sum();
    r.check(covered == h.section(3).len(), || {
        format!(
            "scripts cover {covered} bytes of section 3's {}",
            h.section(3).len()
        )
    });
    let mut events = 0usize;
    let mut opcodes = std::collections::BTreeMap::<u8, usize>::new();
    for (i, s) in map.scripts.iter().enumerate() {
        for e in &s.events {
            events += 1;
            *opcodes.entry(e.opcode()).or_default() += 1;
            r.check((1..=18).contains(&e.opcode()), || {
                format!(
                    "script {i} has opcode {}, outside the guest's 1..=18",
                    e.opcode()
                )
            });
            r.check(e.word & 0x0001_0000 == 0, || {
                format!("script {i} has the runtime 'handled' bit set in the file")
            });
        }
    }
    println!(
        "  scripts             {} records, {events} events",
        map.scripts.len()
    );
    println!(
        "  opcodes             {}",
        opcodes
            .iter()
            .map(|(k, v)| format!("{k}x{v}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    // -- sections 4-7 ---------------------------------------------------------
    let s6 = mpf::section6_pointer(&data, h)?;
    r.check(s6 == h.section(6).start, || {
        format!(
            "section 5 points at {s6:#x}, not section 6's start {:#x}",
            h.section(6).start
        )
    });
    println!(
        "  variables           {}",
        map.variables
            .iter()
            .map(|v| format!("{}={}", v.name, v.value))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for v in &map.variables {
        r.check(!v.name.is_empty() && v.name.is_ascii(), || {
            format!("variable name is not printable ASCII: {:02x?}", v.raw_name)
        });
    }
    for (i, l) in map.mus_links.iter().enumerate() {
        println!(
            "  mus link {i}          checksum {:#010x}  offset {:#x}",
            l.checksum, l.mus_offset
        );
    }

    // -- the cross-file check -------------------------------------------------
    match companion_mus(path) {
        Some(p) => {
            let mdata = std::fs::read(&p)?;
            let mh = mus::Header::parse(&mdata)?;
            r.check(mh.segment_count as usize == map.tempo.len(), || {
                format!(
                    "{}: {} segments but the map has {} tempo records",
                    p.display(),
                    mh.segment_count,
                    map.tempo.len()
                )
            });
            println!(
                "  companion .mus      {} -> {} segments vs {} tempo records  {}",
                p.file_name().unwrap().to_string_lossy(),
                mh.segment_count,
                map.tempo.len(),
                if mh.segment_count as usize == map.tempo.len() {
                    "AGREE"
                } else {
                    "DISAGREE"
                }
            );
        }
        None => println!("  companion .mus      NOT FOUND -- the cross-file check did not run"),
    }
    println!();
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<std::path::PathBuf> = if args.is_empty() {
        ["game.mpf", "ipod.mpf", "world.mpf"]
            .iter()
            .map(|n| std::path::Path::new(DEFAULT_DIR).join(n))
            .collect()
    } else {
        args.iter().map(std::path::PathBuf::from).collect()
    };

    let mut report = Report::new();
    for p in &paths {
        if let Err(e) = verify(p, &mut report) {
            report.failures.push(format!("{}: {e}", p.display()));
            report.checks += 1;
        }
    }

    println!("---");
    println!("{} files, {} checks", paths.len(), report.checks);
    if report.failures.is_empty() {
        println!("all checks agreed");
        Ok(())
    } else {
        println!("{} FAILURES:", report.failures.len());
        for f in &report.failures {
            println!("  {f}");
        }
        std::process::exit(1);
    }
}
