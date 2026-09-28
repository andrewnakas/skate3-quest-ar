//! Headless end-to-end check of the retail player-sound path: the exact worker setup
//! ([`super::prepare`]) and frame ([`super::sound::PlayerSound::frame`]) the game runs, fed
//! synthetic physics ticks, rendered block by block.
//!
//! `cargo test -p skate-game --bin skate3rust --locked -- --ignored headless --nocapture`
//! (assets from `SKATE_ASSETS`, defaulting to the owner's install). Optional `SKATE_HEADLESS_WAV`
//! writes the stereo output.

use crate::skate_audio::{PlayerAudioObservation, RetailAudioInputs};

const DEFAULT_ASSETS: &str = r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets";

/// A grounded roll at `speed` m/s straight along +X on four wheels.
fn rolling(tick: u64, speed: f32) -> PlayerAudioObservation {
    let mut retail = RetailAudioInputs::default();
    retail.dt = 1.0 / 60.0;
    retail.ground_speed = speed;
    retail.com_velocity = [speed, 0.0, 0.0];
    retail.linear_velocity = [speed, 0.0, 0.0];
    retail.wheel_count = 4;
    retail.wheel_contacts = [true; 4];
    retail.truck_contacts = [true; 2];
    retail.state_category = 100;
    retail.state = 100;
    retail.motion_200 = 1.0;
    retail.deck_rows = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    retail.effective_deck_up = [0.0, 1.0, 0.0];
    retail.ground_normal = [0.0, 1.0, 0.0];
    retail.wheel_contact_normals = [[0.0, 1.0, 0.0]; 4];
    retail.wheel_audio_surfaces = [3; 4];
    retail.scorable_id = -1;
    retail.deck_forward = [1.0, 0.0, 0.0];
    // A follow camera three metres behind and above, looking along the roll.
    retail.camera = Some(([1.0, 0.0, 0.0], [-3.0, 1.5, 0.0]));
    PlayerAudioObservation {
        tick,
        state: 100,
        board_speed: speed,
        rider_speed: speed,
        grounded: true,
        grinding: false,
        grind_family: 0,
        grind_substate: 0,
        grind_audio_surface: 0,
        grind_impact_speed: 0.0,
        wiping_out: false,
        landed: false,
        landing_impact_speed: 0.0,
        landing_clean: false,
        landing_sketchy: false,
        landing_type: 0,
        landing_spin: 0.0,
        landing_sideways_speed: 0.0,
        footstep_strength: 0.0,
        footstep_bone: 0,
        foot_push_speed: 0.0,
        feet_supported: [false; 2],
        foot_surface: 0,
        contact_count: 4,
        powersliding: false,
        trick_id: None,
        trick_identifier: None,
        animation_name: None,
        riding_switch: false,
        riding_fakie: false,
        nollie: false,
        wheel_surface: 3,
        events: Vec::new(),
        frontend_sounds: Vec::new(),
        retail,
    }
}

/// The voice-graph arena must plateau. `build_graph` takes a whole graph out of the arena in one
/// allocation and only the deferred teardown (`stop_player`) can give it back; when that free was
/// missing, every one-shot cost a graph for the rest of the session and a long playtest ended in
/// "authored audio guest heap exhausted" with the audio worker dead and the game still running.
///
///     cargo test -p skate-game --bin skate3rust -- --ignored the_voice_arena --nocapture
#[test]
#[ignore = "needs the owner's assets"]
fn the_voice_arena_plateaus_under_repeated_voices() {
    use skate_audio_core::authored::oneshot::{OneshotBus, OneshotVoice};
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let super::Prepared { mut runtime, .. } =
        super::prepare(&assets).expect("prepare the retail player-sound worker");
    // Any installed sample will do; the graph is what is being measured, not the audio.
    let sample = runtime.any_installed_sample().expect("some decoded sample");
    let mut marks = Vec::new();
    for round in 0..40 {
        let mut handle = runtime
            .play_oneshot(&OneshotVoice {
                sample,
                gain: 0.5,
                pitch: 1.0,
                delay: 0.0,
                pan: 0.0,
                bus: OneshotBus::Default,
            })
            .expect("open a one-shot");
        // Run it to the end, rendering blocks as the worker does: the teardown is *deferred*
        // through the command ring, and the ring is only drained while blocks are being rendered.
        for _ in 0..600 {
            let live = runtime.tick_oneshot(&mut handle, 1.0 / 60.0).expect("tick");
            runtime.pump_once().expect("render one block");
            if !live {
                break;
            }
        }
        runtime.release_oneshot(&mut handle).expect("release");
        // A few more blocks so the stop command this release queued is drained too.
        for _ in 0..4 {
            runtime.pump_once().expect("render one block");
        }
        if round % 10 == 9 {
            marks.push(runtime.arena_usage());
        }
    }
    println!("arena (high-water bytes, live blocks) after each 10 voices: {marks:?}");
    let (first, last) = (marks[0], *marks.last().unwrap());
    // The high-water mark must stop moving: the graph each voice takes out of the arena is given
    // back by the deferred teardown and reused by the next voice.
    assert_eq!(
        first.0, last.0,
        "arena high-water grew from {} to {} bytes across 30 further voices",
        first.0, last.0
    );
    // Known residual, and much smaller: the 128-byte decoder `open` allocates per voice is keyed
    // by the voice's stream but freed by the graph teardown's child walk, so it survives when the
    // two do not match. It costs 128 bytes a voice against a 64 MB arena rather than a whole
    // graph, and the free list absorbs it, but it is not zero.
    let leaked = last.1 - first.1;
    println!("residual live blocks across 30 voices: {leaked}");
    assert!(
        leaked <= 30,
        "more than one block per voice survives teardown"
    );
}

