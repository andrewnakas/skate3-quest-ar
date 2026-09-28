//! The input writers against the image constants and the retail capture's extracts
//! (`.local/captures/extract/state.tsv` — frame, ms, 160 words from `state+192` — and
//! `mixmap/<ctrl>.tsv` — the controller's `MC` lines). A test whose data is absent says so.
//!
//! The bridge that writes the state runs in the frame's first half, the MixMap evaluation in the
//! second, and the capture numbers evaluations: the state dumped at `ST` frame *n* is the one the
//! controller inputs of evaluation *n + 1* were written from.

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

fn extract() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/captures/extract");
    if dir.join("state.tsv").is_file() {
        Some(dir)
    } else {
        eprintln!("skipped: no capture extract at {}", dir.display());
        None
    }
}

fn states(dir: &PathBuf) -> HashMap<u32, Vec<u32>> {
    let mut out = HashMap::new();
    for line in std::fs::read_to_string(dir.join("state.tsv"))
        .unwrap()
        .lines()
    {
        let p: Vec<&str> = line.split('\t').collect();
        let words = p[2..162]
            .iter()
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect();
        out.insert(p[0].parse().unwrap(), words);
    }
    out
}

/// A controller's captured input words by evaluation.
fn inputs(dir: &PathBuf, ctrl: &str) -> HashMap<u32, [u32; 16]> {
    let mut out = HashMap::new();
    let text = std::fs::read_to_string(dir.join("mixmap").join(format!("{ctrl}.tsv"))).unwrap();
    for line in text.lines() {
        let p: Vec<&str> = line.split('\t').collect();
        let w: Vec<&str> = p[2].split_whitespace().collect();
        let at = w.iter().position(|&x| x == "|").unwrap();
        let mut words = [0u32; 16];
        for (i, v) in words.iter_mut().enumerate() {
            *v = u32::from_str_radix(w[at + 1 + i], 16).unwrap();
        }
        out.insert(p[0].parse().unwrap(), words);
    }
    out
}

/// A controller's captured output words by evaluation, unpacked into the 32 output ids: the
/// `MC` line's fourth field holds 16 words, word *k* carrying id 2*k* in its low half and id
/// 2*k* + 1 in its high half.
fn outputs(dir: &PathBuf, ctrl: &str) -> HashMap<u32, [u32; 32]> {
    let mut out = HashMap::new();
    let text = std::fs::read_to_string(dir.join("mixmap").join(format!("{ctrl}.tsv"))).unwrap();
    for line in text.lines() {
        let frame: u32 = line.split('\t').next().unwrap().parse().unwrap();
        let mut ids = [0u32; 32];
        for (i, w) in line
            .split('|')
            .nth(3)
            .unwrap()
            .split_whitespace()
            .enumerate()
        {
            let w = u32::from_str_radix(w, 16).unwrap();
            ids[2 * i] = w & 0xFFFF;
            ids[2 * i + 1] = w >> 16;
        }
        out.insert(frame, ids);
    }
    out
}

fn word(s: &[u32], off: u32) -> u32 {
    s[((off - 192) / 4) as usize]
}
fn byte(s: &[u32], off: u32) -> bool {
    (word(s, off & !3) >> (8 * (3 - (off & 3)))) & 0xFF != 0
}

