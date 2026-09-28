//! Run a real player sound's patch as far as the Rust runtime reaches: install the shipped projects
//! and `GRINDS.abk`, post a `Class_grind` message the way the game does, and tick the evaluator.
//!
//! Guest memory is the dumped image (`probe/harness/out/image/g_*.bin`), so every constant cell an
//! opcode reads holds its real value; the runtime's own globals are zeroed first, since the dump was
//! taken at boot. The run stops at the first opcode that is not ported and names it, which is the
//! point: it measures how much of a real grind program the ported table covers.
//!
//!     cargo run --release --example grind_instance -- <audiofiles.big> <image dir> [bank] [object]

use skate_audio_core::eval::interp;
use skate_audio_core::patch::{self, BumpHeap};
use skate_audio_core::voice::{OpenRequest, VoiceDevice};
use skate_audio_core::{Guest, Result as CoreResult, Segment, symbols};
use skate_audio_core::{classes, device, mathlib, modules};
use skate_audio_formats::eaac;

/// Logs every call a patch makes on its voices, and keeps each voice alive.
struct LoggingDevice {
    /// GRAPH=1: open each voice for real with `device::open_voice_graph` and log what it built.
    graph: Option<Graph>,
    voices: u32,
    frame: usize,
    lines: Vec<String>,
    last: std::collections::HashMap<(u32, u32), u32>,
}

/// What a real open needs besides guest memory: its own heap, the audio system, a device object,
/// scratch for the request's blocks, and a stack.
struct Graph {
    heap: BumpHeap,
    system: u32,
    device: u32,
    scratch: u32,
    sp: u32,
}

/// A module class's name, by descriptor.
fn class_name(class: u32) -> &'static str {
    match class {
        0x82FD_28C0 => "Send",
        0x82FC_E4CC => "GainFader",
        _ => classes::CLASSES
            .iter()
            .find(|c| c.1 == class)
            .map(|c| c.2)
            .unwrap_or("?"),
    }
}

impl LoggingDevice {
    fn log(&mut self, line: String) {
        self.lines.push(format!("  frame {:4}: {line}", self.frame));
    }
}

