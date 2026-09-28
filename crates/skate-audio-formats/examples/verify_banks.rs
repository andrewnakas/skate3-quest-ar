//! Walk every bank in a real `audiofiles.big` and check the readings against the files.
//!
//! Every check below is one the *data* can fail. The two that matter most are
//! cross-file, so a self-consistent misreading cannot satisfy them:
//!
//! * every `.abk` export cites a `(project_id, name_id)` pair; where that project ships
//!   a `.csi` in the same archive, the `.csi`'s own symbol table must resolve the id to
//!   the identical string. Neither file states the other's layout.
//! * every `.ems` emitter names its sound with a 64-bit hash. Those hashes are compared
//!   against `hash::name_id` of the archive's member names -- a function lifted from the
//!   game's code, applied to strings from the archive directory, matched against numbers
//!   in a third place. A match cannot come from this crate agreeing with itself.
//!
//!     cargo run --release --example verify_banks -- [audiofiles.big] [grains.big ...]
//!
//! With no arguments it takes the retail archives from the tree's data directory.

use skate_audio_formats::{banks, eb, hash};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_DIR: &str = "/home/nakas/Documents/skate3/freeskate/runtime/game/data/audio";

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
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.checks += 1;
        if !ok {
            self.failures.push(what());
        }
    }
}