#[test]
#[ignore = "needs the owner's assets; run explicitly"]
fn headless_rolling_through_the_retail_path() {
    use skate_audio_core::authored::{PCM_CHANNELS, PCM_FRAMES_PER_BLOCK};
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let started = std::time::Instant::now();
    let super::Prepared {
        mut runtime,
        mut sound,
        ..
    } = super::prepare(&assets).expect("prepare the retail player-sound worker");
    eprintln!("prepared in {:.1} s", started.elapsed().as_secs_f32());

    // (seconds, speed): idle, slow, cruise, fast, stop.
    let script: [(u32, f32); 5] = [(2, 0.0), (3, 2.0), (3, 5.0), (3, 9.0), (2, 0.0)];
    let mut wav: Vec<f32> = Vec::new();
    let mut tick = 0u64;
    let mut blocks_owed = 0.0f64;
    let blocks_per_frame = 48_000.0 / f64::from(PCM_FRAMES_PER_BLOCK) / 60.0;
    let mut worst_ms = 0.0f64;
    for (seconds, speed) in script {
        let (mut peak, mut energy, mut samples, mut silent, mut blocks) =
            (0f32, 0f64, 0usize, 0, 0);
        for _ in 0..seconds * 60 {
            sound
                .frame(&mut runtime, &rolling(tick, speed))
                .expect("player-sound frame");
            tick += 1;
            blocks_owed += blocks_per_frame;
            while blocks_owed >= 1.0 {
                blocks_owed -= 1.0;
                let begin = std::time::Instant::now();
                let native = runtime.pump_once().expect("render one block");
                worst_ms = worst_ms.max(begin.elapsed().as_secs_f64() * 1e3);
                assert_eq!(
                    native.len(),
                    PCM_FRAMES_PER_BLOCK as usize * usize::from(PCM_CHANNELS)
                );
                let stereo = super::downmix(&native);
                let block_peak = stereo.iter().fold(0f32, |p, s| p.max(s.abs()));
                peak = peak.max(block_peak);
                energy += stereo
                    .iter()
                    .map(|s| f64::from(*s) * f64::from(*s))
                    .sum::<f64>();
                samples += stereo.len();
                // Retail fades rolling in over a few frames; judge continuity after 0.25 s.
                if block_peak < 1.0e-5 && blocks >= 47 {
                    silent += 1;
                }
                blocks += 1;
                wav.extend_from_slice(&stereo);
            }
        }
        let stats = runtime.stats();
        if let Some((ctrl, c)) = sound.controls("SkateBoard") {
            use super::components::Controls;
            let levels: Vec<u32> = (0..20).map(|id| c.level(id)).collect();
            let raw: Vec<u32> = (0..20).map(|id| c.raw(id)).collect();
            let inputs: Vec<u32> = (0..16)
                .map(|id| runtime.mixmap_get(ctrl, id).unwrap_or(0))
                .collect();
            eprintln!(
                "  SkateBoard {ctrl:#010x} levels {levels:?}
  raw {raw:?}
  inputs {inputs:?}"
            );
        }
        eprintln!(
            "speed {speed:>4.1} m/s for {seconds} s: peak {:>6.1} dBFS, rms {:>6.1} dBFS, silent blocks {silent}/{blocks}, voices opened {} live {}",
            20.0 * peak.max(1e-9).log10(),
            10.0 * (energy / samples.max(1) as f64).max(1e-18).log10(),
            stats.voices_opened,
            stats.live_voices,
        );
        if speed > 1.0 {
            assert_eq!(
                silent, 0,
                "rolling at {speed} m/s must never drop to silence"
            );
        }
    }
    eprintln!("worst block render {worst_ms:.2} ms (budget 5.33 ms)");
    if let Some(path) = std::env::var_os("SKATE_HEADLESS_WAV") {
        write_wav(std::path::Path::new(&path), &wav).expect("write wav");
        eprintln!("wrote {}", std::path::Path::new(&path).display());
    }
}

