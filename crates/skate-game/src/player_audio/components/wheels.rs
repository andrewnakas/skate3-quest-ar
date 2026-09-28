//! `SFXObj_Wheels` (vtable `0x822FC848`, controller `0x40010020`): the wheel spin-down heard when
//! the board leaves the ground, starts a manual, or is spun in the hand. Played from
//! `data\audio/wheels.big` through the owner's own graphs (`skate_audio_core::grain::stream`), not
//! through posted messages.
//!
//! | retail | here |
//! |---|---|
//! | `sub_824CD6F8` constructor: the two `.snr` resources (`sub_828DC158`, type 4, slots 0/1), the two `.sek` buffers (`sub_8298ED88`), start offsets `+52`/`+56` | [`Wheels::new`] |
//! | `sub_824CE108` bus graph | `Grains::stream_bus` |
//! | slot 7 `sub_824CDC38` enable (`+96` = 1: the first spin uses the Jump set) | [`Wheels::new`] |
//! | slot 9 `sub_82B61BB8` (`blr`) | [`Component::process`] (nothing) |
//! | slot 10 `sub_824CDC70` → `sub_824CDD28` ×3, `sub_824CE488` | [`Component::update`] |
//! | `sub_824CEAF0` start, `sub_824CEE00` gain/pitch, `sub_824CEF60` stop | `Grains::stream_start` / `stream_set` / `stream_stop` |
//!
//! **Triggers** (`sub_824CDC70`), each a latch at `+60+i`:
//! - 0: known air (`+332`);
//! - 1: manual (`+340`);
//! - 2, local player only: walking with the board held (`+716 && +308`) or the handplant bit
//!   (`+760`).
//!
//! On a rising edge `sub_824CDD28` starts voice `i` from `start × (1 − clamp01(v / 50 × 3.6))`
//! seconds; trigger 2 caps the ratio at 0.26 (`0x8208ECFC`). The spin uses the Jump or Man
//! `.snr`/`.sek`/start, alternating on every start (`+96`). While the trigger holds, a finished
//! voice (`[graph+71] == 2`) is stopped. Otherwise it gets:
//! - gain `level(1)`, or `level(5)` on a manual, `level(6)` with the board held, `level(7)` on a
//!   handplant, each / 32767;
//! - pitch `pitch(2)` / 4096.
//!
//! When the trigger drops the voice stops.
//!
//! **Bus** (`sub_824CE488`, while any voice lives):
//! - Pn21 angle `raw(0) × 360/65535`.
//! - Local player only:
//!   - PI20 `{lerp(4500, 800, c)·t, 0.1, 20}`;
//!   - HS20 `{4000·t, lerp(3.0, 0.5, c)}`;
//!   - the `[[manager+116]]` send at `level(4)` / 32767.
//!   - Here `t` = `min(+220, 1)` (time scale) and `c` = audio state `+680`.
//! - Always: the `[[manager+52]]` send at `level(3)` / 32767.
//!
//! **Inputs the audio state does not carry yet** (reported):
//! - `+680`, written by `sub_824B2088` from the camera/listener direction; constructor value 0.0.
//!   Set it with [`Wheels::set_camera_680`].
//! - `+760`, the handplant bit (airborne packet word `+152` bit 5, `sub_824B0DA8`).
//!   Set it with [`Wheels::set_handplant_760`].

use std::path::Path;

use skate_audio_core::fp::{fmadd_single, nmsub_single};
use skate_audio_core::grain::stream::{Bus, Voice};
use skate_data::audio::grains::GrainMember;
use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::vault_word;
use super::words::KMH_PER_MS;
use super::{Component, Tick};

const HOLDER: &str = "Hash_C1831BDB6CB1B1EA";
/// The Wheels instance of the tuning holder's `+80`.
const WHEELS: &str = "Hash_03B710C80E1AC13E";
const EQ_CLASS: &str = "Hash_42AFE160E647167C";
/// `0x8208ECFC`: trigger 2's ratio cap.
const COMBO_CAP: f32 = f32::from_bits(0x3E85_1EB8);
/// `0x822F8898`, `0x822F890C`, `0x822F8C64`.
const INV_32767: f32 = f32::from_bits(0x3800_0100);
const INV_4096: f32 = f32::from_bits(0x3980_0000);
const PAN_SCALE: f32 = f32::from_bits(0x3BB4_00B4);

