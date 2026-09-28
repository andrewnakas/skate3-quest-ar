//! Which Contacts controller outputs move when the landing-weight input moves?
//!
//! `ContactsInputs::process` writes input 2 from the landing class (1 -> 16000,
//! 2 -> 32767, else 0), but the landing one-shot takes a constant gain, so a
//! curb drop and a roof gap open at the same level. To route the one-shot
//! through the controller instead, we need to know which output id that input
//! actually reaches. This drives the real MixMap and diffs the outputs.
//!
//!     cargo run --example contacts_input_probe -- [assets dir]

use skate_audio_core::mixmap::{self, KeyedListener, controller};
use skate_audio_core::patch::BumpHeap;
use skate_audio_core::{Guest, Segment};
use std::path::{Path, PathBuf};

const COLLISION_KEY: u32 = 0x4003_0000;
const DEFAULT_ASSETS: &str = r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets";
/// `SFXObj_Contacts #0`, the local player's.
const CONTACTS_KEY: u32 = 0x4001_0010;
const OUTPUT_IDS: u32 = 24;

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

/// Every output id, read three ways, after settling the graph.
fn sample(g: &mut Guest, host: u32, ctrl: u32) -> Vec<(u32, u32, u32, u32)> {
    // The graph smooths, so let it settle rather than reading one frame.
    for _ in 0..240 {
        mixmap::evaluate(g, host, 0, 1.0 / 60.0).expect("evaluate");
    }
    (0..OUTPUT_IDS)
        .map(|id| {
            (
                id,
                controller::read_u16(g, ctrl, id).unwrap_or(u32::MAX),
                controller::read_gain(g, ctrl, id).unwrap_or(u32::MAX),
                controller::read_pitch(g, ctrl, id).unwrap_or(u32::MAX),
            )
        })
        .collect()
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
    let ctrl = mixmap::find_controller(&g, mm.host, CONTACTS_KEY)
        .expect("lookup")
        .expect("Contacts #0 controller");
    println!("Contacts #0 controller at {ctrl:#010x}");
    // Without the retail pre-roll every upstream controller (global faders,
    // pause, ducking) sits at 0 and the graph collapses to silence, so nothing
    // downstream can move however the probe drives it.
    let ctrls = mixmap::controllers(&g, mm.host).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/captures/extract/mixmap_replay_4400.bin");
    let bytes = std::fs::read(&fixture).expect("replay fixture");
    preroll(&mut g, mm.manager, &ctrls, &bytes, 3600);
    println!("pre-rolled 3600 retail evaluations\n");

    // Input 1 is the landing pulse; retail raises it on the same frame as 2.
    for probe_input in [2u32, 1, 6] {
        let mut seen: Vec<(u32, Vec<(u32, u32, u32, u32)>)> = Vec::new();
        for value in [0u32, 16_000, 32_767] {
            controller::set(
                &mut g,
                ctrl,
                1,
                if probe_input == 1 { value } else { 32_767 },
            )
            .expect("set 1");
            controller::set(&mut g, ctrl, probe_input, value).expect("set probe");
            seen.push((value, sample(&mut g, mm.host, ctrl)));
        }
        let (lo, hi) = (&seen[0].1, &seen[2].1);
        let moved: Vec<_> = lo
            .iter()
            .zip(hi)
            .filter(|(a, b)| a != b)
            .map(|(a, b)| (a.0, a.1, b.1, a.2, b.2))
            .collect();
        if moved.is_empty() {
            println!("input {probe_input}: no output changed between 0 and 32767");
        } else {
            println!("input {probe_input}: outputs that moved (id: u16 lo->hi, gain lo->hi)");
            for (id, u_lo, u_hi, g_lo, g_hi) in moved {
                let mid = seen[1].1[id as usize].1;
                println!("   id {id:2}: u16 {u_lo} -> {mid} -> {u_hi}   gain {g_lo} -> {g_hi}");
            }
        }
        println!();
    }
}
