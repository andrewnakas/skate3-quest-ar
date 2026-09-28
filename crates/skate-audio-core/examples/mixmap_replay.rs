//! Replay the retail capture through the MixMap port and compare every output word.
//!
//! The capture (`retail-audio-capture-*.log`) has, per mixer evaluation, an `MX host dt count` line
//! followed by one `MC ctrl key inptr | 24 input words | outptr | 16 output words` line per
//! controller, dumped when the evaluation returns. The replay builds the host fresh (the capture
//! starts at the first evaluation), and for each `MX`: writes every controller's 16 input words,
//! evaluates with the captured dt, and compares the 16 output words of every controller.
//!
//!     cargo run --release --example mixmap_replay -- [capture.log] [assets dir] [max frames]
//!
//! Environment: `MODE=<hex>` overrides the evaluation's mode word (default FFFFFFFF, the value
//! `sub_824845B8` leaves at `sys+8`); `SHOW=<n>` prints the first n mismatching words.

use skate_audio_core::mixmap::{self, KeyedListener};
use skate_audio_core::patch::BumpHeap;
use skate_audio_core::{Guest, Segment};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const DEFAULT_ASSETS: &str = r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets";

fn image(dir: &Path) -> Vec<Segment> {
    let mut segs = Vec::new();
    for entry in std::fs::read_dir(dir).expect("image dir") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if let Some(hex) = name.strip_prefix("g_").and_then(|s| s.strip_suffix(".bin")) {
            let page = u32::from_str_radix(hex, 16).unwrap();
            segs.push(Segment {
                base: page << 16,
                bytes: std::fs::read(&path).unwrap(),
            });
        }
    }
    segs
}

fn default_capture() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/captures");
    let mut logs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    logs.retain(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("retail-audio-capture-"))
    });
    logs.sort();
    logs.pop()
        .expect("no retail-audio-capture-*.log under .local/captures")
}