/// The vault values the owner reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WheelsVault {
    /// `.snr` and `.sek` members and start offsets of the Jump (`+36`/`+44`/`+52`) and Man
    /// (`+40`/`+48`/`+56`) sets.
    pub snr: [String; 2],
    pub sek: [String; 2],
    pub start: [f32; 2],
    /// `0x4890392C91829954`: the km/h the start ratio is taken against (50).
    pub top_kmh: f32,
    /// eEQChain `0x55E6488906A2E330` (6).
    pub eq_chain: u32,
    /// PI20 frequency at `c` = 0 and 1 (`0x5DDF8C07AC1D35C4` 4500, `0xCB85086FA8B931A1` 800), its
    /// two other properties (`0x3EC2F1716EF20A56` 0.1, `0xCF8F01B573765924` 20).
    pub peak_far: i32,
    pub peak_near: i32,
    pub peak_q: f32,
    pub peak_gain: f32,
    /// HS20 corner (`0xE7759E1A690900F2` 4000) and gain at `c` = 0 and 1 (`0x23B41479F58EFF7F`
    /// 3.0, `0x28F57A87418A5CF3` 0.5).
    pub shelf_corner: i32,
    pub shelf_far: f32,
    pub shelf_near: f32,
}

impl WheelsVault {
    pub(crate) fn load(c: &Collections) -> Result<Self, String> {
        // `EA::Reflection::Text` fields hold the string itself.
        let text = |name: &str| -> Result<String, String> {
            let field = c.field(HOLDER, WHEELS, name)?;
            if field.type_name != "EA::Reflection::Text" {
                return Err(format!("Expected text at {HOLDER}/{WHEELS}/{name}"));
            }
            Ok(field.data.clone())
        };
        let float = |name: &str| c.float(HOLDER, WHEELS, name);
        let int = |name: &str| c.integer(HOLDER, WHEELS, name).map(|v| v as i32);
        Ok(Self {
            snr: [
                text("Hash_044FB3ECB9FCB35F")?,
                text("Hash_243117D2CD2EDC70")?,
            ],
            sek: [
                text("Hash_82F1E6D5A0300576")?,
                text("Hash_6CD2B3AB6ACF77C4")?,
            ],
            start: [
                float("Hash_FD874514FD49261E")?,
                float("Hash_8CC31309D10F1763")?,
            ],
            top_kmh: float("Hash_4890392C91829954")?,
            eq_chain: vault_word(c, EQ_CLASS, "default", "Hash_55E6488906A2E330")?,
            peak_far: int("Hash_5DDF8C07AC1D35C4")?,
            peak_near: int("Hash_CB85086FA8B931A1")?,
            peak_q: float("Hash_3EC2F1716EF20A56")?,
            peak_gain: float("Hash_CF8F01B573765924")?,
            shelf_corner: int("Hash_E7759E1A690900F2")?,
            shelf_far: float("Hash_23B41479F58EFF7F")?,
            shelf_near: float("Hash_28F57A87418A5CF3")?,
        })
    }

    /// The four `wheels.big` members the constructor opens.
    pub(crate) fn members(&self) -> [&str; 4] {
        [&self.snr[0], &self.snr[1], &self.sek[0], &self.sek[1]]
    }
}

/// Load the vault and the decoded `wheels.big` members.
pub(crate) fn load(
    assets: &Path,
    cache: Option<&Path>,
) -> Result<(WheelsVault, Vec<GrainMember>), String> {
    let vault = WheelsVault::load(&Collections::load(assets)?)?;
    let members = skate_data::audio::grains::load_members(
        &assets.join("private/stock/data/audio/wheels.big"),
        &vault.members(),
        cache,
    )
    .map_err(|e| e.to_string())?;
    Ok((vault, members))
}

/// `sub_824CDC70`'s three triggers.
pub(crate) fn triggers(audio: &AudioState, local: bool, handplant_760: bool) -> [bool; 3] {
    [
        audio.in_known_air_332,
        audio.balance_340,
        local && ((audio.walking_716 && audio.board_held_308) || handplant_760),
    ]
}

