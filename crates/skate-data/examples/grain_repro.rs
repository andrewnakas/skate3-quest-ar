//! Headless check of the retail grain player (board rolling) against the owner's assets: builds the
//! real `AuthoredRuntime`, loads a surface's `.grain` member from `grains.big`, creates the board's
//! players (A and B per truck), binds them with the vault `GrainParams`, and drives the per-frame
//! record from `grain::board::board_records` at a few constant speeds with constant MixMap inputs
//! (the retail capture's medians for that speed; override with MIX1 / MIX2 / MIXPITCH).
//!
//! Prints per-second peak/RMS dBFS, the grain start sequence, live voices and render ms per block,
//! and checks continuity: no silent block while the A gain is above zero.
//!
//!     cargo run --release -p skate-data --example grain_repro -- <assets> [seconds per speed] [speeds m/s, comma separated]
//!
//! Environment: SURFACE (default 2, concrete_rough), SOFT=1 for the soft member, TRUCKS (1 or 2,
//! default 2), WAV=<path> to write the 48 kHz six-channel float output, BOARD_TSV=<path> to replay
//! the capture-joined board inputs through `board_records` and compare with the captured records.
//! Each player's voices go through its retail bus chain (`grain::chain`, local player) with the
//! capture's modal chain inputs pushed every frame; CHAIN=0 sends them to the default bus instead.
use skate_audio_core::authored::AuthoredRuntime;
use skate_audio_core::grain::board::{self, BoardInputs, ChainInputs, GrainRecord};
use skate_data::audio::catalog::PlayerAudioCatalog;
use skate_data::audio::grains::{GrainVault, load_grains};
use std::path::PathBuf;

const BLOCKS_PER_SECOND: f64 = 48_000.0 / 256.0;

/// Median MixMap reads (`vfunc60(1)`, `vfunc60(2)`, `vfunc56(3)`) by the local board in the retail
/// capture, by ground speed rounded to m/s (`.local/captures`, 18,544 board updates).
fn capture_mix(speed: f32) -> (i32, i32, i32) {
    match speed.round() as i32 {
        i32::MIN..=0 => (0, 0, 3209),
        1 => (3770, 14253, 3500),
        2 => (2336, 17476, 4173),
        3 => (6337, 20485, 3970),
        4 => (5873, 19008, 4031),
        5 | 6 => (6681, 21475, 4058),
        7 => (6720, 21426, 4086),
        8 => (7351, 23038, 4086),
        9 => (3565, 20215, 4513),
        _ => (4422, 13488, 4079),
    }
}

fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let assets = PathBuf::from(args.next().ok_or("need assets directory")?);
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let speeds: Vec<f32> = args
        .next()
        .unwrap_or_else(|| "2,4,6,8".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let surface: u32 = env("SURFACE").unwrap_or(2);
    let soft = std::env::var_os("SOFT").is_some();
    let trucks: u32 = env::<u32>("TRUCKS").unwrap_or(2).clamp(1, 2);
    let use_chain = env::<u32>("CHAIN").unwrap_or(1) != 0;
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Skate3RustEngine/audio-pcm-cache"));

    let choice = board::grain_for_surface(surface, soft).ok_or("surface has no grain")?;
    let vault = GrainVault::load(&assets)?;
    let tuning = vault.tuning(choice.key)?;
    eprintln!(
        "surface {surface} {} -> {} (vault key {:016X}), max {} km/h, GrainParams {:?}",
        if soft { "soft" } else { "hard" },
        choice.member,
        choice.key,
        tuning.max_kmh,
        tuning.params
    );
    assert_eq!(
        tuning.grain, choice.member,
        "vault filename and sub_824C8370 disagree"
    );

    if let Some(path) = std::env::var_os("BOARD_TSV") {
        return validate_board(&vault, &PathBuf::from(path));
    }

    let catalog = PlayerAudioCatalog::from_assets(&assets, cache.as_deref())?;
    let mut runtime = AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks)?;
    for sample in catalog.samples {
        let base = runtime.bank_base(&sample.bank).ok_or("missing bank")?;
        runtime.insert_pcm(base + sample.header_offset, sample.pcm)?;
    }
    let archive = assets.join("private/stock/data/audio/grains.big");
    let members = load_grains(&archive, &[choice.member], cache.as_deref())?;
    let member = &members[0];
    eprintln!(
        "{}: {} bytes, {} frames at {} Hz",
        member.name,
        member.bytes.len(),
        member.samples.len() / usize::from(member.channels),
        member.rate
    );

    let mut players = Vec::new();
    {
        let mut grains = runtime.grains();
        grains.load(
            &member.name,
            &member.bytes,
            member.samples.clone(),
            member.channels,
            member.rate,
        )?;
        let default_bus = grains.default_bus()?;
        let chain = vault.chain_config(true)?;
        for _ in 0..trucks {
            for which in 0..2 {
                let player = grains.create_player()?;
                let bus = if use_chain {
                    grains.chain(player, &chain)?
                } else {
                    default_bus
                };
                grains.bind(player, &member.name, tuning.params[which], bus)?;
                players.push(player);
            }
        }
    }

    if use_chain {
        let grains = runtime.grains();
        let record = grains.chain_record(players[0]).ok_or("no chain")?;
        for (name, table_at, graph_at) in [
            ("graph 1", 0u32, 4u32),
            ("graph 2", 8, 12),
            ("graph 3", 16, 20),
        ] {
            let graph = grains.g.u32(record + graph_at)?;
            if graph == 0 {
                continue;
            }
            let table = grains.g.u32(record + table_at)?;
            let count = grains.g.u8(graph + 68)?;
            let ids: Vec<String> = (0..u32::from(count))
                .map(|i| {
                    let module = grains.g.u32(table + 4 * i).unwrap();
                    let class = grains.g.u32(module + 20).unwrap();
                    let id = grains.g.u32(class + 36).unwrap().to_be_bytes();
                    format!(
                        "{}({}ch)",
                        String::from_utf8_lossy(&id),
                        grains.g.u8(module + 42).unwrap()
                    )
                })
                .collect();
            println!(
                "player 0 chain {name} (order {}): {}",
                grains.g.u8(graph + 73)?,
                ids.join(" -> ")
            );
        }
    }

    // The capture's first bind: three blocks later the attack timer reads 0x3DAC0831.
    for _ in 0..3 {
        runtime.pump_once()?;
    }
    let first = runtime.grains().view(players[0])?;
    println!(
        "after bind + 3 blocks: slot0 timer {:#010x} state {} (retail capture 0x3dac0831 state 1): {}",
        first.slots[0].timer.to_bits(),
        first.slots[0].state,
        if first.slots[0].timer.to_bits() == 0x3DAC_0831 {
            "match"
        } else {
            "MISMATCH"
        }
    );

    let mut wav: Vec<f32> = Vec::new();
    let write_wav = std::env::var_os("WAV").map(PathBuf::from);
    let mut all_ms = Vec::new();
    let mut total_silent = 0usize;
    for &speed in &speeds {
        let (mix_a, mix_b, mix_pitch) = {
            let (a, b, p) = capture_mix(speed);
            (
                env("MIX1").unwrap_or(a),
                env("MIX2").unwrap_or(b),
                env("MIXPITCH").unwrap_or(p),
            )
        };
        let input = BoardInputs {
            speed,
            speed_scale: None,
            mix_gain_a: mix_a,
            mix_gain_b: mix_b,
            mix_pitch,
            f1164: 0.0,
            f1168: 0.0,
            f1456: None,
            special: false,
            boost: 0.0,
            boost_gain: tuning.boost_gain,
            boost_kmh: tuning.boost_kmh,
        };
        let records: [GrainRecord; 2] = board::board_records(&tuning, &input);
        // The capture's modal `sub_824C9058` reads by the local board: vfunc64(11) 24971,
        // vfunc64(12) 77, vfunc52(0) 8795, vfunc60(13) 2590, vfunc60(21)/(22) 1267/1835.
        let chain_values = board::chain_values(
            &tuning,
            &ChainInputs {
                mix64_11: env("MIX11").unwrap_or(24_971),
                mix64_12: env("MIX12").unwrap_or(77),
                mix52_0: env("MIX52").unwrap_or(8_795),
                mix60_13: env("MIX13").unwrap_or(2_590),
                local: Some((1_267, 1_835)),
                special: false,
                f1152: None,
                boost: 0.0,
                boost_shift_a: tuning.shift_boost,
                boost_shift_b: tuning.shift_boost_b,
            },
        );
        println!(
            "\n== {speed} m/s: mix ({mix_a}, {mix_b}, {mix_pitch}) -> A {:?}  B {:?}",
            records[0], records[1]
        );
        runtime.take_grain_starts();
        let blocks = (seconds * BLOCKS_PER_SECOND) as usize;
        let mut ms = Vec::with_capacity(blocks);
        let (mut peak, mut sum, mut n) = (0f32, 0f64, 0usize);
        let mut silent = 0usize;
        let mut voices_max = 0usize;
        let mut next_frame = 0.0f64;
        for block in 0..blocks {
            let t = block as f64 / BLOCKS_PER_SECOND;
            // The board update runs at the game frame rate (~60 Hz here).
            while next_frame <= t {
                let mut grains = runtime.grains();
                for (i, &player) in players.iter().enumerate() {
                    grains.set_record(player, records[i % 2])?;
                }
                if use_chain {
                    for pair in players.chunks(2) {
                        grains.push_chain(pair[0], pair[1], &chain_values)?;
                    }
                }
                next_frame += 1.0 / 60.0;
            }
            let started = std::time::Instant::now();
            let pcm = runtime.pump_once()?;
            ms.push(started.elapsed().as_secs_f64() * 1000.0);
            let block_peak = pcm.iter().fold(0f32, |m, s| m.max(s.abs()));
            if block_peak < 1e-6 && records[0].gain > 0.0 {
                silent += 1;
            }
            for s in &pcm {
                peak = peak.max(s.abs());
                sum += f64::from(*s) * f64::from(*s);
            }
            n += pcm.len();
            voices_max = voices_max.max(runtime.grains().live_voices()?);
            if write_wav.is_some() {
                wav.extend_from_slice(&pcm);
            }
            if (block + 1) % BLOCKS_PER_SECOND as usize == 0 {
                println!(
                    "  t={:4.1}s peak {:6.1} dBFS  rms {:6.1} dBFS  live grain voices (max) {}",
                    (block + 1) as f64 / BLOCKS_PER_SECOND,
                    db(peak),
                    db((sum / n.max(1) as f64).sqrt() as f32),
                    voices_max
                );
                (peak, sum, n, voices_max) = (0.0, 0.0, 0, 0);
            }
        }
        let starts = runtime.take_grain_starts();
        let list: Vec<String> = starts
            .iter()
            .take(24)
            .map(|s| format!("{:.2}", s.seconds))
            .collect();
        println!(
            "  {} grain starts ({:.1}/s); first: {}",
            starts.len(),
            starts.len() as f64 / seconds,
            list.join(" ")
        );
        if let Some(s) = starts.iter().find(|s| s.seek.is_some()) {
            println!(
                "  e.g. start {:.3}s -> frame {} via {:?}",
                s.seconds,
                s.frame,
                s.seek.unwrap()
            );
        }
        ms.sort_by(|a, b| a.total_cmp(b));
        println!(
            "  render ms/block p50 {:.3} p99 {:.3} max {:.3} (budget 5.333); silent blocks with gain > 0: {silent}",
            ms[ms.len() / 2],
            ms[(ms.len() - 1) * 99 / 100],
            ms[ms.len() - 1]
        );
        total_silent += silent;
        all_ms.extend(ms);
    }
    all_ms.sort_by(|a, b| a.total_cmp(b));
    println!(
        "\noverall: {} blocks, render p50 {:.3} ms p99 {:.3} ms max {:.3} ms, over budget {}, silent blocks {total_silent} ({})",
        all_ms.len(),
        all_ms[all_ms.len() / 2],
        all_ms[(all_ms.len() - 1) * 99 / 100],
        all_ms[all_ms.len() - 1],
        all_ms.iter().filter(|m| **m > 5.333).count(),
        if total_silent == 0 {
            "continuous"
        } else {
            "GAPS"
        }
    );
    if let Some(path) = write_wav {
        write_float_wav(&path, &wav, 6, 48_000)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn write_float_wav(
    path: &std::path::Path,
    samples: &[f32],
    channels: u16,
    rate: u32,
) -> std::io::Result<()> {
    use std::io::Write;
    let data = (samples.len() * 4) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&3u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * u32::from(channels) * 4).to_le_bytes())?;
    f.write_all(&(channels * 4).to_le_bytes())?;
    f.write_all(&32u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()
}

