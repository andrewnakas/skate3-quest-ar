//! Build the retail MixMap from `MixMapSK8.mxb` over the dumped guest image and print every
//! controller (key, owner, output ids the file writes), the capacity check, and a short simulated
//! run of the local SkateBoard controller driven through the audio-state controller's inputs.
//!
//!     cargo run --example mixmap_dump -- [assets dir]
//!
//! The assets dir defaults to the owner install; the image is `private/stock/audio-runtime-image`,
//! the file `private/stock/data/audio/MixMapSK8.mxb`.

use skate_audio_core::mixmap::{self, KeyedListener, controller};
use skate_audio_core::patch::BumpHeap;
use skate_audio_core::{Guest, Segment};
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

/// The SFX object names by (slot, object id), from the factory table at `0x8302CF60..0x8302D3A0`.
fn owner(key: u32) -> String {
    let slot = (key >> 16) & 0xFF;
    let obj = (key >> 4) & 0x7F;
    let group = (key >> 11) & 0x1F;
    let ctl = key >> 29 == 3;
    let name = match (ctl, slot, obj) {
        (true, 1, 0) => "SFXCTL_PlayerPhysics (audio state)",
        (true, 1, 1) => "SFXCTL_3DObjPos",
        (true, 4, 0) => "SFXCTL_TrafficCarPhysics",
        (true, 5, 0) => "SFXCTL_PedestrianPhysics",
        (true, 8, 1) => "SFXCTL_DynamicObjectPhysics",
        (true, 9, 1) => "SFXCTL_SpeakerPhysics",
        (true, 0xC, 0) => "SFXCTL_NISCharacterPhysics",
        (true, 0xD, 0) => "SFXCTL_SpeechPhysics",
        (false, 0, 0) => "SFXObj_Announcer",
        (false, 0, 1) => "SFXObj_Music",
        (false, 0, 2) => "SFXObj_Master",
        (false, 0, 3) => "SFXObj_CameraMan",
        (false, 0, 5) => "SFXObj_Reverb",
        (false, 0, 6) => "SFXObj_NIS",
        (false, 0, 7) => "SFXObj_Pause",
        (false, 0, 8) => "SFXObj_Speech",
        (false, 0, 9) => "SFXObj_Bloom",
        (false, 0, 10) => "SFXObj_VU",
        (false, 0, 11) => "SFXObj_Challenge",
        (false, 0, 12) => "SFXObj_HOM",
        (false, 0, 13) => "SFXObj_Menu",
        (false, 0, 14) => "SFXObj_Jitter",
        (false, 1, 0) => "SFXObj_SkateBoard",
        (false, 1, 1) => "SFXObj_Contacts",
        (false, 1, 2) => "SFXObj_Wheels",
        (false, 1, 3) => "SFXObj_Rail",
        (false, 1, 4) => "SFXObj_Cracks",
        (false, 1, 5) => "SFXObj_Tricks",
        (false, 1, 6) => "SFXObj_Clothing",
        (false, 1, 7) => "SFXObj_Treatments",
        (false, 1, 8) => "SFXObj_SenseOfSpeed",
        (false, 1, 9) => "SFXObj_OffBoard",
        (false, 1, 10) => "SFXObj_HandGrabs",
        (false, 1, 11) => "SFXObj_Takedown",
        (false, 1, 12) => "SFXObj_DropIn",
        (false, 2, 0) => "SFXObj_Ambience",
        (false, 3, 0) => "SFXObj_Collision",
        (false, 4, 0) => "SFXObj_TrafficEngine",
        (false, 4, 1) => "SFXObj_TrafficSkids",
        (false, 4, 2) => "SFXObj_TrafficHorn",
        (false, 4, 3) => "SFXObj_TrafficWoosh",
        (false, 5, 0) => "SFXObj_PedestrianSpeech",
        (false, 5, 1) => "SFXObj_PedestrianSFX",
        (false, 5, 2) => "SFXObj_PedBodyFall",
        (false, 5, 3) => "SFXObj_Tazer",
        (false, 6, 0) => "SFXObj_Emitter",
        (false, 7, 0) => "SFXObj_Crowd",
        (false, 8, 0) => "SFXObj_Dynamic",
        (false, 8, 1) => "SFXObj_Moving",
        (false, 9, 0) => "SFXObj_Speaker",
        (false, 0xA, 0) => "SFXObj_Whoosh",
        (false, 0xB, 0) => "SFXObj_ObjectInstance",
        (false, 0xC, 0) => "SFXObj_NISCharacter",
        (false, 0xD, 0) => "SFXObj_PlayerSpeech",
        _ => "?",
    };
    format!("{name} #{group}")
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
    println!(
        "host {:#010x}  data {:#010x}  heap used {} B",
        mm.host,
        mm.data,
        heap.next - HEAP
    );
    println!("\ncapacity vs used:");
    for (name, cap, used) in mixmap::check_capacities(&g, mm.host).unwrap() {
        println!(
            "  {name:32} {cap:6} {used:6}{}",
            if cap != used { "   <-- differs" } else { "" }
        );
    }
    let ctrls = mixmap::controllers(&g, mm.host).unwrap();
    println!("\n{} controllers:", ctrls.len());
    // Output ids per block: walk the output entries (host+448) for each controller's block.
    let n_out = g.u32(mm.host + 476).unwrap();
    let outs = g.u32(mm.host + 448).unwrap();
    for (c, key) in &ctrls {
        let block = g.u32(c + 12).unwrap();
        let mut ids = Vec::new();
        for i in 0..n_out {
            let e = outs + 8 * i;
            let st = g.u32(e + 4).unwrap();
            if g.u32(st + 16).unwrap() != block || block == 0 {
                continue;
            }
            let def = g.u32(e).unwrap();
            let desc = g.u32(def + 12).unwrap();
            let head = g.u32(desc).unwrap();
            for n in 0..(head & 0x1F) {
                let d = g.u32(desc + 4 + 4 * n).unwrap();
                ids.push(format!("{}:t{}", (d >> 26) & 0x1F, (head >> 24) & 0xF));
            }
        }
        println!(
            "  {c:#010x} key {key:08X} in {:#010x} out {:#010x}  {:40} outputs [{}]",
            g.u32(c + 8).unwrap(),
            block,
            owner(*key),
            ids.join(" ")
        );
    }

    // Input sources: walk every output of every controller back to the controller input words it
    // reads, through products (A), lookups (B), envelopes (F), sums (C) and outputs (E).
    println!(
        "
input sources per output controller (key: ctrl-key.id ...):"
    );
    let deps = Deps::new(&g, mm.host, &ctrls);
    for (c, key) in &ctrls {
        let block = g.u32(c + 12).unwrap();
        if block == 0 {
            continue;
        }
        let mut found = std::collections::BTreeSet::new();
        for i in 0..n_out {
            let e = outs + 8 * i;
            if g.u32(g.u32(e + 4).unwrap() + 16).unwrap() == block {
                deps.output(e, &mut found, 0);
            }
        }
        let list: Vec<String> = found
            .iter()
            .map(|(k, id)| format!("{k:08X}.{id}"))
            .collect();
        println!("  {key:08X} {:32} <- {}", owner(*key), list.join(" "));
    }

    // A short run: the local player's audio-state controller driven like sub_824B19C8 at a ramp
    // of ground speeds, printing the SkateBoard controller's rolling gains (ids 7, 9). The other 246
    // controllers need the game's inputs too (global faders, pause, speech ducking…); when the replay
    // fixture exists, its first 3600 retail evaluations are applied first so they hold retail values.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/captures/extract/mixmap_replay_4400.bin");
    if let Ok(bytes) = std::fs::read(&fixture) {
        preroll(&mut g, mm.manager, &ctrls, &bytes, 3600);
        println!(
            "
(pre-rolled 3600 retail evaluations from {})",
            fixture.display()
        );
    } else {
        println!(
            "
(no replay fixture: every other controller's inputs stay 0, so most gains stay 0)"
        );
    }
    let state = mixmap::find_controller(&g, mm.host, mixmap::key(3, 1, 0, 0))
        .unwrap()
        .expect("state ctrl");
    let board = mixmap::find_controller(&g, mm.host, mixmap::key(2, 1, 0, 0))
        .unwrap()
        .expect("board ctrl");
    println!("\nsimulated run (state {state:#010x}, board {board:#010x}), dt 1/70 s:");
    for frame in 0..40 {
        let v: f64 = if frame < 5 {
            0.0
        } else if frame < 30 {
            5.0
        } else {
            0.0
        };
        let contact = frame < 30;
        drive_state(&mut g, state, v, if contact { 4 } else { 0 });
        mixmap::tick(&mut g, mm.manager, 1.0 / 70.0).unwrap();
        println!(
            "  f{frame:02} v={v:4.1} wheels={}  w7(gain id7)={:5}  w9(gain id9)={:5}  pitch id8={:5}  u16 id0={}",
            if contact { 4 } else { 0 },
            controller::read_gain(&g, board, 7).unwrap(),
            controller::read_gain(&g, board, 9).unwrap(),
            controller::read_pitch(&g, board, 8).unwrap(),
            controller::read_u16(&g, board, 0).unwrap(),
        );
    }
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

/// The state controller inputs `sub_824B19C8` writes that the doc records (ids 0, 1, 2, 7, 8, 10);
/// the rest stay 0. A simulation aid only — the replay test uses the captured words.
fn drive_state(g: &mut Guest, ctrl: u32, v: f64, wheels: u32) {
    let clamp = |x: f64| x.clamp(0.0, 32767.0) as u32;
    controller::set(g, ctrl, 0, clamp(v * 23592.24)).unwrap();
    controller::set(g, ctrl, 1, (v * 3932.04) as u32).unwrap();
    controller::set(g, ctrl, 7, (v * 2359.22) as u32).unwrap();
    controller::set(g, ctrl, 8, (v * 1685.16) as u32).unwrap();
    controller::set(g, ctrl, 2, if wheels == 0 { 32767 } else { 0 }).unwrap();
    controller::set(g, ctrl, 10, [0, 8191, 16383, 24575, 32767][wheels as usize]).unwrap();
}

/// Pointer → node lookups for the dependency walk.
struct Deps<'a> {
    g: &'a Guest,
    inputs: std::collections::HashMap<u32, u32>, // input entry (+8/+12) → entry
    products: std::collections::HashMap<u32, u32>, // e2+8 → product entry (400)
    lookups: std::collections::HashMap<u32, u32>, // st+12/16/20 or entry → lookup entry (416)
    envelopes: std::collections::HashMap<u32, u32>, // st+24/28 → envelope entry (404)
    sums: std::collections::HashMap<u32, u32>,   // s+4 → sum entry (436)
    outputs: std::collections::HashMap<u32, u32>, // st+8 → output entry (448)
    blocks: Vec<(u32, u32)>,                     // (input block, key)
}

impl<'a> Deps<'a> {
    fn new(g: &'a Guest, h: u32, ctrls: &[(u32, u32)]) -> Self {
        let rd = |a: u32| g.u32(a).unwrap();
        let mut d = Deps {
            g,
            inputs: Default::default(),
            products: Default::default(),
            lookups: Default::default(),
            envelopes: Default::default(),
            sums: Default::default(),
            outputs: Default::default(),
            blocks: ctrls
                .iter()
                .filter(|(c, _)| rd(c + 8) != 0)
                .map(|&(c, k)| (rd(c + 8), k))
                .collect(),
        };
        for i in 0..rd(h + 208) {
            let e = rd(h + 388) + 16 * i;
            d.inputs.insert(e + 8, e);
            d.inputs.insert(e + 12, e);
        }
        for i in 0..rd(h + 464) {
            let e = rd(h + 400) + 8 * i;
            d.products.insert(rd(e + 4) + 8, e);
        }
        for i in 0..rd(h + 468) {
            let e = rd(h + 416) + 8 * i;
            let st = rd(e + 4);
            for o in [12, 16, 20] {
                d.lookups.insert(st + o, e);
            }
            d.lookups.insert(e, e);
        }
        for i in 0..rd(h + 480) {
            let e = rd(h + 404) + 8 * i;
            let st = rd(e + 4);
            d.envelopes.insert(st + 24, e);
            d.envelopes.insert(st + 28, e);
        }
        for i in 0..rd(h + 472) {
            let e = rd(h + 436) + 8 * i;
            d.sums.insert(rd(e + 4) + 4, e);
        }
        for i in 0..rd(h + 476) {
            let e = rd(h + 448) + 8 * i;
            d.outputs.insert(rd(e + 4) + 8, e);
        }
        d
    }
    fn rd(&self, a: u32) -> u32 {
        self.g.u32(a).unwrap()
    }
    fn list(
        &self,
        ptrs: u32,
        n: u32,
        out: &mut std::collections::BTreeSet<(u32, u32)>,
        depth: u32,
    ) {
        for m in 0..n {
            self.pointer(self.rd(ptrs + 4 * m), out, depth);
        }
    }
    fn pointer(&self, p: u32, out: &mut std::collections::BTreeSet<(u32, u32)>, depth: u32) {
        if depth > 32 {
            return;
        }
        if let Some(&(b, k)) = self.blocks.iter().find(|&&(b, _)| p >= b && p < b + 64) {
            out.insert((k, (p - b) / 4));
        } else if let Some(&e) = self.inputs.get(&p) {
            self.pointer(self.rd(e + 4), out, depth + 1);
        } else if let Some(&e) = self.products.get(&p) {
            let (def, e2) = (self.rd(e), self.rd(e + 4));
            self.pointer(self.rd(e2) + 12, out, depth + 1);
            if self.rd(e2 + 4) != 0 {
                let n = u32::from(self.g.u8(self.rd(def) + 4).unwrap());
                self.list(self.rd(e2 + 4), n, out, depth + 1);
            }
        } else if let Some(&e) = self.lookups.get(&p) {
            let st = self.rd(e + 4);
            let blk = self.rd(st + 4);
            if let Some(&(_, k)) = self.blocks.iter().find(|&&(b, _)| b == blk) {
                out.insert((k, 99)); // the whole position block
            }
        } else if let Some(&e) = self.envelopes.get(&p) {
            let (def, st) = (self.rd(e), self.rd(e + 4));
            self.pointer(self.rd(st + 16), out, depth + 1);
            if self.rd(st + 20) != 0 {
                let n = u32::from(self.g.u8(self.rd(def) + 4).unwrap());
                self.list(self.rd(st + 20), n, out, depth + 1);
            }
        } else if let Some(&e) = self.sums.get(&p) {
            let (def, s) = (self.rd(e), self.rd(e + 4));
            if self.rd(s) != 0 {
                self.list(self.rd(s), self.rd(def + 8) & 0xFF, out, depth + 1);
            }
        } else if let Some(&e) = self.outputs.get(&p) {
            self.output(e, out, depth + 1);
        }
    }
    fn output(&self, e: u32, out: &mut std::collections::BTreeSet<(u32, u32)>, depth: u32) {
        let (def, st) = (self.rd(e), self.rd(e + 4));
        if self.rd(st + 4) != 0 {
            self.list(self.rd(st + 4), self.rd(def + 8) & 0xFF, out, depth);
        }
        if self.rd(st + 12) != 0 {
            self.list(self.rd(st + 12), self.rd(def + 8) >> 16, out, depth);
        }
    }
}