/// How loud a revert's skid is against the roll it interrupts. RevertGround (102) with State+66
/// set drives the skid counter to its +5/frame ramp; retail's capture (music off) puts a revert
/// at 3–6 m/s **+10.2 dB** over rolling at the same speed, and a powerslide +4.1 dB.
#[test]
#[ignore = "needs the owner's assets; run explicitly"]
fn headless_revert_skid_level() {
    use skate_audio_core::authored::PCM_FRAMES_PER_BLOCK;
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let super::Prepared {
        mut runtime,
        mut sound,
        ..
    } = super::prepare(&assets).expect("prepare the retail player-sound worker");
    let blocks_per_frame = 48_000.0 / f64::from(PCM_FRAMES_PER_BLOCK) / 60.0;
    let (mut tick, mut owed) = (0u64, 0.0f64);
    // (label, frames, reverting)
    let script = [
        ("settle", 120, false),
        ("roll", 60, false),
        ("revert", 40, true),
        ("after", 60, false),
    ];
    for (label, frames, reverting) in script {
        let (mut energy, mut samples, mut peak) = (0f64, 0usize, 0f32);
        for _ in 0..frames {
            let mut o = rolling(tick, 5.0);
            // Deck X across the roll, so rolling has retail's slip word of 1 rather than 11.
            o.retail.deck_rows = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
            if reverting {
                o.state = 102;
                o.retail.state = 102;
                o.retail.state_flags[66 - 52] = true;
            }
            sound.frame(&mut runtime, &o).expect("player-sound frame");
            tick += 1;
            owed += blocks_per_frame;
            while owed >= 1.0 {
                owed -= 1.0;
                let stereo = super::downmix(&runtime.pump_once().expect("render one block"));
                peak = stereo.iter().fold(peak, |p, s| p.max(s.abs()));
                energy += stereo.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
                samples += stereo.len();
            }
        }
        eprintln!(
            "{label:>6} at 5 m/s: rms {:>6.1} dBFS, peak {:>6.1} dBFS",
            10.0 * (energy / samples.max(1) as f64).max(1e-18).log10(),
            20.0 * peak.max(1e-9).log10(),
        );
    }
}

/// Airborne: no wheels down, in the air state, with a hop's air timing.
fn airborne(tick: u64, speed: f32, elapsed: f32, jump_height: f32) -> PlayerAudioObservation {
    let mut observation = rolling(tick, speed);
    observation.state = 201;
    observation.grounded = false;
    observation.contact_count = 0;
    observation.retail.state = 201;
    observation.retail.state_category = 200;
    observation.retail.wheel_count = 0;
    observation.retail.wheel_contacts = [false; 4];
    observation.retail.truck_contacts = [false; 2];
    observation.retail.in_known_air = true;
    observation.retail.air_time_in_state = elapsed;
    observation.retail.air_time_until_landing = (0.7 - elapsed).max(0.0);
    observation.retail.air_jump_height = jump_height;
    observation
}

