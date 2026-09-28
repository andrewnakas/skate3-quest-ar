//! Which *unwritten* MixMap inputs actually move the player's landing, rolling and wind levels?
//!
//!     cargo run --release --example player_input_probe -- [assets dir]
//!
//! `mixmap_dump`'s dependency walk says `SFXObj_Collision`'s outputs — the `controllerOutput`
//! term `sub_824D2318` multiplies every contact voice by, which this port stands in for with
//! `collision_controller_scale`'s median table — depend on **`SFXObj_Contacts` input 7**, and not
//! on Collision's own inputs 0/1 at all. (`collision_output_probe` sweeps 0/1, which is why it
//! sees nothing move.) It also says the SkateBoard and SenseOfSpeed outputs depend on VU id 0,
//! which `inputs::FREE_SKATE_MUSIC_VU` pins at 32767 — its modal value in only 24% of the retail
//! capture's evaluations.
//!
//! This prerolls the retail replay fixture, prints what retail's own words settled to, then drives
//! each candidate input across its range and reports how far each output moves. A candidate that
//! moves nothing is ruled out; one that moves an output is the missing driver for that output.

use skate_audio_core::mixmap::{self, KeyedListener, controller};
use skate_audio_core::patch::BumpHeap;
use skate_audio_core::{Guest, Segment};
use std::path::{Path, PathBuf};

const DEFAULT_ASSETS: &str = r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets";

/// `sub_824D20E8`'s jump table: material category -> the Collision output its voice gain reads.
const CATEGORY_IDS: [u32; 10] = [13, 14, 15, 16, 17, 18, 12, 19, 21, 20];

const COLLISION: u32 = 0x4003_0000;
const CONTACTS: u32 = 0x4001_0010;
const SKATEBOARD: u32 = 0x4001_0000;
const SENSE_OF_SPEED: u32 = 0x4001_0080;
const VU: u32 = 0x4000_00A0;
const STATE_1: u32 = 0x6001_0800;

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

/// Apply the fixture's delta-encoded pre-evaluation words and evaluate, `frames` times.
fn preroll(g: &mut Guest, manager: u32, ctrls: &[(u32, u32)], bytes: &[u8], frames: usize) {
    let u32le = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let n = u32le(8) as usize;
    let mut at = 12 + 4 * n;
    for _ in 0..frames {
        let dt = f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let changes = u32le(at + 8) as usize;
        at += 12;
        for _ in 0..changes {
            let c = u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
            let w = bytes[at + 2] as u32;
            let v = u32le(at + 3);
            at += 7;
            let ctrl = ctrls[c].0;
            let (inb, outb) = (g.u32(ctrl + 8).unwrap(), g.u32(ctrl + 12).unwrap());
            if w < 16 {
                g.set_u32(inb + 4 * w, v).unwrap();
            } else {
                g.set_u32(outb + 60, v).unwrap();
            }
        }
        at += 13 * 64;
        mixmap::tick(g, manager, dt).unwrap();
    }
}

/// The graph smooths, so settle rather than reading one frame.
fn settle(g: &mut Guest, manager: u32) {
    for _ in 0..240 {
        mixmap::tick(g, manager, 1.0 / 60.0).unwrap();
    }
}

/// Every watched output, as the level word each consumer reads.
fn watch(g: &Guest, ctrls: &[(u32, u32)]) -> Vec<(String, u32)> {
    let find = |key: u32| ctrls.iter().find(|(_, k)| *k == key).map(|(c, _)| *c);
    let mut out = Vec::new();
    if let Some(c) = find(COLLISION) {
        for (category, id) in CATEGORY_IDS.iter().enumerate() {
            out.push((
                format!("Collision cat{category} out{id}"),
                controller::read_gain(g, c, *id).unwrap_or(u32::MAX),
            ));
        }
    }
    if let Some(c) = find(SKATEBOARD) {
        // `sub_824C6BD8`: grain gains vfunc60(1)/(2); held rolling layers w11 = level(7)/(9);
        // the grain chain's local sends 21/22; the environment send 13.
        for id in [1u32, 2, 7, 9, 13, 21, 22] {
            out.push((
                format!("SkateBoard level({id})"),
                controller::read_gain(g, c, id).unwrap_or(u32::MAX),
            ));
        }
    }
    if let Some(c) = find(SENSE_OF_SPEED) {
        // wind w7 = level(4); rattle w7 = level(1).
        for id in [1u32, 4, 5] {
            out.push((
                format!("SenseOfSpeed level({id})"),
                controller::read_gain(g, c, id).unwrap_or(u32::MAX),
            ));
        }
    }
    out
}