struct Frame {
    dt: f64,
    /// (key, 16 inputs, 16 outputs)
    ctrls: Vec<(u32, [u32; 16], [u32; 16])>,
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let capture = args
        .next()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_capture);
    let assets = args
        .next()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ASSETS));
    let max_frames: usize = args
        .next()
        .map(|s| s.parse().unwrap())
        .unwrap_or(usize::MAX);
    let mode = std::env::var("MODE")
        .map(|s| hex(&s))
        .unwrap_or(0xFFFF_FFFF);
    let show: usize = std::env::var("SHOW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let file = std::fs::read(assets.join("private/stock/data/audio/MixMapSK8.mxb")).expect("mxb");
    let mut segs = image(&assets.join("private/stock/audio-runtime-image"));
    const HEAP: u32 = 0x4000_0000;
    const HEAP_LEN: u32 = 0x0080_0000;
    segs.push(Segment {
        base: HEAP,
        bytes: vec![0; HEAP_LEN as usize],
    });
    let mut g = Guest::from_segments(segs);
    let mut heap = BumpHeap {
        next: HEAP,
        end: HEAP + HEAP_LEN,
    };
    let mut listener = KeyedListener::retail();
    let mm = mixmap::load(&mut g, &mut heap, &mut listener, &file).expect("build");
    g.set_u32(mm.manager + 4, mode).unwrap();
    let by_key: HashMap<u32, u32> = mixmap::controllers(&g, mm.host)
        .unwrap()
        .into_iter()
        .map(|(c, k)| (k, c))
        .collect();
    eprintln!(
        "capture {}; {} controllers; mode {mode:08X}",
        capture.display(),
        by_key.len()
    );
    // TRACEB=<B key>: print that lookup's input block and state after every evaluation in FROM..TO.
    let trace_b: Option<u32> = std::env::var("TRACEB").ok().map(|s| {
        let e = mixmap::resolve(&mut g, &mut listener, mm.host, hex(&s), 0, 1).unwrap();
        g.u32(e + 4).unwrap()
    });
    let from: usize = std::env::var("FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let to: usize = std::env::var("TO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    let reader = BufReader::with_capacity(1 << 22, std::fs::File::open(&capture).expect("capture"));
    let mut current: Option<Frame> = None;
    let mut frames = 0usize;
    let (mut words, mut bad_words) = (0u64, 0u64);
    let mut bad_frames = 0usize;
    let mut per_key: HashMap<u32, (u64, u64)> = HashMap::new();
    let mut shown = 0usize;
    let mut first_bad: Option<usize> = None;

    // The game's position controllers (`sub_824AEE60`, SFXCTL_3DObjPos) set word 15 bit 31 when
    // id 13 changes sign and bit 30 when id 14 does; the B stage consumes and clears them, so the
    // post-evaluation dump never shows them. Rebuilt here from consecutive captured words
    // (`NOFLAGS=1` disables it). The object's first ever activation reads uninitialised fields and
    // cannot be rebuilt; those frames are expected to differ.
    let rebuild_flags = std::env::var("NOFLAGS").is_err();
    // RULESLOT=<n>: rebuild the sign-change bits only for that SFX slot's controllers.
    let rule_slot: Option<u32> = std::env::var("RULESLOT").ok().and_then(|s| s.parse().ok());
    // DTBIAS=<s>: add to every captured dt (the log prints dt to 6 decimals only).
    let dt_bias: f64 = std::env::var("DTBIAS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let mut prev_rates: HashMap<u32, (f32, f32)> = HashMap::new();
    // INPUTS=<hex,hex,...>: drive only these controllers; COMPARE=<hex,...>: compare only these.
    let list = |name: &str| -> Option<Vec<u32>> {
        std::env::var(name)
            .ok()
            .map(|s| s.split(',').map(hex).collect())
    };
    let only_inputs = list("INPUTS");
    let only_compare = list("COMPARE");
    // EXPORT=<path>: write the exact pre-evaluation guest words of every controller (delta-encoded)
    // plus the captured outputs of the local player's SFXObj controllers, for `mixmap::tests`.
    let export_path = std::env::var("EXPORT").ok();
    let ctrl_list = mixmap::controllers(&g, mm.host).unwrap();
    let player_keys: Vec<u32> = (0..13).map(|o| mixmap::key(2, 1, 0, o)).collect();
    let mut export: Vec<u8> = Vec::new();
    let mut export_frames = 0u32;
    let mut snapshot: Vec<[u32; 17]> = vec![[0; 17]; ctrl_list.len()];
    let mut run = |frame: Frame, g: &mut Guest, index: usize| {
        for (key, inputs, _) in &frame.ctrls {
            if only_inputs.as_ref().is_some_and(|l| !l.contains(key)) {
                continue;
            }
            let Some(&c) = by_key.get(key) else {
                panic!("capture key {key:08X} not built")
            };
            let block = g.u32(c + 8).unwrap();
            if block != 0 {
                let mut words = *inputs;
                // Only while active (word 15 bit 0): `sub_824AEE60` runs from `sub_824AEC70`'s active
                // branch, so its previous-rate fields (+100/+108) keep the last active frame's values.
                if rebuild_flags && key >> 29 == 3 && words[15] & 1 != 0 {
                    let (r13, r14) = (f32::from_bits(words[13]), f32::from_bits(words[14]));
                    let (p13, p14) = prev_rates.get(key).copied().unwrap_or((0.0, 0.0));
                    let flips = |p: f32, n: f32| (p < 0.0 && n > 0.0) || (p > 0.0 && n < 0.0);
                    // Bits the B stage has not consumed yet persist in the block (the game only ever
                    // ORs them in), so carry what our last evaluation left.
                    let pending = g.u32(block + 60).unwrap() & 0xC000_0000;
                    words[15] = (words[15] & 0x3FFF_FFFF) | pending;
                    let apply = rule_slot.is_none_or(|s| (key >> 16) & 0xFF == s);
                    if apply && flips(p13, r13) {
                        words[15] |= 0x8000_0000;
                    }
                    if apply && flips(p14, r14) {
                        words[15] |= 0x4000_0000;
                    }
                    prev_rates.insert(*key, (r13, r14));
                }
                for (i, w) in words.iter().enumerate() {
                    g.set_u32(block + 4 * i as u32, *w).unwrap();
                }
            }
        }
        // Output word 15 is the block's enable word, which only the game writes (controller slots
        // 20/24, `sub_8294BD98`/`sub_8294BDB0`); the evaluation reads it. Apply the captured value.
        for (key, _, outputs) in &frame.ctrls {
            let block = g.u32(by_key[key] + 12).unwrap();
            if block != 0 {
                g.set_u32(block + 60, outputs[15]).unwrap();
            }
        }
        if let Some(st) = trace_b.filter(|_| index >= from && index <= to) {
            let blk = g.u32(st + 4).unwrap();
            let w: Vec<String> = (0..16)
                .map(|k| format!("{:08X}", g.u32(blk + 4 * k).unwrap()))
                .collect();
            println!("f{index} in  {}", w.join(" "));
        }
        if export_path.is_some() {
            let mut changes: Vec<(u16, u8, u32)> = Vec::new();
            for (n, (c, _)) in ctrl_list.iter().enumerate() {
                let (inb, outb) = (g.u32(c + 8).unwrap(), g.u32(c + 12).unwrap());
                for w in 0..17u32 {
                    let v = match w {
                        0..=15 if inb != 0 => g.u32(inb + 4 * w).unwrap(),
                        16 if outb != 0 => g.u32(outb + 60).unwrap(),
                        _ => 0,
                    };
                    if snapshot[n][w as usize] != v {
                        snapshot[n][w as usize] = v;
                        changes.push((n as u16, w as u8, v));
                    }
                }
            }
            export.extend_from_slice(&(frame.dt + dt_bias).to_le_bytes());
            export.extend_from_slice(&(changes.len() as u32).to_le_bytes());
            for (n, w, v) in changes {
                export.extend_from_slice(&n.to_le_bytes());
                export.push(w);
                export.extend_from_slice(&v.to_le_bytes());
            }
            for pk in &player_keys {
                let outs = frame
                    .ctrls
                    .iter()
                    .find(|(k, _, _)| k == pk)
                    .map(|(_, _, o)| *o)
                    .unwrap_or([0; 16]);
                for o in outs {
                    export.extend_from_slice(&o.to_le_bytes());
                }
            }
            export_frames += 1;
        }
        mixmap::tick(g, mm.manager, frame.dt + dt_bias).unwrap();
        if export_path.is_some() {
            // Diff the next frame against what the evaluation left (it clears B flags in place).
            for (n, (c, _)) in ctrl_list.iter().enumerate() {
                let (inb, outb) = (g.u32(c + 8).unwrap(), g.u32(c + 12).unwrap());
                for w in 0..17u32 {
                    snapshot[n][w as usize] = match w {
                        0..=15 if inb != 0 => g.u32(inb + 4 * w).unwrap(),
                        16 if outb != 0 => g.u32(outb + 60).unwrap(),
                        _ => 0,
                    };
                }
            }
        }
        if let Some(st) = trace_b.filter(|_| index >= from && index <= to) {
            let f = |o: u32| f32::from_bits(g.u32(st + o).unwrap());
            println!(
                "f{index} st  i8 {} mb {} lin {} slew {} last {} delta {} dt {}",
                g.u32(st + 8).unwrap() as i32,
                g.u32(st + 12).unwrap() as i32,
                g.u32(st + 16).unwrap() as i32,
                g.u32(st + 20).unwrap() as i32,
                f(24),
                f(28),
                frame.dt
            );
        }
        let mut frame_bad = false;
        for (key, _, outputs) in &frame.ctrls {
            if only_compare.as_ref().is_some_and(|l| !l.contains(key)) {
                continue;
            }
            let c = by_key[key];
            let block = g.u32(c + 12).unwrap();
            let entry = per_key.entry(*key).or_default();
            for (i, want) in outputs.iter().enumerate() {
                let got = if block == 0 {
                    0
                } else {
                    g.u32(block + 4 * i as u32).unwrap()
                };
                words += 1;
                entry.0 += 1;
                if got != *want {
                    bad_words += 1;
                    entry.1 += 1;
                    frame_bad = true;
                    if shown < show {
                        shown += 1;
                        println!(
                            "frame {index} key {key:08X} word {i:2}: got {got:08X} want {want:08X}"
                        );
                    }
                }
            }
        }
        if frame_bad {
            bad_frames += 1;
            if first_bad.is_none() {
                first_bad = Some(index);
            }
        }
    };

    for line in reader.lines() {
        let line = line.unwrap();
        let mut f = line.split_ascii_whitespace();
        let (_ms, fr, tag) = (f.next(), f.next(), f.next());
        match tag {
            Some("MX") => {
                if let Some(frame) = current.take() {
                    frames += 1;
                    run(frame, &mut g, frames);
                    if frames >= max_frames {
                        break;
                    }
                }
                let _host = f.next();
                let dt: f64 = f.next().unwrap().parse().unwrap();
                let _ = fr;
                current = Some(Frame {
                    dt,
                    ctrls: Vec::with_capacity(247),
                });
            }
            Some("MC") => {
                let Some(frame) = current.as_mut() else {
                    continue;
                };
                let _ctrl = f.next();
                let key = hex(f.next().unwrap());
                let _inptr = f.next();
                f.next(); // |
                let mut inputs = [0u32; 16];
                for i in 0..24 {
                    let w = hex(f.next().unwrap());
                    if i < 16 {
                        inputs[i] = w;
                    }
                }
                f.next(); // |
                let _outptr = f.next();
                f.next(); // |
                let mut outputs = [0u32; 16];
                for o in outputs.iter_mut() {
                    *o = hex(f.next().unwrap());
                }
                frame.ctrls.push((key, inputs, outputs));
            }
            _ => {}
        }
    }
    if frames < max_frames {
        if let Some(frame) = current.take() {
            frames += 1;
            run(frame, &mut g, frames);
        }
    }
    drop(run);
    if let Some(path) = &export_path {
        let mut head = b"MMRP".to_vec();
        head.extend_from_slice(&export_frames.to_le_bytes());
        head.extend_from_slice(&(ctrl_list.len() as u32).to_le_bytes());
        for (_, k) in &ctrl_list {
            head.extend_from_slice(&k.to_le_bytes());
        }
        head.extend_from_slice(&export);
        std::fs::write(path, head).unwrap();
        eprintln!("exported {export_frames} frames to {path}");
    }
    println!(
        "\n{frames} evaluations, {words} output words compared, {} mismatching ({:.4}% match); {bad_frames} frames with a mismatch, first {:?}",
        bad_words,
        100.0 * (words - bad_words) as f64 / words.max(1) as f64,
        first_bad
    );
    let mut keys: Vec<_> = per_key.into_iter().filter(|(_, (_, b))| *b > 0).collect();
    keys.sort();
    for (k, (n, b)) in keys {
        println!("  key {k:08X}: {b} of {n} words differ");
    }
}