#[test]
fn the_state_controller_inputs_match_the_capture_word_for_word() {
    let Some(dir) = extract() else { return };
    let st = states(&dir);
    let mc = inputs(&dir, "4A26B1D0"); // 60010000
    let (mut compared, mut frames) = (0usize, 0usize);
    let mut slew = 0.0f32;
    for (&frame, s) in &st {
        let Some(want) = mc.get(&(frame + 1)) else {
            continue;
        };
        let fields = StateFields {
            wheel_count_200: word(s, 200),
            ground_speed_208: f32::from_bits(word(s, 208)),
            com_speed_212: f32::from_bits(word(s, 212)),
            brake_336: byte(s, 336),
            manual_brake_339: byte(s, 339),
            trick_active_343: byte(s, 343),
            local_player_72: true,
            bail_camera_g16: want[11] != 0, // the G+16 byte is not in the dump
            vector_96: [0.0; 4],
            listener_32: Some([0.0; 4]),
        };
        for (id, value) in state_inputs(&fields, &mut slew, 1.0 / 60.0, DISTANCE_RATE, DISTANCE_CAP)
        {
            if id == 11 {
                continue;
            }
            assert_eq!(value, want[id as usize], "evaluation {} id {id}", frame + 1);
            compared += 1;
        }
        // Id 12 through sub_824B23C8 with the local player's flag set: `+684 == 1`.
        let id12 = if player_flag(true, word(s, 684), &[]) == 1 {
            32767
        } else {
            0
        };
        assert_eq!(id12, want[12], "evaluation {} id 12", frame + 1);
        frames += 1;
    }
    assert!(frames > 18_000, "{frames} frames");
    eprintln!("{compared} words over {frames} evaluations");
}

#[test]
fn rail_and_off_board_inputs_match_the_capture() {
    let Some(dir) = extract() else { return };
    let st = states(&dir);
    let rail = inputs(&dir, "4A26A8D0"); // 40010030
    let off = inputs(&dir, "4A26A930"); // 40010090
    let mut frames: Vec<_> = st.keys().copied().collect();
    frames.sort();
    let mut n = 0;
    for frame in frames {
        let s = &st[&frame];
        // `+342` is the bridge's copy of the previous frame's `+341`.
        let (g, g_prev) = (byte(s, 341), byte(s, 342));
        if let Some(w) = rail.get(&(frame + 1)) {
            let [(_, a), (_, b)] = rail_inputs(g, g_prev);
            assert_eq!((a, b), (w[0], w[1]), "rail, evaluation {}", frame + 1);
            n += 1;
        }
        if let Some(w) = off.get(&(frame + 1)) {
            assert_eq!(
                off_board_input(byte(s, 716)).1,
                w[0],
                "off-board, evaluation {}",
                frame + 1
            );
        }
    }
    assert!(n > 18_000);
}

#[test]
fn the_constants_are_the_images() {
    let dir = std::env::var("SKATE3_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets")
        })
        .join("private/stock/audio-runtime-image");
    if !dir.is_dir() {
        eprintln!("skipped: no image");
        return;
    }
    let read = |addr: u32| -> u32 {
        let page = std::fs::read(dir.join(format!("g_{:04X}.bin", addr >> 16))).unwrap();
        let o = (addr & 0xFFFF) as usize;
        u32::from_be_bytes(page[o..o + 4].try_into().unwrap())
    };
    for (i, &w) in SPEED_SCALES.iter().enumerate() {
        assert_eq!(read(0x822F_9520 + 4 * i as u32), w);
    }
    assert_eq!(read(0x8217_47FC), F32767);
    assert_eq!(read(0x8209_975C), HALF);
    assert_eq!(read(0x8211_61AC), SEVEN_TENTHS);
    assert_eq!(
        (u64::from(read(0x822F_8D58)) << 32) | u64::from(read(0x822F_8D5C)),
        THREE_TENTHS
    );
    assert_eq!(read(0x8209_BE90), EPSILON);
    assert_eq!(read(0x822F_8960), ANGLE_SCALE);
    assert_eq!(read(0x822F_8904), INV_TWO_PI);
    assert_eq!(read(0x8216_DEE0), MINUS_ONE_BITS);
    assert_eq!(read(0x822F_B840), ACOS_ONE_PLUS);
    assert_eq!(read(0x8206_3B08), TIER_X3);
    assert_eq!(read(0x8206_0C50), TIER_X2);
    assert_eq!(read(0x8222_49B4), TIER_X1_5);
    for (base, table) in [
        (0x822F_9820u32, ACOS_A),
        (0x822F_9830, ACOS_B),
        (0x822F_9840, ACOS_C),
        (0x822F_9850, ACOS_D),
    ] {
        for (i, &w) in table.iter().enumerate() {
            assert_eq!(read(base + 4 * i as u32), w, "{base:#x}+{}", 4 * i);
        }
    }
}