/// `sub_824CDD28`'s start offset for trigger `index`: `start × (1 − clamp01(v / top × 3.6))`,
/// the ratio capped at 0.26 for trigger 2.
pub(crate) fn start_seconds(speed: f32, top_kmh: f32, index: usize, start: f32) -> f32 {
    let ratio = speed / top_kmh * KMH_PER_MS;
    let clamped = if -ratio >= 0.0 { 0.0 } else { ratio };
    let mut clamped = if 1.0 - clamped >= 0.0 { clamped } else { 1.0 };
    if index == 2 && clamped > COMBO_CAP {
        clamped = COMBO_CAP;
    }
    start * (1.0 - clamped)
}

/// The level id the held gain comes from: 1, or 5 on a manual, 6 with the board held, 7 on a
/// handplant (the first that holds).
pub(crate) fn gain_id(audio: &AudioState, handplant_760: bool) -> u32 {
    if audio.balance_340 {
        5
    } else if audio.board_held_308 {
        6
    } else if handplant_760 {
        7
    } else {
        1
    }
}

/// `sub_824CE488`'s local bus values: PI20 `{frequency, q, gain}` and HS20 `{corner, gain}`.
pub(crate) fn bus_filters(
    vault: &WheelsVault,
    time_scale: f32,
    camera_680: f32,
) -> ([f32; 3], [f32; 2]) {
    let t = if time_scale > 1.0 { 1.0 } else { time_scale };
    let far = vault.peak_far as f32;
    let near = vault.peak_near as f32;
    let f12 = t * far;
    // fmsubs f9 = t·near − f12, then fmadds f1 = f9·c + f12.
    let f9 = -nmsub_single(f64::from(t), f64::from(near), f64::from(f12)) as f32;
    let frequency = fmadd_single(f64::from(f9), f64::from(camera_680), f64::from(f12)) as f32;
    let corner = t * vault.shelf_corner as f32;
    let span = vault.shelf_near - vault.shelf_far;
    let shelf = fmadd_single(
        f64::from(span),
        f64::from(camera_680),
        f64::from(vault.shelf_far),
    ) as f32;
    ([frequency, vault.peak_q, vault.peak_gain], [corner, shelf])
}

/// What `sub_824CDD28` does for one trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Nothing,
    /// A rising edge: start the voice.
    Start,
    /// The voice keeps playing: rewrite gain and pitch.
    Hold,
    /// The voice finished (`[graph+71] == 2`) while the trigger holds: stop it.
    Finished,
    /// The trigger dropped: stop the voice.
    Release,
}

/// `sub_824CDD28`'s branch. `flag` is the latch at `+60+i` and is updated.
pub(crate) fn step(flag: &mut bool, trigger: bool, voice: bool, finished: bool) -> Step {
    if !*flag {
        if trigger {
            *flag = true;
            return Step::Start;
        }
        return Step::Nothing;
    }
    if trigger {
        return match (voice, finished) {
            (false, _) => Step::Nothing,
            (true, true) => Step::Finished,
            (true, false) => Step::Hold,
        };
    }
    *flag = false;
    if voice { Step::Release } else { Step::Nothing }
}

pub(crate) struct Wheels {
    vault: WheelsVault,
    local: bool,
    /// Jump / Man: the `.snr` addresses (the resources) and `.sek` buffers.
    streams: [u32; 2],
    seeks: [u32; 2],
    bus: Bus,
    /// `+96`: the next spin uses the Jump set.
    jump_next: bool,
    flags: [bool; 3],
    voices: [Option<Voice>; 3],
    camera_680: f32,
    handplant_760: bool,
}