/// A hop and its landing must make the contact one-shots (`sub_824B9CC8` pops, `sub_824BA630`
/// landing impact) audible: they are bank voices from `Skate_Collisions.bnk`, not authored patches.
#[test]
#[ignore = "needs the owner's assets; run explicitly"]
fn headless_landing_impact_is_audible() {
    use skate_audio_core::authored::{PCM_CHANNELS, PCM_FRAMES_PER_BLOCK};
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let super::Prepared {
        mut runtime,
        mut sound,
        ..
    } = super::prepare(&assets).expect("prepare the retail player-sound worker");
    let mut tick = 0u64;
    let mut blocks_owed = 0.0f64;
    let blocks_per_frame = 48_000.0 / f64::from(PCM_FRAMES_PER_BLOCK) / 60.0;
    // Roll, hop for 0.7 s, then land on all four and roll on.
    let script: Vec<PlayerAudioObservation> = (0..60)
        .map(|i| rolling(i, 5.0))
        .chain((0..42).map(|i| airborne(60 + i, 5.0, i as f32 / 60.0, 1.2)))
        .chain((0..120).map(|i| rolling(102 + i, 5.0)))
        .collect();
    let mut peaks = Vec::new();
    let mut natives = Vec::new();
    let mut matrices = Vec::new();
    for observation in script {
        sound
            .frame(&mut runtime, &observation)
            .expect("player-sound frame");
        tick += 1;
        blocks_owed += blocks_per_frame;
        // Three meters, because they answer different questions. The native peak is what the
        // retail recomp's own output pass logs, so it is the one that compares with a capture;
        // the matrix peak is the downmix before the host trim and the clamp, which is what
        // decides how much headroom that trim has to leave; the device peak is what is heard.
        let (mut frame_peak, mut frame_native, mut frame_matrix) = (0.0f32, 0.0f32, 0.0f32);
        while blocks_owed >= 1.0 {
            blocks_owed -= 1.0;
            let native = runtime.pump_once().expect("render one block");
            assert_eq!(
                native.len(),
                PCM_FRAMES_PER_BLOCK as usize * usize::from(PCM_CHANNELS)
            );
            for sample in &native {
                frame_native = frame_native.max(sample.abs());
            }
            for frame in native.chunks_exact(usize::from(PCM_CHANNELS)) {
                for (front, surround) in [(frame[0], frame[4]), (frame[1], frame[5])] {
                    frame_matrix =
                        frame_matrix.max((front + 0.707 * frame[2] + 0.5 * surround).abs());
                }
            }
            frame_peak = super::downmix(&native)
                .iter()
                .fold(frame_peak, |peak, sample| peak.max(sample.abs()));
        }
        peaks.push((tick, frame_peak));
        natives.push(frame_native);
        matrices.push(frame_matrix);
    }
    // The airborne stretch is frames 61..102; the landing lands at 103.
    let air = peaks[70..100].iter().fold(0f32, |p, (_, v)| p.max(*v));
    let landing = peaks[102..130].iter().fold(0f32, |p, (_, v)| p.max(*v));
    let db = |v: f32| 20.0 * v.max(1e-9).log10();
    let rolling_native = natives[20..60].iter().fold(0f32, |p, v| p.max(*v));
    let landing_native = natives[102..130].iter().fold(0f32, |p, v| p.max(*v));
    let landing_matrix = matrices[102..130].iter().fold(0f32, |p, v| p.max(*v));
    // Retail, metered at its own output pass with its music off: rolling -22.9 dBFS, landing
    // +0.4 dBFS, i.e. +23.3 dB of impact over the bed and a landing that runs past unity.
    eprintln!(
        "native rolling {:.1} dBFS, native landing {:.1} dBFS (+{:.1} dB; retail +23.3), matrix landing {:.1} dBFS",
        db(rolling_native),
        db(landing_native),
        db(landing_native) - db(rolling_native),
        db(landing_matrix),
    );
    eprintln!(
        "air peak {:.1} dBFS, landing peak {:.1} dBFS",
        db(air),
        db(landing),
    );
    assert!(
        landing > air * 1.5,
        "the landing impact must be clearly louder than the airborne stretch: {landing} vs {air}"
    );
}