#[test]
fn distance_slews_at_the_vault_rate_and_caps() {
    let mut slew = 0.0f32;
    let mut s = StateFields {
        vector_96: [3.0, 4.0, 0.0, 0.0],
        listener_32: Some([0.0; 4]),
        ..Default::default()
    };
    // 5 m away, 100 m/s × 0.01 s = 1 m per call.
    let id13 = |s: &StateFields, slew: &mut f32| {
        state_inputs(s, slew, 0.01, DISTANCE_RATE, DISTANCE_CAP)[12]
    };
    assert_eq!(id13(&s, &mut slew), (13, (1.0f32 / 35.0 * 32767.0) as u32));
    assert_eq!(slew, 1.0);
    for _ in 0..10 {
        id13(&s, &mut slew);
    }
    assert!((slew - 5.0).abs() < 1e-6, "{slew}");
    s.vector_96 = [100.0, 0.0, 0.0, 0.0];
    for _ in 0..40 {
        id13(&s, &mut slew);
    }
    assert_eq!(slew, 35.0, "capped");
    assert_eq!(id13(&s, &mut slew).1, 32767);
}

#[test]
fn listener_facing_is_zero_facing_the_listener_and_full_behind_it() {
    let l = [0.0, 0.0, 1.0, 0.0];
    assert_eq!(listener_facing(l, [0.0; 4], l).0, 0, "a·L = 1 → f1 = 0");
    let (w, v) = listener_facing([0.0, 0.0, -1.0, 0.0], [0.0; 4], l);
    assert_eq!(
        (w, v),
        ((0.7f32 * 32767.0) as u32, 0.7f32),
        "a·L = −1, no up, b ⟂ L → 0.7"
    );
    // |b·L| adds 0.3 × f1: fully behind with b along L saturates at 1.
    assert_eq!(listener_facing([0.0, 0.0, -1.0, 0.0], l, l).0, 32767);
}

#[test]
fn position_controller_on_the_followed_point_reads_zero_distance_and_speed() {
    // The capture's 60010010: emitter = listener pos1, same velocity → ids 0, 2 and 13 are 0.
    let listener = Listener {
        pos0: [0.0, 1.5, -3.0, 1.0],
        dir0: [0.0, 0.0, 1.0, 0.0],
        vel0: [0.0; 4],
        pos1: [0.0, 1.0, 0.0, 1.0],
        dir1: [0.0, 0.0, 1.0, 0.0],
        vel1: [1.0, 0.0, 2.0, 0.0],
        ..Default::default()
    };
    let emitter = Emitter {
        position_32: Some(listener.pos1),
        velocity_36: Some(listener.vel1),
        facing_28: Some([0.0, 0.0, 1.0, 0.0]),
    };
    let mut obj = ObjPos::default();
    let w: HashMap<u32, u32> = obj.update(&listener, &emitter, 0).into_iter().collect();
    assert_eq!(w[&0], 0, "|pos1 − emitter|");
    assert_eq!(
        w[&2], 0,
        "no horizontal offset from frame B's origin → angle 0"
    );
    assert_eq!(w[&13], 0, "|v − vel1|");
    assert_eq!(f32::from_bits(w[&1]), obj.dist0_48);
    assert_eq!(w[&15] & 1, 1);
    // Straight ahead of frame A (pulled back 0.25 along its direction): angle 0.
    assert_eq!(w[&3], 0);
    // Inactive: the −1.0 distances and bit 0 cleared.
    let off: Vec<_> = obj.update(&listener, &Emitter::default(), 0xC000_0001);
    assert_eq!(
        off,
        vec![
            (3, 0),
            (1, 0xBF80_0000),
            (2, 0),
            (0, 0xBF80_0000),
            (15, 0xC000_0000)
        ]
    );
}