fn main() {
    let assets = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ASSETS));
    let file = std::fs::read(assets.join("private/stock/data/audio/MixMapSK8.mxb"))
        .expect("MixMapSK8.mxb");
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
    let ctrls = mixmap::controllers(&g, mm.host).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/captures/extract/mixmap_replay_4400.bin");
    let bytes = std::fs::read(&fixture).expect("replay fixture");
    preroll(&mut g, mm.manager, &ctrls, &bytes, 3600);
    println!("pre-rolled 3600 retail evaluations from the capture fixture\n");

    let find = |key: u32| ctrls.iter().find(|(_, k)| *k == key).map(|(c, _)| *c);
    // What did retail's own words settle to? An input the capture leaves at 0 is one the retail
    // free-skate session never used, not necessarily one that does nothing.
    println!("retail's settled input words after the preroll:");
    for (name, key, ids) in [
        ("Contacts #0", CONTACTS, &[0u32, 1, 2, 6, 7, 8][..]),
        ("Collision #0", COLLISION, &[0, 1][..]),
        ("VU", VU, &[0, 1][..]),
        ("state #1", STATE_1, &[9][..]),
    ] {
        let Some(c) = find(key) else { continue };
        let inb = g.u32(c + 8).unwrap();
        let words: Vec<String> = ids
            .iter()
            .map(|id| format!("{id}={}", g.u32(inb + 4 * id).unwrap()))
            .collect();
        println!("  {name:14} {key:08X}  {}", words.join("  "));
    }

    settle(&mut g, mm.manager);
    let base = watch(&g, &ctrls);
    println!("\nbaseline after settling (the values this port would read):");
    for (name, v) in &base {
        println!("  {name:28} {v:6}");
    }

    // Each candidate: the input `mixmap_dump` says reaches these outputs but this port never
    // writes. Drive it to full scale and report what moved.
    let candidates: [(&str, u32, u32, u32); 5] = [
        ("Contacts #0 input 7 (sub_824BC188)", CONTACTS, 7, 32767),
        ("Contacts #0 input 8 (sub_824BC188)", CONTACTS, 8, 32767),
        ("Collision #0 input 0 (control)", COLLISION, 0, 32767),
        ("Collision #0 input 1 (control)", COLLISION, 1, 32767),
        ("state #1 input 9 (remote player)", STATE_1, 9, 32767),
    ];
    for (label, key, id, value) in candidates {
        let Some(c) = find(key) else {
            println!("\n{label}: controller {key:08X} absent");
            continue;
        };
        let inb = g.u32(c + 8).unwrap();
        let saved = g.u32(inb + 4 * id).unwrap();
        controller::set(&mut g, c, id, value).expect("set");
        settle(&mut g, mm.manager);
        let now = watch(&g, &ctrls);
        let moved: Vec<String> = base
            .iter()
            .zip(&now)
            .filter(|((_, a), (_, b))| a != b)
            .map(|((n, a), (_, b))| format!("{n}: {a} -> {b}"))
            .collect();
        println!("\n{label}  ({key:08X}.{id} := {value}, was {saved})");
        if moved.is_empty() {
            println!("  nothing moved");
        } else {
            for line in moved {
                println!("  {line}");
            }
        }
        controller::set(&mut g, c, id, saved).expect("restore");
        settle(&mut g, mm.manager);
    }

    // Phase 2: the live engine does not run the fixture -- it applies `FREE_SKATE_GLOBALS` and
    // `FREE_SKATE_MUSIC_VU` once at startup and then drives only the player controllers. The
    // fixture's words all settle to 0 above, so the baseline is not what the game sees. Apply the
    // engine's own global setup and ask the same question again.
    println!(
        "
=== phase 2: the live engine's global setup applied over the preroll ==="
    );
    for &(key, id, value) in skate_audio_core::mixmap::inputs::FREE_SKATE_GLOBALS
        .iter()
        .chain(skate_audio_core::mixmap::inputs::FREE_SKATE_MUSIC_VU)
    {
        if let Some(c) = find(key) {
            controller::set(&mut g, c, id, value).expect("global");
        } else {
            println!("  (global {key:08X} absent)");
        }
    }
    settle(&mut g, mm.manager);
    let base2 = watch(&g, &ctrls);
    println!("baseline with the engine's globals:");
    for ((name, before), (_, after)) in base.iter().zip(&base2) {
        let mark = if before == after { "" } else { "   <-- moved" };
        println!("  {name:28} {before:6} -> {after:6}{mark}");
    }

    // The remaining unwritten dependency of the Collision outputs: its own 3D position
    // controller (`60030000`), which this port never creates a contact emitter for.
    println!(
        "
sweeping every input of the Collision position controller 60030000:"
    );
    if let Some(c) = find(0x6003_0000) {
        for id in 0..16u32 {
            let inb = g.u32(c + 8).unwrap();
            let saved = g.u32(inb + 4 * id).unwrap();
            controller::set(&mut g, c, id, 32767).expect("set");
            settle(&mut g, mm.manager);
            let now = watch(&g, &ctrls);
            let moved: Vec<String> = base2
                .iter()
                .zip(&now)
                .filter(|((n, a), (_, b))| a != b && n.starts_with("Collision"))
                .map(|((n, a), (_, b))| format!("{n}: {a} -> {b}"))
                .collect();
            if !moved.is_empty() {
                println!("  input {id} (was {saved}): {}", moved.join("  "));
            }
            controller::set(&mut g, c, id, saved).expect("restore");
        }
        settle(&mut g, mm.manager);
    } else {
        println!("  controller 60030000 absent");
    }

    // And re-ask the Contacts/Collision candidates now that the globals are up.
    println!(
        "
candidates again, with the engine's globals up:"
    );
    for (label, key, id, value) in candidates {
        let Some(c) = find(key) else { continue };
        let inb = g.u32(c + 8).unwrap();
        let saved = g.u32(inb + 4 * id).unwrap();
        controller::set(&mut g, c, id, value).expect("set");
        settle(&mut g, mm.manager);
        let now = watch(&g, &ctrls);
        let moved: Vec<String> = base2
            .iter()
            .zip(&now)
            .filter(|((_, a), (_, b))| a != b)
            .map(|((n, a), (_, b))| format!("{n}: {a} -> {b}"))
            .collect();
        println!(
            "  {label}: {}",
            if moved.is_empty() {
                "nothing moved".to_string()
            } else {
                moved.join("  ")
            }
        );
        controller::set(&mut g, c, id, saved).expect("restore");
        settle(&mut g, mm.manager);
    }

    // Phase 3: id 15 is a bitfield, not a level (`ObjPos::update` or-s bit 0 in when the emitter
    // has a position). Find which bit switches the bus on, and then whether the level that comes
    // up depends on the emitter's distance (ids 0/1) -- i.e. whether a real emitter position is
    // needed or only the active flag.
    println!(
        "
=== phase 3: which bit of 60030000.15, and does distance matter? ==="
    );
    if let Some(c) = find(0x6003_0000) {
        for bit in 0..16u32 {
            controller::set(&mut g, c, 15, 1 << bit).expect("set");
            settle(&mut g, mm.manager);
            let now = watch(&g, &ctrls);
            let live: Vec<String> = now
                .iter()
                .filter(|(n, v)| n.starts_with("Collision") && *v != 0)
                .map(|(n, v)| format!("{}={v}", n.trim_start_matches("Collision ")))
                .collect();
            if !live.is_empty() {
                println!("  bit {bit} (word {}): {}", 1u32 << bit, live.join(" "));
            }
        }
        // Bit 0 alone, then sweep the two distances the panner reads.
        controller::set(&mut g, c, 15, 1).expect("set");
        println!(
            "
  with bit 0 alone, sweeping the emitter distance (ids 0 and 1, f32 metres):"
        );
        for metres in [0.0f32, 1.0, 3.0, 10.0, 30.0, 100.0] {
            controller::set(&mut g, c, 0, metres.to_bits()).expect("d0");
            controller::set(&mut g, c, 1, metres.to_bits()).expect("d1");
            settle(&mut g, mm.manager);
            let now = watch(&g, &ctrls);
            let cats: Vec<String> = now
                .iter()
                .filter(|(n, _)| n.starts_with("Collision"))
                .map(|(_, v)| format!("{v}"))
                .collect();
            println!("    {metres:6.1} m -> [{}]", cats.join(" "));
        }
    }

    // Phase 4: does raising Contacts input 2 -- the landing class, 0 / 16000 / 32767 -- pull any
    // output *down*? The headless ladder has a class-2 landing measuring 0.9 dB quieter than a
    // class-1 one on both peak and RMS, when its sample content and its send are both larger, and
    // input 2 is the only other thing that differs. Print every output of every player controller
    // that moves, not just the ones a landing is known to read.
    println!(
        "
=== phase 4: what moves when the landing class (Contacts input 2) rises? ==="
    );
    if let Some(c) = find(CONTACTS) {
        let watched: Vec<(&str, u32)> = vec![
            ("Contacts", CONTACTS),
            ("SkateBoard", SKATEBOARD),
            ("SenseOfSpeed", SENSE_OF_SPEED),
            ("Collision", COLLISION),
        ];
        let snapshot = |g: &mut Guest, mm_manager: u32| {
            settle(g, mm_manager);
            let mut out = Vec::new();
            for (name, key) in &watched {
                if let Some(ctrl) = find(*key) {
                    for id in 0..24u32 {
                        out.push((
                            format!("{name} out{id}"),
                            controller::read_gain(g, ctrl, id).unwrap_or(u32::MAX),
                        ));
                    }
                }
            }
            out
        };
        controller::set(&mut g, c, 2, 0).expect("set");
        let at0 = snapshot(&mut g, mm.manager);
        for value in [16000u32, 32767] {
            controller::set(&mut g, c, 2, value).expect("set");
            let now = snapshot(&mut g, mm.manager);
            println!("  input 2 = {value} (from 0):");
            let mut moved = 0;
            for ((name, a), (_, b)) in at0.iter().zip(&now) {
                if a != b {
                    let arrow = if b > a { "up  " } else { "DOWN" };
                    println!("    {arrow} {name:22} {a:6} -> {b:6}");
                    moved += 1;
                }
            }
            if moved == 0 {
                println!("    nothing moved");
            }
        }
        controller::set(&mut g, c, 2, 0).expect("restore");
        settle(&mut g, mm.manager);
    }

    // VU id 0 is a sweep, not a switch: the port pins it at 32767 and retail's varies.
    let Some(vu) = find(VU) else { return };
    println!("\nVU id 0 sweep (the port pins it at 32767; retail's varies over a level):");
    for value in [0u32, 8192, 16384, 24576, 32767] {
        controller::set(&mut g, vu, 0, value).expect("set");
        settle(&mut g, mm.manager);
        let now = watch(&g, &ctrls);
        let shown: Vec<String> = now
            .iter()
            .filter(|(n, _)| n.starts_with("SkateBoard") || n.starts_with("SenseOfSpeed"))
            .map(|(n, v)| {
                let short = n
                    .trim_start_matches("SkateBoard ")
                    .trim_start_matches("SenseOfSpeed ");
                format!("{short}={v}")
            })
            .collect();
        println!("  VU0={value:5}  {}", shown.join("  "));
    }
}
