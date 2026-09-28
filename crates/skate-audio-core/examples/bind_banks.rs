//! Install every shipped `.csi` and resolve every `.abk` export through `symbols`, the way the bank
//! installer `sub_82B1DF50` does, then check each outcome against an independent search of the
//! parsed files.
//!
//! The installer builds each query from the export record (`{u16 project, u16 name_id, name}`) as
//! `{name*, project, name_id}` and resolves it into the slot at the export's target, choosing the
//! lookup by the kind word's top byte: 0 is table 2, 1 is table 1, anything else table 0. The
//! independent answer: the export binds iff some project has a symbol in that table with the same
//! name id and name. It binds on the first pass iff one of those projects has the export's own
//! project id. The record it lands on is the first match in list order, newest project first.
//!
//!     cargo run --release --example bind_banks -- <audiofiles.big>

use skate_audio_core::{Guest, symbols};
use skate_audio_formats::{banks, eb};

const ARENA: u32 = 0x5000_0000;
const QUERY: u32 = 0x4000_0000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: bind_banks <audiofiles.big>")?;
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    let mut g = Guest::single(QUERY, 0x200);
    g.put(symbols::PROJECT_LIST_HEAD, vec![0; 4]);
    g.put(symbols::GENERATION, vec![0; 2]);
    let mut next = ARENA;
    let mut place = |g: &mut Guest, bytes: &[u8]| {
        let at = next;
        g.put(at, bytes.to_vec());
        next = (at + bytes.len() as u32 + 0x1000) & !0xFFF;
        at
    };

    // Projects, installed in archive order: list order is the reverse.
    let mut projects: Vec<(u32, banks::Csi)> = Vec::new();
    for e in &archive.entries {
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        if name.ends_with(".csi") {
            let bytes = &data[e.range()];
            let csi = banks::Csi::parse(bytes)?;
            let at = place(&mut g, bytes);
            symbols::install_project(&mut g, at)?;
            projects.push((at, csi));
        }
    }
    projects.reverse();

    let (mut exports, mut first, mut second, mut missing, mut disagree) = (0, 0, 0, 0, 0);
    let player = [
        "Class_grind",
        "Class_Flips",
        "c_board_slide",
        "Class_rolling",
        "playercharacter_footstep",
        "c_body_slide",
        "Class_wheels_skid",
        "Class_foot_drag",
    ];
    for e in &archive.entries {
        let Some(member) = e.name.as_deref() else {
            continue;
        };
        if !member.ends_with(".abk") {
            continue;
        }
        let bytes = &data[e.range()];
        let Ok(abk) = banks::Abk::parse(bytes) else {
            continue;
        };
        let bank = place(&mut g, bytes);
        for x in &abk.exports {
            exports += 1;
            let table = match x.kind >> 24 {
                0 => 2u8,
                1 => 1,
                _ => 0,
            };
            g.set_u32(QUERY, bank + x.record_offset as u32 + 4)?;
            g.set_u16(QUERY + 4, x.project_id)?;
            g.set_u16(QUERY + 6, x.name_id)?;
            let slot = bank + x.target;
            let got = match table {
                2 => symbols::lookup_table2(&mut g, slot, QUERY)?,
                1 => symbols::lookup_table1(&mut g, slot, QUERY)?,
                _ => symbols::lookup_table0(&mut g, slot, QUERY)?,
            };

            // The independent answer, from the parsed files alone.
            let record_of = |csi: &banks::Csi, at: u32, i: usize| -> u32 {
                let c = csi.group_counts;
                let base = at + 40;
                match table {
                    0 => base + 12 * i as u32,
                    1 => base + 12 * c[0] as u32 + 12 * i as u32,
                    _ => base + 12 * (c[0] as u32 + c[1] as u32) + 16 * i as u32,
                }
            };
            let find = |same_project: bool| -> Option<u32> {
                for (at, csi) in &projects {
                    if same_project && csi.project_id != x.project_id {
                        continue;
                    }
                    let mut i = 0;
                    for s in &csi.symbols {
                        if s.group != table {
                            continue;
                        }
                        if s.id == x.name_id && s.name == x.name {
                            return Some(record_of(csi, *at, i));
                        }
                        i += 1;
                    }
                }
                None
            };
            let expected = find(true)
                .map(|r| (r, false))
                .or_else(|| find(false).map(|r| (r, true)));
            let actual = (got.status == 0).then(|| (g.u32(slot).unwrap(), got.second_pass));
            if expected != actual {
                disagree += 1;
                if disagree <= 10 {
                    println!(
                        "  DISAGREE {member} {}: expected {expected:x?}, got {actual:x?}",
                        x.name
                    );
                }
            }
            match actual {
                Some((_, false)) => first += 1,
                Some((_, true)) => second += 1,
                None => {
                    missing += 1;
                    println!(
                        "  not found: {member} {} {:#06x}:{:#06x} table {table}",
                        x.name, x.project_id, x.name_id
                    );
                }
            }
            if player.contains(&x.name.as_str()) {
                println!(
                    "  {member:34} {:26} {:#06x}:{:#06x} table {table}: {}",
                    x.name,
                    x.project_id,
                    x.name_id,
                    match actual {
                        Some((_, false)) => "first pass",
                        Some((_, true)) => "second pass",
                        None => "NOT FOUND",
                    }
                );
            }
        }
    }
    println!(
        "{} projects installed; {exports} exports: {first} first pass, {second} second pass, {missing} not found; {disagree} disagree with the independent search",
        projects.len()
    );
    Ok(())
}