fn write_wav(path: &std::path::Path, stereo: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data = (stereo.len() * 2) as u32;
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&48_000u32.to_le_bytes())?;
    out.write_all(&(48_000u32 * 4).to_le_bytes())?;
    out.write_all(&4u16.to_le_bytes())?;
    out.write_all(&16u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data.to_le_bytes())?;
    for sample in stereo {
        out.write_all(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    out.flush()
}

/// Replay one skid's exact messages from a playtest trace through the headless runtime, one game
/// frame at a time, so a patch decision seen in game (which voices it opens) can be reproduced and
/// bisected. `SKATE_REPLAY_TRACE=<trace>`, `SKATE_REPLAY_FROM` / `_TO` = frames (the post at or
/// before `FROM` is used), `SKATE_REPLAY_ZERO=3,9` zeroes those update words and `SKATE_REPLAY_SET=7=3999` pins one (only within `SKATE_REPLAY_SET_FROM..=_TO` if given). Run with
/// `SKATE_AUDIO_TRACE` set to read the `OP` lines it produces.
#[test]
#[ignore = "needs the owner's assets and a playtest trace; run explicitly"]
fn headless_replay_skid_from_trace() {
    use skate_audio_core::authored::PCM_FRAMES_PER_BLOCK;
    let (Ok(path), Ok(from), Ok(to)) = (
        std::env::var("SKATE_REPLAY_TRACE"),
        std::env::var("SKATE_REPLAY_FROM"),
        std::env::var("SKATE_REPLAY_TO"),
    ) else {
        eprintln!("skipped: set SKATE_REPLAY_TRACE, SKATE_REPLAY_FROM and SKATE_REPLAY_TO");
        return;
    };
    let (from, to): (u64, u64) = (from.parse().unwrap(), to.parse().unwrap());
    let zero: Vec<usize> = std::env::var("SKATE_REPLAY_ZERO")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    // `SKATE_REPLAY_SET=7=3999,10=46` pins update words to constants.
    let set: Vec<(usize, u32)> = std::env::var("SKATE_REPLAY_SET")
        .unwrap_or_default()
        .split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().parse().ok()?, v.trim().parse().ok()?))
        })
        .collect();
    let words = |line: &str| -> Vec<u32> {
        line.split('|')
            .nth(1)
            .unwrap()
            .split_whitespace()
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect()
    };
    let (mut post, mut updates) = (None, Vec::new());
    let text = String::from_utf8_lossy(&std::fs::read(&path).expect("read trace")).into_owned();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(_), Some(frame), Some(tag)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let Ok(frame) = frame.parse::<u64>() else {
            continue;
        };
        if !line.contains("Class_wheels_skid") || frame > to {
            continue;
        }
        if tag == "PO" && frame <= from {
            post = Some((frame, words(line)));
            updates.clear();
        } else if tag == "UP" && post.is_some() {
            updates.push((frame, words(line)));
        }
    }
    let (posted_at, post) = post.expect("a skid post at or before FROM");
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let super::Prepared { mut runtime, .. } =
        super::prepare(&assets).expect("prepare the retail player-sound worker");
    let blocks_per_frame = 48_000.0 / f64::from(PCM_FRAMES_PER_BLOCK) / 60.0;
    let mut owed = 0.0f64;
    let mut pump = |runtime: &mut skate_audio_core::authored::AuthoredRuntime, owed: &mut f64| {
        *owed += blocks_per_frame;
        while *owed >= 1.0 {
            *owed -= 1.0;
            runtime.pump_once().expect("render one block");
        }
    };
    super::trace::frame(posted_at);
    let handle = runtime
        .post("Class_wheels_skid", &post[..18])
        .expect("post the skid");
    pump(&mut runtime, &mut owed);
    for (frame, mut w) in updates {
        for &i in &zero {
            w[i] = 0;
        }
        let (set_from, set_to) = (
            std::env::var("SKATE_REPLAY_SET_FROM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            std::env::var("SKATE_REPLAY_SET_TO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(u64::MAX),
        );
        if (set_from..=set_to).contains(&frame) {
            for &(i, v) in &set {
                w[i] = v;
            }
        }
        super::trace::frame(frame);
        runtime.redeliver(handle, &w[..18]).expect("redeliver");
        pump(&mut runtime, &mut owed);
    }
    eprintln!("replayed skid posted at {posted_at} through {to}");
}

/// What do the `SFXObj_Collision` controllers actually output in the live runtime?
///
/// `sub_824D20E8` scales every contact voice by the Collision controller output for its material
/// category (13,14,15,16,17,18,12,19,21,20 for categories 0..=9). A retail capture of a playtest
/// reads those outputs at 660 / 455 / 14669 / 39 / 38 (outputs 12 / 13 / 17 / 18 / 20), i.e. -34
/// to -59 dB except 17's -7 dB -- which is why a landing's two collision voices, played here at
/// full material level, drowned the class voice. `collision_output_probe` reads zero for all of
/// them, but it drives only the Contacts inputs; these controllers are fed by the global mix
/// controllers, so this asks the question inside the runtime the game actually builds.
#[test]
#[ignore = "needs the owner's assets; run explicitly"]
fn headless_collision_controller_outputs() {
    use skate_audio_core::authored::PCM_FRAMES_PER_BLOCK;
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let super::Prepared {
        mut runtime,
        mut sound,
        ..
    } = super::prepare(&assets).expect("prepare the retail player-sound worker");
    let blocks_per_frame = 48_000.0 / f64::from(PCM_FRAMES_PER_BLOCK) / 60.0;
    let mut owed = 0.0f64;
    // Roll, hop, land: the landing is when these voices are opened.
    let script: Vec<PlayerAudioObservation> = (0..60)
        .map(|i| rolling(i, 5.0))
        .chain((0..42).map(|i| airborne(60 + i, 5.0, i as f32 / 60.0, 1.2)))
        .chain((0..60).map(|i| rolling(102 + i, 5.0)))
        .collect();
    for observation in &script {
        sound.frame(&mut runtime, observation).expect("frame");
        owed += blocks_per_frame;
        while owed >= 1.0 {
            owed -= 1.0;
            runtime.pump_once().expect("render one block");
        }
    }
    for instance in 0..3u32 {
        let key = 0x4003_0000 + 0x800 * instance;
        let Some(controller) = runtime.mixmap_controller(key) else {
            eprintln!("SFXObj_Collision #{instance}: no controller {key:#010x}");
            continue;
        };
        let levels: Vec<u32> = (12..=22)
            .map(|id| runtime.mixmap_level(controller, id))
            .collect();
        eprintln!("SFXObj_Collision #{instance} ({controller:#010x}) outputs 12..=22: {levels:?}");
    }
}