impl Wheels {
    /// `sub_824CD6F8` (+ slot 7): place the `.snr`/`.sek` members, build the bus.
    pub(crate) fn new(
        tick: &mut Tick,
        vault: WheelsVault,
        members: &[GrainMember],
        local: bool,
    ) -> Result<Self, String> {
        let find = |name: &str| {
            members
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("wheels.big member {name} is not loaded"))
        };
        let mut g = tick.runtime.grains();
        let mut streams = [0; 2];
        let mut seeks = [0; 2];
        for k in 0..2 {
            let snr = find(&vault.snr[k])?;
            streams[k] = g
                .load_resident(
                    &snr.name,
                    &snr.bytes,
                    snr.samples.clone(),
                    snr.channels,
                    snr.rate,
                )
                .map_err(|e| e.to_string())?;
            seeks[k] = g
                .place(&find(&vault.sek[k])?.bytes)
                .map_err(|e| e.to_string())?;
        }
        g.reserve_voices(3);
        let bus = g.stream_bus(vault.eq_chain).map_err(|e| e.to_string())?;
        Ok(Self {
            vault,
            local,
            streams,
            seeks,
            bus,
            jump_next: true,
            flags: [false; 3],
            voices: [None; 3],
            camera_680: 0.0,
            handplant_760: false,
        })
    }

    /// Audio state `+680` (see the module note).
    pub(crate) fn set_camera_680(&mut self, value: f32) {
        self.camera_680 = value;
    }

    /// Audio state `+760` (see the module note).
    pub(crate) fn set_handplant_760(&mut self, value: bool) {
        self.handplant_760 = value;
    }

    /// `sub_824CDD28` for trigger `index`.
    fn spin(&mut self, tick: &mut Tick, index: usize, trigger: bool) -> Result<(), String> {
        let voice = self.voices[index];
        let finished = match voice {
            Some(v) => tick
                .runtime
                .grains()
                .stream_finished(v)
                .map_err(|e| e.to_string())?,
            None => false,
        };
        let c = tick.controls;
        match step(&mut self.flags[index], trigger, voice.is_some(), finished) {
            Step::Nothing => {}
            Step::Start => {
                let set = usize::from(!self.jump_next);
                self.jump_next = !self.jump_next;
                let start = start_seconds(
                    tick.audio.ground_speed_208,
                    self.vault.top_kmh,
                    index,
                    self.vault.start[set],
                );
                if voice.is_none() {
                    let v = tick
                        .runtime
                        .grains()
                        .stream_start(self.bus, self.streams[set], self.seeks[set], start)
                        .map_err(|e| e.to_string())?;
                    self.voices[index] = Some(v);
                }
                if self.local {
                    let level = c.level(4) as i32 as f32 * INV_32767;
                    tick.runtime
                        .grains()
                        .stream_post(self.bus, 3, 0, level)
                        .map_err(|e| e.to_string())?;
                }
            }
            Step::Hold => {
                let gain =
                    c.level(gain_id(tick.audio, self.handplant_760)) as i32 as f32 * INV_32767;
                let pitch = c.pitch(2) as f32 * INV_4096;
                let v = voice.expect("held voice");
                tick.runtime
                    .grains()
                    .stream_set(v, gain, pitch)
                    .map_err(|e| e.to_string())?;
            }
            Step::Finished | Step::Release => {
                let v = self.voices[index].take().expect("voice to stop");
                tick.runtime
                    .grains()
                    .stream_stop(v)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// `sub_824CE488`.
    fn bus_update(&mut self, tick: &mut Tick) -> Result<(), String> {
        if self.voices.iter().all(Option::is_none) {
            return Ok(());
        }
        let c = tick.controls;
        let mut g = tick.runtime.grains();
        let e = |e: skate_audio_core::Error| e.to_string();
        let pan = c.raw(0) as i32 as f32 * PAN_SCALE;
        g.stream_post(self.bus, 5, 0, pan).map_err(e)?;
        if self.local {
            let (peak, shelf) =
                bus_filters(&self.vault, tick.audio.time_scale_220, self.camera_680);
            for (id, value) in peak.into_iter().enumerate() {
                g.stream_post(self.bus, 1, id as u32, value).map_err(e)?;
            }
            for (id, value) in shelf.into_iter().enumerate() {
                g.stream_post(self.bus, 2, id as u32, value).map_err(e)?;
            }
            let level = c.level(4) as i32 as f32 * INV_32767;
            g.stream_post(self.bus, 3, 0, level).map_err(e)?;
        }
        let level = c.level(3) as i32 as f32 * INV_32767;
        g.stream_post(self.bus, 4, 0, level).map_err(e)
    }
}

impl Component for Wheels {
    /// Slot 9 is `blr`.
    fn process(&mut self, _tick: &mut Tick) -> Result<(), String> {
        Ok(())
    }

    /// Slot 10, `sub_824CDC70`.
    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        // +680 is written by the bridge (`sub_824B2088`) before the components run.
        self.camera_680 = tick.audio.listener_facing_680;
        let triggers = triggers(tick.audio, self.local, self.handplant_760);
        for (index, trigger) in triggers.into_iter().enumerate() {
            self.spin(tick, index, trigger)?;
        }
        self.bus_update(tick)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::Controls;
    use super::*;

    pub(crate) struct Fixed(pub &'static [(u32, u32, u32)]);
    impl Controls for Fixed {
        fn raw(&self, id: u32) -> u32 {
            self.0
                .iter()
                .find(|r| r.0 == 52 && r.1 == id)
                .map_or(0, |r| r.2)
        }
        fn pitch(&self, id: u32) -> i32 {
            self.0
                .iter()
                .find(|r| r.0 == 56 && r.1 == id)
                .map_or(0, |r| r.2 as i32)
        }
        fn level(&self, id: u32) -> u32 {
            self.0
                .iter()
                .find(|r| r.0 == 60 && r.1 == id)
                .map_or(0, |r| r.2)
        }
    }

    /// The vault's values (`skater-collections.json`, holder instance `03B710C80E1AC13E`).
    pub(crate) fn vault() -> WheelsVault {
        WheelsVault {
            snr: [
                "Whls_spins_Jump_1.snr".into(),
                "Whls_spins_Man_1.snr".into(),
            ],
            sek: [
                "Whls_spins_Jump_1.sek".into(),
                "Whls_spins_Man_1.sek".into(),
            ],
            start: [14.0, 14.0],
            top_kmh: 50.0,
            eq_chain: 6,
            peak_far: 4500,
            peak_near: 800,
            peak_q: f32::from_bits(0x3DCC_CCCD),
            peak_gain: 20.0,
            shelf_corner: 4000,
            shelf_far: 3.0,
            shelf_near: 0.5,
        }
    }

    #[test]
    fn start_offsets_follow_the_speed_and_cap_the_combo() {
        // At rest the spin starts 14 s in (its last 0.81 s); at 50 km/h from the top.
        assert_eq!(start_seconds(0.0, 50.0, 0, 14.0), 14.0);
        assert_eq!(start_seconds(50.0 / 3.6, 50.0, 1, 14.0), 0.0);
        // 5 m/s: 5 / 50 × 3.6 = 0.36 (single precision), 14 × 0.64.
        let ratio = 5.0f32 / 50.0 * 3.6;
        assert_eq!(start_seconds(5.0, 50.0, 0, 14.0), 14.0 * (1.0 - ratio));
        // Trigger 2 caps the ratio at 0.26.
        assert_eq!(start_seconds(5.0, 50.0, 2, 14.0), 14.0 * (1.0 - COMBO_CAP));
        assert_eq!(start_seconds(-3.0, 50.0, 0, 14.0), 14.0);
    }

    #[test]
    fn the_latch_starts_holds_and_releases() {
        let mut flag = false;
        assert_eq!(step(&mut flag, false, false, false), Step::Nothing);
        assert_eq!(step(&mut flag, true, false, false), Step::Start);
        assert!(flag);
        assert_eq!(step(&mut flag, true, true, false), Step::Hold);
        assert_eq!(step(&mut flag, true, true, true), Step::Finished);
        // A finished voice is gone; the latch stays until the trigger drops.
        assert_eq!(step(&mut flag, true, false, false), Step::Nothing);
        assert_eq!(step(&mut flag, false, false, false), Step::Nothing);
        assert!(!flag);
        assert_eq!(step(&mut flag, true, false, false), Step::Start);
        assert_eq!(step(&mut flag, false, true, false), Step::Release);
    }

    #[test]
    fn triggers_and_gain_levels() {
        let mut s = AudioState::default();
        assert_eq!(triggers(&s, true, false), [false; 3]);
        s.in_known_air_332 = true;
        s.walking_716 = true;
        s.board_held_308 = true;
        assert_eq!(triggers(&s, true, false), [true, false, true]);
        assert_eq!(triggers(&s, false, false), [true, false, false]);
        assert_eq!(gain_id(&s, false), 6);
        s.balance_340 = true;
        assert_eq!(gain_id(&s, false), 5);
        let s = AudioState::default();
        assert_eq!(gain_id(&s, true), 7);
        assert_eq!(gain_id(&s, false), 1);
        assert_eq!(triggers(&s, true, true), [false, false, true]);
    }

    #[test]
    fn bus_filters_interpolate_on_the_camera_factor() {
        let v = vault();
        assert_eq!(
            bus_filters(&v, 1.0, 0.0),
            ([4500.0, v.peak_q, 20.0], [4000.0, 3.0])
        );
        assert_eq!(
            bus_filters(&v, 1.0, 1.0),
            ([800.0, v.peak_q, 20.0], [4000.0, 0.5])
        );
        // The time scale is clamped to 1 and scales both corners.
        assert_eq!(bus_filters(&v, 2.0, 0.0).0[0], 4500.0);
        assert_eq!(
            bus_filters(&v, 0.5, 0.0),
            ([2250.0, v.peak_q, 20.0], [2000.0, 3.0])
        );
        // The capture's usual +680: fmsubs/fmadds, one rounding each.
        let c = 0.820_902_5_f32;
        let (peak, shelf) = bus_filters(&v, 1.0, c);
        assert_eq!(peak[0], (-3700.0f64 * f64::from(c) + 4500.0) as f32);
        assert_eq!(shelf[1], (-2.5f64 * f64::from(c) + 3.0) as f32);
    }

    /// Replays the retail capture through the triggers and the latch: every frame where the Wheels
    /// controller (`4A26A8C0`) was read, the starts (`60:4` at `0x824CDE78`) and holds (`60:1` at
    /// `0x824CDFB4`, then `60:5`/`60:6`/`60:7` for the gain level) must be the ones the port
    /// takes on the state one frame earlier. A voice that finished while its trigger held stops
    /// being held in retail; those lifetimes must show the retail holds as a prefix of ours.
    #[test]
    #[ignore = "needs the retail capture in .local"]
    fn wheels_replay_the_retail_capture() {
        use super::super::contacts::capture;
        use std::collections::{BTreeMap, HashMap};
        use std::io::BufRead;
        let Some(root) = capture::root() else { return };
        let states: BTreeMap<u32, AudioState> = capture::states(&root)
            .iter()
            .map(|(f, w)| (*f, AudioState::from_capture(w)))
            .collect();
        let mut retail: HashMap<u32, Vec<(bool, u32)>> = HashMap::new();
        let file = std::fs::File::open(root.join("vf.tsv")).unwrap();
        for line in std::io::BufReader::new(file).lines() {
            let line = line.unwrap();
            if !line.contains("4A26A8C0") {
                continue;
            }
            let mut columns = line.split('\t');
            let frame: u32 = columns.next().unwrap().parse().unwrap();
            let call: Vec<&str> = columns.nth(1).unwrap().split(' ').collect();
            if call[2] != "4A26A8C0" {
                continue;
            }
            let events = retail.entry(frame).or_default();
            match call[5] {
                "824CDE78" => events.push((true, 0)),
                "824CDFB4" => events.push((false, 1)),
                "824CE000" | "824CE034" => {
                    events.last_mut().unwrap().1 = call[3].parse().unwrap();
                }
                lr if lr.starts_with("824CE0") && call[0] == "60" && call[3] == "7" => {
                    events.last_mut().unwrap().1 = 7;
                }
                _ => {}
            }
        }
        let mut flags = [false; 3];
        let mut voices = [false; 3];
        let mut lifetimes: Vec<(u32, Vec<u32>)> = Vec::new();
        let mut current = [usize::MAX; 3];
        let mut ours: HashMap<u32, Vec<(bool, u32)>> = HashMap::new();
        for (&frame, _) in states.range(states.keys().next().unwrap() + 1..) {
            let Some(audio) = states.get(&(frame - 1)) else {
                continue;
            };
            let mut events = Vec::new();
            for (index, trigger) in triggers(audio, true, false).into_iter().enumerate() {
                match step(&mut flags[index], trigger, voices[index], false) {
                    Step::Start => {
                        voices[index] = true;
                        events.push((true, 0));
                        current[index] = lifetimes.len();
                        lifetimes.push((frame, Vec::new()));
                    }
                    Step::Hold => {
                        events.push((false, gain_id(audio, false)));
                        lifetimes[current[index]].1.push(frame);
                    }
                    Step::Release | Step::Finished => voices[index] = false,
                    Step::Nothing => {}
                }
            }
            ours.insert(frame, events);
        }
        let identical = retail
            .iter()
            .filter(|(f, e)| ours.get(f).is_some_and(|o| o == *e))
            .count();
        let held: std::collections::HashSet<u32> = retail
            .iter()
            .filter(|(_, e)| e.iter().any(|x| !x.0))
            .map(|(f, _)| *f)
            .collect();
        let mut finished_early = Vec::new();
        let mut prefix = 0;
        for (start, holds) in &lifetimes {
            let got: Vec<u32> = holds.iter().copied().filter(|h| held.contains(h)).collect();
            if got[..] == holds[..got.len()] {
                prefix += 1;
                if got.len() < holds.len() {
                    finished_early.push((*start, got.len(), holds.len()));
                }
            }
        }
        let starts = retail.values().flatten().filter(|e| e.0).count();
        println!(
            "frames with Wheels reads {}, identical {identical}; starts {starts}; lifetimes {}, retail holds a prefix of ours in {prefix}; finished while held (start, retail holds, ours) {finished_early:?}",
            retail.len(),
            lifetimes.len()
        );
        assert_eq!(identical, retail.len());
        assert_eq!(prefix, lifetimes.len());
    }

    /// Plays a spin headlessly on the authored runtime: the retail seek must land on the start
    /// frame, the voice must sound while held and fall silent after the trigger drops.
    #[test]
    #[ignore = "needs the installed assets"]
    fn wheels_play_headless() {
        use skate_audio_core::authored::AuthoredRuntime;
        use skate_data::audio::catalog::PlayerAudioCatalog;
        let assets = std::path::PathBuf::from(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
        );
        let cache = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Skate3RustEngine/audio-pcm-cache"));
        let catalog = PlayerAudioCatalog::from_assets(&assets, cache.as_deref()).unwrap();
        let mut runtime =
            AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks).unwrap();
        let (vault, members) = load(&assets, cache.as_deref()).unwrap();
        let controls = Fixed(&[
            (60, 1, 24_000),
            (56, 2, 4096),
            (60, 3, 0),
            (60, 4, 0),
            (52, 0, 0),
        ]);
        let mut audio = AudioState::default();
        audio.time_scale_220 = 1.0;
        audio.ground_speed_208 = 5.0;
        let mut wheels = {
            let mut tick = Tick {
                runtime: &mut runtime,
                audio: &audio,
                controls: &controls,
                dt: 1.0 / 60.0,
                tick: 0,
            };
            Wheels::new(&mut tick, vault, &members, true).unwrap()
        };
        let mut rms = Vec::new();
        let mut blocks = 0.0f64;
        for frame in 0..150u64 {
            audio.in_known_air_332 = (10..100).contains(&frame);
            {
                let mut tick = Tick {
                    runtime: &mut runtime,
                    audio: &audio,
                    controls: &controls,
                    dt: 1.0 / 60.0,
                    tick: frame,
                };
                wheels.update(&mut tick).unwrap();
            }
            // 48000 / 256 blocks per second, 60 frames per second.
            blocks += 48_000.0 / 256.0 / 60.0;
            let (mut sum, mut n) = (0.0f64, 0usize);
            while blocks >= 1.0 {
                blocks -= 1.0;
                let pcm = runtime.pump_once().unwrap();
                sum += pcm
                    .iter()
                    .map(|s| f64::from(*s) * f64::from(*s))
                    .sum::<f64>();
                n += pcm.len();
            }
            rms.push((sum / n.max(1) as f64).sqrt());
        }
        let starts = runtime.take_grain_starts();
        println!("starts {starts:?}");
        assert_eq!(starts.len(), 1);
        let expected = start_seconds(5.0, 50.0, 0, 14.0);
        assert_eq!(starts[0].seconds, f64::from(expected));
        assert_eq!(starts[0].frame, (48_000.0 * f64::from(expected)) as u32);
        assert!(starts[0].seek.is_some());
        let loud = rms[14..95].iter().filter(|r| **r > 1e-4).count();
        let after = rms[110..].iter().fold(0.0f64, |m, r| m.max(*r));
        println!("frames with sound while held {loud}/81, max rms after release {after:e}");
        assert!(loud > 70, "the held spin is silent");
        assert!(after < 1e-6, "the spin keeps sounding after release");
    }
}
