//! Tests over the real guest image and `MixMapSK8.mxb` (both from the owner install; set
//! `SKATE3_ASSETS` to its `assets` dir). A test whose data is absent says so and returns.
//!
//! The replay test additionally reads `.local/captures/extract/mixmap_replay_4400.bin`, written by
//! `examples/mixmap_replay.rs` (`EXPORT=… mixmap_replay "" "" 4400`) from the retail capture: the
//! exact pre-evaluation words of every controller for the first 4400 evaluations, and the captured
//! outputs of the local player's 13 SFXObj controllers.

use super::controller::{self, get_output, read_gain, read_pitch, read_u16};
use super::tables::{cents_to_ratio, curve, lin_to_mb, mb_to_lin};
use super::*;
use crate::patch::BumpHeap;
use crate::{Guest, Segment};
use std::path::PathBuf;

const HEAP: u32 = 0x4000_0000;
const HEAP_LEN: u32 = 0x0080_0000;

fn assets() -> Option<PathBuf> {
    let dir = std::env::var("SKATE3_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets")
        });
    dir.join("private/stock/audio-runtime-image")
        .is_dir()
        .then_some(dir)
}

/// The dumped image plus an empty heap window.
fn image_guest() -> Option<Guest> {
    let Some(dir) = assets() else {
        eprintln!("skipped: no guest image (set SKATE3_ASSETS)");
        return None;
    };
    let mut segs = Vec::new();
    for entry in std::fs::read_dir(dir.join("private/stock/audio-runtime-image")).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_name()?.to_string_lossy().to_string();
        if let Some(hex) = name.strip_prefix("g_").and_then(|s| s.strip_suffix(".bin")) {
            let page = u32::from_str_radix(hex, 16).ok()?;
            segs.push(Segment {
                base: page << 16,
                bytes: std::fs::read(&path).ok()?,
            });
        }
    }
    segs.push(Segment {
        base: HEAP,
        bytes: vec![0; HEAP_LEN as usize],
    });
    Some(Guest::from_segments(segs))
}

fn built() -> Option<(Guest, MixMap, KeyedListener)> {
    let mut g = image_guest()?;
    let file = match std::fs::read(assets()?.join("private/stock/data/audio/MixMapSK8.mxb")) {
        Ok(f) => f,
        Err(_) => {
            eprintln!("skipped: MixMapSK8.mxb not staged");
            return None;
        }
    };
    let mut heap = BumpHeap {
        next: HEAP,
        end: HEAP + HEAP_LEN,
    };
    let mut listener = KeyedListener::retail();
    let mm = load(&mut g, &mut heap, &mut listener, &file).expect("build");
    Some((g, mm, listener))
}

// ---------------------------------------------------------------------- leaves

#[test]
fn lin_to_mb_walks_the_table_per_octave() {
    let Some(g) = image_guest() else { return };
    // 0 and anything ≥ 32768 (or negative) take the site's default.
    assert_eq!(lin_to_mb(&g, 0, -10000).unwrap(), -10000);
    assert_eq!(lin_to_mb(&g, 32768, -7).unwrap(), -7);
    assert_eq!(lin_to_mb(&g, 0xFFFF_FFFF, -7).unwrap(), -7);
    // Full scale: the top octave's last entry minus 602.
    let top = g.u32(tables::LOG_TABLE + 4 * 511).unwrap() as i32 - 602;
    assert_eq!(lin_to_mb(&g, 32767, -10000).unwrap(), top);
    // Each halving costs 602 mB exactly at the octave boundaries (the table's entry 0 is 0).
    assert_eq!(g.u32(tables::LOG_TABLE).unwrap(), 0);
    for c in 0..6 {
        let x = 16384u32 >> c;
        assert_eq!(
            lin_to_mb(&g, x, 0).unwrap(),
            -602 * (c as i32 + 1),
            "x = {x}"
        );
    }
    // The rlwimi octaves (6..14) are monotone and land in their octave's range.
    let mut last = i32::MIN;
    for x in 1..512u32 {
        let v = lin_to_mb(&g, x, 0).unwrap();
        assert!(v >= last, "monotone at {x}");
        last = v;
        let c = x.leading_zeros() as i32 - 17;
        assert!(
            v <= -602 * (c + 1) + 602 && v >= -602 * (c + 2),
            "x = {x} → {v}"
        );
    }
}