#[test]
fn position_angles_turn_through_the_horizontal_plane() {
    let listener = Listener {
        dir0: [0.0, 0.0, 1.0, 0.0],
        dir1: [0.0, 0.0, 1.0, 0.0],
        ..Default::default()
    };
    let at = |x: f32, z: f32| {
        let mut obj = ObjPos {
            pullback_112: 0.0,
            ..Default::default()
        };
        let e = Emitter {
            position_32: Some([x, 5.0, z, 1.0]),
            ..Default::default()
        };
        let w: HashMap<u32, u32> = obj.update(&listener, &e, 0).into_iter().collect();
        (w[&3], w[&2])
    };
    // Ahead (+z): 0. To the side: a quarter turn, one side mirrored to 65535 − q.
    assert_eq!(at(0.0, 10.0).0, 0);
    let (right, _) = at(10.0, 0.0);
    let (left, _) = at(-10.0, 0.0);
    assert!((16380..=16390).contains(&right.min(left)), "{right} {left}");
    assert_eq!(right + left, 65535);
    let (behind, _) = at(0.0, -10.0);
    assert!((32760..=32775).contains(&behind), "{behind}");
}

#[test]
fn position_rates_flag_a_sign_change() {
    let listener = Listener {
        dir0: [0.0, 0.0, 1.0, 0.0],
        dir1: [0.0, 0.0, 1.0, 0.0],
        ..Default::default()
    };
    let mut obj = ObjPos::default();
    let v = Some([0.0, 0.0, 3.0, 0.0]);
    // Receding along +z: positive rates.
    for z in [5.0, 6.0] {
        obj.update(
            &listener,
            &Emitter {
                position_32: Some([0.0, 0.0, z, 1.0]),
                velocity_36: v,
                facing_28: None,
            },
            1,
        );
    }
    assert!(obj.rate1_96 > 0.0);
    // Approaching: the rates turn negative and both flags are raised.
    let w = obj.update(
        &listener,
        &Emitter {
            position_32: Some([0.0, 0.0, 4.0, 1.0]),
            velocity_36: v,
            facing_28: None,
        },
        1,
    );
    assert!(obj.rate1_96 < 0.0 && obj.rate0_104 < 0.0);
    assert!(
        w.contains(&(15, 0x8000_0001)) && w.contains(&(15, 0xC000_0001)),
        "{w:?}"
    );
    assert_eq!(w.last(), Some(&(14, obj.rate0_104.to_bits())));
}

#[test]
fn contacts_landing_inputs_match_the_capture() {
    let Some(dir) = extract() else { return };
    let st = states(&dir);
    let mc = inputs(&dir, "4A26A8B0"); // 40010010
    let mut frames: Vec<_> = st.keys().copied().collect();
    frames.sort();
    let mut contacts = Contacts::default();
    let mut words = [0u32; 16];
    let (mut n, mut landings) = (0, 0);
    for frame in frames {
        let s = &st[&frame];
        let f = ContactsFields {
            airborne_332: byte(s, 332),
            grinding_341: byte(s, 341),
            wheel_contact_464: [byte(s, 464), byte(s, 465), byte(s, 466), byte(s, 467)],
            wheel_word_448: [0, 1, 2, 3].map(|i| word(s, 448 + 4 * i) as i32),
            wheel_material_620: [0, 1, 2, 3].map(|i| word(s, 620 + 4 * i)),
            local_player: true,
        };
        let writes = contacts.process(&f);
        if writes.len() > 3 {
            landings += 1;
        }
        for (id, v) in writes {
            words[id as usize] = v;
        }
        if let Some(w) = mc.get(&(frame + 1)) {
            for id in [1usize, 2, 6] {
                assert_eq!(words[id], w[id], "evaluation {} id {id}", frame + 1);
            }
            n += 1;
        }
    }
    assert!(n > 18_000 && landings >= 40, "{n} {landings}");
}