impl VoiceDevice for LoggingDevice {
    fn open(&mut self, g: &mut Guest, r: &OpenRequest) -> CoreResult<u32> {
        self.voices += 1;
        let voice = 0x7F00_0000 + self.voices * 0x100;
        let header = g
            .span(r.sample as u32, 16)
            .ok()
            .and_then(|bytes| eaac::Header::parse(bytes, 0).ok())
            .map(|h| {
                format!(
                    "{} Hz, {} ch, {} samples ({:.2} s){}",
                    h.sample_rate,
                    h.channels(),
                    h.num_samples,
                    h.duration_secs(),
                    if h.looping { ", looping" } else { "" }
                )
            })
            .unwrap_or_else(|| "no EAAC header".into());
        self.log(format!(
            "open voice {voice:#x}: sample index {} at {:#x} [{header}], byte2 {}, descriptor {:x?}, bank+72 {:#x}, arg8 {:#x}, {} records",
            r.index, r.sample, r.byte2, r.shifted, r.bank_72, r.arg8, r.record_count
        ));
        let Some(graph) = self.graph.as_mut() else {
            return Ok(voice);
        };
        for (i, w) in r.shifted.iter().enumerate() {
            g.set_u32(graph.scratch + 4 * i as u32, *w)?;
        }
        g.set_u32(graph.scratch + 24, r.record_count)?;
        g.set_u32(graph.scratch + 28, r.records)?;
        let ring_start = g.u32(graph.system + 204)?;
        let voice = device::open_voice_graph(
            g,
            &mut graph.heap,
            &mut mathlib::Image,
            &mut device::NoBuses,
            graph.device,
            r.sample as u32,
            r.byte2 as u32,
            graph.scratch,
            r.bank_72,
            r.arg8 as u32,
            graph.scratch + 24,
            graph.sp,
        )?;
        let player = g.u32(voice + 4)?;
        let modules: Vec<String> = (0..g.u8(player + 68)? as u32)
            .map(|i| {
                let instance = g.u32(player + 80 + 4 * i).unwrap();
                let class = g.u32(instance + 20).unwrap();
                format!("{}x{}", class_name(class), g.u8(instance + 42).unwrap())
            })
            .collect();
        let line = format!(
            "  graph for voice {voice:#x}: player {player:#x}, {} modules [{}], duration {:.3} s",
            modules.len(),
            modules.join(" "),
            f64::from_bits(g.u64(voice + 56)?)
        );
        self.lines.push(line);
        let (ring, end) = (g.u32(graph.system + 48)?, g.u32(graph.system + 204)?);
        let mut at = ring_start;
        let mut commands = Vec::new();
        while at < end {
            let record = ring + at;
            let (handler, target) = (g.u32(record)?, g.u32(record + 4)?);
            let (size, what) = match handler {
                modules::INSTALL_COMMAND => (8, "install".to_string()),
                device::COMMAND_PLAYER_FLOAT => (12, format!("player+56={}", g.f32(record + 8)?)),
                device::COMMAND_STAMP => {
                    let name = g.u32(target + 20).map(class_name).unwrap_or("?");
                    (
                        16,
                        format!("{name} id {} = {}", g.u32(record + 8)?, g.f32(record + 12)?),
                    )
                }
                device::COMMAND_SEND_BUS => (16, format!("send bus {:#x}", g.u32(record + 12)?)),
                device::COMMAND_PLAY => (
                    g.u16(record + 44)? as u32,
                    format!(
                        "play sample {:#x} counter {}",
                        g.u32(record + 32)?,
                        g.f32(record + 48)?
                    ),
                ),
                _ => break,
            };
            commands.push(what);
            at += size;
        }
        self.lines
            .push(format!("  commands: {}", commands.join("; ")));
        Ok(voice)
    }
    fn release(&mut self, _g: &mut Guest, v: u32) -> CoreResult<()> {
        self.log(format!("release {v:#x}"));
        Ok(())
    }
    fn suspend(&mut self, _g: &mut Guest, v: u32) -> CoreResult<()> {
        self.log(format!("suspend {v:#x}"));
        Ok(())
    }
    fn resume(&mut self, _g: &mut Guest, v: u32) -> CoreResult<()> {
        self.log(format!("resume {v:#x}"));
        Ok(())
    }
    fn set(&mut self, _g: &mut Guest, v: u32, id: u32, value: u32) -> CoreResult<()> {
        if self.last.insert((v, id), value) != Some(value) {
            self.log(format!("{v:#x} property {id} = {value}"));
        }
        Ok(())
    }
    fn set_alternate(&mut self, _g: &mut Guest, v: u32, value: u32) -> CoreResult<()> {
        if self.last.insert((v, 3), value) != Some(value) {
            self.log(format!("{v:#x} property 3 = {value}"));
        }
        Ok(())
    }
    fn query(&mut self, _g: &mut Guest, _v: u32, out: &mut [u32; 11]) -> CoreResult<()> {
        out[0] = 1;
        Ok(())
    }
    fn poke(&mut self, _g: &mut Guest, _v: u32) -> CoreResult<()> {
        Ok(())
    }
}
use skate_audio_formats::eb;

