//! Playtest packet trace in the retail recomp capture's own line format, so
//! `tools/audio_capture_extract.py` splits a Rust playtest exactly like the retail capture and the
//! two can be compared word for word.
//!
//! Enabled by `SKATE_AUDIO_TRACE=<file>`. Lines (`<ms> <frame> <TAG> …`, hex words):
//!
//! * `PO <slot> <index> <name> <payload> 0 | 28 words | handle=<handle>` — a post;
//! * `UP <node> <name> <payload> | 28 words` — a redelivery;
//! * `RL <node> <name>` — a release;
//! * `OP <index> byte=<b> desc=[6 words] records=[id:a/value …] -> voice=<voice>` — a voice open,
//!   the capture's `skate3-audio-open` line; follows the post or update whose patch opened it;
//! * `VR <voice>` — that voice released;
//! * `AS st=<state> cat=<category> <field>=<value> …` — the physical state id and the
//!   audio-state fields the families' triggers read;
//! * `BC bail676=<b> … slide528=[…] mat560=[…] impact496=[…]` — the ragdoll contact fields, on
//!   frames where a bail is contacting, sliding or holding a loose board;
//! * `OUT <p0..p5> | rms <r> frames <n> ch <c>` — one per rendered block, in the retail capture's
//!   own format (the six native channels before the host downmix), so the two can be compared
//!   directly; `DEV <peak> <rms>` follows it with the stereo the host actually receives.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;

use super::audio_state::AudioState;
use crate::skate_audio::RetailAudioInputs;

struct Trace {
    out: std::io::BufWriter<std::fs::File>,
    start: Instant,
    frame: u64,
    names: HashMap<u32, String>,
}

static TRACE: Mutex<Option<Trace>> = Mutex::new(None);
static CHECKED: std::sync::Once = std::sync::Once::new();

fn with<F: FnOnce(&mut Trace)>(f: F) {
    CHECKED.call_once(|| {
        if let Some(path) = std::env::var_os("SKATE_AUDIO_TRACE") {
            if let Ok(file) = std::fs::File::create(&path) {
                *TRACE.lock().unwrap_or_else(|p| p.into_inner()) = Some(Trace {
                    out: std::io::BufWriter::with_capacity(1 << 20, file),
                    start: Instant::now(),
                    frame: 0,
                    names: HashMap::new(),
                });
                skate_audio_core::voice::observe_opens(open);
                skate_audio_core::voice::observe_releases(voice_release);
            }
        }
    });
    if let Some(trace) = TRACE.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        f(trace);
    }
}