#[test]
fn jitter_walks_inside_its_bounds_with_the_shared_generator() {
    let mut g = crate::Guest::single(0x82FD_0000, 0x1_0000);
    for (i, w) in [
        0x1234_5678u32,
        0x9ABC_DEF0,
        0x0F1E_2D3C,
        0x4B5A_6978,
        0x8796_A5B4,
        0xC3D2_E1F0,
    ]
    .iter()
    .enumerate()
    {
        g.set_u32(crate::grain::rng::STATE + 4 * i as u32, *w)
            .unwrap();
    }
    let mut j = Jitter::retail();
    assert_eq!(j.channels.len(), 24);
    for _ in 0..2000 {
        let w = j.process(&mut g).unwrap();
        // Six enabled channels, ids 4, 3, 0, 1, 5, 2 in key order.
        assert_eq!(
            w.iter().map(|&(id, _)| id).collect::<Vec<_>>(),
            vec![4, 3, 0, 1, 5, 2]
        );
        for ch in &j.channels {
            let [c, r, max, _] = ch.params;
            assert!(ch.value >= c - r && ch.value <= c + r, "{ch:?}");
            assert!(ch.velocity.abs() <= max, "{ch:?}");
        }
        assert_eq!(w[4], (5, 0), "id 5 has all-zero params");
    }
}

#[test]
fn jitter_step_reduces_the_draw_mod_2001() {
    // The mulhwu sequence is r mod 2001 for every word.
    for r in [
        0u32,
        1,
        2000,
        2001,
        2002,
        0x7FFF_FFFF,
        0x8000_0000,
        0xFFFF_FFFE,
        0xFFFF_FFFF,
        123_456_789,
    ] {
        let hi = ((u64::from(r) * 0x0603_538B) >> 32) as u32;
        let q = ((r.wrapping_sub(hi) >> 1).wrapping_add(hi)) >> 10;
        assert_eq!(r - q * 2001, r % 2001, "{r}");
    }
    // A draw of 1000 (+0 after the offset) pushes by the minimum step only.
    let mut ch = Jitter::retail().channels[23]; // id 2: centre 16384, range 16383, 31000, 19000
    ch.step(1000); // velocity 19000, value 35384 > 32767: bounced
    assert_eq!(
        ch.value,
        16384.0 + 19000.0 - 2.0 * (16384.0 + 19000.0 - 32767.0)
    );
    assert_eq!(ch.velocity.to_bits(), (-19000.0f32).to_bits(), "bounced");
}

#[test]
fn the_listener_update_takes_camera_rows_and_the_first_record() {
    let mut l = Listener::default();
    let rec = PlayerRecord {
        position_0: [1.0, 2.0, 3.0, 1.0],
        facing_16: [0.0, 0.0, 1.0, 0.0],
        velocity_32: [4.0, 0.0, 0.0, 0.0],
        ..Default::default()
    };
    l.update(
        [0.0, 0.0, 2.0, 0.0],
        [0.0, 1.0, 0.0, 1.0],
        true,
        0.5,
        Some(&rec),
    );
    assert_eq!(l.dir0[2], 1.0);
    assert_eq!(
        l.vel0,
        [0.0, 2.0, 0.0, 2.0],
        "(pos − prev) / dt over all four lanes"
    );
    l.update(
        [0.0, 0.0, 2.0, 0.0],
        [1.0, 1.0, 0.0, 1.0],
        false,
        0.5,
        Some(&rec),
    );
    assert_eq!(
        l.vel0,
        [0.0, 2.0, 0.0, 2.0],
        "kept when the camera counter did not change"
    );
    assert_eq!(
        (l.pos1, l.vel1, l.prev_pos1_80),
        (rec.position_0, rec.velocity_32, rec.position_0)
    );
    let (a, b) = rec.emitters();
    assert_eq!(a.position_32, Some(rec.position_0));
    assert_eq!(b.velocity_36, Some(rec.velocity_80));
}