const MISC: u32 = 0x4000_0000;
const ARENA: u32 = 0x5000_0000;
const HEAP: u32 = 0x6000_0000;
const HEAP_BYTES: u32 = 0x40_0000;
const STACK: u32 = 0x7000_0000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let archive_path = args
        .next()
        .ok_or("usage: grind_instance <audiofiles.big> <image dir> [bank] [object]")?;
    let image = args.next().ok_or("need the image dump directory")?;
    let bank_name = args.next().unwrap_or_else(|| "GRINDS.abk".into());
    let object = args.next().unwrap_or_else(|| "Class_grind".into());

    let mut segments = Vec::new();
    for entry in std::fs::read_dir(&image)? {
        let path = entry?.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(hex) = stem.strip_prefix("g_") else {
            continue;
        };
        segments.push(Segment {
            base: u32::from_str_radix(hex, 16)? << 16,
            bytes: std::fs::read(&path)?,
        });
    }
    let mut g = Guest::from_segments(segments);
    g.put(MISC, vec![0; 0x1000]);
    g.put(HEAP, vec![0; HEAP_BYTES as usize]);
    g.put(STACK - 0x1000, vec![0; 0x2000]);
    for cell in [
        symbols::PROJECT_LIST_HEAD,
        interp::LIST_HEAD,
        patch::BANK_LIST,
        interp::DELTA_CACHE,
        interp::FRAME_COUNT,
        interp::COUNTDOWN,
        interp::SCALE_GLOBAL,
    ] {
        g.set_u32(cell, 0)?;
    }
    g.set_u16(symbols::GENERATION, 0)?;
    // Game init (sub_826D4C30) sets the evaluator's period denominator to 30.0 after the dump was
    // taken, so a period is 1/30 s in play, not the dump's 1/41.6.
    g.set_u32(interp::PERIOD_DENOM, 30.0f32.to_bits())?;

    let data = std::fs::read(&archive_path)?;
    let archive = eb::Archive::parse(&data)?;
    let mut next = ARENA;
    let mut place = |g: &mut Guest, bytes: &[u8]| {
        let at = next;
        g.put(at, bytes.to_vec());
        next = (at + bytes.len() as u32 + 0x1000) & !0xFFF;
        at
    };
    // Every project, so the object resolves through the same two passes as in the game.
    let mut game_project = None;
    for e in &archive.entries {
        let Some(name) = e.name.as_deref() else {
            continue;
        };
        if name.ends_with(".csi") {
            let bytes = &data[e.range()];
            let csi = skate_audio_formats::banks::Csi::parse(bytes)?;
            if let Some(s) = csi
                .symbols
                .iter()
                .find(|s| s.group == 1 && s.name == object)
            {
                game_project = Some((csi.project_id, s.id));
            }
            let at = place(&mut g, bytes);
            symbols::install_project(&mut g, at)?;
        }
    }
    let (project, name_id) = game_project.ok_or("no shipped project names the object")?;
    let member = archive
        .entries
        .iter()
        .find(|e| e.name.as_deref() == Some(bank_name.as_str()))
        .ok_or("bank not in the archive")?;
    let bank = place(&mut g, &data[member.range()]);
    let (id, first) = patch::load_bank(&mut g, bank, STACK)?;
    println!("{bank_name} installed at {bank:#x}: id {id}, first bank {first}");

    // The game's side: a slot resolved from its object table entry, then a post.
    let (slot, query, message) = (MISC + 0x10, MISC + 0x20, MISC + 0x100);
    g.set_span(MISC + 0x40, object.as_bytes())?;
    g.set_u8(MISC + 0x40 + object.len() as u32, 0)?;
    g.set_u32(query, MISC + 0x40)?;
    g.set_u16(query + 4, project)?;
    g.set_u16(query + 6, name_id)?;
    let found = symbols::lookup_table1(&mut g, slot, query)?;
    println!("{object} {project:#06x}:{name_id:#06x} resolved: {found:?}");

    // Class_grind's payload as sub_824AF8C8 builds it: fixed header words, then a speed of 5000,
    // 1024, surface class 3, variant 0, level 20000, three flags off, and two zero words.
    let mut payload: Vec<u32> = vec![
        0, 32767, 0, 0, 0, 25000, 0, 5000, 1024, 3, 0, 20000, 0, 0, 0, 0, 0,
    ];
    // PAYLOAD="hex words ..." replaces it, e.g. a payload copied from a skate3-audio-msg trace line.
    if let Ok(words) = std::env::var("PAYLOAD") {
        payload = words
            .split_whitespace()
            .map(|w| u32::from_str_radix(w, 16))
            .collect::<std::result::Result<_, _>>()?;
    }
    for (i, w) in payload.iter().enumerate() {
        g.set_u32(message + 4 + 4 * i as u32, *w)?;
    }
    let mut heap = BumpHeap {
        next: HEAP,
        end: HEAP + HEAP_BYTES,
    };
    let status = patch::post(&mut g, &mut heap, slot, message + 4, message)?;
    let head = g.u32(interp::LIST_HEAD)?;
    println!("post: {status}; interpreter list head {head:#x}");
    if head == 0 {
        return Ok(());
    }
    let instance = head - 8;
    let program = g.u32(instance + 16)?;
    let block = g.u32(instance + 20)?;
    let first_words: Vec<String> = (0..12)
        .map(|i| format!("{:08x}", g.u32(block + 4 * i).unwrap()))
        .collect();
    println!(
        "instance {instance:#x}: program {program:#x}, block {block:#x}, block words {}",
        first_words.join(" ")
    );

    // With DISASM set, list the program: each op's absolute block offset and where its pairs write.
    if std::env::var_os("DISASM").is_some() {
        let mut at = program;
        let mut blk: i64 = 0;
        let mut n = 0;
        loop {
            let op = g.u8(at)?;
            if op == 255 {
                println!("  [{n:3}] end at block +{blk}");
                break;
            }
            let pairs = g.u8(at + 1)? as u32;
            let mut wiring = Vec::new();
            for p in 0..pairs {
                let src = g.u32(at + 4 + 8 * p)? as i32 as i64;
                let dst = g.u32(at + 8 + 8 * p)? as i32 as i64;
                if src == -1 {
                    wiring.push(format!("r3->+{}", blk + dst));
                } else {
                    wiring.push(format!("+{}->+{}", blk + src, blk + dst));
                }
            }
            let advance = g.u32(at + 4 + 8 * pairs)? as i32 as i64;
            let name = skate_audio_core::eval::TABLE
                .get(op as usize)
                .map(|s| s.name)
                .unwrap_or("?");
            println!(
                "  [{n:3}] op {op:2} {name} block +{blk} {}",
                wiring.join(" ")
            );
            blk += advance;
            at += 8 + 8 * pairs;
            n += 1;
        }
    }

    // 256 samples at 48 kHz per audio frame.
    let delta = (256.0f32 / 48_000.0) as f64;
    let mut ops = 0usize;
    // GRAPH=1 sets up what a real open reads: an audio system with a command ring, the Send class
    // registered (the game registers it on another path), and a bus manager whose buses 0-17 exist.
    let graph = if std::env::var_os("GRAPH").is_some() {
        const SPACE: u32 = 0x4100_0000;
        const GRAPH_HEAP: u32 = 0x6800_0000;
        g.put(SPACE, vec![0; 0x2_0000]);
        g.put(GRAPH_HEAP, vec![0; 0x40_0000]);
        let mut heap = BumpHeap {
            next: GRAPH_HEAP,
            end: GRAPH_HEAP + 0x40_0000,
        };
        let (system, ring, manager, root, device_object) = (
            SPACE,
            SPACE + 0x1000,
            SPACE + 0x8000,
            SPACE + 0x9000,
            SPACE + 0x9100,
        );
        g.set_u32(modules::SYSTEM, system)?;
        g.set_u32(system + 48, ring)?;
        let registry = classes::class_registry(&mut g, &mut heap, system)?;
        classes::register_class(&mut g, registry, 0x82FD_28C0)?;
        g.set_u32(device::BUS_MANAGER, manager)?;
        g.set_u32(manager + 52, SPACE + 0x9200)?;
        g.set_u32(SPACE + 0x9200, 0xB0B0_0FFF)?;
        for i in 0..18u32 {
            g.set_u32(manager + (271 + i) * 4, SPACE + 0x9300 + 4 * i)?;
            g.set_u32(SPACE + 0x9300 + 4 * i, 0xB0B0_0000 + i)?;
            g.set_u8(manager + 1148 + i, 1)?;
        }
        g.set_u32(classes::BUS_ROOT, root)?;
        g.set_u32(root + 44, SPACE + 0x9400)?;
        g.set_u32(SPACE + 0x9400, 0xB0B0_0DEF)?;
        g.set_u32(classes::DEFAULT_BUS, 0)?;
        g.set_u32(classes::REGISTERED, 0)?;
        Some(Graph {
            heap,
            system,
            device: device_object,
            scratch: SPACE + 0xA000,
            sp: SPACE + 0x1_F000,
        })
    } else {
        None
    };
    let mut device = LoggingDevice {
        graph,
        voices: 0,
        frame: 0,
        lines: Vec::new(),
        last: Default::default(),
    };
    let frames: usize = std::env::var("FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(375);
    // UPDATES re-delivers the payload every frame the way the grind updater sub_824C39E0 does:
    // word 0 becomes 32767, and volume, two cutoffs and speed are refreshed (values chosen here).
    let updates = std::env::var_os("UPDATES").is_some();
    // UPDATE_FILE: one traced update payload per line (hex words, as `skate3-audio-update` logs them),
    // re-delivered from frame 1 over the words the post supplied, one every UPDATE_EVERY audio frames
    // (default 1). The game re-delivers from its own thread about every 25 ms, near 6 audio frames.
    let traced: Vec<Vec<u32>> = match std::env::var("UPDATE_FILE") {
        Ok(path) => std::fs::read_to_string(path)?
            .lines()
            .map(|l| {
                l.split_whitespace()
                    .filter_map(|w| u32::from_str_radix(w, 16).ok())
                    .collect()
            })
            .filter(|v: &Vec<u32>| !v.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    };
    let every: usize = std::env::var("UPDATE_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let node = g.u32(message)?;
    for frame in 0..frames {
        device.frame = frame;
        if frame >= 1 && (frame - 1) % every == 0 && (frame - 1) / every < traced.len() {
            for (i, w) in traced[(frame - 1) / every].iter().enumerate() {
                g.set_u32(message + 4 + 4 * i as u32, *w)?;
            }
            patch::redeliver(&mut g, node, message + 4)?;
        }
        if updates && frame >= 20 {
            for (field, value) in [
                (4u32, 32767u32),
                (8, 20000),
                (12, 0),
                (16, 0),
                (20, 0),
                (24, 25000),
                (28, 25000),
                (32, 5000),
            ] {
                g.set_u32(message + field, value)?;
            }
            patch::redeliver(&mut g, node, message + 4)?;
        }
        match interp::tick_with(
            &mut g,
            delta,
            &mut patch::PatchHost {
                heap: &mut heap,
                device: &mut device,
            },
        ) {
            Ok(t) => {
                ops += t.ops;
                if frame < 12 && t.walked {
                    println!("  frame {frame}: walked {} nodes, {} ops", t.nodes, t.ops);
                }
                // WATCH="off,off,..." prints those operand-block words after chosen walks.
                if t.walked && matches!(frame, 5 | 11 | 17 | 59 | 179 | 371) {
                    if let Ok(list) = std::env::var("WATCH") {
                        let words: Vec<String> = list
                            .split(',')
                            .filter_map(|o| o.trim().parse::<u32>().ok())
                            .map(|o| {
                                format!(
                                    "+{o}={}",
                                    g.u32(block + o).map(|w| w as i32).unwrap_or(i32::MIN)
                                )
                            })
                            .collect();
                        println!("  watch frame {frame}: {}", words.join(" "));
                    }
                }
                if frame + 1 == frames {
                    println!("  ran {frames} frames, {ops} ops, the instance still live");
                }
                if t.walked && g.u32(interp::LIST_HEAD)? == 0 {
                    println!("  frame {frame}: the list emptied");
                    break;
                }
            }
            Err(e) => {
                println!(
                    "  frame {frame}: stopped after {ops} ops: {} (at {:#x})",
                    e.message, e.address
                );
                let record = g.u32(instance + 16)?;
                println!(
                    "  period {} frames, scale {}",
                    g.u32(interp::FRAME_COUNT)?,
                    g.f32(interp::SCALE_GLOBAL)?
                );
                let _ = record;
                break;
            }
        }
    }
    for line in device.lines.iter().take(80) {
        println!("{line}");
    }
    if device.lines.len() > 80 {
        println!("  ... {} more device calls", device.lines.len() - 80);
    }
    Ok(())
}
