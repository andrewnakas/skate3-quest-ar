//! Inventory the banks that sound like the player character: which `.abk` members export the
//! player-side object names, how many samples each holds, and what those samples are. Also dumps the
//! head of every `.grain` member, a format nothing here decodes yet.
//!
//! Not a unit test: it needs the user's own copy of the game.
//!
//!     cargo run --release --example player_sounds -- [audio dir]

use skate_audio_formats::{banks, eaac, eb};

const DEFAULT_DIR: &str = "/home/nakas/Documents/skate3/freeskate/runtime/game/data/audio";

/// Object and port names the executable's string pool ties to the skater, and member-name fragments
/// that suggest the same. A match is a lead to check by ear, not a classification.
const PLAYER_EXPORTS: &[&str] = &[
    "Class_rolling",
    "c_board_slide",
    "Class_grind",
    "c_body_slide",
    "cloth_",
    "playercharacter",
    "Class_foot_drag",
    "Grab_",
    "Grit",
];
const PLAYER_MEMBERS: &[&str] = &[
    "patchbank",
    "grind",
    "board",
    "bodyslide",
    "cloth",
    "foot",
    "bail",
    "land",
    "pop",
    "ollie",
    "slide",
    "wheel",
    "roll",
    "trick",
    "flip",
    "impact",
    "body",
    "skate",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_DIR.to_string());
    let path = format!("{dir}/audiofiles.big");
    let data = std::fs::read(&path)?;
    let archive = eb::Archive::parse(&data)?;
    let mut banks_seen = 0;
    let mut player_banks = 0;
    let mut player_samples = 0;
    for entry in &archive.entries {
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".abk") {
            continue;
        }
        let Some(bytes) = data.get(entry.range()) else {
            continue;
        };
        let Ok(abk) = banks::Abk::parse(bytes) else {
            continue;
        };
        banks_seen += 1;
        let lower = name.to_ascii_lowercase();
        let export_hit: Vec<&str> = abk
            .exports
            .iter()
            .map(|e| e.name.as_str())
            .filter(|n| PLAYER_EXPORTS.iter().any(|p| n.contains(p)))
            .collect();
        let member_hit = PLAYER_MEMBERS.iter().any(|p| lower.contains(p));
        if export_hit.is_empty() && !member_hit {
            continue;
        }
        player_banks += 1;
        let present = abk.present();
        player_samples += present;
        let mut total_secs = 0.0;
        let mut formats = std::collections::BTreeMap::new();
        for i in 0..present {
            if let Some(range) = abk.sample_range(i) {
                if let Ok(h) = eaac::Header::parse(&bytes[range], 0) {
                    total_secs += h.duration_secs();
                    *formats
                        .entry(format!(
                            "{:?} {}ch {}Hz",
                            h.codec,
                            h.channels(),
                            h.sample_rate
                        ))
                        .or_insert(0) += 1;
                }
            }
        }
        let mut exports: Vec<&str> = abk.exports.iter().map(|e| e.name.as_str()).collect();
        exports.sort_unstable();
        exports.dedup();
        println!(
            "{name:32.32} {present:>4} samples {total_secs:>7.2}s {formats:?}\n    exports: {}",
            exports.join(" ")
        );
    }
    println!(
        "\n{player_banks} of {banks_seen} banks look player-side, holding {player_samples} samples"
    );

    let grains = std::fs::read(format!("{dir}/grains.big"))?;
    let grain_archive = eb::Archive::parse(&grains)?;
    println!("\ngrains.big, the head of every member:");
    for entry in &grain_archive.entries {
        let bytes = &grains[entry.range()];
        let words: Vec<String> = bytes
            .chunks(4)
            .take(9)
            .map(|w| w.iter().map(|b| format!("{b:02x}")).collect::<String>())
            .collect();
        println!(
            "  {:30.30} {:>7}  {}",
            entry.name.as_deref().unwrap_or("?"),
            bytes.len(),
            words.join(" ")
        );
    }
    Ok(())
}