/// The landing ladder: does a landing's level follow retail's, all the way down to a 50 ms hop?
///
/// The owner's report is that *this port's* low ollies sound unlike retail's while its bigger
/// drops sound about right. Retail's own curve, extracted from
/// `.local/captures/retail-lowollie-20260920-205434.log` (35 landings; `OUT` block peaks in the
/// twelve frames after `Class_Treatment` word 7 returns to zero, binned by the air time that word
/// carried):
///
/// | air ms | n | p50 dBFS |
/// |---|---|---|
/// | 0–60 | 3 | −8.4 |
/// | 60–120 | 1 | −6.7 |
/// | 120–200 | 1 | −1.4 |
/// | 200–350 | 4 | −4.1 |
/// | 350–600 | 13 | −4.7 |
/// | 600–1100 | 9 | −1.2 |
/// | 1100+ | 4 | +6.8 |
///
/// Note the step at the bottom rather than a curve: below six frames of air the per-wheel
/// touchdown never latches (`sub_82772FD8`'s `frames > 5`, `audio_state.rs:392-415`), so the class
/// voice — which `ContactsOwner::landing` gates on `wheel_landed_464` — and the two collision
/// voices with it never play, and only the fixed impact and the ladder voice are left.
///
///     cargo test -p skate-game --bin skate3rust --locked -- --ignored landing_ladder --nocapture
#[test]
#[ignore = "needs the owner's assets; run explicitly"]
fn headless_landing_ladder_matches_retails_curve() {
    use skate_audio_core::authored::{PCM_CHANNELS, PCM_FRAMES_PER_BLOCK};
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    let db = |v: f32| 20.0 * v.max(1e-9).log10();
    println!("air ms |   min RMS | mean 200ms | mean 1.0 s |    spread   (retail 200 ms mean)");
    let mut rows = Vec::new();
    // Retail's own (air time, jump height) pairs, not a guess: `Class_Treatment` word 9 is
    // `jump_height × 166.667`, and holding it constant across the rungs mismeasures the heavy end,
    // where retail carries 1.5-2.2 m. These are the medians per band over the same 35 landings.
    for (frames, height) in [
        (3u64, 0.28f32),
        (6, 0.28),
        (9, 0.37),
        (15, 0.37),
        (27, 0.37),
        (48, 1.00),
        (72, 1.81),
    ] {
        let air_ms = (frames as f32 / 60.0 * 1000.0).round() as i32;
        // Band *means*, not p50s: retail's peaks scatter about 8 dB inside a single air-time band,
        // so a p50 over the three or four landings a band holds lands near its maximum.
        // Retail's own block RMS, same twelve-frame window, same binning.
        let retail = match air_ms {
            0..=119 => "-22.9",
            120..=499 => "-20.8",
            500..=999 => "-17.4",
            _ => "-12.8",
        };
        let mut repeats: Vec<f32> = Vec::new();
        let mut longs: Vec<f32> = Vec::new();
        {
            // **Six consecutive landings in one worker, not six fresh ones.** The Splice `Rand` is
            // seeded to 1 at construction (`contact_voices.rs:373`) and only advances when a one-shot
            // resolves, so a fresh worker always draws the *same* members, gains and delays — five
            // fresh workers measure one draw five times. Retail's generator is global, shared by ~100
            // call sites and running since boot, so every retail landing draws from a different point
            // in the sequence. Landing repeatedly inside one session is the closest this port gets to
            // that, and it is also what play actually does.
            let roll = 60u64;
            let super::Prepared {
                mut runtime,
                mut sound,
                ..
            } = super::prepare(&assets).expect("prepare the retail player-sound worker");
            let air_seconds = frames as f32 / 60.0;
            // `roll` varies the pre-roll length so each repeat advances the shared `Rand` differently.
            // Retail's generator is shared game-wide and cannot be reproduced, so a single draw is not
            // comparable to retail's mean: a landing's level depends on which members each group drew
            // and how their delays landed. Repeats give this side a spread to compare against retail's.
            // roll, hop, land, settle -- six times over.
            const CYCLES: usize = 6;
            let settle = 90u64;
            let mut script: Vec<PlayerAudioObservation> = Vec::new();
            let mut landing_frames = Vec::new();
            let mut tick = 0u64;
            for _ in 0..CYCLES {
                for _ in 0..roll {
                    script.push(rolling(tick, 5.0));
                    tick += 1;
                }
                for i in 0..frames {
                    let mut o = airborne(tick, 5.0, i as f32 / 60.0, height);
                    o.retail.air_time_until_landing = (air_seconds - i as f32 / 60.0).max(0.0);
                    script.push(o);
                    tick += 1;
                }
                landing_frames.push(script.len());
                for _ in 0..settle {
                    script.push(rolling(tick, 5.0));
                    tick += 1;
                }
            }
            let mut blocks_owed = 0.0f64;
            let blocks_per_frame = 48_000.0 / f64::from(PCM_FRAMES_PER_BLOCK) / 60.0;
            // RMS as well as peak, computed exactly as `trace::output` writes the `rms` field of an
            // `OUT` line, so it compares directly with the recomp capture's own. Retail's landing peak
            // scatters (25.8 dB inside one air band) while its RMS is cleanly monotonic
            // -22.9 / -20.8 / -17.4 / -12.8 dBFS, so RMS is the meter that answers "how heavy did that
            // landing sound".
            let (mut natives, mut matrices, mut devices) = (Vec::new(), Vec::new(), Vec::new());
            let mut rmss: Vec<f32> = Vec::new();
            for observation in script {
                sound
                    .frame(&mut runtime, &observation)
                    .expect("player-sound frame");
                blocks_owed += blocks_per_frame;
                let (mut native_peak, mut matrix_peak, mut device_peak) = (0.0f32, 0.0f32, 0.0f32);
                let mut frame_rms = 0.0f32;
                while blocks_owed >= 1.0 {
                    blocks_owed -= 1.0;
                    let native = runtime.pump_once().expect("render one block");
                    for sample in &native {
                        native_peak = native_peak.max(sample.abs());
                    }
                    for frame in native.chunks_exact(usize::from(PCM_CHANNELS)) {
                        for (front, surround) in [(frame[0], frame[4]), (frame[1], frame[5])] {
                            matrix_peak =
                                matrix_peak.max((front + 0.707 * frame[2] + 0.5 * surround).abs());
                        }
                    }
                    device_peak = super::downmix(&native)
                        .iter()
                        .fold(device_peak, |p, s| p.max(s.abs()));
                    let energy: f64 = native
                        .iter()
                        .filter(|s| s.is_finite())
                        .map(|s| f64::from(*s) * f64::from(*s))
                        .sum();
                    frame_rms = frame_rms.max((energy / native.len().max(1) as f64).sqrt() as f32);
                }
                rmss.push(frame_rms);
                natives.push(native_peak);
                matrices.push(matrix_peak);
                devices.push(device_peak);
            }
            // The same twelve-frame window the retail extraction used, once per landing.
            // Two windows. The 12-frame one is what the retail extraction used; the 60-frame one
            // exists because a class-2 container's kids are ~900 ms samples (record `+24` reads 890
            // and 966 ms) against class 1's ~500 ms, so a short window can under-read a heavier
            // landing whose energy is spread over a longer sample.
            let _ = (&matrices, &devices, &natives);
            for at in landing_frames {
                let short = rmss[at..(at + 12).min(rmss.len())]
                    .iter()
                    .fold(0f32, |p, s| p.max(*s));
                let long: f32 = {
                    let w = &rmss[at..(at + 60).min(rmss.len())];
                    (w.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>()
                        / w.len().max(1) as f64)
                        .sqrt() as f32
                };
                repeats.push(db(short));
                longs.push(db(long));
            }
        }
        repeats.sort_by(f32::total_cmp);
        let mean = repeats.iter().sum::<f32>() / repeats.len() as f32;
        let long_mean = longs.iter().sum::<f32>() / longs.len().max(1) as f32;
        println!(
            "{air_ms:6} | {:9.1} | {:9.1} | {:9.1} | {:9.1}   ({retail})",
            repeats[0],
            mean,
            long_mean,
            repeats[repeats.len() - 1] - repeats[0],
        );
        rows.push((air_ms, mean));
    }
    // The shape is the claim, not any single value. Retail's RMS rises 10.1 dB from its lightest
    // band (-22.9) to its heaviest (-12.8) and never falls; this engine rises 2.6 dB and then
    // drops at class 2, because the only class step the authored data supplies is
    // `landing_send_scale`'s +1.4 dB and the class-2 container averages fewer members than the
    // class-1 one. See `docs/engine-defects.md` 12. The assertion below is deliberately the weak
    // form -- monotonicity -- so it starts passing the moment the missing term is recovered,
    // rather than encoding a number that would then need changing.
    let low = rows.first().expect("a rung").1;
    let high = rows.last().expect("a rung").1;
    let heaviest_two = rows[rows.len() - 2].1;
    assert!(
        high > low,
        "a 1.2 s drop must be louder than a 50 ms hop: {high:.1} vs {low:.1} dBFS RMS"
    );
    assert!(
        high >= heaviest_two,
        "the ladder must not fall at the top: 1.2 s reads {high:.1} dBFS RMS against          0.8 s at {heaviest_two:.1} -- the class-2 step is missing, see engine-defects 12"
    );
}

