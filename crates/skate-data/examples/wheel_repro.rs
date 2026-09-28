//! Headless wheel-audio check against the owner's retail assets: no Bevy, physics or controller.
//! Installs every player bank, posts the three boot utilities, then drives `Class_rolling` and
//! `Rolling_Rattle_Class` with the retail-exact 12-word packets that `player_audio.rs` sends,
//! re-delivering rolling at 60 Hz and re-posting rattle about every 1.07 s, at a fixed board speed.
//! Reports native (6-channel) and per-channel peaks so levels can be checked without a playtest.
//!
//!     cargo run --release -p skate-data --example wheel_repro -- <assets> [seconds] [speed m/s]
use skate_audio_core::authored::AuthoredRuntime;
use skate_data::audio::catalog::PlayerAudioCatalog;
use std::path::PathBuf;

const BLOCKS_PER_SECOND: f64 = 48_000.0 / 256.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let assets = PathBuf::from(args.next().ok_or("need assets directory")?);
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(6.0);
    let speed: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(6.0);
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Skate3RustEngine/audio-pcm-cache"));

    let catalog = PlayerAudioCatalog::from_assets(&assets, cache.as_deref())?;
    let mut runtime = AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks)?;
    for sample in catalog.samples {
        let base = runtime.bank_base(&sample.bank).ok_or("missing bank")?;
        runtime.insert_pcm(base + sample.header_offset, sample.pcm)?;
    }
    let utility_relocations = [
        (3, 0x1c),
        (7, 0x2c),
        (11, 0x3c),
        (15, 0x4c),
        (19, 0x5c),
        (23, 0x6c),
        (27, 0x7c),
    ];
    for object in ["c_emitter_utility", "Start_up_Play_ctl", "c_foley_utility"] {
        if runtime.has_object(object) {
            runtime.post_relocated(object, &[0u32; 28], &utility_relocations)?;
        }
    }

    // FOOTSTEP=1: the two held retail footstep messages (sub_824E9FD8 constructor packets), a
    // push foot-plant on the second one every 0.6 s, idle updates otherwise.
    if std::env::var_os("FOOTSTEP").is_some() {
        let mut constructor = [0u32; 25];
        constructor[2] = 0x1000;
        constructor[4] = 25_000;
        constructor[7] = 32_767;
        constructor[12] = 1;
        constructor[13] = 1;
        constructor[16] = 1;
        constructor[17] = 1;
        constructor[24] = 12;
        let feet = [
            runtime.post("playercharacter_footstep", &constructor)?,
            runtime.post("playercharacter_footstep", &constructor)?,
        ];
        let w10: u32 = std::env::var("W10").ok().and_then(|v| v.parse().ok()).unwrap_or(513);
        let w14: u32 = std::env::var("W14").ok().and_then(|v| v.parse().ok()).unwrap_or(297);
        let hold: usize = std::env::var("HOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
        let frames = (seconds * 60.0) as usize;
        let mut peak = 0.0f32;
        let mut second = 0.0f32;
        let mut block = 0usize;
        for frame in 0..frames {
            let phase = frame % 36;
            for (i, &handle) in feet.iter().enumerate() {
                let planted = i == 1 && phase < hold;
                let mut update = [0u32; 25];
                update[0] = 32_767;
                update[2] = 4_086;
                update[4] = 24_971;
                update[5] = 77;
                update[6] = 1_789;
                update[7] = 9_202;
                update[8] = u32::from(planted);
                update[9] = if planted { 2 } else { 0 };
                update[10] = w10;
                update[12] = 1;
                update[13] = 1;
                update[14] = w14;
                update[15] = 1;
                update[16] = 2;
                update[17] = 1;
                update[18..24].copy_from_slice(&[32_767, 10_000, 15_000, 25_000, 32_767, 28_000]);
                update[24] = 12;
                runtime.redeliver(handle, &update)?;
            }
            // 60 Hz frames over 187.5 Hz blocks.
            let target = ((frame + 1) as f64 * BLOCKS_PER_SECOND / 60.0) as usize;
            while block < target {
                let pcm = runtime.pump_once().map_err(|e| format!("block {block}: {e}"))?;
                for s in &pcm {
                    peak = peak.max(s.abs());
                    second = second.max(s.abs());
                }
                block += 1;
                if block % BLOCKS_PER_SECOND as usize == 0 {
                    eprintln!(
                        "t={:4.1}s opened={} live={} peak {:.1} dBFS",
                        block as f64 / BLOCKS_PER_SECOND,
                        runtime.stats().voices_opened,
                        runtime.stats().live_voices,
                        20.0 * second.max(1e-9).log10()
                    );
                    second = 0.0;
                }
            }
        }
        println!(
            "footstep: overall peak {:.1} dBFS, {:?}",
            20.0 * peak.max(1e-9).log10(),
            runtime.stats()
        );
        return Ok(());
    }

    // AttribSys vault tuning: rolling max speed 70 km/h, rattle divisor 30 km/h.
    let rolling_word = ((speed * 3.6 / 70.0).clamp(0.0, 1.0) * 10_000.0) as u32;
    let rattle_word = (((speed - 1.0) * 3.6 / 30.0).clamp(0.0, 1.0) * 10_000.0) as u32;
    let layers = [(0u32, 12_999u32), (3, 4_913)];
    let rolling_only = std::env::var_os("NO_RATTLE").is_some();
    let rattle_only = std::env::var_os("NO_ROLLING").is_some();
    eprintln!("speed {speed} m/s -> rolling word {rolling_word}, rattle word {rattle_word}");

    let mut rolling = Vec::new();
    if !rattle_only {
        for (selector, _) in layers {
            let post = [0, 0, 0x1000, rolling_word, selector, 0, 2, 0, 0, 25_000, 0, 32_767];
            rolling.push((runtime.post("Class_rolling", &post)?, selector));
        }
    }
    let mut rattle: Option<u32> = None;
    let blocks = (seconds * BLOCKS_PER_SECOND) as usize;
    let mut next_update = 0.0f64;
    let mut next_rattle = 0.0f64;
    let mut second_peak = [0.0f32; 6];
    let mut overall = [0.0f32; 6];
    let mut nonzero_blocks = 0usize;
    let mut render_ms: Vec<f64> = Vec::new();
    for block in 0..blocks {
        let t = block as f64 / BLOCKS_PER_SECOND;
        if !rolling_only && t >= next_rattle {
            if let Some(handle) = rattle.take() {
                runtime.release(handle)?;
            }
            let post = [0, 0, 0x1000, rattle_word, 2, 0, 1, 0, 25_000, 0, 32_767, 8];
            rattle = Some(runtime.post("Rolling_Rattle_Class", &post)?);
            next_rattle += 64.0 / 60.0;
        }
        while t >= next_update {
            let contact_gain = ((rolling_word as u64 * 12_999) / 460).min(12_999);
            for &(handle, selector) in &rolling {
                let cap = layers.iter().find(|l| l.0 == selector).unwrap().1;
                let gain = ((rolling_word as u64 * cap as u64) / 460).min(cap as u64) as u32;
                let update = [32_767, 0, 4_086, rolling_word, selector, 0, 2, 0, 0, 24_971, 77, gain];
                runtime.redeliver(handle, &update)?;
            }
            if let Some(handle) = rattle {
                let update = [
                    32_767,
                    0,
                    4_055,
                    rattle_word,
                    2,
                    0,
                    1,
                    (contact_gain * 131 / 1000) as u32,
                    24_971,
                    77,
                    (contact_gain * 9226 / 10_000) as u32,
                    8,
                ];
                runtime.redeliver(handle, &update)?;
            }
            next_update += 1.0 / 60.0;
        }
        let started = std::time::Instant::now();
        let pcm = runtime
            .pump_once()
            .map_err(|e| format!("block {block}: {e}"))?;
        render_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let mut any = false;
        for frame in pcm.chunks(6) {
            for (c, s) in frame.iter().enumerate() {
                let a = s.abs();
                second_peak[c] = second_peak[c].max(a);
                overall[c] = overall[c].max(a);
                any |= a > 1e-8;
            }
        }
        nonzero_blocks += usize::from(any);
        if (block + 1) % BLOCKS_PER_SECOND as usize == 0 {
            let db: Vec<String> = second_peak
                .iter()
                .map(|p| format!("{:6.1}", 20.0 * p.max(1e-9).log10()))
                .collect();
            eprintln!(
                "t={:4.1}s live={} peak dBFS per channel [{}]",
                t,
                runtime.stats().live_voices,
                db.join(" ")
            );
            second_peak = [0.0; 6];
        }
    }
    let db: Vec<String> = overall
        .iter()
        .map(|p| format!("{:.1}", 20.0 * p.max(1e-9).log10()))
        .collect();
    println!(
        "{blocks} blocks, {nonzero_blocks} nonzero, overall peak dBFS [{}], {:?}",
        db.join(" "),
        runtime.stats()
    );
    render_ms.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| render_ms[((render_ms.len() - 1) as f64 * p) as usize];
    println!(
        "render per 256-frame block (budget 5.33 ms): p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms, over budget {}",
        pct(0.5),
        pct(0.99),
        render_ms.last().copied().unwrap_or(0.0),
        render_ms.iter().filter(|ms| **ms > 5.333).count()
    );
    Ok(())
}