fn words(words: &[u32]) -> String {
    (0..28)
        .map(|i| format!("{:08X}", words.get(i).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Trace {
    fn line(&mut self, body: std::fmt::Arguments) {
        let ms = self.start.elapsed().as_millis();
        let _ = writeln!(self.out, "{ms} {} {body}", self.frame);
    }
}

pub(crate) fn frame(frame: u64) {
    with(|t| {
        t.frame = frame;
        if frame % 60 == 0 {
            let _ = t.out.flush();
        }
    });
}

fn voice_release(voice: u32) {
    with(|t| t.line(format_args!("VR {voice:08X}")));
}

fn open(request: &skate_audio_core::voice::OpenRequest, records: &[(u8, i32, i32)], voice: u32) {
    let desc = request
        .shifted
        .iter()
        .map(|w| format!("{w:08X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let records = records
        .iter()
        .map(|(id, a, value)| format!("{id}:{a}/{value}"))
        .collect::<Vec<_>>()
        .join(" ");
    with(|t| {
        t.line(format_args!(
            "OP {} byte={} desc=[{desc}] records=[{records}] -> voice={voice:08X}",
            request.index, request.byte2
        ))
    });
}

pub(crate) fn post(object: &str, handle: u32, payload: &[u32]) {
    with(|t| {
        t.names.insert(handle, object.to_owned());
        t.line(format_args!(
            "PO 00000000 0 {object} {handle:08X} 0 | {} | handle={handle:08X}",
            words(payload)
        ));
    });
}

pub(crate) fn update(handle: u32, payload: &[u32]) {
    with(|t| {
        let name = t
            .names
            .get(&handle)
            .cloned()
            .unwrap_or_else(|| "(unknown)".into());
        t.line(format_args!(
            "UP {handle:08X} {name} {handle:08X} | {}",
            words(payload)
        ));
    });
}

pub(crate) fn release(handle: u32) {
    with(|t| {
        let name = t
            .names
            .remove(&handle)
            .unwrap_or_else(|| "(unknown)".into());
        t.line(format_args!("RL {handle:08X} {name}"));
    });
}

/// One rendered block: the six native channels (as retail's output pass logs them) and then the
/// stereo the host receives after the downmix and the interim trim.
pub(crate) fn output(native: &[f32], channels: usize, stereo: &[f32]) {
    with(|t| {
        let frames = native.len() / channels.max(1);
        let mut line = String::from("OUT");
        let mut energy = 0.0f64;
        for channel in 0..channels {
            let mut peak = 0.0f32;
            for frame in 0..frames {
                let sample = native[frame * channels + channel];
                if sample.is_finite() {
                    peak = peak.max(sample.abs());
                    energy += f64::from(sample) * f64::from(sample);
                }
            }
            line += &format!(" {peak:.6}");
        }
        let rms = (energy / native.len().max(1) as f64).sqrt();
        line += &format!(" | rms {rms:.6} frames {frames} ch {channels}");
        t.line(format_args!("{line}"));
        let device_peak = stereo.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        let device_rms = (stereo
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / stereo.len().max(1) as f64)
            .sqrt();
        t.line(format_args!("DEV {device_peak:.6} {device_rms:.6}"));
    });
}

/// The physical state id travels alongside the audio state because several
/// gates (the squeak's especially) only mean anything while their owning state
/// is running: `tilt264` is the steering tilt, not a slide-specific angle, so a
/// passing gate proves nothing on its own. `st` is State+16, `cat` is State+12.
pub(crate) fn state(a: &AudioState, inputs: &RetailAudioInputs) {
    with(|t| {
        t.line(format_args!(
            "AS st={} cat={} v208={:.3} com212={:.3} turn204={:.4} wheels200={} tilt264={:.4} air332={} grind341={} feet615={} feet616={} slip232={:.4} trick348={} bail676={} walk716={} down724={} down725={} land448={:?} landed464={:?}",
            inputs.state,
            inputs.state_category,
            a.ground_speed_208,
            a.com_speed_212,
            a.turn_204,
            a.wheel_count_200,
            a.deck_tilt_264,
            u8::from(a.in_known_air_332),
            u8::from(a.grinding_341),
            u8::from(a.foot_in_deck_box_615),
            u8::from(a.foot_in_deck_box_616),
            a.slip_232,
            a.audio_trick_348 as i32,
            u8::from(a.bail_676),
            u8::from(a.walking_716),
            u8::from(a.foot_down_right_724),
            u8::from(a.foot_down_left_725),
            a.wheel_landing_bucket_448,
            a.wheel_landed_464,
        ));
    });
    body(a);
}

/// The board's four grain-player records, in the retail capture's `BD` sense.
///
/// The capture's own `BD` line carries the whole `GrainPlayer` object; this carries the four words
/// the owner writes into it each frame (`sub_824C6BD8`), which is what
/// `docs/player-audio-retail-drivers.md` §11 tabulates against ground speed. Without it a playtest
/// cannot be compared against that table at all, and the B player's gain is the one number that
/// says whether the offset second layer is running when retail's would be silent.
///
/// `a`/`b` are the two players of a truck: `GR <truck> a=<gain>,<pitch>,<position> b=...`.
pub(crate) fn grain_records(truck: usize, a: (f32, f32, f32), b: (f32, f32, f32)) {
    if truck < 2 {
        // Kept whether or not a trace file is open: the headless sweep reads these back directly
        // rather than parsing its own trace, and a trace file is a playtest concern.
        *LAST_GRAIN[truck].lock().unwrap_or_else(|p| p.into_inner()) = Some((a, b));
    }
    with(|t| {
        t.line(format_args!(
            "GR {truck} a={:.6},{:.4},{:.4} b={:.6},{:.4},{:.4}",
            a.0, a.1, a.2, b.0, b.1, b.2
        ));
    });
}

type GrainPair = ((f32, f32, f32), (f32, f32, f32));
static LAST_GRAIN: [Mutex<Option<GrainPair>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// The last `{gain, pitch, position}` pair written for a truck, for the headless sweeps.
pub(crate) fn last_grain_records(truck: usize) -> Option<GrainPair> {
    *LAST_GRAIN
        .get(truck)?
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// The ragdoll contact fields, on their own line and only while a bail is doing something.
///
/// `c_body_slide` and `c_cloth_falls` read nothing else, and none of it appears on the `AS` line,
/// so a playtest could not tell a silent bail from a bail whose material never resolved. `mat` is
/// the one to watch: it is `tag & 0x7f` off the contact row, and `body_contact` maps a zero to the
/// default slide type 2 -- so an all-zero `mat` during a real slide means body-vs-material is not
/// happening, whatever the family posts.
fn body(a: &AudioState) {
    let sliding = a.body_slide_528.iter().any(|v| v.abs() > 0.0);
    let impacting = a.body_impact_496.iter().any(|v| v.abs() > 0.0);
    if !(sliding || impacting || a.bail_676 || a.bail_over_677 || a.loose_board_780 != 0) {
        return;
    }
    with(|t| {
        t.line(format_args!(
            "BC bail676={} over677={} spin328={:.4} limb672={:.4} com212={:.3} groin592={} face593={} board780={} slide528={:?} mat560={:?} impact496={:?}",
            u8::from(a.bail_676),
            u8::from(a.bail_over_677),
            a.ragdoll_spin_328,
            a.limb_speed_672,
            a.com_speed_212,
            u8::from(a.groin_contact_592),
            u8::from(a.face_contact_593),
            a.loose_board_780,
            a.body_slide_528,
            a.body_material_560,
            a.body_impact_496,
        ));
    });
}