#[test]
fn multiplier_tiers_follow_the_published_multiplier() {
    assert_eq!(
        multiplier_flags(None),
        0,
        "no score object: all tier bits clear"
    );
    let cases = [
        (1.0, 0),
        (1.49, 0),
        (1.5, MULTIPLIER_X1_5),
        (1.99, MULTIPLIER_X1_5),
        (2.0, MULTIPLIER_X2),
        (2.5, MULTIPLIER_X2),
        (3.0, MULTIPLIER_X3),
        (4.0, MULTIPLIER_X3),
    ];
    for (m, want) in cases {
        assert_eq!(multiplier_flags(Some(m)), want, "{m}");
    }
    assert_eq!(
        multiplier_flags(Some(f32::NAN)),
        MULTIPLIER_X3,
        "unordered compares fall through"
    );
    assert!(multiplier_x3(MULTIPLIER_X3, false));
    assert!(!multiplier_x3(MULTIPLIER_X2, false));
    assert!(multiplier_x3(0, true));
}

#[test]
fn music_emphasis_slews_at_the_vault_rates() {
    let dt = 1.0 / 60.0;
    let mut m = MusicEmphasis::default();
    // Up at fctiwz(3000 × 1/60) = 50 per frame toward the tier's target.
    assert_eq!(m.process(MULTIPLIER_X2, false, dt), [(3, 0), (6, 50)]);
    for _ in 0..300 {
        m.process(MULTIPLIER_X2, false, dt);
    }
    assert_eq!(m.value_176, 12000);
    // x3 raises id 3 at once and keeps slewing id 6.
    assert_eq!(
        m.process(MULTIPLIER_X3, false, dt),
        [(3, 32767), (6, 12050)]
    );
    // Down at fctiwz(9000 × 1/60) = 150 per frame, landing on the target.
    m.value_176 = 5100;
    assert_eq!(m.process(MULTIPLIER_X1_5, false, dt), [(3, 0), (6, 5000)]);
    m.value_176 = 5200;
    assert_eq!(m.process(0, false, dt)[1], (6, 5050));
    // dt ≤ 0 (or NaN) zeroes the slew.
    assert_eq!(m.process(MULTIPLIER_X3, false, 0.0), [(3, 32767), (6, 0)]);
    m.value_176 = 700;
    m.process(0, false, f32::NAN);
    assert_eq!(m.value_176, 0);
}

#[test]
fn music_emphasis_tuning_is_the_vaults() {
    let path = std::env::var("SKATE3_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets")
        })
        .join("private/stock/skater-collections.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: no vault at {}", path.display());
        return;
    };
    let start = text
        .find("\"key\": \"Hash_47EC76B4F9FC79F6\"")
        .expect("tuning record");
    let record = &text[start..start + text[start..].find("\"source\"").unwrap()];
    assert!(text[..start].ends_with("{\"class\": \"Hash_C1831BDB6CB1B1EA\", "));
    let field = |hash: &str| -> u32 {
        let at = record
            .find(&format!("\"Hash_{hash}\""))
            .unwrap_or_else(|| panic!("{hash}"));
        let data = &record[at..];
        let d = data.find("\"data\": \"").unwrap() + 9;
        u32::from_str_radix(&data[d..d + 8], 16).unwrap()
    };
    assert_eq!(field("6EE4718F1A7EB772") as i32, MUSIC_EMPHASIS_TARGETS[0]);
    assert_eq!(field("50F6520E2DD3D54C") as i32, MUSIC_EMPHASIS_TARGETS[1]);
    assert_eq!(field("A6AA0C534DEA29E7") as i32, MUSIC_EMPHASIS_TARGETS[2]);
    assert_eq!(field("7C44AE016D99A9EE"), MUSIC_EMPHASIS_DOWN);
    assert_eq!(field("1666A4A45EC309AE"), MUSIC_EMPHASIS_UP);
}

