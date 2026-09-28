//! For each named object, list every `.abk` export of that name with the project id it cites.
//!
//! The game resolves an object through one `.csi` project, but banks export the same name under
//! other project ids. This shows, per object, which ids the banks use.
//!
//!     cargo run --release --example export_projects -- <audiofiles.big> <name>...

use skate_audio_formats::{banks, eb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: export_projects <archive> <name>...")?;
    let names: Vec<String> = args.collect();
    let data = std::fs::read(&path)?;
    for e in &eb::Archive::parse(&data)?.entries {
        let Some(member) = e.name.as_deref() else {
            continue;
        };
        if !member.ends_with(".abk") {
            continue;
        }
        let Some(bytes) = data.get(e.range()) else {
            continue;
        };
        let Ok(abk) = banks::Abk::parse(bytes) else {
            continue;
        };
        for x in &abk.exports {
            if names.iter().any(|n| n == &x.name) {
                println!(
                    "{:28} {:#06x}:{:#06x} kind {:08x} target {:#x} in {member}",
                    x.name, x.project_id, x.name_id, x.kind, x.target
                );
            }
        }
    }
    Ok(())
}