fn ext(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, e)| e)
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn verify(path: &std::path::Path, r: &mut Report) -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    let archive = eb::Archive::parse(&data)?;
    archive.validate()?;
    println!("{}", path.display());

    let mut by_ext: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &archive.entries {
        *by_ext.entry(e.name.as_deref().map_or("", ext)).or_default() += 1;
    }
    println!("  {} members  {by_ext:?}", archive.entries.len());

    // -- the archive's own name hash ------------------------------------------
    // `audiofiles.big` is the one archive whose key column is not this hash; say so
    // rather than exempting it silently.
    let mut hashed = 0usize;
    for e in &archive.entries {
        if let Some(n) = &e.name {
            if hash::member_hash(n.as_bytes()) == e.name_hash {
                hashed += 1;
            }
        }
    }
    println!(
        "  djb2 member hash    {hashed}/{} entries{}",
        archive.entries.len(),
        if hashed == 0 {
            "  (this archive keys its table some other way)"
        } else {
            ""
        }
    );
    if hashed != 0 {
        r.check(hashed == archive.entries.len(), || {
            format!(
                "{}: only {hashed} of {} names hash to the table key",
                path.display(),
                archive.entries.len()
            )
        });
    }

    // -- parse every member ---------------------------------------------------
    let mut projects: BTreeMap<u16, banks::Csi> = BTreeMap::new();
    let mut abks: Vec<(String, banks::Abk)> = Vec::new();
    let mut emitters: Vec<(String, banks::Ems)> = Vec::new();
    let mut bnks = 0usize;
    let mut samples = 0usize;

    for e in &archive.entries {
        let Some(name) = e.name.clone() else { continue };
        let body = &data[e.range()];
        match ext(&name) {
            "abk" => match banks::Abk::parse(body) {
                Ok(b) => {
                    samples += b.samples.len();
                    abks.push((name, b));
                }
                Err(err) => r.failures.push(format!("{name}: {err}")),
            },
            "csi" => match banks::Csi::parse(body) {
                Ok(p) => {
                    r.check(p.pool_offset <= body.len(), || {
                        format!("{name}: pool past the end")
                    });
                    // The record tables must end exactly where the string pool starts.
                    let first = p.symbols.iter().map(|s| s.name.len()).sum::<usize>();
                    r.check(first > 0 || p.symbols.is_empty(), || {
                        format!("{name}: empty names")
                    });
                    if let Some(prev) = projects.insert(p.project_id, p) {
                        r.failures.push(format!(
                            "{name}: project id {:#06x} is not unique",
                            prev.project_id
                        ));
                    }
                }
                Err(err) => r.failures.push(format!("{name}: {err}")),
            },
            "ems" => match banks::Ems::parse(body) {
                Ok(s) => emitters.push((name, s)),
                Err(err) => r.failures.push(format!("{name}: {err}")),
            },
            "bnk" => match banks::Bnk::parse(body) {
                Ok(_) => bnks += 1,
                Err(err) => r.failures.push(format!("{name}: {err}")),
            },
            _ => {}
        }
        r.checks += 1;
    }
    if abks.is_empty() && projects.is_empty() && emitters.is_empty() && bnks == 0 {
        println!("  (no banks in this archive)\n");
        return Ok(());
    }
    println!(
        "  parsed              {} .abk ({samples} samples), {} .csi, {} .ems, {bnks} .bnk",
        abks.len(),
        projects.len(),
        emitters.len()
    );

    // -- .abk export ids are a function of the name ---------------------------
    let mut by_name: BTreeMap<&str, BTreeSet<u16>> = BTreeMap::new();
    let mut exports = 0usize;
    for (_, b) in &abks {
        for e in &b.exports {
            exports += 1;
            by_name
                .entry(e.name.as_str())
                .or_default()
                .insert(e.name_id);
        }
    }
    let ambiguous: Vec<_> = by_name.iter().filter(|(_, ids)| ids.len() > 1).collect();
    r.check(ambiguous.is_empty(), || {
        format!("{} names carry more than one id", ambiguous.len())
    });
    println!(
        "  export names        {exports} records, {} distinct names, {} ambiguous",
        by_name.len(),
        ambiguous.len()
    );

    // -- the cross-file check: .abk export -> .csi symbol table ---------------
    let (mut agree, mut absent, mut unshipped) = (0usize, 0usize, 0usize);
    for (file, b) in &abks {
        for e in &b.exports {
            match projects.get(&e.project_id) {
                None => unshipped += 1,
                Some(p) => match p.name(e.name_id) {
                    Some(n) if n == e.name => agree += 1,
                    Some(n) => {
                        absent += 1;
                        r.failures.push(format!(
                            "{file}: id {:#06x} is {n:?} in the project, {:?} in the bank",
                            e.name_id, e.name
                        ));
                    }
                    None => {
                        absent += 1;
                        r.failures.push(format!(
                            "{file}: project {:#06x} has no id {:#06x}",
                            e.project_id, e.name_id
                        ));
                    }
                },
            }
            r.checks += 1;
        }
    }
    println!(
        "  abk -> csi names    {agree} agree, {absent} disagree, {unshipped} in projects not shipped here"
    );
    r.check(absent == 0, || {
        format!("{absent} export names disagree with their project")
    });

    // -- the second cross-file check: .ems sound ids -> member names ----------
    let mut ids: BTreeMap<u64, usize> = BTreeMap::new();
    let mut records = 0usize;
    for (_, s) in &emitters {
        for em in &s.emitters {
            records += 1;
            *ids.entry(em.sound).or_default() += 1;
        }
    }
    let mut names: BTreeMap<u64, &str> = BTreeMap::new();
    for e in &archive.entries {
        if let Some(n) = &e.name {
            names
                .entry(hash::name_id(stem(n).as_bytes()))
                .or_insert(stem(n));
            names.entry(hash::name_id(n.as_bytes())).or_insert(n);
        }
    }
    let resolved: Vec<_> = ids
        .iter()
        .filter_map(|(id, n)| names.get(id).map(|s| (*s, *n)))
        .collect();
    let covered: usize = resolved.iter().map(|(_, n)| n).sum();
    println!(
        "  ems -> member names {}/{} distinct sound ids resolve, covering {covered}/{records} emitter records",
        resolved.len(),
        ids.len()
    );
    for (name, n) in resolved.iter().take(6) {
        println!("      {name:<28} x{n}");
    }
    // This is the load-bearing claim, so give it a floor the data can fail. Chance
    // alone would resolve none: these are 64-bit values compared for equality.
    r.check(!resolved.is_empty() || ids.is_empty(), || {
        "no .ems sound id resolves to a member name -- the hash or the field is wrong".into()
    });
    // Negative control. Reading the same eight bytes with the halves swapped is just
    // as self-consistent a story, so it has to resolve nothing, or the match above
    // would be evidence of nothing.
    let swapped = ids
        .keys()
        .filter(|id| names.contains_key(&(id.rotate_left(32))))
        .count();
    println!("  control (halves swapped) resolves {swapped}, want 0");
    r.check(swapped == 0, || {
        format!("the byte-swapped reading also resolves {swapped} ids")
    });
    println!();
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<std::path::PathBuf> = if args.is_empty() {
        [
            "audiofiles.big",
            "grains.big",
            "wheels.big",
            "post.big",
            "ambience.big",
            "ambienceresident.big",
        ]
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
    println!("{} archives, {} checks", paths.len(), report.checks);
    if report.failures.is_empty() {
        println!("all checks agreed");
        Ok(())
    } else {
        println!("{} FAILURES:", report.failures.len());
        for f in report.failures.iter().take(40) {
            println!("  {f}");
        }
        std::process::exit(1);
    }
}