#[test]
fn mb_to_lin_reads_the_descending_table_and_shifts_by_octave() {
    let Some(g) = image_guest() else { return };
    // 0 mB is table entry 0 (32767 × 2^-1/602 = 32730), not 32767: retail's own rounding.
    assert_eq!(mb_to_lin(&g, 0).unwrap(), 32730);
    assert_eq!(mb_to_lin(&g, -602).unwrap(), 32730 >> 1);
    assert_eq!(mb_to_lin(&g, -602 * 15).unwrap(), 32730 >> 15);
    // Sixteen octaves down (−9632 and below) is silence.
    assert_eq!(mb_to_lin(&g, -602 * 16).unwrap(), 0);
    assert_eq!(mb_to_lin(&g, -10000).unwrap(), 0);
    // A small positive value has octave 0 and a negative remainder, so it reads *forward* past the
    // table end into the log table (0x82FDBFAC + 20 = its entry 4). Every caller clamps to <= 0 first but one: the
    // flagged envelope path of `sub_82950250`.
    assert_eq!(
        mb_to_lin(&g, 5).unwrap(),
        g.u32(tables::LOG_TABLE + 16).unwrap() as i32
    );
    // Larger positive values have a negative octave, rejected as unsigned: 0.
    assert_eq!(mb_to_lin(&g, 700).unwrap(), 0);
}

#[test]
fn the_ten_curve_kinds() {
    let Some(g) = image_guest() else { return };
    for x in [0, 1, 100, 16384, 32767] {
        assert_eq!(curve(&g, x, 8).unwrap(), 32767 - x, "1 - x");
        assert_eq!(curve(&g, x, 9).unwrap(), x, "x");
        assert_eq!(curve(&g, x, 1).unwrap(), curve(&g, 32767 - x, 0).unwrap());
        let t = curve(&g, x, 0).unwrap();
        assert_eq!(curve(&g, x, 2).unwrap(), t * t >> 15);
        let m = curve(&g, x, 4).unwrap();
        assert_eq!(curve(&g, x, 6).unwrap(), m * m >> 15);
    }
    // Kind 0 interpolates table[x >> 6] toward the next entry by a fraction whose low ten bits are
    // always set (`li r8,1023 ; rlwimi`), so x = 0 is already 1023/32768 of the way: 32766.
    let (t0, t1) = (
        g.u32(tables::CURVE_TABLE).unwrap() as i32,
        g.u32(tables::CURVE_TABLE + 4).unwrap() as i32,
    );
    assert_eq!((t0, t1), (32767, 32766));
    assert_eq!(curve(&g, 0, 0).unwrap(), t0 + ((t1 - t0) * 1023 >> 15));
    assert_eq!(curve(&g, -1, 0).unwrap(), 0);
    assert_eq!(curve(&g, 511 << 6, 0).unwrap(), 0);
    assert!(curve(&g, 0, 10).is_err());
}

#[test]
fn cents_to_ratio_by_octaves_semitones_and_fine_steps() {
    let Some(g) = image_guest() else { return };
    assert_eq!(cents_to_ratio(&g, 0).unwrap(), 1.0);
    assert_eq!(cents_to_ratio(&g, 1200).unwrap(), 2.0);
    assert_eq!(cents_to_ratio(&g, 2400).unwrap(), 4.0);
    assert_eq!(cents_to_ratio(&g, -1200).unwrap(), 0.5);
    // Output type 2's retail words: −10000 → 77 (0x4D) and −2 → 24971 (0x618B), both in the capture.
    let k = crate::fp::load_single(&g, tables::K_25000).unwrap();
    let word =
        |c| crate::fp::fctiwz_low_word(crate::fp::mul_single(cents_to_ratio(&g, c).unwrap(), k));
    assert_eq!(word(-10000), 0x4D);
    assert_eq!(word(-2), 0x618B);
}

// ---------------------------------------------------------------------- controller