/// The rolling bed across speed **and** surface, against retail's measured table.
///
/// A playtest cannot answer this cleanly: `docs/player-audio-retail-drivers.md` §11 tabulates
/// retail's grain records against ground speed over a session that crossed many surfaces, and the
/// grain position is a Bezier of `speed / maxKmh` where `maxKmh` is *per surface* (55-74 km/h).
/// Comparing a playtest pinned to one surface against that average cannot separate "the Bezier is
/// wrong" from "we are on a different surface", and the test world's collision tags are quad-group
/// indices rather than materials so it cannot vary them either. Here both are controlled.
///
/// Retail, for reference (truck 0, A player): position 0.121 at 10 km/h, 0.407 at 20, 0.340 at 25,
/// 0.394 at 30, 0.515 at 35, 0.634 at 40; gain A 0.06-0.17 across that range.
///
///     cargo test -p skate-game --bin skate3rust --locked -- --ignored speed_surface --nocapture
#[test]
#[ignore = "needs the owner's assets; run explicitly"]
fn headless_rolling_speed_and_surface_sweep() {
    let assets = std::env::var_os("SKATE_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_ASSETS.into());
    // Retail surface tags seen on University, plus the concrete this port used to pin everything
    // to. The tag is the raw `surface_tag & 0x7f`; the audio material is `tag - 1`.
    const TAGS: [(u32, &str); 5] = [
        (3, "material 2"),
        (4, "material 3"),
        (17, "material 16"),
        (42, "material 41"),
        (54, "material 53"),
    ];
    // Retail's position for the A player at each speed bin (§11), for the columns below.
    let retail_position = |kmh: u32| match kmh {
        10 => Some(0.121),
        15 => Some(0.140),
        20 => Some(0.407),
        25 => Some(0.340),
        30 => Some(0.394),
        35 => Some(0.515),
        40 => Some(0.634),
        45 => Some(0.596),
        _ => None,
    };
    for (tag, label) in TAGS {
        let super::Prepared {
            mut runtime,
            mut sound,
            ..
        } = super::prepare(&assets).expect("prepare the retail player-sound worker");
        println!("\nsurface tag {tag} ({label})");
        println!(" km/h |   gain A |   gain B |    pos A |    pos B | retail pos A");
        // `SKATE_SWEEP_SPEEDS=60,70` overrides the ladder; the order matters when chasing a
        // fault, because it distinguishes a speed-dependent one from a cumulative one.
        let speeds: Vec<u32> = std::env::var("SKATE_SWEEP_SPEEDS")
            .ok()
            .map(|v| v.split(',').filter_map(|p| p.trim().parse().ok()).collect())
            .unwrap_or_else(|| vec![5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 60, 70]);
        for kmh in speeds {
            let speed = kmh as f32 / 3.6;
            // Settle: the intensity slew and the MixMap envelopes both need a run-up, and the
            // grain position is read from the record the owner last wrote.
            // **Settle for five seconds, not 1.5.** The MixMap's outputs run through attack/hold/
            // release envelopes (`mixmap/eval.rs`, stage 4), so a short run reads them mid-ramp.
            // At 90 frames this sweep put the wind level at 694 where a three-second run gives 925
            // against retail's 975 -- a 2.9 dB error that is entirely settling, not engine
            // behaviour. Sampling only the last second keeps the reading on the plateau.
            let mut last = None;
            for tick in 0..300u64 {
                let mut observation = rolling(tick, speed);
                observation.retail.wheel_audio_surfaces = [tag; 4];
                observation.wheel_surface = tag;
                sound
                    .frame(&mut runtime, &observation)
                    .expect("player-sound frame");
                if tick >= 240 {
                    last = super::trace::last_grain_records(0).or(last);
                }
            }
            match last {
                Some((a, b)) => {
                    let reference = retail_position(kmh)
                        .map(|p| format!("{p:.3}"))
                        .unwrap_or_else(|| "  -- ".into());
                    println!(
                        "{kmh:5} | {:8.4} | {:8.4} | {:8.4} | {:8.4} | {reference}",
                        a.0, b.0, a.2, b.2
                    );
                }
                None => println!("{kmh:5} | (no grain record -- surface has no grain bed)"),
            }
        }
    }
}