/// The Flips emphasis (`sub_824CD170`, packet w12: 250/700/1000 for flags 0x8000/0x4000/0x2000)
/// and Music ids 3/6 read the same flags word: wherever both sit on a target in the capture they
/// name the same tier, and SkateBoard ids 21/22 take one pair of values per tier.
#[test]
fn music_emphasis_tiers_match_the_flips_emphasis_in_the_capture() {
    let Some(dir) = extract() else { return };
    let music = inputs(&dir, "4A26A840");
    let flips = std::fs::read_to_string(dir.join("updates/Class_Flips.tsv")).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for line in flips.lines() {
        let frame: u32 = line.split('\t').next().unwrap().parse().unwrap();
        let words: Vec<u32> = line
            .split('|')
            .nth(1)
            .unwrap()
            .split_whitespace()
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect();
        let Some(m) = music.get(&frame) else { continue };
        let tier = |w12: u32, id6: u32| match (w12, id6) {
            (0, 0) => Some(0),
            (250, 5000) => Some(MULTIPLIER_X1_5),
            (700, 12000) => Some(MULTIPLIER_X2),
            (1000, 32767) => Some(MULTIPLIER_X3),
            _ => None,
        };
        let plateau =
            [0, 250, 700, 1000].contains(&words[12]) && [0, 5000, 12000, 32767].contains(&m[6]);
        if plateau {
            let flags = tier(words[12], m[6])
                .unwrap_or_else(|| panic!("frame {frame}: w12 {} vs id6 {}", words[12], m[6]));
            assert_eq!(
                m[3],
                if multiplier_x3(flags, false) {
                    32767
                } else {
                    0
                },
                "frame {frame}"
            );
            seen.insert(flags);
        }
        if words[12] == 1000 {
            assert_eq!(
                m[3], 32767,
                "frame {frame}: the x3 emphasis raises Music id 3"
            );
        }
    }
    assert_eq!(seen.len(), 4, "every tier appears: {seen:?}");

    // SkateBoard 21/22 per Music (3, 6) plateau.
    let text = std::fs::read_to_string(dir.join("mixmap/4A26A8A0.tsv")).unwrap();
    let mut pairs =
        std::collections::BTreeMap::<(u32, u32), std::collections::BTreeMap<(u32, u32), u32>>::new(
        );
    for line in text.lines() {
        let frame: u32 = line.split('\t').next().unwrap().parse().unwrap();
        let Some(m) = music.get(&frame) else { continue };
        if ![0, 5000, 12000, 32767].contains(&m[6]) {
            continue;
        }
        let outs: Vec<u32> = line
            .split('|')
            .nth(3)
            .unwrap()
            .split_whitespace()
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect();
        let (id21, id22) = (outs[10] >> 16, outs[11] & 0xFFFF);
        if id21 != 0 {
            *pairs
                .entry((m[3], m[6]))
                .or_default()
                .entry((id21, id22))
                .or_default() += 1;
        }
    }
    let modal = |k: (u32, u32)| {
        pairs[&k]
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(v, _)| *v)
            .unwrap()
    };
    assert_eq!(modal((0, 0)), (1267, 1835));
    assert_eq!(modal((0, 5000)), (1287, 1865));
    assert_eq!(modal((0, 12000)), (1969, 2853));
    assert_eq!(modal((32767, 32767)), (28343, 32730));
}