#[test]
fn controller_slots_read_and_write_the_blocks() {
    let Some(mut g) = image_guest() else { return };
    let (ctrl, inputs, outputs) = (HEAP, HEAP + 0x100, HEAP + 0x200);
    // No blocks yet: Set is a no-op, Get and GetOutput read 0.
    controller::set(&mut g, ctrl, 3, 7).unwrap();
    assert_eq!(controller::get(&g, ctrl, 3).unwrap(), 0);
    assert_eq!(get_output(&g, ctrl, 0, 0).unwrap(), 0);
    controller::set_inputs(&mut g, ctrl, inputs).unwrap();
    controller::set_outputs(&mut g, ctrl, outputs).unwrap();
    controller::set(&mut g, ctrl, 3, 7).unwrap();
    assert_eq!(g.u32(inputs + 12).unwrap(), 7);
    controller::set_float(&mut g, ctrl, 4, -2.9).unwrap();
    assert_eq!(controller::get(&g, ctrl, 4).unwrap() as i32, -2);
    // Word 1 holds ids 2 (low half) and 3 (high half).
    g.set_u32(outputs + 4, 0xFE0C_8123).unwrap();
    assert_eq!(get_output(&g, ctrl, 2, 0).unwrap(), 0x0123);
    assert_eq!(get_output(&g, ctrl, 2, 3).unwrap(), 0x8123);
    assert_eq!(get_output(&g, ctrl, 3, 3).unwrap(), 0xFE0C);
    assert_eq!(get_output(&g, ctrl, 3, 4).unwrap(), 0x7E0C);
    assert_eq!(get_output(&g, ctrl, 3, 5).unwrap(), 0);
    // Type 1: 0xFE0C = −500 cents → 2^(−500/1200) × 4096.
    assert_eq!(
        get_output(&g, ctrl, 3, 1).unwrap(),
        read_pitch(&g, ctrl, 3).unwrap()
    );
    let expect = crate::fp::fctiwz_low_word(crate::fp::mul_single(
        cents_to_ratio(&g, -500).unwrap(),
        4096.0,
    ));
    assert_eq!(read_pitch(&g, ctrl, 3).unwrap(), expect);
    assert_eq!(read_u16(&g, ctrl, 3).unwrap(), 0xFE0C);
    assert_eq!(read_gain(&g, ctrl, 3).unwrap(), 0x7E0C);
    // The owner readers go through owner+12.
    let owner = HEAP + 0x300;
    g.set_u32(owner + 12, ctrl).unwrap();
    assert_eq!(controller::owner_read_u16(&g, owner, 2).unwrap(), 0x8123);
    controller::disable(&mut g, ctrl).unwrap();
    assert_eq!(g.u32(outputs + 60).unwrap(), 0);
    controller::enable(&mut g, ctrl).unwrap();
    assert_eq!(g.u32(outputs + 60).unwrap(), 1);
}

// ---------------------------------------------------------------------- build

#[test]
fn the_build_makes_retails_247_controllers_in_retails_order() {
    let Some((g, mm, listener)) = built() else {
        return;
    };
    let ctrls = controllers(&g, mm.host).unwrap();
    assert_eq!(ctrls.len(), 247);
    // The capture's controller array (0x4A26A830 + 16·i) begins with these keys, in this order.
    let head: Vec<u32> = ctrls.iter().take(9).map(|&(_, k)| k).collect();
    assert_eq!(
        head,
        [
            0x4000_0000,
            0x4000_0010,
            0x4000_0020,
            0x4000_0030,
            0x4000_0050,
            0x4000_0060,
            0x4000_0090,
            0x4001_0000,
            0x4001_0010
        ]
    );
    // Every controller was handed to the manager once.
    assert_eq!(listener.bound.len(), 247);
    // The player's audio state (SFXCTL_PlayerPhysics) and its SkateBoard.
    let state = find_controller(&g, mm.host, key(3, 1, 0, 0))
        .unwrap()
        .expect("state");
    let board = find_controller(&g, mm.host, key(2, 1, 0, 0))
        .unwrap()
        .expect("board");
    assert_ne!(
        g.u32(state + 8).unwrap(),
        0,
        "the state controller has inputs"
    );
    assert_eq!(g.u32(state + 12).unwrap(), 0, "and no outputs");
    assert_ne!(g.u32(board + 12).unwrap(), 0);
    // sub_8294F228 left every output block zeroed with its enable word at 1.
    assert_eq!(g.u32(g.u32(board + 12).unwrap() + 60).unwrap(), 1);
}

#[test]
fn every_array_is_filled_to_the_size_the_sizing_pass_computed() {
    let Some((g, mm, _)) = built() else { return };
    for (name, cap, used) in check_capacities(&g, mm.host).unwrap() {
        match name {
            // Retail over-allocates these three; the counts are the file's, not a port artefact.
            "output blocks 376 (×16 words)" => assert_eq!((cap, used), (3056, 2464)),
            "block pool 380" => assert_eq!((cap, used), (216, 132)),
            "controllers 164" => assert_eq!((cap, used), (275, 247)),
            _ => assert_eq!(cap, used, "{name}"),
        }
    }
}

// ---------------------------------------------------------------------- replay

struct Replay {
    keys: Vec<u32>,
    frames: Vec<(f64, Vec<(u16, u8, u32)>, Vec<[u32; 16]>)>,
}