/// Replay capture-joined board updates (`frame speed v60_1 v60_2 v56_3 f1164 f1168 f1456 b1464
/// w1500 f1508` then gain/pitch/position/data for P00 P01 P10 P11) through `board_records`.
fn validate_board(vault: &GrainVault, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let rows: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
    let speed_of: std::collections::HashMap<u32, f32> = rows
        .iter()
        .map(|r| {
            (
                r[0].parse().unwrap(),
                f32::from_bits(u32::from_str_radix(r[1], 16).unwrap()),
            )
        })
        .collect();
    let mut names = std::collections::HashMap::new();
    for surface in 1..=9 {
        for soft in [false, true] {
            if let Some(c) = board::grain_for_surface(surface, soft) {
                names.insert(c.member, c.key);
            }
        }
    }
    // Grain data addresses from the capture's GL lines.
    let data_names = [
        ("4A8509C0", "asphalt_rough_hard.grain"),
        ("4A811310", "asphalt_rough_soft.grain"),
        ("4A7CBBF0", "asphalt_smooth_hard.grain"),
        ("4A787160", "asphalt_smooth_soft.grain"),
        ("4A743580", "concrete_aggregate_hard.grain"),
        ("4A702DC0", "concrete_aggregate_soft.grain"),
        ("4A6C35A0", "concrete_rough_hard.grain"),
        ("4A686360", "concrete_rough_soft.grain"),
        ("4A648060", "concrete_smooth_hard.grain"),
        ("4A60AA50", "concrete_smooth_soft.grain"),
        ("4A5C6F00", "metal_smooth_hard.grain"),
        ("4A591650", "wood_ramp_hard.grain"),
        ("4A55E0C0", "wood_ramp_soft.grain"),
    ];
    let mut tunings = std::collections::HashMap::new();
    for (addr, name) in data_names {
        tunings.insert(addr, vault.tuning(names[name])?);
    }
    for offset in [-1i64, 0, 1] {
        let (mut total, mut gain_ok, mut pitch_ok, mut pos_ok, mut pos2_ok) = (0, 0, 0, 0, 0);
        let mut first_bad = None;
        let mut neighbour = std::collections::BTreeMap::new();
        for r in &rows {
            let frame: u32 = r[0].parse()?;
            let Some(tuning) = tunings.get(r[14]) else {
                continue;
            };
            let Some(&speed) = speed_of.get(&((i64::from(frame) + offset) as u32)) else {
                continue;
            };
            let f = |s: &str| -> f32 { s.parse().unwrap_or(0.0) };
            let input = BoardInputs {
                speed,
                speed_scale: None,
                mix_gain_a: r[2].parse()?,
                mix_gain_b: r[3].parse()?,
                mix_pitch: r[4].parse()?,
                f1164: f(r[5]),
                f1168: f(r[6]),
                f1456: (r[8] != "0").then(|| f(r[7])),
                special: false,
                boost: f(r[10]),
                boost_gain: tuning.boost_gain,
                boost_kmh: tuning.boost_kmh,
            };
            let [a, b] = board::board_records(tuning, &input);
            let hexf = |s: &str| u32::from_str_radix(s, 16).unwrap_or(0);
            // Only updates in which the board wrote the grain records (its grain branch ran):
            // the pitch is re-read from the MixMap every write, so a stale record shows as a
            // pitch that is not this frame's `vfunc56(3) / 4096`.
            if a.pitch.to_bits() != hexf(r[12]) {
                continue;
            }
            total += 1;
            gain_ok += usize::from(a.gain.to_bits() == hexf(r[11]));
            pitch_ok += usize::from(b.pitch.to_bits() == hexf(r[16]));
            let p_ok = a.position.to_bits() == hexf(r[13]);
            pos_ok += usize::from(p_ok);
            pos2_ok += usize::from(b.position.to_bits() == hexf(r[17]));
            if !p_ok && offset == -1 {
                // Does the speed of a neighbouring state line explain the miss?
                for other in [-2i64, 0] {
                    if let Some(&s2) = speed_of.get(&((i64::from(frame) + other) as u32)) {
                        let [a2, _] =
                            board::board_records(tuning, &BoardInputs { speed: s2, ..input });
                        if a2.position.to_bits() == hexf(r[13]) {
                            *neighbour.entry(other).or_insert(0usize) += 1;
                        }
                    }
                }
            }
            if !p_ok && first_bad.is_none() {
                first_bad = Some((frame, speed, a.position, f32::from_bits(hexf(r[13]))));
            }
        }
        println!(
            "speed frame offset {offset:+}: {total} grain-branch updates; bit-exact: A gain {gain_ok}, B pitch {pitch_ok}, A position {pos_ok}, B position {pos2_ok}; first A position miss {first_bad:?}; misses explained by the state line at offset {neighbour:?}"
        );
    }
    Ok(())
}