#[test]
fn the_pause_writer_latches_only_outside_the_free_skate_path() {
    // Free skate: the request goes straight to id 0, and id 2 stays clear.
    assert_eq!(pause_inputs(true), [(0, 32767), (1, 0), (2, 0)]);
    assert_eq!(pause_inputs(false), [(0, 0), (1, 0), (2, 0)]);
    // sys+1064 != 1 (the capture's start-up path): the first requesting frame only clears the
    // latch, later frames hold id 2, and releasing the request re-arms it.
    let mut pause = Pause::retail();
    assert!(pause.armed_28);
    let f = PauseFields {
        request: true,
        mode_1064: false,
        state_6: false,
    };
    assert_eq!(pause.process(f), [(0, 0), (1, 0), (2, 0)]);
    assert!(!pause.armed_28);
    assert_eq!(pause.process(f), [(0, 0), (1, 0), (2, 32767)]);
    assert_eq!(
        pause.process(PauseFields::default()),
        [(0, 0), (1, 0), (2, 0)]
    );
    assert!(pause.armed_28);
    // id 1 is the game-flow state alone.
    assert_eq!(
        pause.process(PauseFields {
            request: false,
            mode_1064: true,
            state_6: true
        }),
        [(0, 0), (1, 32767), (2, 0)]
    );
}

/// The capture's pause: `pause_inputs` reproduces the Pause controller's words, and while id 0 is
/// held every player controller's level outputs are silent. The extract is
/// `mixmap/4A26B250.tsv` (the `MC` lines of `40000070`).
#[test]
fn the_pause_inputs_and_the_player_silence_match_the_capture() {
    let Some(dir) = extract() else { return };
    if !dir.join("mixmap/4A26B250.tsv").is_file() {
        eprintln!("skipped: no 40000070 extract (mixmap/4A26B250.tsv)");
        return;
    }
    let pause = inputs(&dir, "4A26B250");
    let mut held = Vec::new();
    for (&frame, words) in &pause {
        for id in 3..16 {
            assert_eq!(words[id], 0, "frame {frame}: Pause id {id}");
        }
        assert_eq!(
            words[1], 0,
            "frame {frame}: the capture never reaches game-flow state 6"
        );
        assert!(
            words[0] == 0 || words[2] == 0,
            "frame {frame}: ids 0 and 2 are exclusive"
        );
        for id in [0, 2] {
            assert!(
                words[id] == 0 || words[id] == 32767,
                "frame {frame}: Pause id {id}"
            );
        }
        if words[0] != 0 {
            held.push(frame);
        }
        // The free-skate path (`mode_1064`) writes id 0 alone, so wherever id 2 is clear the
        // ported writer reproduces the capture's words exactly.
        if words[2] == 0 {
            assert_eq!(
                pause_inputs(words[0] != 0),
                [(0, words[0]), (1, words[1]), (2, words[2])],
                "frame {frame}"
            );
        }
    }
    held.sort_unstable();
    assert_eq!(held.len(), 3949, "the capture's one pause-menu pause");
    let (first, last) = (held[0], held[held.len() - 1]);
    assert_eq!(last - first + 1, held.len() as u32, "one contiguous pause");

    // Every player controller's level outputs are 0 while the pause is held, within 12
    // evaluations of it starting. The levels here are the ones the components read as levels
    // (`read_gain`), i.e. the outputs that are non-zero before the pause and fall to 0.
    for ctrl in [
        "4A26A8A0", "4A26A8B0", "4A26A8C0", "4A26A8D0", "4A26A8E0", "4A26A8F0", "4A26A900",
        "4A26A910", "4A26A920", "4A26A930",
    ] {
        let outs = outputs(&dir, ctrl);
        let before = &outs[&(first - 5)];
        let after = &outs[&(first + 12)];
        let fell: Vec<usize> = (0..32)
            .filter(|&i| before[i] > 1000 && after[i] <= 1)
            .collect();
        assert!(
            !fell.is_empty(),
            "{ctrl}: no output fell silent during the pause"
        );
        for &i in &fell {
            for frame in (first + 12)..=last {
                assert!(
                    outs[&frame][i] <= 1,
                    "{ctrl} output {i} at {frame}: {}",
                    outs[&frame][i]
                );
            }
        }
    }
}