fn replay_fixture() -> Option<Replay> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/captures/extract/mixmap_replay_4400.bin");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!(
            "skipped: {} absent (see examples/mixmap_replay.rs, EXPORT)",
            path.display()
        );
        return None;
    };
    let mut at = 0usize;
    let mut take = |n: usize| {
        let s = &bytes[at..at + n];
        at += n;
        s
    };
    assert_eq!(take(4), b"MMRP");
    let u32le = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
    let frames = u32le(take(4)) as usize;
    let n = u32le(take(4)) as usize;
    let keys = (0..n).map(|_| u32le(take(4))).collect();
    let mut out = Vec::with_capacity(frames);
    for _ in 0..frames {
        let dt = f64::from_le_bytes(take(8).try_into().unwrap());
        let changes = u32le(take(4)) as usize;
        let list = (0..changes)
            .map(|_| {
                let c = u16::from_le_bytes(take(2).try_into().unwrap());
                let w = take(1)[0];
                (c, w, u32le(take(4)))
            })
            .collect();
        let player = (0..13)
            .map(|_| {
                let mut o = [0u32; 16];
                for v in o.iter_mut() {
                    *v = u32le(take(4));
                }
                o
            })
            .collect();
        out.push((dt, list, player));
    }
    Some(Replay { keys, frames: out })
}

/// Rolling gains: SkateBoard ids 7 (layer 0, `Class_rolling` w11) and 9 (layer 3).
#[test]
fn the_skateboard_rolling_gains_reproduce_the_retail_capture() {
    let Some(fixture) = replay_fixture() else {
        return;
    };
    let Some((mut g, mm, _)) = built() else {
        return;
    };
    let ctrls = controllers(&g, mm.host).unwrap();
    assert_eq!(
        ctrls.iter().map(|&(_, k)| k).collect::<Vec<_>>(),
        fixture.keys
    );
    let board = find_controller(&g, mm.host, key(2, 1, 0, 0))
        .unwrap()
        .unwrap();
    let player: Vec<u32> = (0..13)
        .map(|o| {
            find_controller(&g, mm.host, key(2, 1, 0, o))
                .unwrap()
                .unwrap()
        })
        .collect();
    let mut rolling = Vec::new();
    let mut other_mismatches = Vec::new();
    for (index, (dt, changes, expected)) in fixture.frames.iter().enumerate() {
        for &(c, w, v) in changes {
            let ctrl = ctrls[c as usize].0;
            let block = if w < 16 {
                g.u32(ctrl + 8).unwrap()
            } else {
                g.u32(ctrl + 12).unwrap()
            };
            let at = if w < 16 {
                block + 4 * u32::from(w)
            } else {
                block + 60
            };
            g.set_u32(at, v).unwrap();
        }
        tick(&mut g, mm.manager, *dt).unwrap();
        let (id7, id9) = (
            read_gain(&g, board, 7).unwrap(),
            read_gain(&g, board, 9).unwrap(),
        );
        let want = (expected[0][3] >> 16 & 0x7FFF, expected[0][4] >> 16 & 0x7FFF);
        assert_eq!((id7, id9), want, "evaluation {}", index + 1);
        rolling.push((id7, id9));
        for (o, &c) in player.iter().enumerate() {
            let block = g.u32(c + 12).unwrap();
            for w in 0..16 {
                if g.u32(block + 4 * w).unwrap() != expected[o][w as usize] {
                    other_mismatches.push((index + 1, o, w));
                }
            }
        }
    }
    // Speed onset (evaluations 3620–3622) and the cruise cap, then contact loss (3848–3853).
    let id7: Vec<u32> = rolling.iter().map(|r| r.0).collect();
    assert_eq!(&id7[3618..3622], &[579, 4369, 11519, 12954]);
    assert_eq!(
        rolling[3846],
        (12999, 4913),
        "the cruise cap, layer 0 / layer 3"
    );
    assert_eq!(&id7[3846..3853], &[12999, 10602, 8383, 5920, 3671, 1348, 0]);
    // Everything else the local player's 13 controllers output matches too, except the words the
    // replay cannot reproduce: evaluations 2709–2713 (the position controller's first activation
    // reads uninitialised game memory) and single evaluations whose captured dt (printed to six
    // decimals) is too coarse for an envelope boundary.
    // Evaluations 1174 and 2435 are the two such frames in this window: a ±3e-7 s change to the dt
    // moves or removes them (`DTBIAS` in the replay example), nothing else does.
    let unexplained: Vec<_> = other_mismatches
        .iter()
        .filter(|(f, _, _)| !(2709..=2713).contains(f) && *f != 1174 && *f != 2435)
        .collect();
    assert!(unexplained.is_empty(), "{unexplained:?}");
    assert!(other_mismatches.len() <= 5 + 10, "{other_mismatches:?}");
}
